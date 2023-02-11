use async_trait::async_trait;
use std::sync::Arc;

use crate::rtc::server::server_states::ServerStates;

use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TaggedBytesMut;

/// match_range is a MatchFunc that accepts packets with the first byte in [lower..upper]
fn match_range(lower: u8, upper: u8, buf: &[u8]) -> bool {
    if buf.is_empty() {
        return false;
    }
    let b = buf[0];
    b >= lower && b <= upper
}

/// MatchFuncs as described in RFC7983
/// <https://tools.ietf.org/html/rfc7983>
///              +----------------+
///              |        [0..3] -+--> forward to STUN
///              |                |
///              |      [16..19] -+--> forward to ZRTP
///              |                |
///  packet -->  |      [20..63] -+--> forward to DTLS
///              |                |
///              |      [64..79] -+--> forward to TURN Channel
///              |                |
///              |    [128..191] -+--> forward to RTP/RTCP
///              +----------------+
/// match_dtls is a MatchFunc that accepts packets with the first byte in [20..63]
/// as defied in RFC7983
fn match_dtls(b: &[u8]) -> bool {
    match_range(20, 63, b)
}

/// match_srtp is a MatchFunc that accepts packets with the first byte in [128..191]
/// as defied in RFC7983
fn match_srtp(b: &[u8]) -> bool {
    match_range(128, 191, b)
}

struct UDPDemuxerDecoder {
    server_states: Arc<ServerStates>,
}
struct UDPDemuxerEncoder;

pub struct UDPDemuxerHandler {
    decoder: UDPDemuxerDecoder,
    encoder: UDPDemuxerEncoder,
}

impl UDPDemuxerHandler {
    pub fn new(server_states: Arc<ServerStates>) -> Self {
        UDPDemuxerHandler {
            decoder: UDPDemuxerDecoder { server_states },
            encoder: UDPDemuxerEncoder {},
        }
    }
}

#[async_trait]
impl InboundHandler for UDPDemuxerDecoder {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;

    async fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if match_dtls(&msg.message) {
            //TODO: dispatch the packet to Data (DTLS) Pipeline
            unimplemented!()
        } else if match_srtp(&msg.message) {
            //TODO: dispatch the packet to Media (RTP) Pipeline
        } else {
            // dispatch the packet to next handler for STUN (ICE) processing
            ctx.fire_read(msg).await;
        }
    }
}

#[async_trait]
impl OutboundHandler for UDPDemuxerEncoder {
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    async fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg).await;
    }
}

impl Handler for UDPDemuxerHandler {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "UDPDemuxerHandler"
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
