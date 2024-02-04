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
    fn read(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent>;
    fn write(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent>;
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
    builders: Vec<Box<dyn InterceptorBuilder>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    /// add a new InterceptorBuilder to the registry.
    pub fn add(&mut self, builder: Box<dyn InterceptorBuilder>) {
        self.builders.push(builder);
    }

    /// build a single Interceptor from an InterceptorRegistry
    pub fn build(&self, id: &str) -> Box<dyn Interceptor> {
        if self.builders.is_empty() {
            Box::new(NoOp)
        } else {
            Box::new(self.build_chain(id))
        }
    }

    // construct a non-type erased Chain from an Interceptor registry.
    fn build_chain(&self, id: &str) -> Chain {
        if self.builders.is_empty() {
            Chain::new(vec![Box::new(NoOp {})])
        } else {
            let interceptors: Vec<_> = self.builders.iter().map(|b| b.build(id)).collect();
            Chain::new(interceptors)
        }
    }
}

/// NoOp is an Interceptor that does not modify any packets. It can embedded in other interceptors, so it's
/// possible to implement only a subset of the methods.
struct NoOp;

impl Interceptor for NoOp {
    fn read(&mut self, _msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        vec![]
    }

    fn write(&mut self, _msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
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
    interceptors: Vec<Box<dyn Interceptor>>,
}

impl Chain {
    // new returns a new Chain interceptor.
    fn new(interceptors: Vec<Box<dyn Interceptor>>) -> Self {
        Chain { interceptors }
    }

    fn add(&mut self, icpr: Box<dyn Interceptor>) {
        self.interceptors.push(icpr);
    }
}

impl Interceptor for Chain {
    fn read(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        for interceptor in &mut self.interceptors {
            let mut events = interceptor.read(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn write(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        for interceptor in &mut self.interceptors {
            let mut events = interceptor.write(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn handle_timeout(&mut self, now: Instant) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        for interceptor in &mut self.interceptors {
            let mut events = interceptor.handle_timeout(now);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn poll_timeout(&mut self, eto: &mut Instant) {
        for interceptor in &mut self.interceptors {
            interceptor.poll_timeout(eto);
        }
    }
}
