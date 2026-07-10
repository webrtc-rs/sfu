use crate::demuxer::Demuxer;
use crate::event::SFUEvent;
use crate::room::{Room, RoomId};
use log::{info, warn};
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Instant;

pub type SfuId = u64;

pub struct Sfu {
    id: SfuId,
    local_addr: SocketAddr,
    demuxer: Demuxer,
    rooms: HashMap<RoomId, Room>,

    writes: VecDeque<TaggedBytesMut>,
    events: VecDeque<SFUEvent>,
}

impl Sfu {
    pub fn new(id: SfuId, local_addr: SocketAddr) -> Self {
        Self {
            id,
            local_addr,

            demuxer: Default::default(),
            rooms: Default::default(),
            writes: Default::default(),
            events: Default::default(),
        }
    }
}

impl Protocol<TaggedBytesMut, Infallible, SFUEvent> for Sfu {
    type Rout = Infallible;
    type Wout = TaggedBytesMut;
    type Eout = SFUEvent;
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, msg: TaggedBytesMut) -> Result<(), Self::Error> {
        if let Some((room_id, _client_id)) = self.demuxer.demux(&msg) {
            if let Some(room) = self.rooms.get_mut(&room_id) {
                room.handle_read(msg)?;
            } else {
                warn!("Received message for unknown room {}", room_id);
            }
        } else {
            warn!(
                "unroutable message from {} to {}",
                msg.transport.peer_addr, msg.transport.local_addr
            );
        }
        Ok(())
    }

    fn poll_read(&mut self) -> Option<Self::Rout> {
        for room in self.rooms.values_mut() {
            while let Some(msg) = room.poll_read() {
                info!("process room's poll_read {:?}, should always be None", msg);
            }
        }
        None
    }

    fn handle_write(&mut self, _msg: Infallible) -> Result<(), Self::Error> {
        match _msg {}
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        for room in self.rooms.values_mut() {
            while let Some(msg) = room.poll_write() {
                self.writes.push_back(msg);
            }
        }
        self.writes.pop_front()
    }

    fn handle_event(&mut self, evt: SFUEvent) -> Result<(), Self::Error> {
        if let Some(room_id) = evt.room_id() {
            let mut remove_room = false;
            if let Some(room) = self.rooms.get_mut(&room_id) {
                let is_leave_event = matches!(evt, SFUEvent::Leave { .. });
                room.handle_event(evt)?;
                if is_leave_event && room.is_empty() {
                    remove_room = true;
                }
            } else if let SFUEvent::Join { .. } = &evt {
                let mut room = Room::new(room_id, self.local_addr);
                room.handle_event(evt)?;
                self.rooms.insert(room_id, room);
            }

            if remove_room {
                self.rooms.remove(&room_id);
            }
        } else if let SFUEvent::Err {
            request_id, reason, ..
        } = evt
        {
            warn!("{} receives err due to {}", request_id, reason);
        } else if let SFUEvent::Ok { request_id, .. } = evt {
            warn!("{} receives ok", request_id);
        }

        Ok(())
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
        for room in self.rooms.values_mut() {
            while let Some(event) = room.poll_event() {
                self.events.push_back(event);
            }
        }

        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> Result<(), Self::Error> {
        for room in self.rooms.values_mut() {
            let _ = room.handle_timeout(now);
        }
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        let mut eto: Option<Instant> = None;
        for room in self.rooms.values_mut() {
            if let Some(next) = room.poll_timeout() {
                eto = Some(eto.map_or(next, |curr| std::cmp::min(curr, next)));
            }
        }
        eto
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.rooms.clear();
        self.writes.clear();
        self.events.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RequestId;
    use crate::event::SFUEvent;
    use rtc::peer_connection::RTCPeerConnectionBuilder;
    use rtc::peer_connection::configuration::media_engine::MediaEngine;
    use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    };
    use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};

    const ROOM: RoomId = 100;
    const CLIENT: crate::ClientId = 200;

    /// A browser-side peer connection that publishes one video track, used only to
    /// produce a valid SDP offer to feed into the SFU.
    fn build_offer() -> RTCSessionDescription {
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");

        let mut offerer = RTCPeerConnectionBuilder::new()
            .with_media_engine(media_engine)
            .build()
            .expect("offerer peer connection should build");

        // Publish sendonly with an explicit SSRC, like the real browser (chat.html), so
        // the SFU answers recvonly rather than mirroring a sendrecv transceiver and
        // re-offering.
        offerer
            .add_transceiver_from_kind(
                RtpCodecKind::Video,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Sendonly,
                    streams: Vec::new(),
                    send_encodings: vec![RTCRtpEncodingParameters {
                        rtp_coding_parameters: RTCRtpCodingParameters {
                            ssrc: Some(111_111),
                            ..Default::default()
                        },
                        active: true,
                        ..Default::default()
                    }],
                }),
            )
            .expect("video transceiver should be added");

        let offer = offerer.create_offer(None).expect("offer should be created");
        assert_eq!(offer.sdp_type, RTCSdpType::Offer);
        assert!(!offer.sdp.is_empty());
        offer
    }

    fn build_offer_with_extra_video_codec(
        payload_type: u8,
        codec_name: &str,
    ) -> RTCSessionDescription {
        let mut offer = build_offer();
        let mut lines: Vec<String> = offer.sdp.split("\r\n").map(str::to_owned).collect();

        let video_line = lines
            .iter_mut()
            .find(|line| line.starts_with("m=video "))
            .expect("offer should contain a video m-line");
        video_line.push_str(&format!(" {payload_type}"));

        let insert_at = lines
            .iter()
            .rposition(|line| !line.is_empty())
            .map(|idx| idx + 1)
            .unwrap_or(lines.len());
        lines.insert(
            insert_at,
            format!("a=rtpmap:{payload_type} {codec_name}/90000"),
        );

        offer.sdp = lines.join("\r\n");
        offer
    }

    fn join(sfu: &mut Sfu, request_id: RequestId) {
        join_client(sfu, request_id, CLIENT);
    }

    fn join_client(sfu: &mut Sfu, request_id: RequestId, client_id: crate::ClientId) {
        sfu.handle_event(SFUEvent::Join {
            request_id,
            room_id: ROOM,
            client_id,
        })
        .expect("join should succeed");
    }

    fn drain_events(sfu: &mut Sfu) -> Vec<SFUEvent> {
        let mut events = Vec::new();
        while let Some(event) = sfu.poll_event() {
            events.push(event);
        }
        events
    }

    #[test]
    fn join_creates_room_and_client() {
        let mut sfu = Sfu::new(0, "0.0.0.0:0".parse().unwrap());
        assert!(sfu.rooms.is_empty());

        join(&mut sfu, 1);

        let room = sfu.rooms.get(&ROOM).expect("room should exist after join");
        assert_eq!(room.id(), ROOM);
        assert!(!room.is_empty(), "room should contain the joined client");
    }

    #[test]
    fn leave_removes_client_and_reaps_empty_room() {
        let mut sfu = Sfu::new(0, "0.0.0.0:0".parse().unwrap());
        join(&mut sfu, 1);
        assert!(sfu.rooms.contains_key(&ROOM));

        sfu.handle_event(SFUEvent::Leave {
            request_id: 2,
            room_id: ROOM,
            client_id: CLIENT,
            reason: "bye".to_string(),
        })
        .expect("leave should succeed");

        // The last client left, so the SFU self-reaps the now-empty room.
        assert!(
            !sfu.rooms.contains_key(&ROOM),
            "empty room should be removed after the last client leaves"
        );
    }

    #[test]
    fn session_description_offer_returns_answer() {
        let mut sfu = Sfu::new(0, "0.0.0.0:0".parse().unwrap());
        join(&mut sfu, 1);

        let request_id: RequestId = 2;
        sfu.handle_event(SFUEvent::SessionDescription {
            request_id,
            room_id: ROOM,
            client_id: CLIENT,
            sdp: build_offer(),
        })
        .expect("handling the offer should succeed");

        let event = sfu
            .poll_event()
            .expect("the SFU should emit an answer for the offer");

        match event {
            SFUEvent::SessionDescription {
                request_id: got_request_id,
                room_id,
                client_id,
                sdp,
            } => {
                assert_eq!(got_request_id, request_id);
                assert_eq!(room_id, ROOM);
                assert_eq!(client_id, CLIENT);
                assert_eq!(
                    sdp.sdp_type,
                    RTCSdpType::Answer,
                    "the SFU should answer an offer"
                );
                assert!(!sdp.sdp.is_empty(), "the answer SDP should not be empty");
            }
            other => panic!("expected a SessionDescription answer, got {:?}", other),
        }

        // Only the answer is surfaced (a lone sendonly publisher has no subscribers, so
        // reconcile adds no forwarding senders and no subscribe offer is produced).
        assert!(sfu.poll_event().is_none());
    }

    /// The forwarding track carries every codec the publisher advertised (one per coding),
    /// so the subscriber's server-initiated offer advertises them all — not just the
    /// primary — letting it receive whichever codec the publisher actually sends.
    #[test]
    fn subscribe_offer_advertises_all_publisher_codecs() {
        const SUBSCRIBER: crate::ClientId = 300;

        let mut sfu = Sfu::new(0, "0.0.0.0:0".parse().unwrap());
        join_client(&mut sfu, 1, CLIENT);
        join_client(&mut sfu, 2, SUBSCRIBER);

        sfu.handle_event(SFUEvent::SessionDescription {
            request_id: 3,
            room_id: ROOM,
            client_id: CLIENT,
            sdp: build_offer(),
        })
        .expect("handling the publisher offer should succeed");

        let events = drain_events(&mut sfu);
        let offer = events
            .iter()
            .find_map(|event| match event {
                SFUEvent::SessionDescription { client_id, sdp, .. }
                    if *client_id == SUBSCRIBER && sdp.sdp_type == RTCSdpType::Offer =>
                {
                    Some(&sdp.sdp)
                }
                _ => None,
            })
            .expect("subscriber should receive a server-initiated offer");

        // The default video media engine registers many codecs; the forwarded m-line must
        // advertise more than the single primary one.
        let codec_count = offer.matches("a=rtpmap:").count();
        assert!(
            codec_count > 1,
            "subscribe offer should advertise all publisher codecs, got {codec_count} rtpmap(s)"
        );
    }

    #[test]
    fn publish_triggers_subscribe_offer_to_other_client() {
        const SUBSCRIBER: crate::ClientId = 300;

        let mut sfu = Sfu::new(0, "0.0.0.0:0".parse().unwrap());
        join_client(&mut sfu, 1, CLIENT);
        join_client(&mut sfu, 2, SUBSCRIBER);

        // CLIENT publishes one sendonly video track.
        sfu.handle_event(SFUEvent::SessionDescription {
            request_id: 3,
            room_id: ROOM,
            client_id: CLIENT,
            sdp: build_offer(),
        })
        .expect("handling the publisher offer should succeed");

        let events = drain_events(&mut sfu);

        // The publisher gets its answer...
        assert!(
            events.iter().any(|e| matches!(
                e,
                SFUEvent::SessionDescription { client_id, sdp, .. }
                    if *client_id == CLIENT && sdp.sdp_type == RTCSdpType::Answer
            )),
            "publisher should receive an answer, got {events:?}"
        );

        // ...and reconcile forwards the track to the subscriber, whose peer connection
        // fires OnNegotiationNeeded, producing a subscribe *offer* addressed to it.
        assert!(
            events.iter().any(|e| matches!(
                e,
                SFUEvent::SessionDescription { client_id, sdp, .. }
                    if *client_id == SUBSCRIBER && sdp.sdp_type == RTCSdpType::Offer
            )),
            "subscriber should receive a server-initiated offer, got {events:?}"
        );
    }

    #[test]
    fn subscribe_offer_filters_unsupported_publisher_codecs() {
        const SUBSCRIBER: crate::ClientId = 300;
        const UNSUPPORTED_PT: u8 = 123;

        let mut sfu = Sfu::new(0, "0.0.0.0:0".parse().unwrap());
        join_client(&mut sfu, 1, CLIENT);
        join_client(&mut sfu, 2, SUBSCRIBER);

        sfu.handle_event(SFUEvent::SessionDescription {
            request_id: 3,
            room_id: ROOM,
            client_id: CLIENT,
            sdp: build_offer_with_extra_video_codec(UNSUPPORTED_PT, "UNSUPPORTED"),
        })
        .expect("handling the publisher offer should succeed");

        let events = drain_events(&mut sfu);
        let subscribe_offer = events
            .iter()
            .find_map(|event| match event {
                SFUEvent::SessionDescription { client_id, sdp, .. }
                    if *client_id == SUBSCRIBER && sdp.sdp_type == RTCSdpType::Offer =>
                {
                    Some(&sdp.sdp)
                }
                _ => None,
            })
            .expect("subscriber should still receive a server-initiated offer");

        assert!(
            !subscribe_offer.contains(&format!("a=rtpmap:{UNSUPPORTED_PT} UNSUPPORTED/90000")),
            "subscribe offer should not advertise unsupported passthrough codecs: {subscribe_offer}"
        );
    }

    #[test]
    fn republish_same_offer_is_idempotent() {
        const SUBSCRIBER: crate::ClientId = 300;

        let mut sfu = Sfu::new(0, "0.0.0.0:0".parse().unwrap());
        join_client(&mut sfu, 1, CLIENT);
        join_client(&mut sfu, 2, SUBSCRIBER);

        let offer = build_offer();
        sfu.handle_event(SFUEvent::SessionDescription {
            request_id: 3,
            room_id: ROOM,
            client_id: CLIENT,
            sdp: offer.clone(),
        })
        .expect("first publish should succeed");
        let first = drain_events(&mut sfu);
        let first_subscribe_offers = first
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SFUEvent::SessionDescription { client_id, sdp, .. }
                        if *client_id == SUBSCRIBER && sdp.sdp_type == RTCSdpType::Offer
                )
            })
            .count();
        assert_eq!(first_subscribe_offers, 1, "first publish forwards once");

        // Re-applying the same publish offer must not add a duplicate forwarding sender,
        // so no new subscribe offer is generated (reconcile is idempotent).
        sfu.handle_event(SFUEvent::SessionDescription {
            request_id: 4,
            room_id: ROOM,
            client_id: CLIENT,
            sdp: offer,
        })
        .expect("re-publish should succeed");
        let second = drain_events(&mut sfu);
        let second_subscribe_offers = second
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    SFUEvent::SessionDescription { client_id, sdp, .. }
                        if *client_id == SUBSCRIBER && sdp.sdp_type == RTCSdpType::Offer
                )
            })
            .count();
        assert_eq!(
            second_subscribe_offers, 0,
            "re-publishing the same track must not re-forward, got {second:?}"
        );
    }
}
