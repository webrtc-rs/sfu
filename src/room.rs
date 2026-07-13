use crate::client::{Client, ClientBuilder, ClientEvent, ClientId, Mid};
use crate::demuxer::Demuxer;
use crate::event::SFUEvent;
use crate::forward::{ForwardKey, ForwardTable};
use crate::rtcp_forwarder::RtcpForwarderBuilder;
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
use rtc::rtcp::Packet;
use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp_transceiver::rtp_sender::RTCRtpHeaderExtensionParameters;
use rtc::sdp::extmap::SDES_MID_URI;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::{Error, flatten_errs};
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
        // Outermost layer: surface inbound RTCP (a subscriber's PLI/FIR keyframe requests)
        // to poll_read so the SFU can relay them upstream to the publisher; the default
        // chain would otherwise consume RTCP before the application sees it.
        let registry = registry.with(RtcpForwarderBuilder::new().build());
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

    /// Build the RTP packet to forward to one subscriber.
    ///
    /// Clones `rtp_packet`, rewrites its payload type to the subscriber's negotiated
    /// `outbound_payload_type`, and translates each RTP header extension id from the publisher's
    /// negotiated id to the subscriber's — matching extensions by uri, dropping any the subscriber
    /// did not negotiate, and stamping the subscriber's own m-line `mid` as the payload of the
    /// `sdes:mid` extension. The header-extension translation only runs when *both* the publisher and
    /// the subscriber negotiated header extensions for this stream; otherwise the cloned packet's
    /// extensions are forwarded untouched.
    fn translate_rtp_for_subscriber(
        rtp_packet: &rtc::rtp::Packet,
        outbound_payload_type: u8,
        publisher_extensions: Option<&[RTCRtpHeaderExtensionParameters]>,
        subscriber_extensions: Option<&[RTCRtpHeaderExtensionParameters]>,
        subscriber_mid: Option<&str>,
    ) -> rtc::rtp::Packet {
        let mut forwarded_rtp = rtp_packet.clone();
        forwarded_rtp.header.payload_type = outbound_payload_type;

        if let (Some(pub_exts), Some(sub_exts)) = (publisher_extensions, subscriber_extensions) {
            let mut new_extensions = Vec::new();
            for ext in &forwarded_rtp.header.extensions {
                if let Some(pub_param) = pub_exts.iter().find(|pe| pe.id as u8 == ext.id)
                    && let Some(sub_param) = sub_exts.iter().find(|se| se.uri == pub_param.uri)
                {
                    let mut new_ext = ext.clone();
                    new_ext.id = sub_param.id as u8;
                    if sub_param.uri == SDES_MID_URI
                        && let Some(mid) = subscriber_mid
                    {
                        new_ext.payload = ::bytes::Bytes::copy_from_slice(mid.as_bytes());
                    }
                    // TODO: If we ever support forwarding multiple simulcast layers to a
                    //  subscriber that negotiated RID/RRID, we would need to map the publisher's
                    //  RID/RRID payload values to the subscriber's corresponding layer IDs here.
                    //  "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id",
                    //  "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id",
                    new_extensions.push(new_ext);
                }
            }
            forwarded_rtp.header.extensions = new_extensions;
            forwarded_rtp.header.extension = !forwarded_rtp.header.extensions.is_empty();
        }

        forwarded_rtp
    }

    /// Forward one publisher RTP packet to every subscriber bound to its SSRC, translating the
    /// payload type and header extension ids for each subscriber leg (see
    /// [`translate_rtp_for_subscriber`]). Drops the packet if its SSRC is not (yet) bound, or if
    /// it arrives from a client other than the SSRC's bound publisher.
    fn forward_rtp(&mut self, client_id: ClientId, rtp_packet: &rtc::rtp::Packet) {
        let ssrc = rtp_packet.header.ssrc;
        let Some((key, subscribers)) = self.forward.route_by_ssrc(ssrc) else {
            trace!(
                "{}: no forward binding for rtp ssrc {} from {}",
                self.id, ssrc, client_id
            );
            return;
        };
        if key.publisher != client_id {
            warn!(
                "{}: rtp ssrc {} from {} is bound to publisher {} — dropping",
                self.id, ssrc, client_id, key.publisher
            );
            return;
        }

        let inbound_payload_type = rtp_packet.header.payload_type;
        let Some(incoming_codec) = self
            .clients
            .get_mut(&client_id)
            .and_then(|publisher| publisher.incoming_codec_for_rtp(ssrc, inbound_payload_type))
        else {
            warn!(
                "{}: unable to resolve incoming codec for {} rtp ssrc {} pt {}",
                self.id, client_id, ssrc, inbound_payload_type
            );
            return;
        };
        let publisher_extensions = self
            .clients
            .get_mut(&client_id)
            .and_then(|publisher| publisher.incoming_header_extensions_for_rtp(ssrc));

        for (subscriber, sender_id) in subscribers {
            let Some(peer) = self.clients.get_mut(subscriber) else {
                continue;
            };
            // Skip subscribers whose transport isn't up yet: the SRTP context isn't set until
            // DTLS completes, so forwarding now would just be dropped ("local_srtp_context is not
            // set yet"). Once connected, the subscriber requests a keyframe and media flows.
            if !peer.is_connected() {
                continue;
            }
            let Some(outbound_payload_type) =
                peer.outgoing_payload_type_for_codec(*sender_id, &incoming_codec)
            else {
                warn!(
                    "{}: unable to map codec {} for {}->{} rtp ssrc {} via sender {:?}",
                    self.id,
                    incoming_codec.mime_type.as_str(),
                    client_id,
                    subscriber,
                    ssrc,
                    sender_id
                );
                continue;
            };
            // The subscriber leg's negotiated header extensions and this sender's m-line mid
            // drive the header-extension-id translation.
            let subscriber_extensions = peer
                .sender_parameters(*sender_id)
                .map(|params| params.rtp_parameters.header_extensions);
            let subscriber_mid = peer.transceiver_mid(*sender_id);

            let forwarded_rtp = Room::translate_rtp_for_subscriber(
                rtp_packet,
                outbound_payload_type,
                publisher_extensions.as_deref(),
                subscriber_extensions.as_deref(),
                subscriber_mid.as_deref(),
            );

            trace!(
                "{}: {}->{} forward rtp ssrc {} pt {} -> {} via sender {:?} codec {}",
                self.id,
                client_id,
                subscriber,
                ssrc,
                inbound_payload_type,
                outbound_payload_type,
                sender_id,
                incoming_codec.mime_type.as_str()
            );
            if let Err(err) = peer.write_rtp(*sender_id, forwarded_rtp) {
                warn!(
                    "{}: {}->{} forward rtp ssrc {} err: {}",
                    self.id, client_id, subscriber, ssrc, err
                );
            }
        }
    }

    /// Route one publisher's compound RTCP to the subscribers of the stream it describes, and
    /// relay a subscriber's keyframe requests (PLI/FIR) upstream to the publisher. Drops RTCP
    /// whose SSRC routes to no forwarding entry.
    fn forward_rtcp(&mut self, client_id: ClientId, rtcp_packets: &[Box<dyn Packet>]) {
        // Route by the SSRCs the compound packet describes (a publisher's SenderReport carries
        // its media SSRC).
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
            return;
        };
        if key.publisher != client_id {
            // RTCP from a subscriber is feedback about a publisher's stream; relay its keyframe
            // requests upstream.
            let publisher_id = key.publisher;
            self.relay_keyframe_request(client_id, publisher_id, ssrc, rtcp_packets);
            return;
        }
        for (subscriber, sender_id) in subscribers {
            // Only forward once the subscriber's transport is up (see forward_rtp).
            if let Some(peer) = self.clients.get_mut(subscriber)
                && peer.is_connected()
                && let Err(err) = peer.write_rtcp(*sender_id, rtcp_packets.to_vec())
            {
                warn!(
                    "{}: {}->{} forward rtcp ssrc {} err: {}",
                    self.id, client_id, subscriber, ssrc, err
                );
            }
        }
    }

    /// Relay a subscriber's keyframe requests (PLI/FIR) about `ssrc` upstream to `publisher_id`,
    /// so the publisher's encoder emits a keyframe; without this a subscriber that renegotiates or
    /// drops a frame freezes until the publisher's next natural keyframe. RR/NACK are left to the
    /// SFU's per-leg interceptors. Non-PLI/FIR feedback (and feedback for a departed publisher) is
    /// dropped.
    fn relay_keyframe_request(
        &mut self,
        subscriber_id: ClientId,
        publisher_id: ClientId,
        ssrc: u32,
        rtcp_packets: &[Box<dyn Packet>],
    ) {
        let keyframe_requests: Vec<Box<dyn Packet>> = rtcp_packets
            .iter()
            .filter(|packet| {
                let any = packet.as_any();
                any.is::<PictureLossIndication>() || any.is::<FullIntraRequest>()
            })
            .map(|packet| packet.cloned())
            .collect();
        if keyframe_requests.is_empty() {
            trace!(
                "{}: rtcp from subscriber {} about publisher {} ssrc {} carries no PLI/FIR — ignored",
                self.id, subscriber_id, publisher_id, ssrc
            );
            return;
        }
        trace!(
            "{}: subscriber {} -> publisher {} keyframe request ({} PLI/FIR) for ssrc {}",
            self.id,
            subscriber_id,
            publisher_id,
            keyframe_requests.len(),
            ssrc
        );
        let Some(publisher) = self.clients.get_mut(&publisher_id) else {
            trace!(
                "{}: publisher {} no longer in room — keyframe request for ssrc {} dropped",
                self.id, publisher_id, ssrc
            );
            return;
        };
        if let Err(err) = publisher.request_keyframe(ssrc, keyframe_requests) {
            warn!(
                "{}: failed to forward keyframe request to publisher {} for ssrc {}: {}",
                self.id, publisher_id, ssrc, err
            );
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
                    RTCMessage::RtpPacket(_, rtp_packet) => self.forward_rtp(client_id, rtp_packet),
                    RTCMessage::RtcpPacket(_, rtcp_packets) => {
                        self.forward_rtcp(client_id, rtcp_packets)
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
                needs_reconcile = false;
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
        let mut errs: Vec<Error> = vec![];
        for client in self.clients.values_mut() {
            if let Err(err) = client.handle_timeout(now) {
                errs.push(err);
            }
        }
        flatten_errs(errs)
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
