use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use crate::server::states::ServerStates;
use bytes::BytesMut;
use log::{debug, error};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use shared::{
    error::{Error, Result},
    marshal::{Marshal, Unmarshal},
    util::is_rtcp,
};
use std::cell::RefCell;
use std::rc::Rc;

struct SrtpInbound {
    server_states: Rc<RefCell<ServerStates>>, // for remote_srtp_context
}
struct SrtpOutbound {
    server_states: Rc<RefCell<ServerStates>>, // for local_srtp_context
}

/// SrtpHandler implements SRTP/RTP/RTCP Protocols handling
pub struct SrtpHandler {
    srtp_inbound: SrtpInbound,
    srtp_outbound: SrtpOutbound,
}

impl SrtpHandler {
    pub fn new(server_states: Rc<RefCell<ServerStates>>) -> Self {
        SrtpHandler {
            srtp_inbound: SrtpInbound {
                server_states: Rc::clone(&server_states),
            },
            srtp_outbound: SrtpOutbound { server_states },
        }
    }
}

impl InboundHandler for SrtpInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, mut msg: Self::Rin) {
        if let MessageEvent::Rtp(RTPMessageEvent::Raw(message)) = msg.message {
            debug!("srtp read {:?}", msg.transport.peer_addr);
            let try_read = || -> Result<MessageEvent> {
                let four_tuple = (&msg.transport).into();
                let mut server_states = self.server_states.borrow_mut();
                let transport = server_states.get_mut_transport(&four_tuple)?;

                if is_rtcp(&message) {
                    let mut remote_context = transport.remote_srtp_context();
                    if let Some(context) = remote_context.as_mut() {
                        let mut decrypted = context.decrypt_rtcp(&message)?;
                        let rtcp_packets = rtcp::packet::unmarshal(&mut decrypted)?;
                        if rtcp_packets.is_empty() {
                            return Err(Error::Other("empty rtcp_packets".to_string()));
                        }
                        Ok(MessageEvent::Rtp(RTPMessageEvent::Rtcp(rtcp_packets)))
                    } else {
                        Err(Error::Other(format!(
                            "remote_srtp_context is not set yet for four_tuple {:?}",
                            four_tuple
                        )))
                    }
                } else {
                    let mut remote_context = transport.remote_srtp_context();
                    if let Some(context) = remote_context.as_mut() {
                        let mut decrypted = context.decrypt_rtp(&message)?;
                        let rtp_packet = rtp::Packet::unmarshal(&mut decrypted)?;
                        Ok(MessageEvent::Rtp(RTPMessageEvent::Rtp(rtp_packet)))
                    } else {
                        Err(Error::Other(format!(
                            "remote_srtp_context is not set yet for four_tuple {:?}",
                            four_tuple
                        )))
                    }
                }
            };

            match try_read() {
                Ok(message) => {
                    msg.message = message;
                    ctx.fire_read(msg);
                }
                Err(err) => {
                    error!("try_read got error {}", err);
                    ctx.fire_read_exception(Box::new(err))
                }
            };
        } else {
            debug!("bypass srtp read {:?}", msg.transport.peer_addr);
            ctx.fire_read(msg);
        }
    }
}

impl OutboundHandler for SrtpOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, mut msg: Self::Win) {
        if let MessageEvent::Rtp(message) = msg.message {
            debug!("srtp write {:?}", msg.transport.peer_addr);
            let try_write = || -> Result<BytesMut> {
                let four_tuple = (&msg.transport).into();
                let mut server_states = self.server_states.borrow_mut();
                let transport = server_states.get_mut_transport(&four_tuple)?;

                match message {
                    RTPMessageEvent::Rtcp(rtcp_packets) => {
                        if rtcp_packets.is_empty() {
                            return Err(Error::Other("empty rtcp_packets".to_string()));
                        };

                        let mut local_context = transport.local_srtp_context();
                        if let Some(context) = local_context.as_mut() {
                            let packet = rtcp::packet::marshal(&rtcp_packets)?;
                            context.encrypt_rtcp(&packet)
                        } else {
                            Err(Error::Other(format!(
                                "local_srtp_context is not set yet for four_tuple {:?}",
                                four_tuple
                            )))
                        }
                    }
                    RTPMessageEvent::Rtp(rtp_message) => {
                        let mut local_context = transport.local_srtp_context();
                        if let Some(context) = local_context.as_mut() {
                            let packet = rtp_message.marshal()?;
                            context.encrypt_rtp(&packet)
                        } else {
                            Err(Error::Other(format!(
                                "local_srtp_context is not set yet for four_tuple {:?}",
                                four_tuple
                            )))
                        }
                    }
                    RTPMessageEvent::Raw(raw_packet) => {
                        // Bypass
                        debug!("Bypass srtp write {:?}", msg.transport.peer_addr);
                        Ok(raw_packet)
                    }
                }
            };

            match try_write() {
                Ok(encrypted) => {
                    msg.message = MessageEvent::Rtp(RTPMessageEvent::Raw(encrypted));
                    ctx.fire_write(msg);
                }
                Err(err) => {
                    error!("try_write with error {}", err);
                    ctx.fire_write_exception(Box::new(err))
                }
            };
        } else {
            // Bypass
            debug!("Bypass srtp write {:?}", msg.transport.peer_addr);
            ctx.fire_write(msg);
        }
    }
}

impl Handler for SrtpHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "SrtpHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.srtp_inbound), Box::new(self.srtp_outbound))
    }
}
