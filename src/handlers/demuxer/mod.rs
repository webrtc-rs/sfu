use async_trait::async_trait;
use std::sync::Arc;

use crate::server::ServerStates;

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

struct DemuxerDecoder {
    server_states: Arc<ServerStates>,
}
struct DemuxerEncoder;

pub struct DemuxerHandler {
    decoder: DemuxerDecoder,
    encoder: DemuxerEncoder,
}

impl DemuxerHandler {
    pub fn new(server_states: Arc<ServerStates>) -> Self {
        DemuxerHandler {
            decoder: DemuxerDecoder { server_states },
            encoder: DemuxerEncoder {},
        }
    }
}

#[async_trait]
impl InboundHandler for DemuxerDecoder {
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
impl OutboundHandler for DemuxerEncoder {
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    async fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg).await;
    }
}

impl Handler for DemuxerHandler {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "DemuxerHandler"
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
