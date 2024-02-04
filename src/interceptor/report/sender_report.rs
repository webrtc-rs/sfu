use crate::interceptor::report::ReportBuilder;
use crate::interceptor::{Interceptor, InterceptorEvent};
use crate::messages::TaggedMessageEvent;
use std::time::Instant;

pub(crate) struct SenderReport {
    pub(super) next: Option<Box<dyn Interceptor>>,
}

impl SenderReport {
    pub(crate) fn builder() -> ReportBuilder {
        ReportBuilder {
            is_rr: false,
            ..Default::default()
        }
    }
}

impl Interceptor for SenderReport {
    fn chain(mut self: Box<Self>, next: Box<dyn Interceptor>) -> Box<dyn Interceptor> {
        self.next = Some(next);
        self
    }

    fn read(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        if let Some(next) = self.next.as_mut() {
            let mut events = next.read(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn write(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        let mut interceptor_events = vec![];
        if let Some(next) = self.next.as_mut() {
            let mut events = next.write(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn handle_timeout(&mut self, now: Instant) -> Vec<InterceptorEvent> {
        // SenderReport in SFU does nothing, but just forwarding it to other Endpoints

        let mut interceptor_events = vec![];
        if let Some(next) = self.next.as_mut() {
            let mut events = next.handle_timeout(now);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn poll_timeout(&mut self, eto: &mut Instant) {
        // SenderReport in SFU does nothing, but just forwarding it to other Endpoints

        if let Some(next) = self.next.as_mut() {
            next.poll_timeout(eto);
        }
    }
}
