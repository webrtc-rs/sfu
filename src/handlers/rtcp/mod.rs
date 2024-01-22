use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use log::debug;
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};

#[derive(Default)]
struct RtcpInbound;
#[derive(Default)]
struct RtcpOutbound;
#[derive(Default)]
pub struct RtcpHandler {
    rtcp_inbound: RtcpInbound,
    rtcp_outbound: RtcpOutbound,
}

impl RtcpHandler {
    pub fn new() -> Self {
        Self::default()
    }
}

impl InboundHandler for RtcpInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if let MessageEvent::RTP(RTPMessageEvent::RTCP(rtcp_messages)) = &msg.message {
            if let Some(rtcp_message) = rtcp_messages.first() {
                debug!("rtcp read {:?}", rtcp_message.header());
            }
        }

        ctx.fire_read(msg);
    }
}

impl OutboundHandler for RtcpOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        if let MessageEvent::RTP(RTPMessageEvent::RTCP(rtcp_messages)) = &msg.message {
            if let Some(rtcp_message) = rtcp_messages.first() {
                debug!("rtcp write {:?}", rtcp_message.header());
            }
        }

        ctx.fire_write(msg);
    }
}

impl Handler for RtcpHandler {
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
        (Box::new(self.rtcp_inbound), Box::new(self.rtcp_outbound))
    }
}
