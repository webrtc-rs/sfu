use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use log::debug;
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use std::time::Instant;

#[derive(Default)]
struct InterceptorInbound;
#[derive(Default)]
struct InterceptorOutbound;
#[derive(Default)]
pub struct InterceptorHandler {
    interceptor_inbound: InterceptorInbound,
    interceptor_outbound: InterceptorOutbound,
}

impl InterceptorHandler {
    pub fn new() -> Self {
        Self::default()
    }
}

impl InboundHandler for InterceptorInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if let MessageEvent::Rtp(RTPMessageEvent::Rtcp(rtcp_messages)) = &msg.message {
            if let Some(rtcp_message) = rtcp_messages.first() {
                debug!("rtcp read {:?}", rtcp_message.header());
            }
        }

        ctx.fire_read(msg);
    }

    fn handle_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, now: Instant) {
        ctx.fire_handle_timeout(now);
    }

    fn poll_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, eto: &mut Instant) {
        ctx.fire_poll_timeout(eto);
    }
}

impl OutboundHandler for InterceptorOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        if let MessageEvent::Rtp(RTPMessageEvent::Rtcp(rtcp_messages)) = &msg.message {
            if let Some(rtcp_message) = rtcp_messages.first() {
                debug!("rtcp write {:?}", rtcp_message.header());
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
