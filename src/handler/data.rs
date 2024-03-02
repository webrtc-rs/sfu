use crate::messages::{
    ApplicationMessage, DTLSMessageEvent, DataChannelEvent, DataChannelMessage,
    DataChannelMessageParams, DataChannelMessageType, MessageEvent, TaggedMessageEvent,
};
use datachannel::message::{message_channel_ack::*, message_channel_open::*, message_type::*, *};
use log::{debug, error, warn};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use shared::error::Result;
use shared::marshal::*;

#[derive(Default)]
struct DataChannelInbound;
#[derive(Default)]
struct DataChannelOutbound;

/// DataChannelHandler implements DataChannel Protocol handling
#[derive(Default)]
pub struct DataChannelHandler {
    data_channel_inbound: DataChannelInbound,
    data_channel_outbound: DataChannelOutbound,
}

impl DataChannelHandler {
    pub fn new() -> Self {
        DataChannelHandler::default()
    }
}

impl InboundHandler for DataChannelInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if let MessageEvent::Dtls(DTLSMessageEvent::Sctp(message)) = msg.message {
            debug!(
                "recv SCTP DataChannelMessage from {:?}",
                msg.transport.peer_addr
            );
            let try_read =
                || -> Result<(Option<ApplicationMessage>, Option<DataChannelMessage>)> {
                    if message.data_message_type == DataChannelMessageType::Control {
                        let mut buf = &message.payload[..];
                        if MessageType::unmarshal(&mut buf)? == MessageType::DataChannelOpen {
                            debug!("DataChannelOpen for association_handle {} and stream_id {} and data_message_type {:?}",
                            message.association_handle,
                            message.stream_id,
                            message.data_message_type);

                            let _open = DataChannelOpen::unmarshal(&mut buf)?;

                            let payload = Message::DataChannelAck(DataChannelAck {}).marshal()?;
                            Ok((
                                Some(ApplicationMessage {
                                    association_handle: message.association_handle,
                                    stream_id: message.stream_id,
                                    data_channel_event: DataChannelEvent::Open,
                                }),
                                Some(DataChannelMessage {
                                    association_handle: message.association_handle,
                                    stream_id: message.stream_id,
                                    data_message_type: DataChannelMessageType::Control,
                                    params: DataChannelMessageParams::Outbound {
                                        ordered: true,
                                        reliable: true,
                                        max_rtx_count: 0,
                                        max_rtx_millis: 0,
                                    },
                                    payload,
                                }),
                            ))
                        } else {
                            Ok((None, None))
                        }
                    } else {
                        Ok((
                            Some(ApplicationMessage {
                                association_handle: message.association_handle,
                                stream_id: message.stream_id,
                                data_channel_event: DataChannelEvent::Message(message.payload),
                            }),
                            None,
                        ))
                    }
                };

            match try_read() {
                Ok((inbound_message, outbound_message)) => {
                    // first outbound message
                    if let Some(data_channel_message) = outbound_message {
                        debug!("send DataChannelAck message {:?}", msg.transport.peer_addr);
                        ctx.fire_write(TaggedMessageEvent {
                            now: msg.now,
                            transport: msg.transport,
                            message: MessageEvent::Dtls(DTLSMessageEvent::Sctp(
                                data_channel_message,
                            )),
                        });
                    }

                    // then inbound message
                    if let Some(application_message) = inbound_message {
                        debug!("recv application message {:?}", msg.transport.peer_addr);
                        ctx.fire_read(TaggedMessageEvent {
                            now: msg.now,
                            transport: msg.transport,
                            message: MessageEvent::Dtls(DTLSMessageEvent::DataChannel(
                                application_message,
                            )),
                        })
                    }
                }
                Err(err) => {
                    error!("try_read with error {}", err);
                    ctx.fire_read_exception(Box::new(err))
                }
            };
        } else {
            // Bypass
            debug!("bypass DataChannel read {:?}", msg.transport.peer_addr);
            ctx.fire_read(msg);
        }
    }
}

impl OutboundHandler for DataChannelOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        if let MessageEvent::Dtls(DTLSMessageEvent::DataChannel(message)) = msg.message {
            debug!("send application message {:?}", msg.transport.peer_addr);

            if let DataChannelEvent::Message(payload) = message.data_channel_event {
                ctx.fire_write(TaggedMessageEvent {
                    now: msg.now,
                    transport: msg.transport,
                    message: MessageEvent::Dtls(DTLSMessageEvent::Sctp(DataChannelMessage {
                        association_handle: message.association_handle,
                        stream_id: message.stream_id,
                        data_message_type: DataChannelMessageType::Text,
                        params: DataChannelMessageParams::Outbound {
                            ordered: true,
                            reliable: true,
                            max_rtx_count: 0,
                            max_rtx_millis: 0,
                        },
                        payload,
                    })),
                });
            } else {
                warn!(
                    "drop unsupported DATACHANNEL message to {}",
                    msg.transport.peer_addr
                );
            }
        } else {
            // Bypass
            debug!("bypass DataChannel write {:?}", msg.transport.peer_addr);
            ctx.fire_write(msg);
        }
    }
}

impl Handler for DataChannelHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "DataChannelHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (
            Box::new(self.data_channel_inbound),
            Box::new(self.data_channel_outbound),
        )
    }
}
