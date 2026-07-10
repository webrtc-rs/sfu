use crate::client::ClientId;
use rtc::rtp_transceiver::{RTCRtpSenderId, SSRC};
use std::collections::{HashMap, HashSet};

/// Identity of one publishing track being forwarded: the publishing client plus the
/// **mid** of its m-line. The mid is stable across the publisher's renegotiations
/// (a stopped/muted track keeps its m-line and only flips direction), so it — not the
/// SSRC — is the correct dedup key for the forwarding graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ForwardKey {
    pub(crate) publisher: ClientId,
    pub(crate) mid: String,
}

/// The `(publisher mid) x (subscriber)` forwarding matrix, plus the wire-level routing
/// index that maps a publisher's RTP SSRC to its forward key.
///
/// For each publish track the matrix records which subscribers already have a forwarding
/// sender and the `RTCRtpSenderId` of that sender on the subscriber's peer connection
/// (needed both to tear it down and to route packets to it). This is the dedup state that
/// makes track extraction idempotent: a publisher re-offering the same tracks must not
/// add duplicate senders.
///
/// The SSRC index is what turns an inbound `RtpPacket`/`RtcpPacket` (identified on the
/// wire only by SSRC) into the set of subscriber senders it fans out to. It is seeded
/// from the SDP at reconcile time when the offer names the SSRC (`a=ssrc`), and completed
/// from the publisher's `OnTrack(OnOpen)` otherwise (bare m-line, RID-based simulcast).
#[derive(Debug, Default)]
pub(crate) struct ForwardTable {
    entries: HashMap<ForwardKey, HashMap<ClientId, RTCRtpSenderId>>,
    ssrc_index: HashMap<SSRC, ForwardKey>,
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

    /// Bind a publisher's wire SSRC to its forward key so inbound packets carrying that
    /// SSRC can be routed. Idempotent; re-binding an SSRC follows the publisher's latest
    /// negotiation. Called from reconcile (SSRC known from `a=ssrc`) or from the
    /// publisher's `OnTrack(OnOpen)` (packet-time binding for bare m-lines / simulcast
    /// RID layers — each simulcast layer's SSRC binds to the same key).
    pub(crate) fn bind_ssrc(&mut self, ssrc: SSRC, key: ForwardKey) {
        self.ssrc_index.insert(ssrc, key);
    }

    /// Resolve a packet's SSRC to its forward key and the subscriber senders it fans out
    /// to. `None` until the SSRC is bound and at least one subscriber sender exists.
    pub(crate) fn route_by_ssrc(
        &self,
        ssrc: SSRC,
    ) -> Option<(&ForwardKey, &HashMap<ClientId, RTCRtpSenderId>)> {
        let key = self.ssrc_index.get(&ssrc)?;
        let subscribers = self.entries.get(key)?;
        Some((key, subscribers))
    }

    /// Drop forwardings that are no longer wanted and collect their senders so the caller
    /// can `remove_track` them from the subscriber peer connections:
    ///   - the `(publisher, mid)` is no longer published (not in `desired`), or
    ///   - the publisher or the subscriber has left the room (not in `live_clients`).
    ///
    /// SSRC bindings whose key vanished are pruned with it. Everything still wanted is
    /// kept, so re-running this with an unchanged room is a no-op (the intersection case).
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

        let entries = &self.entries;
        self.ssrc_index.retain(|_, key| entries.contains_key(key));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.ssrc_index.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(publisher: ClientId, mid: &str) -> ForwardKey {
        ForwardKey {
            publisher,
            mid: mid.to_owned(),
        }
    }

    #[test]
    fn routes_bound_ssrc_to_subscriber_senders() {
        let mut table = ForwardTable::default();
        let k = key(1, "0");
        table.insert(k.clone(), 2, RTCRtpSenderId::from(7));
        table.bind_ssrc(1111, k.clone());

        let (routed_key, subscribers) = table.route_by_ssrc(1111).expect("ssrc should route");
        assert_eq!(routed_key, &k);
        assert_eq!(subscribers.get(&2), Some(&RTCRtpSenderId::from(7)));

        // Unbound SSRC routes nowhere.
        assert!(table.route_by_ssrc(2222).is_none());
    }

    #[test]
    fn simulcast_layers_bind_to_the_same_key() {
        let mut table = ForwardTable::default();
        let k = key(1, "0");
        table.insert(k.clone(), 2, RTCRtpSenderId::from(7));
        // Two layers, learned at packet time (OnTrack per RID).
        table.bind_ssrc(1111, k.clone());
        table.bind_ssrc(1112, k.clone());

        assert!(table.route_by_ssrc(1111).is_some());
        assert!(table.route_by_ssrc(1112).is_some());
    }

    #[test]
    fn retain_prunes_ssrc_bindings_with_their_key() {
        let mut table = ForwardTable::default();
        let k = key(1, "0");
        table.insert(k.clone(), 2, RTCRtpSenderId::from(7));
        table.bind_ssrc(1111, k.clone());

        // Publisher 1 gone: entry and its SSRC binding must both go.
        let mut removed = Vec::new();
        let desired = HashSet::from([k]);
        let live = HashSet::from([2]);
        table.retain(&desired, &live, &mut removed);

        assert_eq!(removed, vec![(2, RTCRtpSenderId::from(7))]);
        assert!(table.route_by_ssrc(1111).is_none());
        assert!(table.is_empty());
    }
}
