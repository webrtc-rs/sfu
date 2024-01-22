use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use crate::server::states::ServerStates;
use bytes::BytesMut;
use log::{debug, error};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use shared::{
    error::{Error, Result},
    util::is_rtcp,
};
use std::rc::Rc;

struct SrtpInbound {
    server_states: Rc<ServerStates>, //remote_contexts: Rc<RefCell<HashMap<SocketAddr, Context>>>,
}
struct SrtpOutbound {
    server_states: Rc<ServerStates>, //local_contexts: Rc<RefCell<HashMap<SocketAddr, Context>>>,
}
pub struct SrtpHandler {
    srtp_inbound: SrtpInbound,
    srtp_outbound: SrtpOutbound,
}

impl SrtpHandler {
    pub fn new(server_states: Rc<ServerStates>) -> Self {
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
        if let MessageEvent::RTP(RTPMessageEvent::RAW(rtp_message)) = msg.message {
            debug!("srtp read {:?}", msg.transport.peer_addr);
            let try_read = || -> Result<BytesMut> {
                let four_tuple = (&msg.transport).into();
                let endpoint =
                    self.server_states
                        .find_endpoint(&four_tuple)
                        .ok_or(Error::Other(format!(
                            "can't find endpoint with four_tuple {:?}",
                            four_tuple
                        )))?;
                let transport =
                    endpoint
                        .get_transport(&four_tuple)
                        .ok_or(Error::Other(format!(
                            "can't find transport with four_tuple {:?} for endpoint id {}",
                            four_tuple,
                            endpoint.endpoint_id(),
                        )))?;

                if is_rtcp(&rtp_message) {
                    let mut remote_context = transport.remote_srtp_context().borrow_mut();
                    if let Some(context) = remote_context.as_mut() {
                        context.decrypt_rtcp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "remote_srtp_context is not set yet for four_tuple {:?} of endpoint id {}",
                            four_tuple, endpoint.endpoint_id()
                        )))
                    }
                } else {
                    let mut remote_context = transport.remote_srtp_context().borrow_mut();
                    if let Some(context) = remote_context.as_mut() {
                        context.decrypt_rtp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "remote_srtp_context is not set yet for four_tuple {:?} of endpoint id {}",
                            four_tuple, endpoint.endpoint_id()
                        )))
                    }
                }
            };

            match try_read() {
                Ok(decrypted) => {
                    msg.message = MessageEvent::RTP(RTPMessageEvent::RAW(decrypted));
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
        if let MessageEvent::RTP(RTPMessageEvent::RAW(rtp_message)) = msg.message {
            debug!("srtp write {:?}", msg.transport.peer_addr);
            let try_write = || -> Result<BytesMut> {
                let four_tuple = (&msg.transport).into();
                let endpoint =
                    self.server_states
                        .find_endpoint(&four_tuple)
                        .ok_or(Error::Other(format!(
                            "can't find endpoint with four_tuple {:?}",
                            four_tuple
                        )))?;
                let transport =
                    endpoint
                        .get_transport(&four_tuple)
                        .ok_or(Error::Other(format!(
                            "can't find transport with four_tuple {:?} for endpoint id {}",
                            four_tuple,
                            endpoint.endpoint_id(),
                        )))?;

                if is_rtcp(&rtp_message) {
                    let mut local_context = transport.local_srtp_context().borrow_mut();
                    if let Some(context) = local_context.as_mut() {
                        context.encrypt_rtcp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "local_srtp_context is not set yet for four_tuple {:?} of endpoint id {}",
                            four_tuple, endpoint.endpoint_id()
                        )))
                    }
                } else {
                    let mut local_context = transport.local_srtp_context().borrow_mut();
                    if let Some(context) = local_context.as_mut() {
                        context.encrypt_rtp(&rtp_message)
                    } else {
                        Err(Error::Other(format!(
                            "local_srtp_context is not set yet for four_tuple {:?} of endpoint id {}",
                            four_tuple, endpoint.endpoint_id()
                        )))
                    }
                }
            };

            match try_write() {
                Ok(encrypted) => {
                    msg.message = MessageEvent::RTP(RTPMessageEvent::RAW(encrypted));
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
