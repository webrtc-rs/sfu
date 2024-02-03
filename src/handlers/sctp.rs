use crate::messages::{
    DTLSMessageEvent, DataChannelMessage, DataChannelMessageParams, DataChannelMessageType,
    MessageEvent, TaggedMessageEvent,
};
use crate::server::states::ServerStates;
use bytes::BytesMut;
use log::{debug, error};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TransportContext;
use sctp::{
    AssociationEvent, AssociationHandle, DatagramEvent, EndpointEvent, Event, Payload,
    PayloadProtocolIdentifier, ReliabilityType, StreamEvent, Transmit,
};
use shared::error::{Error, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

struct SctpInbound {
    local_addr: SocketAddr,
    server_states: Rc<RefCell<ServerStates>>,
    sctp_endpoint: Rc<RefCell<sctp::Endpoint>>,

    transmits: VecDeque<Transmit>,
    internal_buffer: Vec<u8>,
}
struct SctpOutbound {
    local_addr: SocketAddr,
    server_states: Rc<RefCell<ServerStates>>,
    sctp_endpoint: Rc<RefCell<sctp::Endpoint>>,

    transmits: VecDeque<Transmit>,
}
pub struct SctpHandler {
    sctp_inbound: SctpInbound,
    sctp_outbound: SctpOutbound,
}

impl SctpHandler {
    pub fn new(
        local_addr: SocketAddr,
        server_states: Rc<RefCell<ServerStates>>,
        sctp_endpoint_config: sctp::EndpointConfig,
    ) -> Self {
        let sctp_server_config =
            Arc::clone(&server_states.borrow().server_config().sctp_server_config);
        let sctp_endpoint = Rc::new(RefCell::new(sctp::Endpoint::new(
            sctp_endpoint_config,
            Some(Arc::clone(&sctp_server_config)),
        )));

        SctpHandler {
            sctp_inbound: SctpInbound {
                local_addr,
                server_states: Rc::clone(&server_states),
                sctp_endpoint: Rc::clone(&sctp_endpoint),

                transmits: VecDeque::new(),
                internal_buffer: vec![
                    0u8;
                    sctp_server_config.transport.max_message_size() as usize
                ],
            },
            sctp_outbound: SctpOutbound {
                local_addr,
                server_states,
                sctp_endpoint,

                transmits: VecDeque::new(),
            },
        }
    }
}

impl InboundHandler for SctpInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if let MessageEvent::Dtls(DTLSMessageEvent::Raw(dtls_message)) = msg.message {
            debug!("recv sctp RAW {:?}", msg.transport.peer_addr);
            let try_read = || -> Result<Vec<DataChannelMessage>> {
                let handle_result = {
                    let mut sctp_endpoint = self.sctp_endpoint.borrow_mut();
                    sctp_endpoint.handle(
                        msg.now,
                        msg.transport.peer_addr,
                        Some(msg.transport.local_addr.ip()),
                        msg.transport.ecn,
                        dtls_message.freeze(), //TODO: switch API Bytes to BytesMut
                    )
                };

                let mut server_states = self.server_states.borrow_mut();

                let mut sctp_events: HashMap<AssociationHandle, VecDeque<AssociationEvent>> =
                    HashMap::new();
                if let Some((ch, event)) = handle_result {
                    match event {
                        DatagramEvent::NewAssociation(conn) => {
                            let sctp_associations = server_states.get_mut_sctp_associations();
                            sctp_associations.insert(ch, conn);
                        }
                        DatagramEvent::AssociationEvent(event) => {
                            sctp_events.entry(ch).or_default().push_back(event);
                        }
                    }
                }

                let mut messages = vec![];
                {
                    let mut endpoint_events: Vec<(AssociationHandle, EndpointEvent)> = vec![];

                    let sctp_associations = server_states.get_mut_sctp_associations();
                    for (ch, conn) in sctp_associations.iter_mut() {
                        for (event_ch, conn_events) in sctp_events.iter_mut() {
                            if ch == event_ch {
                                for event in conn_events.drain(..) {
                                    debug!("association_handle {} handle_event", ch.0);
                                    conn.handle_event(event);
                                }
                            }
                        }

                        while let Some(event) = conn.poll() {
                            if let Event::Stream(StreamEvent::Readable { id }) = event {
                                let mut stream = conn.stream(id)?;
                                while let Some(chunks) = stream.read_sctp()? {
                                    let n = chunks.read(&mut self.internal_buffer)?;
                                    messages.push(DataChannelMessage {
                                        association_handle: ch.0,
                                        stream_id: id,
                                        data_message_type: to_data_message_type(chunks.ppi),
                                        params: DataChannelMessageParams::Inbound {
                                            seq_num: chunks.ssn,
                                        },
                                        payload: BytesMut::from(&self.internal_buffer[0..n]),
                                    });
                                }
                            }
                        }

                        while let Some(event) = conn.poll_endpoint_event() {
                            endpoint_events.push((*ch, event));
                        }

                        while let Some(x) = conn.poll_transmit(msg.now) {
                            self.transmits.extend(split_transmit(x));
                        }
                    }

                    let mut sctp_endpoint = self.sctp_endpoint.borrow_mut();
                    for (ch, event) in endpoint_events {
                        sctp_endpoint.handle_event(ch, event); // handle drain event
                        sctp_associations.remove(&ch);
                    }
                }

                Ok(messages)
            };
            match try_read() {
                Ok(messages) => {
                    for message in messages {
                        debug!(
                            "recv sctp data channel message {:?}",
                            msg.transport.peer_addr
                        );
                        ctx.fire_read(TaggedMessageEvent {
                            now: msg.now,
                            transport: msg.transport,
                            message: MessageEvent::Dtls(DTLSMessageEvent::Sctp(message)),
                        })
                    }
                }
                Err(err) => {
                    error!("try_read with error {}", err);
                    ctx.fire_read_exception(Box::new(err))
                }
            };
            handle_outgoing(ctx, &mut self.transmits, msg.transport.local_addr);
        } else {
            // Bypass
            debug!("bypass sctp read {:?}", msg.transport.peer_addr);
            ctx.fire_read(msg);
        }
    }

    fn handle_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, now: Instant) {
        let mut try_timeout = || -> Result<()> {
            let mut endpoint_events: Vec<(AssociationHandle, EndpointEvent)> = vec![];
            let mut server_states = self.server_states.borrow_mut();

            let sctp_associations = server_states.get_mut_sctp_associations();
            for (ch, conn) in sctp_associations.iter_mut() {
                conn.handle_timeout(now);

                while let Some(event) = conn.poll_endpoint_event() {
                    endpoint_events.push((*ch, event));
                }

                while let Some(x) = conn.poll_transmit(now) {
                    self.transmits.extend(split_transmit(x));
                }
            }

            let mut sctp_endpoint = self.sctp_endpoint.borrow_mut();
            for (ch, event) in endpoint_events {
                sctp_endpoint.handle_event(ch, event); // handle drain event
                sctp_associations.remove(&ch);
            }

            Ok(())
        };
        if let Err(err) = try_timeout() {
            error!("try_timeout with error {}", err);
            ctx.fire_read_exception(Box::new(err));
        }
        handle_outgoing(ctx, &mut self.transmits, self.local_addr);

        ctx.fire_handle_timeout(now);
    }

    fn poll_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, eto: &mut Instant) {
        {
            let mut server_states = self.server_states.borrow_mut();
            let sctp_associations = server_states.get_mut_sctp_associations();
            for (_, conn) in sctp_associations.iter_mut() {
                if let Some(timeout) = conn.poll_timeout() {
                    if timeout < *eto {
                        *eto = timeout;
                    }
                }
            }
        }
        ctx.fire_poll_timeout(eto);
    }
}

