use crate::messages::TaggedMessageEvent;
use log::error;
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use std::error::Error;

#[derive(Default)]
struct ExceptionInbound;
#[derive(Default)]
struct ExceptionOutbound;

#[derive(Default)]
pub struct ExceptionHandler {
    exception_inbound: ExceptionInbound,
    exception_outbound: ExceptionOutbound,
}

impl ExceptionHandler {
    pub fn new() -> Self {
        ExceptionHandler::default()
    }
}

impl InboundHandler for ExceptionInbound {
    type Rin = TaggedMessageEvent;
    type Rout = TaggedMessageEvent;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        ctx.fire_read(msg);
    }

    fn read_exception(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, err: Box<dyn Error>) {
        error!("ExceptionHandler::read_exception {}", err);
        ctx.fire_read_exception(err);
    }
}

impl OutboundHandler for ExceptionOutbound {
    type Win = TaggedMessageEvent;
    type Wout = TaggedMessageEvent;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg);
    }

    fn write_exception(
        &mut self,
        ctx: &OutboundContext<Self::Win, Self::Wout>,
        err: Box<dyn Error>,
    ) {
        error!("ExceptionHandler::write_exception {}", err);
        ctx.fire_write_exception(err);
    }
}

impl Handler for ExceptionHandler {
    type Rin = TaggedMessageEvent;
    type Rout = TaggedMessageEvent;
    type Win = TaggedMessageEvent;
    type Wout = TaggedMessageEvent;

    fn name(&self) -> &str {
        "ExceptionHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (
            Box::new(self.exception_inbound),
            Box::new(self.exception_outbound),
        )
    }
}
