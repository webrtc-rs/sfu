use crate::interceptor::report::ReportBuilder;
use crate::interceptor::{Interceptor, InterceptorEvent};
use crate::messages::{MessageEvent, RTPMessageEvent, TaggedMessageEvent};
use rtcp::header::PacketType;
use rtcp::source_description::{SdesType, SourceDescription};
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

        if let MessageEvent::Rtp(RTPMessageEvent::Rtcp(rtcp_packets)) = &msg.message {
            let mut inbound_rtcp_packets = vec![];

            if rtcp_packets.len() > 1 {
                let mut has_sender_report = false;
                for rtcp_packet in rtcp_packets {
                    let packet_type = rtcp_packet.header().packet_type;
                    if packet_type == PacketType::SenderReport {
                        inbound_rtcp_packets.push(rtcp_packet.clone());
                        has_sender_report = true;
                    } else if has_sender_report && packet_type == PacketType::SourceDescription {
                        if let Some(e) = rtcp_packet.as_any().downcast_ref::<SourceDescription>() {
                            let mut has_cname = false;
                            for c in &e.chunks {
                                for it in &c.items {
                                    if it.sdes_type == SdesType::SdesCname {
                                        has_cname = true
                                    }
                                }
                            }
                            if has_cname {
                                inbound_rtcp_packets.push(rtcp_packet.clone());
                            }
                        }
                    }
                }
            } else if !rtcp_packets.is_empty()
                && rtcp_packets[0].header().packet_type == PacketType::SenderReport
            {
                inbound_rtcp_packets.push(rtcp_packets[0].clone());
            }

            interceptor_events.push(InterceptorEvent::Inbound(TaggedMessageEvent {
                now: msg.now,
                transport: msg.transport,
                message: MessageEvent::Rtp(RTPMessageEvent::Rtcp(inbound_rtcp_packets)),
            }));
        }

        if let Some(next) = self.next.as_mut() {
            let mut events = next.read(msg);
            interceptor_events.append(&mut events);
        }
        interceptor_events
    }

    fn write(&mut self, msg: &mut TaggedMessageEvent) -> Vec<InterceptorEvent> {
        // SenderReport in SFU does nothing, but just forwarding it to other Endpoints

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