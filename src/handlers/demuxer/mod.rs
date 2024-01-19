use crate::messages::{
    DTLSMessageEvent, MessageEvent, RTPMessageEvent, STUNMessageEvent, TaggedMessageEvent,
};
use log::{debug, error};
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

#[derive(Default)]
struct DemuxerInbound;
#[derive(Default)]
struct DemuxerOutbound;

#[derive(Default)]
pub struct DemuxerHandler {
    demuxer_inbound: DemuxerInbound,
    demuxer_outbound: DemuxerOutbound,
}

impl DemuxerHandler {
    pub fn new() -> Self {
        DemuxerHandler::default()
    }
}

impl InboundHandler for DemuxerInbound {
    type Rin = TaggedBytesMut;
    type Rout = TaggedMessageEvent;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if msg.message.is_empty() {
            error!("drop invalid packet due to zero length");
        } else if match_dtls(&msg.message) {
            ctx.fire_read(TaggedMessageEvent {
                now: msg.now,
                transport: msg.transport,
                message: MessageEvent::DTLS(DTLSMessageEvent::RAW(msg.message)),
            });
        } else if match_srtp(&msg.message) {
            ctx.fire_read(TaggedMessageEvent {
                now: msg.now,
                transport: msg.transport,
                message: MessageEvent::RTP(RTPMessageEvent::RAW(msg.message)),
            });
        } else {
            ctx.fire_read(TaggedMessageEvent {
                now: msg.now,
                transport: msg.transport,
                message: MessageEvent::STUN(STUNMessageEvent::RAW(msg.message)),
            });
        }
    }
}

impl OutboundHandler for DemuxerOutbound {
    type Win = TaggedMessageEvent;
    type Wout = TaggedBytesMut;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        match msg.message {
            MessageEvent::STUN(STUNMessageEvent::RAW(message))
            | MessageEvent::DTLS(DTLSMessageEvent::RAW(message))
            | MessageEvent::RTP(RTPMessageEvent::RAW(message)) => {
                ctx.fire_write(TaggedBytesMut {
                    now: msg.now,
                    transport: msg.transport,
                    message,
                });
            }
            _ => {
                debug!("drop non-RAW packet {:?}", msg.message);
            }
        }
    }
}

impl Handler for DemuxerHandler {
    type Rin = TaggedBytesMut;
    type Rout = TaggedMessageEvent;
    type Win = TaggedMessageEvent;
    type Wout = TaggedBytesMut;

    fn name(&self) -> &str {
        "DemuxerHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (
            Box::new(self.demuxer_inbound),
            Box::new(self.demuxer_outbound),
        )
    }
}
