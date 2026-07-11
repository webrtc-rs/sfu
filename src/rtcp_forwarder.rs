//! An interceptor that surfaces inbound RTCP to the application via `poll_read()`.
//!
//! The default interceptor chain consumes inbound RTCP (for NACK, reports, TWCC, etc.), so
//! it never reaches the application's `poll_read()`. The SFU, however, needs to see a
//! subscriber's keyframe requests (PLI/FIR) about a forwarded stream in order to relay them
//! to the publisher. Installed as the outermost layer, this interceptor queues a copy of
//! every inbound RTCP packet for the application while still passing the original down the
//! chain for the default processing.

use log::trace;
use rtc::interceptor::{Interceptor, Packet, StreamInfo, TaggedPacket, interceptor};
use rtc::rtcp::Packet as RtcpPacket;
use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::sansio;
use rtc::shared::error::Error;
use std::collections::VecDeque;

/// Builder for [`RtcpForwarderInterceptor`], plugged into a `Registry` via `.with(...)`.
pub(crate) struct RtcpForwarderBuilder<P> {
    _phantom: std::marker::PhantomData<P>,
}

impl<P> Default for RtcpForwarderBuilder<P> {
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<P> RtcpForwarderBuilder<P> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn build(self) -> impl FnOnce(P) -> RtcpForwarderInterceptor<P> {
        move |inner| RtcpForwarderInterceptor::new(inner)
    }
}

/// Copies inbound RTCP to the application's `poll_read()` without consuming it from the
/// normal interceptor chain.
#[derive(Interceptor)]
pub(crate) struct RtcpForwarderInterceptor<P> {
    #[next]
    next: P,
    read_queue: VecDeque<TaggedPacket>,
}

impl<P> RtcpForwarderInterceptor<P> {
    fn new(next: P) -> Self {
        Self {
            next,
            read_queue: VecDeque::new(),
        }
    }
}

#[interceptor]
impl<P: Interceptor> RtcpForwarderInterceptor<P> {
    #[overrides]
    fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        // Surface only keyframe requests (PLI/FIR) to the application — those are the RTCP
        // the SFU relays upstream to publishers. Everything else (SR/RR/NACK/TWCC) is left
        // to the default chain and not duplicated to poll_read.
        if let Packet::Rtcp(rtcp_packets) = &msg.message {
            let keyframe_requests: Vec<Box<dyn RtcpPacket>> = rtcp_packets
                .iter()
                .filter(|packet| {
                    let any = packet.as_any();
                    any.is::<PictureLossIndication>() || any.is::<FullIntraRequest>()
                })
                .map(|packet| packet.cloned())
                .collect();
            if !keyframe_requests.is_empty() {
                trace!(
                    "RtcpForwarder: surfacing {} PLI/FIR from {} to the application",
                    keyframe_requests.len(),
                    msg.transport.peer_addr
                );
                self.read_queue.push_back(TaggedPacket {
                    now: msg.now,
                    transport: msg.transport,
                    message: Packet::Rtcp(keyframe_requests),
                });
            }
        }
        // Always pass the original down the chain for normal processing.
        self.next.handle_read(msg)
    }

    #[overrides]
    fn poll_read(&mut self) -> Option<Self::Rout> {
        if let Some(pkt) = self.read_queue.pop_front() {
            return Some(pkt);
        }
        self.next.poll_read()
    }

    #[overrides]
    fn close(&mut self) -> Result<(), Self::Error> {
        self.read_queue.clear();
        self.next.close()
    }
}