impl OutboundHandler for SctpOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        if let MessageEvent::Dtls(DTLSMessageEvent::Sctp(message)) = msg.message {
            debug!(
                "send sctp data channel message {:?}",
                msg.transport.peer_addr
            );
            let mut try_write = || -> Result<()> {
                let mut server_states = self.server_states.borrow_mut();
                let max_message_size = {
                    server_states
                        .server_config()
                        .sctp_server_config
                        .transport
                        .max_message_size() as usize
                };
                if message.payload.len() > max_message_size {
                    return Err(Error::ErrOutboundPacketTooLarge);
                }

                let sctp_associations = server_states.get_mut_sctp_associations();
                if let Some(conn) =
                    sctp_associations.get_mut(&AssociationHandle(message.association_handle))
                {
                    if let DataChannelMessageParams::Outbound {
                        ordered,
                        reliable,
                        max_rtx_count,
                        max_rtx_millis,
                    } = message.params
                    {
                        let mut stream = conn.stream(message.stream_id)?;
                        let (rel_type, rel_val) = if !reliable {
                            if max_rtx_millis == 0 {
                                (ReliabilityType::Rexmit, max_rtx_count)
                            } else {
                                (ReliabilityType::Timed, max_rtx_millis)
                            }
                        } else {
                            (ReliabilityType::Reliable, 0)
                        };

                        stream.set_reliability_params(!ordered, rel_type, rel_val)?;
                        stream.write_with_ppi(
                            &message.payload,
                            to_ppid(message.data_message_type, message.payload.len()),
                        )?;

                        while let Some(x) = conn.poll_transmit(msg.now) {
                            self.transmits.extend(split_transmit(x));
                        }
                    }
                } else {
                    return Err(Error::ErrAssociationNotExisted);
                }
                Ok(())
            };
            if let Err(err) = try_write() {
                error!("try_write with error {}", err);
                ctx.fire_write_exception(Box::new(err));
            }
            handle_outgoing(ctx, &mut self.transmits, msg.transport.local_addr);
        } else {
            // Bypass
            debug!("Bypass sctp write {:?}", msg.transport.peer_addr);
            ctx.fire_write(msg);
        }
    }

    fn close(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>) {
        {
            let mut server_states = self.server_states.borrow_mut();
            let sctp_associations = server_states.get_mut_sctp_associations();
            for (_, conn) in sctp_associations.iter_mut() {
                let _ = conn.close();
            }
        }
        handle_outgoing(ctx, &mut self.transmits, self.local_addr);

        ctx.fire_close();
    }
}

