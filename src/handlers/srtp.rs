use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use crate::server::states::ServerStates;
use bytes::BytesMut;
use log::{debug, error};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use shared::{
    error::{Error, Result},
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
        if let MessageEvent::Rtp(RTPMessageEvent::Raw(rtp_message)) = msg.message {
            debug!("srtp read {:?}", msg.transport.peer_addr);
            let try_read = || -> Result<BytesMut> {
                let four_tuple = (&msg.transport).into();
                let mut server_states = self.server_states.borrow_mut();
                let transport = match server_states.get_mut_transport(&four_tuple) {
                    Ok(transport) => transport,
                    Err(err) => {
                        return Err(err);
                    }
                };

                if is_rtcp(&rtp_message) {
                    let mut remote_context = transport.remote_srtp_context();
                    if let Some(context) = remote_context.as_mut() {
                        context.decrypt_rtcp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "remote_srtp_context is not set yet for four_tuple {:?}",
                            four_tuple
                        )))
                    }
                } else {
                    let mut remote_context = transport.remote_srtp_context();
                    if let Some(context) = remote_context.as_mut() {
                        context.decrypt_rtp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "remote_srtp_context is not set yet for four_tuple {:?}",
                            four_tuple
                        )))
                    }
                }
            };

            match try_read() {
                Ok(decrypted) => {
                    msg.message = MessageEvent::Rtp(RTPMessageEvent::Raw(decrypted));
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
        if let MessageEvent::Rtp(RTPMessageEvent::Raw(rtp_message)) = msg.message {
            debug!("srtp write {:?}", msg.transport.peer_addr);
            let try_write = || -> Result<BytesMut> {
                let four_tuple = (&msg.transport).into();
                let mut server_states = self.server_states.borrow_mut();
                let transport = match server_states.get_mut_transport(&four_tuple) {
                    Ok(transport) => transport,
                    Err(err) => {
                        return Err(err);
                    }
                };
                if is_rtcp(&rtp_message) {
                    let mut local_context = transport.local_srtp_context();
                    if let Some(context) = local_context.as_mut() {
                        context.encrypt_rtcp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "local_srtp_context is not set yet for four_tuple {:?}",
                            four_tuple
                        )))
                    }
                } else {
                    let mut local_context = transport.local_srtp_context();
                    if let Some(context) = local_context.as_mut() {
                        context.encrypt_rtp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "local_srtp_context is not set yet for four_tuple {:?}",
                            four_tuple
                        )))
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
