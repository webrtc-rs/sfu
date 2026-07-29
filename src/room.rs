use crate::client::{Client, ClientBuilder, ClientEvent, ClientId, Mid};
use crate::demuxer::Demuxer;
use crate::event::SFUEvent;
use crate::forward::{ForwardKey, ForwardTable};
use crate::rtcp_forwarder::RtcpForwarderBuilder;
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
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
use uuid::Uuid;

/// Rooms are identified by a UUID minted by the signaling authority. The SFU treats it as
/// an opaque, `Copy` identifier: it never interprets the layout, and only requires a
/// rendering that is legal inside an ICE ufrag (see [`Room::build_client`]).
pub type RoomId = Uuid;

/// Render a room id as the string that appears as base64, unpadded.
fn encode_room_id(room_id: &RoomId) -> String {
    STANDARD_NO_PAD.encode(room_id.as_bytes())
}

/// Parse a room id from base64, unpadded.
fn decode_room_id(token: &str) -> Option<RoomId> {
    let bytes = STANDARD_NO_PAD.decode(token).ok()?;
    let bytes: [u8; 16] = bytes.try_into().ok()?;
    let room_id = Uuid::from_bytes(bytes);

    Some(room_id)
}

// The SFU has one UDP socket per media shard, so an arriving packet has to say which room
// and client it belongs to. ICE gives us exactly one field to carry that: the ufrag the
// browser echoes in every STUN binding request. These two functions are the only places
// that know its layout, and they must stay inverses of each other.
//
//   USERNAME        = local_ufrag ":" remote_ufrag
//   ufrag           = 4*256ice-char                  // length range [4, 256]
//   ice-char        = ALPHA / DIGIT / "+" / "/"
//   local_ufrag     = base64_room_id "/" digit_client_id "+" alpha_ufrag
//   base64_room_id  = ALPHA / DIGIT / "+" / "/"
//   digit_client_id = DIGIT
//   alpha_ufrag     = ALPHA

/// Build the local ufrag that names `room_id`/`client_id`, with a random suffix so two
/// clients of one room never share credentials.
pub(crate) fn encode_local_ufrag(room_id: &RoomId, client_id: ClientId) -> String {
    format!(
        "{}/{}+{}",
        encode_room_id(room_id), // ALPHA / DIGIT / "+" / "/"
        client_id,
        generate_ufrag()
    )
}

