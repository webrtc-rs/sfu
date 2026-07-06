use crate::demuxer::Demuxer;
use crate::event::Event;
use crate::room::{Room, RoomId};
use log::warn;
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

    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<Event>,
}

impl Sfu {
    pub fn new(id: SfuId, local_addr: SocketAddr) -> Self {
        Self {
            id,
            local_addr,

            demuxer: Default::default(),
            rooms: Default::default(),
            transmits: Default::default(),
            events: Default::default(),
        }
    }
}

impl Protocol<TaggedBytesMut, Infallible, Event> for Sfu {
    type Rout = Infallible;
    type Wout = TaggedBytesMut;
    type Eout = Event;
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
        None
    }

    fn handle_write(&mut self, _msg: Infallible) -> Result<(), Self::Error> {
        match _msg {}
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        for room in self.rooms.values_mut() {
            while let Some(transmit) = room.poll_write() {
                self.transmits.push_back(transmit);
            }
        }
        self.transmits.pop_front()
    }

    fn handle_event(&mut self, evt: Event) -> Result<(), Self::Error> {
        if let Some(room_id) = evt.room_id() {
            let mut remove_room = false;
            if let Some(room) = self.rooms.get_mut(&room_id) {
                let is_leave_event = matches!(evt, Event::Leave { .. });
                room.handle_event(evt)?;
                if is_leave_event && room.is_empty() {
                    remove_room = true;
                }
            } else if let Event::Join { .. } = &evt {
                let mut room = Room::new(room_id, self.local_addr);
                room.handle_event(evt)?;
                self.rooms.insert(room_id, room);
            }

            if remove_room {
                self.rooms.remove(&room_id);
            }
        } else if let Event::Err {
            request_id, reason, ..
        } = evt
        {
            warn!("{} receives err due to {}", request_id, reason);
        } else if let Event::Ok { request_id, .. } = evt {
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
        self.transmits.clear();
        self.events.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RequestId;
    use crate::event::Event;
    use rtc::peer_connection::RTCPeerConnectionBuilder;
    use rtc::peer_connection::configuration::media_engine::MediaEngine;
    use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
    use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;

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

        offerer
            .add_transceiver_from_kind(RtpCodecKind::Video, None)
            .expect("video transceiver should be added");

        let offer = offerer.create_offer(None).expect("offer should be created");
        assert_eq!(offer.sdp_type, RTCSdpType::Offer);
        assert!(!offer.sdp.is_empty());
        offer
    }

    fn join(sfu: &mut Sfu, request_id: RequestId) {
        sfu.handle_event(Event::Join {
            request_id,
            room_id: ROOM,
            client_id: CLIENT,
        })
        .expect("join should succeed");
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

        sfu.handle_event(Event::Leave {
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
        sfu.handle_event(Event::SessionDescription {
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
            Event::SessionDescription {
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

        // Only the answer is surfaced.
        assert!(sfu.poll_event().is_none());
    }
}
