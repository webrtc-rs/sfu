use crate::messages::{MessageEvent, STUNMessageEvent, TaggedMessageEvent};
use bytes::BytesMut;
use log::{debug, warn};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use shared::error::Result;
use stun::message::Message;

#[derive(Default)]
struct StunInbound;
#[derive(Default)]
struct StunOutbound;
#[derive(Default)]
pub struct StunHandler {
    stun_inbound: StunInbound,
    stun_outbound: StunOutbound,
}

impl StunHandler {
    pub fn new() -> Self {
        StunHandler::default()
    }
}

impl InboundHandler for StunInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if let MessageEvent::Stun(STUNMessageEvent::Raw(message)) = msg.message {
            let try_read = || -> Result<Message> {
                let mut stun_message = Message {
                    raw: message.to_vec(),
                    ..Default::default()
                };
                stun_message.decode()?;
                debug!(
                    "StunMessage type {} received from {}",
                    stun_message.typ, msg.transport.peer_addr
                );
                Ok(stun_message)
            };

            match try_read() {
                Ok(stun_message) => {
                    ctx.fire_read(TaggedMessageEvent {
                        now: msg.now,
                        transport: msg.transport,
                        message: MessageEvent::Stun(STUNMessageEvent::Stun(stun_message)),
                    });
                }
                Err(err) => {
                    warn!("try_read got error {}", err);
                    ctx.fire_read_exception(Box::new(err));
                }
            }
        } else {
            debug!("bypass StunInbound for {}", msg.transport.peer_addr);
            ctx.fire_read(msg);
        }
    }
}

impl OutboundHandler for StunOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        if let MessageEvent::Stun(STUNMessageEvent::Stun(mut stun_message)) = msg.message {
            debug!(
                "StunMessage type {} sent to {}",
                stun_message.typ, msg.transport.peer_addr
            );
            stun_message.encode();
            let message = BytesMut::from(&stun_message.raw[..]);
            ctx.fire_write(TaggedMessageEvent {
                now: msg.now,
                transport: msg.transport,
                message: MessageEvent::Stun(STUNMessageEvent::Raw(message)),
            });
        } else {
            debug!("bypass StunOutbound for {}", msg.transport.peer_addr);
            ctx.fire_write(msg);
        }
    }
}

impl Handler for StunHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "StunHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.stun_inbound), Box::new(self.stun_outbound))
    }
}
