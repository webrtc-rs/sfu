use crate::interceptor::InterceptorEvent;
use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use crate::types::FourTuple;
use crate::ServerStates;
use log::error;
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use shared::error::Result;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

struct InterceptorInbound {
    server_states: Rc<RefCell<ServerStates>>,
}
struct InterceptorOutbound {
    server_states: Rc<RefCell<ServerStates>>,
}

/// InterceptorHandler implements RTCP feedback handling
pub struct InterceptorHandler {
    interceptor_inbound: InterceptorInbound,
    interceptor_outbound: InterceptorOutbound,
}

impl InterceptorHandler {
    pub fn new(server_states: Rc<RefCell<ServerStates>>) -> Self {
        Self {
            interceptor_inbound: InterceptorInbound {
                server_states: Rc::clone(&server_states),
            },
            interceptor_outbound: InterceptorOutbound { server_states },
        }
    }
}

impl InboundHandler for InterceptorInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, mut msg: Self::Rin) {
        if let MessageEvent::Rtp(RTPMessageEvent::Rtp(_))
        | MessageEvent::Rtp(RTPMessageEvent::Rtcp(_)) = &msg.message
        {
            let mut try_read = || -> Result<Vec<InterceptorEvent>> {
                let mut server_states = self.server_states.borrow_mut();
                let four_tuple = (&msg.transport).into();
                let endpoint = server_states.get_mut_endpoint(&four_tuple)?;
                let interceptor = endpoint.get_mut_interceptor();
                Ok(interceptor.read(&mut msg))
            };

            match try_read() {
                Ok(events) => {
                    for event in events {
                        match event {
                            InterceptorEvent::Inbound(inbound) => {
                                ctx.fire_read(inbound);
                            }
                            InterceptorEvent::Outbound(outbound) => {
                                ctx.fire_write(outbound);
                            }
                            InterceptorEvent::Error(err) => {
                                error!("try_read got error {}", err);
                                ctx.fire_read_exception(err);
                            }
                        }
                    }
                }
                Err(err) => {
                    error!("try_read with error {}", err);
                    ctx.fire_read_exception(Box::new(err))
                }
            };

            if let MessageEvent::Rtp(RTPMessageEvent::Rtcp(_)) = &msg.message {
                // RTCP message read must end here in SFU case. If any rtcp packet needs to be forwarded to other Endpoints,
                // just add a new interceptor to forward it.
                return;
            }
        }

        ctx.fire_read(msg);
    }

    fn handle_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, now: Instant) {
        let try_handle_timeout = || -> Result<Vec<InterceptorEvent>> {
            let mut interceptor_events = vec![];

            let mut server_states = self.server_states.borrow_mut();
            let sessions = server_states.get_mut_sessions();
            for session in sessions.values_mut() {
                let endpoints = session.get_mut_endpoints();
                for endpoint in endpoints.values_mut() {
                    #[allow(clippy::map_clone)]
                    let four_tuples: Vec<FourTuple> = endpoint
                        .get_transports()
                        .keys()
                        .map(|four_tuple| *four_tuple)
                        .collect();
                    let interceptor = endpoint.get_mut_interceptor();
                    let mut events = interceptor.handle_timeout(now, &four_tuples);
                    interceptor_events.append(&mut events);
                }
            }

            Ok(interceptor_events)
        };

        match try_handle_timeout() {
            Ok(events) => {
                for event in events {
                    match event {
                        InterceptorEvent::Inbound(_) => {
                            error!("unexpected inbound message from try_handle_timeout");
                        }
                        InterceptorEvent::Outbound(outbound) => {
                            ctx.fire_write(outbound);
                        }
                        InterceptorEvent::Error(err) => {
                            error!("try_read got error {}", err);
                            ctx.fire_read_exception(err);
                        }
                    }
                }
            }
            Err(err) => {
                error!("try_handle_timeout with error {}", err);
                ctx.fire_read_exception(Box::new(err))
            }
        }

        ctx.fire_handle_timeout(now);
    }

    fn poll_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, eto: &mut Instant) {
        {
            let mut server_states = self.server_states.borrow_mut();
            let sessions = server_states.get_mut_sessions();
            for session in sessions.values_mut() {
                let endpoints = session.get_mut_endpoints();
                for endpoint in endpoints.values_mut() {
                    let interceptor = endpoint.get_mut_interceptor();
                    interceptor.poll_timeout(eto)
                }
            }
        }

        ctx.fire_poll_timeout(eto);
    }
}

impl OutboundHandler for InterceptorOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, mut msg: Self::Win) {
        if let MessageEvent::Rtp(RTPMessageEvent::Rtp(_))
        | MessageEvent::Rtp(RTPMessageEvent::Rtcp(_)) = &msg.message
        {
            let mut try_write = || -> Result<Vec<InterceptorEvent>> {
                let mut server_states = self.server_states.borrow_mut();
                let four_tuple = (&msg.transport).into();
                let endpoint = server_states.get_mut_endpoint(&four_tuple)?;
                let interceptor = endpoint.get_mut_interceptor();
                Ok(interceptor.write(&mut msg))
            };

            match try_write() {
                Ok(events) => {
                    for event in events {
                        match event {
                            InterceptorEvent::Inbound(_) => {
                                error!("unexpected inbound message from try_write");
                            }
                            InterceptorEvent::Outbound(outbound) => {
                                ctx.fire_write(outbound);
                            }
                            InterceptorEvent::Error(err) => {
                                error!("try_write got error {}", err);
                                ctx.fire_write_exception(err);
                            }
                        }
                    }
                }
                Err(err) => {
                    error!("try_write with error {}", err);
                    ctx.fire_write_exception(Box::new(err))
                }
            };
        }

        ctx.fire_write(msg);
    }
}

impl Handler for InterceptorHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "RtcpHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (
            Box::new(self.interceptor_inbound),
            Box::new(self.interceptor_outbound),
        )
    }
}
