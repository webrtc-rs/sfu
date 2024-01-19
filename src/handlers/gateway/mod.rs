use crate::messages::{MessageEvent, STUNMessageEvent, TaggedMessageEvent};
use crate::server::states::ServerStates;
use log::warn;
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TransportContext;
use shared::error::Result;
use std::rc::Rc;
use std::time::Instant;

struct GatewayInbound {
    server_states: Rc<ServerStates>,
}
struct GatewayOutbound {
    server_states: Rc<ServerStates>,
}

pub struct GatewayHandler {
    gateway_inbound: GatewayInbound,
    gateway_outbound: GatewayOutbound,
}

impl GatewayHandler {
    pub fn new(server_states: Rc<ServerStates>) -> Self {
        GatewayHandler {
            gateway_inbound: GatewayInbound {
                server_states: Rc::clone(&server_states),
            },
            gateway_outbound: GatewayOutbound { server_states },
        }
    }
}

impl InboundHandler for GatewayInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        let try_read = || -> Result<()> {
            match msg.message {
                MessageEvent::STUN(STUNMessageEvent::STUN(message)) => {
                    self.handle_stun_message(ctx, msg.now, msg.transport, message)
                }
                _ => {
                    warn!(
                        "drop unsupported message {:?} from {}",
                        msg.message, msg.transport.peer_addr
                    );
                    Ok(())
                }
            }
        };

        if let Err(err) = try_read() {
            warn!("try_read got error {}", err);
            ctx.fire_read_exception(Box::new(err));
        }
    }
}

impl OutboundHandler for GatewayOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg);
    }
}

impl Handler for GatewayHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "GatewayHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (
            Box::new(self.gateway_inbound),
            Box::new(self.gateway_outbound),
        )
    }
}

impl GatewayInbound {
    fn handle_stun_message(
        &mut self,
        _ctx: &InboundContext<TaggedMessageEvent, TaggedMessageEvent>,
        _now: Instant,
        _transport: TransportContext,
        _request: stun::message::Message,
    ) -> Result<()> {
        Ok(())
    }
}
