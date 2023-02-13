use async_trait::async_trait;
use std::sync::Arc;

use crate::rtc::server::ServerStates;

use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TaggedBytesMut;

struct ICEDecoder {
    server_states: Arc<ServerStates>,
}
struct ICEEncoder;

pub struct ICEHandler {
    decoder: ICEDecoder,
    encoder: ICEEncoder,
}

impl ICEHandler {
    pub fn new(server_states: Arc<ServerStates>) -> Self {
        ICEHandler {
            decoder: ICEDecoder { server_states },
            encoder: ICEEncoder {},
        }
    }
}

#[async_trait]
impl InboundHandler for ICEDecoder {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;

    async fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        ctx.fire_read(msg).await;
    }
}

#[async_trait]
impl OutboundHandler for ICEEncoder {
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    async fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg).await;
    }
}

impl Handler for ICEHandler {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "ICEHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.decoder), Box::new(self.encoder))
    }
}
