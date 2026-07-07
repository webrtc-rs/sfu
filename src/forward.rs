use crate::client::ClientId;
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp_transceiver::RTCRtpSenderId;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use std::collections::{HashMap, HashSet};

/// One forward track advertised by a client's *remote* description (i.e. media the
/// client is sending toward the SFU). Extracted at `set_remote_description` time — as
/// soon as the offer is applied, before any RTP arrives — so the SFU can immediately set
/// up the subscribe forwarding to other clients (see `Room::reconcile`).
#[derive(Debug, Clone)]
pub(crate) struct ForwardTrack {
    /// m-line id — stable across the publisher's renegotiations; the forwarding dedup key.
    pub(crate) mid: String,
    pub(crate) kind: RtpCodecKind,
    /// Primary SSRC from `a=ssrc`, reused verbatim on the forwarding sender.
    pub(crate) ssrc: u32,
    pub(crate) stream_id: String,
    pub(crate) track_id: String,
}

impl ForwardTrack {
    /// Build the media track the SFU sends to a subscriber for one publisher track. The
    /// publisher's SSRC is reused verbatim (the design forwards RTP without rewriting SSRC /
    /// payload type); the codec is left default so the subscriber peer connection offers all
    /// registered codecs of that kind and negotiates one with the browser.
    pub(crate) fn build_forward_track(&self, publisher: ClientId) -> MediaStreamTrack {
        let stream_id = if self.stream_id.is_empty() {
            format!("stream-{}-{}", publisher, self.mid)
        } else {
            self.stream_id.clone()
        };
        let track_id = if self.track_id.is_empty() {
            format!("track-{}-{}", publisher, self.mid)
        } else {
            self.track_id.clone()
        };

        MediaStreamTrack::new(
            stream_id,
            track_id,
            format!("{}-{}", publisher, self.mid),
            self.kind,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(self.ssrc),
                    ..Default::default()
                },
                active: true,
                codec: RTCRtpCodec::default(),
                ..Default::default()
            }],
        )
    }
}

/// Identity of one publishing track being forwarded: the publishing client plus the
/// **mid** of its m-line. The mid is stable across the publisher's renegotiations
/// (a stopped/muted track keeps its m-line and only flips direction), so it — not the
/// SSRC — is the correct dedup key for the forwarding graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ForwardKey {
    pub(crate) publisher: ClientId,
    pub(crate) mid: String,
}

/// The `(publisher mid) x (subscriber)` forwarding matrix.
///
/// For each publish track it records which subscribers already have a forwarding sender
/// and the `RTCRtpSenderId` of that sender on the subscriber's peer connection (needed to
/// tear it down). This is the dedup state that makes track extraction idempotent: a
/// publisher re-offering the same tracks must not add duplicate senders.
#[derive(Debug, Default)]
pub(crate) struct ForwardTable {
    entries: HashMap<ForwardKey, HashMap<ClientId, RTCRtpSenderId>>,
}

impl ForwardTable {
    /// Whether `subscriber` already has a forwarding sender for `key`.
    pub(crate) fn has_subscriber(&self, key: &ForwardKey, subscriber: &ClientId) -> bool {
        self.entries
            .get(key)
            .is_some_and(|subs| subs.contains_key(subscriber))
    }

    /// Record a newly created forwarding sender.
    pub(crate) fn insert(
        &mut self,
        key: ForwardKey,
        subscriber: ClientId,
        sender_id: RTCRtpSenderId,
    ) {
        self.entries
            .entry(key)
            .or_default()
            .insert(subscriber, sender_id);
    }

    /// Drop forwardings that are no longer wanted and collect their senders so the caller
    /// can `remove_track` them from the subscriber peer connections:
    ///   - the `(publisher, mid)` is no longer published (not in `desired`), or
    ///   - the publisher or the subscriber has left the room (not in `live_clients`).
    ///
    /// Everything still wanted is kept, so re-running this with an unchanged room is a
    /// no-op (the intersection case).
    pub(crate) fn retain(
        &mut self,
        desired: &HashSet<ForwardKey>,
        live_clients: &HashSet<ClientId>,
        removed: &mut Vec<(ClientId, RTCRtpSenderId)>,
    ) {
        self.entries.retain(|key, subs| {
            let key_alive = desired.contains(key) && live_clients.contains(&key.publisher);
            subs.retain(|subscriber, sender| {
                let keep = key_alive && live_clients.contains(subscriber);
                if !keep {
                    removed.push((*subscriber, *sender));
                }
                keep
            });
            !subs.is_empty()
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
