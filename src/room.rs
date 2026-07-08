use crate::client::{Client, ClientBuilder, ClientEvent, ClientId, Mid};
use crate::demuxer::Demuxer;
use crate::event::SFUEvent;
use crate::forward::{ForwardKey, ForwardTable, ForwardTrack};
use log::warn;
use rtc::ice::rand::{generate_pwd, generate_ufrag};
use rtc::interceptor::Registry;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::setting_engine::SettingEngine;
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
        let publishers: Vec<(ClientId, HashMap<Mid, ForwardTrack>)> = self
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
                for &subscriber in &live {
                    if subscriber == *publisher || self.forward.has_subscriber(&key, &subscriber) {
                        continue;
                    }
                    if let Some(client) = self.clients.get_mut(&subscriber) {
                        match client.add_forward_track(track.build()) {
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

        for (client_id, mut reads) in forwardings.drain() {
            while let Some(msg) = reads.pop_front() {
                for (peer_id, peer) in &mut self.clients {
                    //TODO: Selective Forwarding RTP Packets by using ForwardTable?
                    if client_id != *peer_id
                        && let Err(err) = peer.handle_write(msg.clone())
                    {
                        warn!(
                            "{}: {}->{} forward packet got err: {}",
                            self.id, client_id, peer_id, err
                        );
                    }
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
        for client in self.clients.values_mut() {
            while let Some(event) = client.poll_event() {
                match event {
                    ClientEvent::SFUEvent(evt) => {
                        self.events.push_back(evt);
                    }
                    ClientEvent::PeerConnectionEvent(_) => {
                        //TODO:
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
