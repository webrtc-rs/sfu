use crate::interceptor::{Interceptor, InterceptorEvent};
use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use crate::ServerStates;
use log::error;
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

struct InterceptorInbound {
    interceptor: Rc<RefCell<Box<dyn Interceptor>>>,
}
struct InterceptorOutbound {
    interceptor: Rc<RefCell<Box<dyn Interceptor>>>,
}
pub struct InterceptorHandler {
    interceptor_inbound: InterceptorInbound,
    interceptor_outbound: InterceptorOutbound,
}

impl InterceptorHandler {
    pub fn new(server_states: Rc<RefCell<ServerStates>>) -> Self {
        let server_states = server_states.borrow();
        let registry = server_states.server_config().media_config.registry();
        let interceptor = Rc::new(RefCell::new(registry.build(""))); //TODO: use named registry id

        Self {
            interceptor_inbound: InterceptorInbound {
                interceptor: Rc::clone(&interceptor),
            },
            interceptor_outbound: InterceptorOutbound { interceptor },
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
            let mut try_read = || -> Vec<InterceptorEvent> {
                let mut interceptor = self.interceptor.borrow_mut();
                interceptor.read(&mut msg)
            };

            for event in try_read() {
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

        ctx.fire_read(msg);
    }

    fn handle_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, now: Instant) {
        let try_handle_timeout = || -> Vec<InterceptorEvent> {
            let mut interceptor = self.interceptor.borrow_mut();
            interceptor.handle_timeout(now)
        };

        for event in try_handle_timeout() {
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

        ctx.fire_handle_timeout(now);
    }

    fn poll_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, eto: &mut Instant) {
        {
            let mut interceptor = self.interceptor.borrow_mut();
            interceptor.poll_timeout(eto);
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
            let mut try_write = || -> Vec<InterceptorEvent> {
                let mut interceptor = self.interceptor.borrow_mut();
                interceptor.write(&mut msg)
            };

            for event in try_write() {
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
