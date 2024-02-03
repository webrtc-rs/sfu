use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use log::{debug, error};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use shared::{
    marshal::{Marshal, Unmarshal},
    util::is_rtcp,
};

#[derive(Default)]
struct RtpInbound;
#[derive(Default)]
struct RtpOutbound;
#[derive(Default)]
pub struct RtpHandler {
    rtp_inbound: RtpInbound,
    rtp_outbound: RtpOutbound,
}

impl RtpHandler {
    pub fn new() -> Self {
        Self::default()
    }
}

impl InboundHandler for RtpInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, mut msg: Self::Rin) {
        debug!("rtp read {:?}", msg.transport.peer_addr);
        if let MessageEvent::Rtp(RTPMessageEvent::Raw(mut message)) = msg.message {
            if is_rtcp(&message) {
                match rtcp::packet::unmarshal(&mut message) {
                    Ok(rtcp_packet) => {
                        msg.message = MessageEvent::Rtp(RTPMessageEvent::Rtcp(rtcp_packet));
                        ctx.fire_read(msg);
                    }
                    Err(err) => {
                        error!("try_read with error {}", err);
                        ctx.fire_read_exception(Box::new(err))
                    }
                }
            } else {
                match rtp::Packet::unmarshal(&mut message) {
                    Ok(rtp_packet) => {
                        msg.message = MessageEvent::Rtp(RTPMessageEvent::Rtp(rtp_packet));
                        ctx.fire_read(msg);
                    }
                    Err(err) => {
                        error!("try_read with error {}", err);
                        ctx.fire_read_exception(Box::new(err))
                    }
                }
            }
        } else {
            debug!("bypass rtp read {:?}", msg.transport.peer_addr);
            ctx.fire_read(msg);
        }
    }
}

impl OutboundHandler for RtpOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, mut msg: Self::Win) {
        debug!("rtp write {:?}", msg.transport.peer_addr);
        match msg.message {
            MessageEvent::Rtp(RTPMessageEvent::Rtcp(rtcp_message)) => {
                match rtcp::packet::marshal(&rtcp_message) {
                    Ok(message) => {
                        msg.message = MessageEvent::Rtp(RTPMessageEvent::Raw(message));
                        ctx.fire_write(msg);
                    }
                    Err(err) => {
                        error!("try_write with error {}", err);
                        ctx.fire_write_exception(Box::new(err))
                    }
                }
            }
            MessageEvent::Rtp(RTPMessageEvent::Rtp(rtp_message)) => match rtp_message.marshal() {
                Ok(message) => {
                    msg.message = MessageEvent::Rtp(RTPMessageEvent::Raw(message));
                    ctx.fire_write(msg);
                }
                Err(err) => {
                    error!("try_write with error {}", err);
                    ctx.fire_write_exception(Box::new(err))
                }
            },
            _ => {
                // Bypass
                debug!("Bypass rtp write {:?}", msg.transport.peer_addr);
                ctx.fire_write(msg);
            }
        }
    }
}

impl Handler for RtpHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "RtpHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.rtp_inbound), Box::new(self.rtp_outbound))
    }
}
