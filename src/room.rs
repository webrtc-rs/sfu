use crate::client::{Client, ClientBuilder, ClientEvent, ClientId, Mid};
use crate::demuxer::Demuxer;
use crate::event::SFUEvent;
use crate::forward::{ForwardKey, ForwardTable};
use log::{trace, warn};
use rtc::ice::rand::{generate_pwd, generate_ufrag};
use rtc::interceptor::Registry;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::setting_engine::SettingEngine;
use rtc::peer_connection::event::{RTCPeerConnectionEvent, RTCTrackEvent};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::transport::RTCDtlsRole;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Instant;

pub type RoomId = u64;

pub(crate) struct Room {
    id: RoomId,
    local_addr: SocketAddr,
    demuxer: Demuxer,
    clients: HashMap<ClientId, Client>,
    forward: ForwardTable,

    writes: VecDeque<TaggedBytesMut>,
    events: VecDeque<SFUEvent>,
}

impl Room {
    pub(crate) fn new(id: RoomId, local_addr: SocketAddr) -> Self {
        Self {
            id,
            local_addr,

            demuxer: Default::default(),
            clients: Default::default(),
            forward: Default::default(),
            writes: Default::default(),
            events: Default::default(),
        }
    }

    pub(crate) fn id(&self) -> RoomId {
        self.id
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Build a client with the default media engine (default codecs), the default
    /// interceptor chain, and default setting engine.
    fn build_client(&self, client_id: ClientId, room_id: RoomId) -> Result<Client, Error> {
        // USERNAME = local_ufrag ":" remote_ufrag
        // ufrag = 4*256ice-char // length range [4, 256]
        // ice-char = ALPHA / DIGIT / "+" / "/"
        let mut setting_engine = SettingEngine::default();
        setting_engine.set_ice_credentials(
            format!("{}/{}+{}", room_id, client_id, generate_ufrag()),
            generate_pwd(),
        );
        setting_engine.set_lite(true);
        // The SFU is ICE-lite (controlled) and DTLS-passive: it answers `a=setup:passive`
        // so the browser is the DTLS client and initiates the handshake (sends the
        // ClientHello) once ICE connects. Without this, the answer defaults to
        // `a=setup:active` (DTLS client) — a mismatch that deadlocks the handshake.
        setting_engine.set_answering_dtls_role(RTCDtlsRole::Server)?;
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
        ClientBuilder::new(client_id, room_id, self.local_addr)
            .with_setting_engine(setting_engine)
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build()
    }

    /// Reconcile the forwarding graph with the room's current publish state.
    ///
    /// For every publisher's live publish track, each *other* client should have exactly
    /// one forwarding sender. This diffs that desired matrix against the `ForwardTable`
    /// (keyed by `{publisher, mid}`) and only applies the delta:
    ///   - subscribers that left, or tracks no longer published → `remove_forwarding_track`,
    ///   - `(publisher, mid, subscriber)` cells not yet present → `add_forwarding_track`,
    ///   - everything already wired → left untouched.
    ///
    /// It is therefore idempotent: calling it after a publisher re-offers the same tracks
    /// adds nothing. Run it whenever publish state may have changed (join / leave / an
    /// applied session description).
    fn reconcile(&mut self) {
        // Snapshot publish state before mutating any client. `get_forward_tracks` reads
        // the negotiated receivers (needs `&mut`), so this collects owned tracks first
        // and releases the borrow before the add/remove passes below.
        let live: HashSet<ClientId> = self.clients.keys().copied().collect();
        let publishers: Vec<(ClientId, HashMap<Mid, MediaStreamTrack>)> = self
            .clients
            .iter_mut()
            .map(|(id, client)| (*id, client.get_forward_tracks()))
            .filter(|(_, tracks)| !tracks.is_empty())
            .collect();

        let desired: HashSet<ForwardKey> = publishers
            .iter()
            .flat_map(|(publisher, tracks)| {
                tracks.keys().map(move |mid| ForwardKey {
                    publisher: *publisher,
                    mid: mid.clone(),
                })
            })
            .collect();

        // 1. Tear down forwardings that are no longer wanted.
        let mut removed = Vec::new();
        self.forward.retain(&desired, &live, &mut removed);
        for (subscriber, sender) in removed {
            if let Some(client) = self.clients.get_mut(&subscriber)
                && let Err(err) = client.remove_forward_track(sender)
            {
                warn!("{}: failed to remove forwarding sender: {}", self.id, err);
            }
        }

        // 2. Add the forwardings that are missing. The publisher's track
        //    is forwarded verbatim onto a sendonly transceiver per subscriber.
        for (publisher, tracks) in &publishers {
            for (mid, track) in tracks {
                let key = ForwardKey {
                    publisher: *publisher,
                    mid: mid.clone(),
                };

                // Bind the publisher's wire SSRC(s) for packet routing (idempotent).
                // Tracks whose SSRC the SDP couldn't name (bare m-line, RID simulcast)
                // are bound later from the publisher's OnTrack(OnOpen) in poll_event.
                for ssrc in track.ssrcs() {
                    self.forward.bind_ssrc(ssrc, key.clone());
                }

                for &subscriber in &live {
                    if subscriber == *publisher || self.forward.has_subscriber(&key, &subscriber) {
                        continue;
                    }
                    if let Some(client) = self.clients.get_mut(&subscriber) {
                        match client.add_forward_track(track.clone()) {
                            Ok(sender) => self.forward.insert(key.clone(), subscriber, sender),
                            Err(err) => warn!(
                                "{}: failed to add forwarding {}->{} for mid {}: {}",
                                self.id, publisher, subscriber, mid, err
                            ),
                        }
                    }
                }
            }
        }
    }
}

impl Protocol<TaggedBytesMut, Infallible, SFUEvent> for Room {
    type Rout = Infallible;
    type Wout = TaggedBytesMut;
    type Eout = SFUEvent;
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, msg: TaggedBytesMut) -> Result<(), Self::Error> {
        if let Some((room_id, client_id)) = self.demuxer.demux(&msg) {
            if room_id != self.id {
                warn!(
                    "Invalid room {}'s message routed to room {}",
                    room_id, self.id
                );
                return Err(Error::Other(format!(
                    "Invalid room {}'s message routed to room {}",
                    self.id, self.id
                )));
            }

            if let Some(client) = self.clients.get_mut(&client_id) {
                client.handle_read(msg)?;
            } else {
                warn!("Received message for unknown client {}", client_id);
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
        let mut forwardings: HashMap<ClientId, VecDeque<RTCMessage>> = HashMap::new();
        for (client_id, client) in &mut self.clients {
            while let Some(msg) = client.poll_read() {
                if let RTCMessage::DataChannelMessage(data_channel_id, _) = &msg {
                    warn!(
                        "Drop data channel message for data channel id {}",
                        data_channel_id
                    );
                } else {
                    forwardings.entry(*client_id).or_default().push_back(msg);
                }
            }
        }

        // Selective forwarding: resolve each packet's SSRC through the forward table to
        // the per-subscriber senders it fans out to. Packets whose SSRC is not bound yet
        // (first packets of a bare-m-line/simulcast publish, racing OnTrack) are dropped
        // quietly — the binding lands in this same drive iteration via poll_event.
        for (client_id, mut reads) in forwardings.drain() {
            while let Some(msg) = reads.pop_front() {
                match &msg {
                    RTCMessage::RtpPacket(_, rtp_packet) => {
                        let ssrc = rtp_packet.header.ssrc;
                        let Some((key, subscribers)) = self.forward.route_by_ssrc(ssrc) else {
                            trace!(
                                "{}: no forward binding for rtp ssrc {} from {}",
                                self.id, ssrc, client_id
                            );
                            continue;
                        };
                        if key.publisher != client_id {
                            warn!(
                                "{}: rtp ssrc {} from {} is bound to publisher {} — dropping",
                                self.id, ssrc, client_id, key.publisher
                            );
                            continue;
                        }
                        for (subscriber, sender_id) in subscribers {
                            if let Some(peer) = self.clients.get_mut(subscriber)
                                && let Err(err) = peer.write_rtp(*sender_id, rtp_packet.clone())
                            {
                                warn!(
                                    "{}: {}->{} forward rtp ssrc {} err: {}",
                                    self.id, client_id, subscriber, ssrc, err
                                );
                            }
                        }
                    }
                    RTCMessage::RtcpPacket(_, rtcp_packets) => {
                        // Route by the SSRCs the compound packet describes (a publisher's
                        // SenderReport carries its media SSRC).
                        let route = rtcp_packets
                            .iter()
                            .flat_map(|packet| packet.destination_ssrc())
                            .find_map(|ssrc| {
                                self.forward
                                    .route_by_ssrc(ssrc)
                                    .map(|(key, subscribers)| (ssrc, key, subscribers))
                            });
                        let Some((ssrc, key, subscribers)) = route else {
                            trace!(
                                "{}: no forward binding for rtcp from {}",
                                self.id, client_id
                            );
                            continue;
                        };
                        if key.publisher != client_id {
                            // Subscriber feedback (RR/NACK/PLI) about a publisher's
                            // stream. Upstream feedback forwarding is TODO — the SFU's
                            // own interceptors already generate feedback on each leg.
                            trace!(
                                "{}: rtcp about ssrc {} from subscriber {} — not forwarded",
                                self.id, ssrc, client_id
                            );
                            continue;
                        }
                        for (subscriber, sender_id) in subscribers {
                            if let Some(peer) = self.clients.get_mut(subscriber)
                                && let Err(err) = peer.write_rtcp(*sender_id, rtcp_packets.clone())
                            {
                                warn!(
                                    "{}: {}->{} forward rtcp ssrc {} err: {}",
                                    self.id, client_id, subscriber, ssrc, err
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        None
    }

    fn handle_write(&mut self, _msg: Infallible) -> Result<(), Self::Error> {
        match _msg {}
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        for client in self.clients.values_mut() {
            while let Some(msg) = client.poll_write() {
                self.writes.push_back(msg);
            }
        }

        self.writes.pop_front()
    }

    fn handle_event(&mut self, evt: SFUEvent) -> Result<(), Self::Error> {
        let room_id = if let Some(room_id) = evt.room_id() {
            if room_id != self.id {
                return Err(Error::Other(format!("invalid room id: {}", room_id)));
            }
            room_id
        } else {
            return Err(Error::Other("empty room id".to_string()));
        };

        if let Some(client_id) = evt.client_id() {
            // Join, Leave, and applying remote description can all
            // change the room's publish state, so reconcile the forwarding graph after.
            let mut needs_reconcile = false;
            let mut remove_client = false;
            if let Some(client) = self.clients.get_mut(&client_id) {
                if let SFUEvent::Leave { .. } = &evt {
                    client.close()?;
                    remove_client = true;
                    needs_reconcile = true;
                } else {
                    needs_reconcile = matches!(evt, SFUEvent::SessionDescription { .. });
                    client.handle_event(ClientEvent::SFUEvent(evt))?;
                }
            } else if let SFUEvent::Join { .. } = &evt {
                let client = self.build_client(client_id, room_id)?;
                self.clients.insert(client_id, client);
                needs_reconcile = true;
            }

            if remove_client {
                self.clients.remove(&client_id);
            }

            if needs_reconcile {
                self.reconcile();
            }
        } else if let SFUEvent::Err {
            request_id, reason, ..
        } = evt
        {
            warn!("{}:{} receives err due to {}", request_id, room_id, reason);
        } else if let SFUEvent::Ok { request_id, .. } = evt {
            warn!("{}:{} receives ok", request_id, room_id,);
        }

        Ok(())
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
        for (client_id, client) in &mut self.clients {
            while let Some(event) = client.poll_event() {
                match event {
                    ClientEvent::SFUEvent(evt) => {
                        self.events.push_back(evt);
                    }
                    ClientEvent::PeerConnectionEvent(RTCPeerConnectionEvent::OnTrack(
                        RTCTrackEvent::OnOpen(init),
                    )) => {
                        // Packet-time SSRC binding: the definitive wire SSRC for publish
                        // streams the SDP couldn't name up front (bare m-line without
                        // `a=ssrc`, or RID-based simulcast — one OnOpen per layer, all
                        // binding to the same {publisher, mid} key).
                        if let Some(mid) = client.transceiver_mid(init.receiver_id) {
                            self.forward.bind_ssrc(
                                init.ssrc,
                                ForwardKey {
                                    publisher: *client_id,
                                    mid,
                                },
                            );
                        } else {
                            warn!(
                                "{}: OnTrack(OnOpen) ssrc {} from {} has no mid — not bound",
                                self.id, init.ssrc, client_id
                            );
                        }
                    }
                    ClientEvent::PeerConnectionEvent(_) => {
                        //TODO: remaining peer connection events
                    }
                }
            }
        }
        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> Result<(), Self::Error> {
        for client in self.clients.values_mut() {
            let _ = client.handle_timeout(now);
        }
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        let mut eto: Option<Instant> = None;
        for client in self.clients.values_mut() {
            if let Some(next) = client.poll_timeout() {
                eto = Some(eto.map_or(next, |curr| std::cmp::min(curr, next)));
            }
        }
        eto
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.clients.clear();
        self.forward.clear();
        self.writes.clear();
        self.events.clear();
        Ok(())
    }
}