impl Handler for SctpHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "SctpHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.sctp_inbound), Box::new(self.sctp_outbound))
    }
}

fn handle_outgoing(
    ctx: &OutboundContext<TaggedMessageEvent, TaggedMessageEvent>,
    transmits: &mut VecDeque<Transmit>,
    local_addr: SocketAddr,
) {
    while let Some(transmit) = transmits.pop_front() {
        if let Payload::RawEncode(raw_data) = transmit.payload {
            for raw in raw_data {
                ctx.fire_write(TaggedMessageEvent {
                    now: transmit.now,
                    transport: TransportContext {
                        local_addr,
                        peer_addr: transmit.remote,
                        ecn: transmit.ecn,
                    },
                    message: MessageEvent::Dtls(DTLSMessageEvent::Raw(BytesMut::from(&raw[..]))),
                });
            }
        }
    }
}

fn split_transmit(transmit: Transmit) -> Vec<Transmit> {
    let mut transmits = Vec::new();
    if let Payload::RawEncode(contents) = transmit.payload {
        for content in contents {
            transmits.push(Transmit {
                now: transmit.now,
                remote: transmit.remote,
                payload: Payload::RawEncode(vec![content]),
                ecn: transmit.ecn,
                local_ip: transmit.local_ip,
            });
        }
    }

    transmits
}

fn to_data_message_type(ppid: PayloadProtocolIdentifier) -> DataChannelMessageType {
    match ppid {
        PayloadProtocolIdentifier::Dcep => DataChannelMessageType::Control,
        PayloadProtocolIdentifier::String | PayloadProtocolIdentifier::StringEmpty => {
            DataChannelMessageType::Text
        }
        PayloadProtocolIdentifier::Binary | PayloadProtocolIdentifier::BinaryEmpty => {
            DataChannelMessageType::Binary
        }
        _ => DataChannelMessageType::None,
    }
}

fn to_ppid(message_type: DataChannelMessageType, length: usize) -> PayloadProtocolIdentifier {
    match message_type {
        DataChannelMessageType::Text => {
            if length > 0 {
                PayloadProtocolIdentifier::String
            } else {
                PayloadProtocolIdentifier::StringEmpty
            }
        }
        DataChannelMessageType::Binary => {
            if length > 0 {
                PayloadProtocolIdentifier::Binary
            } else {
                PayloadProtocolIdentifier::BinaryEmpty
            }
        }
        _ =>
        // case DataMessageType::CONTROL: // TODO: remove DataMessageType::NONE once DcSctp is landed
        {
            PayloadProtocolIdentifier::Dcep
        }
    }
}
