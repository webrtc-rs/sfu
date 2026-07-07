use crate::client::ClientId;
use rtc::rtp_transceiver::RTCRtpSenderId;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use std::collections::HashMap;

/// One publishing track advertised by a client's *remote* description (i.e. media the
/// client is sending toward the SFU). Extracted at `set_remote_description` time — as
/// soon as the offer is applied, before any RTP arrives — so the SFU can immediately set
/// up the subscribe forwarding to other clients (see `Room::reconcile`).
#[derive(Debug, Clone)]
pub(crate) struct PublishingTrack {
    /// m-line id — stable across the publisher's renegotiations; the forwarding dedup key.
    pub(crate) mid: String,
    pub(crate) kind: RtpCodecKind,
    /// Primary SSRC from `a=ssrc`, reused verbatim on the forwarding sender.
    pub(crate) ssrc: u32,
    pub(crate) stream_id: String,
    pub(crate) track_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ForwardKey {
    publisher: ClientId,
    track: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ForwardEntry {
    subscribers: HashMap<ClientId, RTCRtpSenderId>,
}

#[derive(Debug, Default)]
pub(crate) struct ForwardTable {
    entries: HashMap<ForwardKey, ForwardEntry>,
}

impl ForwardTable {
    pub(crate) fn entry_mut(&mut self, key: ForwardKey) -> &mut ForwardEntry {
        self.entries.entry(key).or_default()
    }

    pub(crate) fn get(&self, key: &ForwardKey) -> Option<&ForwardEntry> {
        self.entries.get(key)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
