//! An interceptor that lets a subscriber's keyframe requests reach the application.
//!
//! A chain ends the inbound RTCP path at its terminus, so control traffic the interceptors act
//! on — reports, NACK, TWCC — does not arrive mixed in with the media the application asked for.
//! The SFU is the exception the escape hatch exists for: it has to see a subscriber's PLI/FIR
//! about a forwarded stream so it can relay them upstream to the publisher.
//!
//! Getting past the terminus is per-packet and is done by attaching
//! [`Attribute::DeliverToApplication`], which is why this is a judgement an interceptor makes
//! rather than a chain-wide switch: the SFU vouches for keyframe requests and leaves the reports
//! its own chain is acting on alone.

use log::trace;
use rtc::interceptor::{Attribute, Interceptor, Packet, StreamInfo, TaggedPacket};
use rtc::rtcp::Packet as RtcpPacket;
use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::sansio::Protocol;
use rtc::shared::error::Error;
use std::collections::VecDeque;
use std::time::Instant;

/// Builder for [`RtcpForwarderInterceptor`], added to a `Registry` with `.with(slot, ..)`.
#[derive(Default)]
pub(crate) struct RtcpForwarderBuilder;

impl RtcpForwarderBuilder {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn build(self) -> RtcpForwarderInterceptor {
        RtcpForwarderInterceptor::default()
    }
}

/// Whether `packet` is a keyframe request.
fn is_keyframe_request(packet: &dyn RtcpPacket) -> bool {
    let any = packet.as_any();
    any.is::<PictureLossIndication>() || any.is::<FullIntraRequest>()
}

/// Marks inbound RTCP carrying PLI/FIR for delivery to the application.
///
/// Both queues exist because the chain feeds each interceptor from a shared belt and collects what
/// it returns: what `handle_*` takes in, `poll_*` gives back. Queueing nothing would drop the
/// packet, so even the untouched write path has to hand its packets back.
#[derive(Default)]
pub(crate) struct RtcpForwarderInterceptor {
    read_queue: VecDeque<TaggedPacket>,
    write_queue: VecDeque<TaggedPacket>,
}

impl Protocol<TaggedPacket, TaggedPacket, ()> for RtcpForwarderInterceptor {
    type Rout = TaggedPacket;
    type Wout = TaggedPacket;
    type Eout = ();
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, mut msg: TaggedPacket) -> Result<(), Self::Error> {
        // Only keyframe requests are vouched for. Everything else — SR/RR, NACK, TWCC — is the
        // chain's own business and stops at the terminus as usual.
        if let Packet::Rtcp(rtcp_packets) = &msg.message.packet
            && rtcp_packets
                .iter()
                .any(|packet| is_keyframe_request(packet.as_ref()))
        {
            trace!(
                "RtcpForwarder: delivering PLI/FIR from {} to the application",
                msg.transport.peer_addr
            );
            msg.message.add(Attribute::DeliverToApplication);
        }

        self.read_queue.push_back(msg);
        Ok(())
    }

    fn poll_read(&mut self) -> Option<Self::Rout> {
        self.read_queue.pop_front()
    }

    fn handle_write(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        self.write_queue.push_back(msg);
        Ok(())
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        self.write_queue.pop_front()
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.read_queue.clear();
        self.write_queue.clear();
        Ok(())
    }
}

impl Interceptor for RtcpForwarderInterceptor {
    fn bind_local_stream(&mut self, _info: &StreamInfo) {}
    fn unbind_local_stream(&mut self, _info: &StreamInfo) {}
    fn bind_remote_stream(&mut self, _info: &StreamInfo) {}
    fn unbind_remote_stream(&mut self, _info: &StreamInfo) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc::rtcp::receiver_report::ReceiverReport;
    use rtc::shared::{TransportContext, TransportProtocol};
    use std::net::SocketAddr;

    fn inbound(packets: Vec<Box<dyn RtcpPacket>>) -> TaggedPacket {
        TaggedPacket {
            now: Instant::now(),
            transport: TransportContext {
                local_addr: "127.0.0.1:5000".parse::<SocketAddr>().unwrap(),
                peer_addr: "127.0.0.1:6000".parse::<SocketAddr>().unwrap(),
                ecn: None,
                transport_protocol: TransportProtocol::UDP,
            },
            message: Packet::Rtcp(packets).into(),
        }
    }

    /// The SFU vouches for keyframe requests, so they get past the chain's terminus.
    #[test]
    fn marks_keyframe_requests_for_the_application() {
        let mut forwarder = RtcpForwarderBuilder::new().build();

        forwarder
            .handle_read(inbound(vec![Box::new(PictureLossIndication::default())]))
            .expect("handle a PLI");

        let out = forwarder
            .poll_read()
            .expect("the packet carries on up the chain");
        assert!(
            out.message.has(&Attribute::DeliverToApplication),
            "a PLI must reach the application, or the SFU cannot relay it upstream"
        );
    }

    /// Everything else is the chain's own business and stops at the terminus as usual.
    #[test]
    fn leaves_other_rtcp_to_the_chain() {
        let mut forwarder = RtcpForwarderBuilder::new().build();

        forwarder
            .handle_read(inbound(vec![Box::new(ReceiverReport::default())]))
            .expect("handle a receiver report");

        let out = forwarder
            .poll_read()
            .expect("the packet still passes through");
        assert!(
            !out.message.has(&Attribute::DeliverToApplication),
            "reception reports are for the interceptors, not the application"
        );
    }

    /// A pass-through still has to hand packets back, or the write path swallows them.
    #[test]
    fn passes_the_write_path_through_untouched() {
        let mut forwarder = RtcpForwarderBuilder::new().build();

        forwarder
            .handle_write(inbound(vec![Box::new(PictureLossIndication::default())]))
            .expect("handle an outbound packet");

        let out = forwarder
            .poll_write()
            .expect("outbound packets must not be swallowed");
        assert!(
            !out.message.has(&Attribute::DeliverToApplication),
            "the write path is not the forwarder's business"
        );
    }
}