/// Recover the room and client a local ufrag names, or `None` if it was not built by
/// [`encode_local_ufrag`].
///
/// Both separators can also occur *inside* the base64 room id, so the split works from
/// the right: the last `+` is the one before the alphabetic suffix, and the last `/` the
/// one before the decimal client id. Splitting from the left would truncate the room id.
pub(crate) fn decode_local_ufrag(local_ufrag: &str) -> Option<(RoomId, ClientId)> {
    let (room_str, client_str) = local_ufrag.rsplit_once('+')?.0.rsplit_once('/')?;
    Some((decode_room_id(room_str)?, client_str.parse().ok()?))
}

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
        let mut setting_engine = SettingEngine::default();
        setting_engine.set_ice_credentials(encode_local_ufrag(&room_id, client_id), generate_pwd());
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
            // Map each extension the packet carries to the subscriber's negotiated id, matching by
            // uri: publisher id -> uri (pub_exts) -> subscriber id (sub_exts). Extensions the
            // subscriber didn't negotiate are dropped; the sdes:mid payload is replaced with the
            // subscriber's own mid.
            let mut translated: Vec<(u8, ::bytes::Bytes)> = Vec::new();
            let ids = forwarded_rtp.header.get_extension_ids();
            for id in &ids {
                let Some(payload) = forwarded_rtp.header.get_extension(*id) else {
                    continue;
                };
                let Some(pub_param) = pub_exts.iter().find(|pe| pe.id as u8 == *id) else {
                    continue;
                };
                let Some(sub_param) = sub_exts.iter().find(|se| se.uri == pub_param.uri) else {
                    continue;
                };
                let payload = if sub_param.uri == SDES_MID_URI
                    && let Some(mid) = subscriber_mid
                {
                    ::bytes::Bytes::copy_from_slice(mid.as_bytes())
                } else {
                    payload
                };
                // TODO: If we ever support forwarding multiple simulcast layers to a subscriber
                //  that negotiated RID/RRID, we would need to map the publisher's RID/RRID payload
                //  values to the subscriber's corresponding layer IDs here.
                //  "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id",
                //  "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id",
                translated.push((sub_param.id as u8, payload));
            }

            // Rebuild the extensions through the public API so `extensions_padding` and the
            // extension flag stay consistent — a direct `header.extensions = …` assignment leaves
            // them stale and breaks marshalling. Clear all first to avoid id collisions during the
            // remap (an old id may be another extension's new id).
            for id in ids {
                let _ = forwarded_rtp.header.del_extension(id);
            }
            for (id, payload) in translated {
                let _ = forwarded_rtp.header.set_extension(id, payload);
            }
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
            let subscriber_extensions = peer.rtp_sender(*sender_id).map(|mut sender| {
                sender
                    .get_parameters()
                    .rtp_parameters
                    .header_extensions
                    .clone()
            });
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
            let write_result = peer
                .rtp_sender(*sender_id)
                .ok_or(Error::ErrRTPSenderNotExisted)
                .and_then(|mut sender| sender.write_rtp(forwarded_rtp));
            if let Err(err) = write_result {
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
                && let Err(err) = peer
                    .rtp_sender(*sender_id)
                    .ok_or(Error::ErrRTPSenderNotExisted)
                    .and_then(|mut sender| sender.write_rtcp(rtcp_packets.to_vec()))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A room id whose base64 contains **both** ufrag separators (`dzax4qN2/q/IfVWKlhcc+w`),
    /// which is the case the `{room}/{client}+{ufrag}` scheme has to survive.
    const ADVERSARIAL: &str = "7736b1e2-a376-feaf-c87d-558a96171cfb";

    fn uuid(text: &str) -> RoomId {
        RoomId::parse_str(text).expect("test uuid")
    }

    #[test]
    fn round_trips_every_uuid_shape() {
        let ids = [
            RoomId::nil(),
            RoomId::max(),
            RoomId::from_u128(42), // the numeric room ids the chat example widens
            RoomId::from_u128(u64::MAX as u128),
            uuid(ADVERSARIAL),
            uuid("a2b40a86-cc37-fc72-37d5-4940fc52c41e"), // two slashes
            RoomId::from_bytes([0x7f; 16]),
            RoomId::from_bytes([
                0x00, 0xff, 0x10, 0xef, 0x20, 0xdf, 0x30, 0xcf, 0x40, 0xbf, 0x50, 0xaf, 0x60, 0x9f,
                0x70, 0x8f,
            ]),
        ];
        for id in ids {
            let token = encode_room_id(&id);
            assert_eq!(token.len(), 22, "16 bytes is always 22 unpadded characters");
            assert_eq!(
                decode_room_id(&token),
                Some(id),
                "round trip failed for {id}"
            );
        }
    }

    #[test]
    fn decodes_only_sixteen_byte_payloads() {
        let token = encode_room_id(&uuid(ADVERSARIAL));
        for invalid in [
            "",                         // empty
            "AAAA",                     // 3 bytes
            &token[..token.len() - 1],  // 21 characters: not a whole 16 bytes
            &format!("{token}AA"),      // 24 characters: 18 bytes
            &format!("{token}{token}"), // 32 bytes
        ] {
            assert_eq!(decode_room_id(invalid), None, "accepted {invalid:?}");
        }
    }

    #[test]
    fn decodes_only_the_canonical_spelling() {
        let id = uuid(ADVERSARIAL);
        let token = encode_room_id(&id);

        // 128 bits is not a multiple of 6, so the final character carries 2 significant
        // bits and 4 that must be zero. If those were ignored, sixteen different strings
        // would decode to this same UUID — and since rooms are keyed by the decoded value,
        // a client could reach one room under a spelling the server never issued.
        let alphabet: Vec<char> =
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
                .chars()
                .collect();
        let last = alphabet
            .iter()
            .position(|c| *c == token.chars().last().unwrap())
            .expect("final character is in the alphabet");
        let mut twins = 0;
        for bump in 1..16 {
            let twin: String = token[..token.len() - 1]
                .chars()
                .chain(std::iter::once(alphabet[last + bump]))
                .collect();
            assert_ne!(twin, token);
            assert_eq!(decode_room_id(&twin), None, "accepted twin {twin:?}");
            twins += 1;
        }
        assert_eq!(
            twins, 15,
            "every non-canonical trailing-bit spelling was tested"
        );
    }

    #[test]
    fn decodes_only_the_standard_alphabet() {
        // The room id shares the ufrag with `+` and `/` separators, so this codec uses the
        // standard alphabet rather than base64url. A URL-safe spelling of the same bytes
        // must not decode, or two encodings would name the same room.
        let id = uuid(ADVERSARIAL);
        let url_safe = encode_room_id(&id).replace('+', "-").replace('/', "_");
        assert_ne!(url_safe, encode_room_id(&id));
        assert_eq!(decode_room_id(&url_safe), None);

        // Padding is not part of the encoding either.
        assert_eq!(decode_room_id(&format!("{}==", encode_room_id(&id))), None);
        assert_eq!(decode_room_id("not base64!!!!!!!!!!!!"), None);
    }

    #[test]
    fn local_ufrag_round_trips_through_both_separators() {
        // Standard base64 can contain `+` and `/`, which are also this scheme's own
        // separators, so a room id carrying both is the case that has to survive.
        for id in [
            uuid(ADVERSARIAL),                            // base64 holds both `+` and `/`
            uuid("a2b40a86-cc37-fc72-37d5-4940fc52c41e"), // two slashes
            RoomId::nil(),
            RoomId::max(),
            RoomId::from_u128(42),
        ] {
            for client_id in [0 as ClientId, 1, 537_821_252, ClientId::MAX] {
                let local_ufrag = encode_local_ufrag(&id, client_id);
                assert_eq!(
                    decode_local_ufrag(&local_ufrag),
                    Some((id, client_id)),
                    "round trip failed for {local_ufrag}"
                );

                // RFC 8839: ufrag = 4*256(ALPHA / DIGIT / "+" / "/").
                assert!((4..=256).contains(&local_ufrag.len()));
                assert!(
                    local_ufrag
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/'),
                    "ufrag has a character ICE does not allow: {local_ufrag}"
                );
            }
        }
        let token = encode_room_id(&uuid(ADVERSARIAL));
        assert!(
            token.contains('+') && token.contains('/'),
            "the fixture no longer exercises both separators: {token}"
        );
    }

    #[test]
    fn local_ufrag_decoding_rejects_malformed_input() {
        let token = encode_room_id(&uuid(ADVERSARIAL));
        for invalid in [
            String::new(),                    // empty
            token.clone(),                    // no separators at all
            format!("{token}/7"),             // missing the random suffix
            format!("{token}+abc"),           // missing the client id
            format!("{token}/notdigits+abc"), // client id is not a number
            format!("{token}/-1+abc"),        // negative client id
            "zzzz/7+abc".to_string(),         // room id is not 16 bytes
        ] {
            assert_eq!(decode_local_ufrag(&invalid), None, "accepted {invalid:?}");
        }
    }

    #[test]
    fn local_ufrag_is_unique_per_client() {
        // The random suffix is what stops two clients of one room from sharing ICE
        // credentials, so the same inputs must not produce the same ufrag twice.
        let id = uuid(ADVERSARIAL);
        let first = encode_local_ufrag(&id, 7);
        let second = encode_local_ufrag(&id, 7);
        assert_ne!(first, second);
        assert_eq!(decode_local_ufrag(&first), decode_local_ufrag(&second));
    }
}
