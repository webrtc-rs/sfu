use crate::messages::TaggedMessageEvent;
use std::time::Instant;

pub(crate) mod nack;
pub(crate) mod report;
pub(crate) mod twcc;

pub enum InterceptorEvent {
    Inbound(TaggedMessageEvent),
    Outbound(TaggedMessageEvent),
    Error(Box<dyn std::error::Error>),
}

pub trait Interceptor {
    fn chain(self: Box<Self>, next: Box<dyn Interceptor>) -> Box<dyn Interceptor>;
    fn read(&mut self, msg: TaggedMessageEvent) -> Vec<InterceptorEvent>;
    fn write(&mut self, msg: TaggedMessageEvent) -> Vec<InterceptorEvent>;
    fn handle_timeout(&mut self, now: Instant) -> Vec<InterceptorEvent>;
    fn poll_timeout(&mut self, eto: &mut Instant);
}

/// InterceptorBuilder provides an interface for constructing interceptors
pub trait InterceptorBuilder {
    fn build(&self, id: &str) -> Box<dyn Interceptor>;
}

/// Registry is a collector for interceptors.
#[derive(Default)]
pub struct Registry {
    builders: Vec<Box<dyn InterceptorBuilder + Send + Sync>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// add a new InterceptorBuilder to the registry.
    pub fn add(&mut self, builder: Box<dyn InterceptorBuilder + Send + Sync>) {
        self.builders.push(builder);
    }

    /// build a single Interceptor from an InterceptorRegistry
    pub fn build(&self, id: &str) -> Box<dyn Interceptor> {
        if self.builders.is_empty() {
            Box::new(NoOp)
        } else {
            let mut next = Box::<Chain>::default() as Box<dyn Interceptor>;
            for interceptor in self.builders.iter().rev().map(|b| b.build(id)) {
                next = interceptor.chain(next);
            }
            next
        }
    }
}

/// NoOp is an Interceptor that does not modify any packets. It can embedded in other interceptors, so it's
/// possible to implement only a subset of the methods.
struct NoOp;

impl Interceptor for NoOp {
    fn chain(self: Box<Self>, _next: Box<dyn Interceptor>) -> Box<dyn Interceptor> {
        self
    }

    fn read(&mut self, _msg: TaggedMessageEvent) -> Vec<InterceptorEvent> {
        vec![]
    }

    fn write(&mut self, _msg: TaggedMessageEvent) -> Vec<InterceptorEvent> {
        vec![]
    }

    fn handle_timeout(&mut self, _now: Instant) -> Vec<InterceptorEvent> {
        vec![]
    }

    fn poll_timeout(&mut self, _eto: &mut Instant) {}
}

/// Chain is an interceptor that runs all child interceptors in order.
#[derive(Default)]
struct Chain {
    next: Option<Box<dyn Interceptor>>,
}

impl Interceptor for Chain {
    fn chain(mut self: Box<Self>, next: Box<dyn Interceptor>) -> Box<dyn Interceptor> {
        self.next = Some(next);
        self
    }

    fn read(&mut self, msg: TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        if let Some(next) = self.next.as_mut() {
            let mut events = next.read(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn write(&mut self, msg: TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        if let Some(next) = self.next.as_mut() {
            let mut events = next.write(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn handle_timeout(&mut self, now: Instant) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        if let Some(next) = self.next.as_mut() {
            let mut events = next.handle_timeout(now);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn poll_timeout(&mut self, eto: &mut Instant) {
        if let Some(next) = self.next.as_mut() {
            next.poll_timeout(eto);
        }
    }
}
