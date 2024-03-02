use std::cell::RefCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;

use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};

struct SyncTransportDecoder<R> {
    phantom: PhantomData<R>,
}
struct SyncTransportEncoder<R> {
    writer: Rc<RefCell<VecDeque<R>>>,
}

/// Asynchronous transport handler that writes R
pub struct SyncTransport<R> {
    decoder: SyncTransportDecoder<R>,
    encoder: SyncTransportEncoder<R>,
}

impl<R> SyncTransport<R> {
    /// Creates new synchronous transport handler
    pub fn new(writer: Rc<RefCell<VecDeque<R>>>) -> Self {
        SyncTransport {
            decoder: SyncTransportDecoder {
                phantom: PhantomData,
            },
            encoder: SyncTransportEncoder { writer },
        }
    }
}

impl<R: 'static> InboundHandler for SyncTransportDecoder<R> {
    type Rin = R;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        ctx.fire_read(msg);
    }
}

impl<R: 'static> OutboundHandler for SyncTransportEncoder<R> {
    type Win = R;
    type Wout = Self::Win;

    fn write(&mut self, _ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        let mut writer = self.writer.borrow_mut();
        writer.push_back(msg);
    }
}

impl<R: 'static> Handler for SyncTransport<R> {
    type Rin = R;
    type Rout = Self::Rin;
    type Win = Self::Rin;
    type Wout = Self::Rin;

    fn name(&self) -> &str {
        "SyncTransport"
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
