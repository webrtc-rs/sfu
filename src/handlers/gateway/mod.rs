use crate::server::states::ServerStates;
use crate::shared::messages::TaggedMessageEvent;
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use std::rc::Rc;

struct GatewayInbound {
    server_states: Rc<ServerStates>,
}
struct GatewayOutbound;

pub struct GatewayHandler {
    gateway_inbound: GatewayInbound,
    gateway_outbound: GatewayOutbound,
}

impl GatewayHandler {
    pub fn new(server_states: Rc<ServerStates>) -> Self {
        GatewayHandler {
            gateway_inbound: GatewayInbound { server_states },
            gateway_outbound: GatewayOutbound {},
        }
    }
}

impl InboundHandler for GatewayInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, _ctx: &InboundContext<Self::Rin, Self::Rout>, _msg: Self::Rin) {}
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
