use crate::interceptor::report::ReportBuilder;
use crate::interceptor::{Interceptor, InterceptorEvent};
use crate::messages::TaggedMessageEvent;
use std::time::{Duration, Instant};

pub(crate) struct ReceiverReport {
    pub(super) interval: Duration,
    pub(super) eto: Instant,
    pub(super) next: Option<Box<dyn Interceptor>>,
}

impl ReceiverReport {
    pub(crate) fn builder() -> ReportBuilder {
        ReportBuilder {
            is_rr: true,
            ..Default::default()
        }
    }
}

impl Interceptor for ReceiverReport {
    fn chain(mut self: Box<Self>, next: Box<dyn Interceptor>) -> Box<dyn Interceptor> {
        self.next = Some(next);
        self
    }

    fn read(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];

        //TODO:

        if let Some(next) = self.next.as_mut() {
            let mut events = next.read(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn write(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];

        //TODO:

        if let Some(next) = self.next.as_mut() {
            let mut events = next.write(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn handle_timeout(&mut self, now: Instant) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];

        //TODO:
        self.eto = now + self.interval;

        if let Some(next) = self.next.as_mut() {
            let mut events = next.handle_timeout(now);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn poll_timeout(&mut self, eto: &mut Instant) {
        if self.eto < *eto {
            *eto = self.eto
        }

        if let Some(next) = self.next.as_mut() {
            next.poll_timeout(eto);
        }
    }
}
