use crate::room::RoomId;
use crate::{RequestId, SFUEvent};
use log::{trace, warn};
use rtc::ice::candidate::CandidateConfig;
use rtc::interceptor::{BoxedInterceptor, Interceptor, Registry};
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::RTCPeerConnection;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::peer_connection::configuration::RTCConfiguration;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::setting_engine::SettingEngine;
use rtc::peer_connection::event::RTCPeerConnectionEvent;
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
use rtc::peer_connection::state::{RTCPeerConnectionState, RTCSignalingState};
use rtc::peer_connection::transport::{CandidateHostConfig, RTCIceCandidate};
use rtc::rtcp;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RTCRtpHeaderExtensionParameters,
    RTCRtpReceiveParameters, RTCRtpSendParameters,
};
use rtc::rtp_transceiver::{
    RTCRtpReceiverId, RTCRtpSenderId, RTCRtpTransceiverDirection, RTCRtpTransceiverId,
    RTCRtpTransceiverInit, SSRC,
};
use rtc::sdp::MediaDescription;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::{Error, Result, flatten_errs};
use sansio::Protocol;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::time::{Duration, Instant};

/// The interceptor chain is assembled at runtime, so its type is erased to
/// [`BoxedInterceptor`]: every client's peer connection then has the same concrete type,
/// [`RTCPeerConnection<BoxedInterceptor>`], and neither this builder nor [`Client`] has to
/// be generic over the chain.
pub(crate) struct ClientBuilder {
    id: ClientId,
    room_id: RoomId,
    local_addr: SocketAddr,
    peer_connection_builder: RTCPeerConnectionBuilder<BoxedInterceptor>,
}

impl ClientBuilder {
    pub(crate) fn new(id: ClientId, room_id: RoomId, local_addr: SocketAddr) -> Self {
        Self {
            id,
            room_id,
            local_addr,
            peer_connection_builder: RTCPeerConnectionBuilder::new()
                .with_interceptor_registry(Registry::new().boxed()),
        }
    }

    pub(crate) fn with_configuration(mut self, configuration: RTCConfiguration) -> Self {
        self.peer_connection_builder = self
            .peer_connection_builder
            .with_configuration(configuration);
        self
    }

    pub(crate) fn with_media_engine(mut self, media_engine: MediaEngine) -> Self {
        self.peer_connection_builder = self.peer_connection_builder.with_media_engine(media_engine);
        self
    }

    pub(crate) fn with_setting_engine(mut self, setting_engine: SettingEngine) -> Self {
        self.peer_connection_builder = self
            .peer_connection_builder
            .with_setting_engine(setting_engine);
        self
    }

    pub(crate) fn with_interceptor_registry<P>(mut self, interceptor_registry: Registry<P>) -> Self
    where
        P: Interceptor + 'static,
    {
        self.peer_connection_builder = self
            .peer_connection_builder
            .with_interceptor_registry(interceptor_registry.boxed());
        self
    }

    pub(crate) fn build(self) -> Result<Client> {
        Ok(Client {
            id: self.id,
            room_id: self.room_id,
            local_addr: self.local_addr,
            peer_connection: self.peer_connection_builder.build()?,

            next_request_id: 0,
            curr_request_id: None,
            renegotiation_pending: false,
            signaling_state: RTCSignalingState::Stable,
            client_negotiated: false,
            connection_state: RTCPeerConnectionState::New,

            reads: Default::default(),
            writes: Default::default(),
            events: Default::default(),
        })
    }
}

pub type ClientId = u64;

/// SDP media identification tag (`a=mid`) of one m-line — the stable key for a publish
/// track across renegotiations.
pub(crate) type Mid = String;

//TODO: make it configurable
const ONGOING_NEGOTIATION_TIMEOUT_IN_SECOND: Duration = Duration::from_secs(5);

pub(crate) struct Client {
    id: ClientId,
    room_id: RoomId,
    local_addr: SocketAddr,
    peer_connection: RTCPeerConnection<BoxedInterceptor>,

    next_request_id: RequestId,
    curr_request_id: Option<(RequestId, Instant)>,

    /// Set when the peer connection reports negotiation is needed (a forwarding transceiver was
    /// added/removed) but an offer can't be sent yet. The deferred renegotiation is re-driven
    /// once the peer connection returns to a stable signaling state. See
    /// [`Client::drive_pending_renegotiation`].
    renegotiation_pending: bool,

    /// Latest `OnSignalingStateChangeEvent` — the SFU only creates a subscribe-renegotiation
    /// offer from `Stable`, never while it is answering the subscriber's publish
    /// (`HaveRemoteOffer`) or awaiting an answer to a prior offer (`HaveLocalOffer`).
    signaling_state: RTCSignalingState,

    /// Whether the SFU has answered the client's first offer, completing the initial round of
    /// SDP negotiation. The SFU must never make the *first* offer — only the client can. Until
    /// the client's first offer is answered (which is also when the SFU learns the client's
    /// codec and RTP-header-extension id assignments), no subscribe re-offer is created, so a
    /// forward's m-lines can adopt ids consistent with the client's own m-lines.
    client_negotiated: bool,

    /// Latest `OnConnectionStateChangeEvent` — tracked so the room only forwards media once the
    /// subscriber's transport (ICE + DTLS/SRTP) is ready. See [`Client::is_connected`].
    connection_state: RTCPeerConnectionState,

    reads: VecDeque<RTCMessage>,
    writes: VecDeque<TaggedBytesMut>,
    events: VecDeque<ClientEvent>,
}

impl Deref for Client {
    type Target = RTCPeerConnection<BoxedInterceptor>;

    fn deref(&self) -> &Self::Target {
        &self.peer_connection
    }
}

impl DerefMut for Client {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.peer_connection
    }
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum ClientEvent {
    SFUEvent(SFUEvent),
    PeerConnectionEvent(RTCPeerConnectionEvent),
}

impl Protocol<TaggedBytesMut, RTCMessage, ClientEvent> for Client {
    type Rout = RTCMessage;
    type Wout = TaggedBytesMut;
    type Eout = ClientEvent;
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, msg: TaggedBytesMut) -> std::result::Result<(), Self::Error> {
        self.peer_connection.handle_read(msg)
    }

    fn poll_read(&mut self) -> Option<Self::Rout> {
        while let Some(msg) = self.peer_connection.poll_read() {
            self.reads.push_back(msg);
        }
        self.reads.pop_front()
    }

    fn handle_write(&mut self, msg: RTCMessage) -> std::result::Result<(), Self::Error> {
        self.peer_connection.handle_write(msg)
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        while let Some(msg) = self.peer_connection.poll_write() {
            self.writes.push_back(msg);
        }

        self.writes.pop_front()
    }

    fn handle_event(&mut self, evt: ClientEvent) -> std::result::Result<(), Self::Error> {
        match evt {
            ClientEvent::SFUEvent(evt) => self.handle_sfu_event(evt),
            ClientEvent::PeerConnectionEvent(_) => Ok(()),
        }
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
        while let Some(evt) = self.peer_connection.poll_event() {
            match evt {
                RTCPeerConnectionEvent::OnNegotiationNeededEvent => {
                    trace!(
                        "[{}/{}] got negotiation needed event",
                        self.room_id, self.id
                    );

                    if let Err(err) = self.on_negotiation_needed() {
                        warn!(
                            "{}:{} failed to create renegotiation offer: {}",
                            self.room_id, self.id, err
                        );
                    }
                }
                RTCPeerConnectionEvent::OnSignalingStateChangeEvent(state) => {
                    self.signaling_state = state;
                    // Returning to stable frees the connection for the SFU's next offer: drive
                    // any renegotiation deferred while it was answering the subscriber's publish
                    // or awaiting an answer to a prior offer.
                    if state == RTCSignalingState::Stable
                        && let Err(err) = self.drive_pending_renegotiation()
                    {
                        warn!(
                            "{}:{} failed to drive deferred renegotiation: {}",
                            self.room_id, self.id, err
                        );
                    }
                }
                other => {
                    if let RTCPeerConnectionEvent::OnConnectionStateChangeEvent(state) = &other {
                        self.connection_state = *state;
                    }
                    self.events
                        .push_back(ClientEvent::PeerConnectionEvent(other));
                }
            }
        }

        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> std::result::Result<(), Self::Error> {
        let mut errs: Vec<Error> = vec![];

        if let Err(err) = self.peer_connection.handle_timeout(now) {
            errs.push(err);
        }

        if let Some((_, next_timeout)) = self.curr_request_id.as_ref()
            && next_timeout <= &now
        {
            if let Some(mut sdp) = self.peer_connection.local_description() {
                sdp.sdp_type = RTCSdpType::Rollback;
                if let Err(err) = self.peer_connection.set_local_description(sdp) {
                    errs.push(err);
                }
            }

            // mark current negotiation done
            self.curr_request_id = None;
            if let Err(err) = self.mark_curr_negotiation_complete() {
                errs.push(err);
            }
        }

        flatten_errs(errs)
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        let mut eto: Option<Instant> = self.peer_connection.poll_timeout();
        if let Some((_, next)) = self.curr_request_id.as_ref() {
            eto = Some(eto.map_or(*next, |curr| std::cmp::min(curr, *next)));
        }
        eto
    }

    fn close(&mut self) -> std::result::Result<(), Self::Error> {
        self.peer_connection.close()
    }
}

impl Client {
    /// Whether this client's transport is fully connected (ICE + DTLS/SRTP established). The
    /// room only forwards media to a subscriber once this is true; forwarding earlier would just
    /// be dropped by the SRTP layer (`local_srtp_context is not set yet`).
    pub(crate) fn is_connected(&self) -> bool {
        self.connection_state == RTCPeerConnectionState::Connected
    }

    pub(crate) fn incoming_codec_for_rtp(
        &mut self,
        ssrc: u32,
        payload_type: u8,
    ) -> Option<RTCRtpCodec> {
        let receiver_ids: Vec<RTCRtpReceiverId> = self.peer_connection.get_receivers().collect();
        for receiver_id in receiver_ids {
            let Some(mut receiver) = self.peer_connection.rtp_receiver(receiver_id) else {
                continue;
            };
            if !receiver
                .track()
                .ssrcs()
                .any(|track_ssrc| track_ssrc == ssrc)
            {
                continue;
            }

            if let Some(codec) =
                Client::codec_for_payload_type(receiver.get_parameters(), payload_type)
            {
                return Some(codec);
            }
        }

        None
    }

    /// The header extensions this client negotiated on the receiver whose track carries `ssrc`,
    /// as sent by the publisher. Used to map the publisher's extension ids to a subscriber's on
    /// forward. Mirrors [`Client::incoming_codec_for_rtp`].
    pub(crate) fn incoming_header_extensions_for_rtp(
        &mut self,
        ssrc: u32,
    ) -> Option<Vec<RTCRtpHeaderExtensionParameters>> {
        let receiver_ids: Vec<RTCRtpReceiverId> = self.peer_connection.get_receivers().collect();
        for receiver_id in receiver_ids {
            let Some(mut receiver) = self.peer_connection.rtp_receiver(receiver_id) else {
                continue;
            };
            if !receiver
                .track()
                .ssrcs()
                .any(|track_ssrc| track_ssrc == ssrc)
            {
                continue;
            }

            return Some(
                receiver
                    .get_parameters()
                    .rtp_parameters
                    .header_extensions
                    .clone(),
            );
        }

        None
    }

    pub(crate) fn outgoing_payload_type_for_codec(
        &mut self,
        sender_id: RTCRtpSenderId,
        codec: &RTCRtpCodec,
    ) -> Option<u8> {
        let mut sender = self.peer_connection.rtp_sender(sender_id)?;
        Client::payload_type_for_codec(sender.get_parameters(), codec)
    }

    /// The tracks this client is sending toward the SFU, keyed by m-line `mid`, ready to
    /// be forwarded. The base track for each receiving m-line comes from the negotiated
    /// receiver's track identity (`receiver.track()`) plus its negotiated receive codec
    /// list (`receiver.get_parameters()`), which is already filtered to what this core
    /// supports even before `OnTrack` populates deferred track codings. The remote
    /// description then supplies the send-side SSRC (`a=ssrc`) for m-lines whose
    /// negotiated codings don't carry one yet.
    ///
    /// The SSRCs on the returned tracks seed the SSRC-based routing in the forward table
    /// (`Room::reconcile` binds them). A track without any SSRC (a bare m-line without
    /// `a=ssrc`, or RID-based simulcast) is **still returned** so the subscriber leg is
    /// negotiated immediately; its SSRC routing entry is deferred until the publisher's
    /// `OnTrack(OnOpen)` delivers the wire SSRC (`Room::poll_event` binds it).
    ///
    /// `get_receivers()` lists only currently-receiving m-lines, so the SFU's own sendonly
    /// forwarding transceivers and muted/stopped publishes (`recvonly`/`inactive`) drop
    /// out here, and data-channel sections never appear.
    pub(crate) fn get_forward_tracks(&mut self) -> HashMap<Mid, MediaStreamTrack> {
        let mut tracks = HashMap::new();

        let parsed = self
            .peer_connection
            .remote_description()
            .and_then(|remote| remote.unmarshal().ok());

        let receiver_ids: Vec<RTCRtpReceiverId> = self.peer_connection.get_receivers().collect();
        for receiver_id in receiver_ids {
            let Some(mid) = self.transceiver_mid(receiver_id) else {
                continue;
            };
            // Cloned out so the peer connection borrow ends before
            // `track_with_codings_from_media_description` takes `&self`.
            let Some((track, parameters)) =
                self.peer_connection.rtp_receiver(receiver_id).map(|mut r| {
                    let track = r.track().clone();
                    (track, r.get_parameters().clone())
                })
            else {
                continue;
            };

            if let Some(media) = parsed.as_ref().and_then(|parsed| {
                parsed
                    .media_descriptions
                    .iter()
                    .find(|media| media.attribute("mid").flatten() == Some(mid.as_str()))
            }) {
                tracks.insert(
                    mid,
                    self.track_with_codings_from_media_description(track, &parameters, media),
                );
            }
        }
        tracks
    }

    /// Rebuild `track` from the receiver's negotiated parameters, then fill in the publish-side
    /// primary SSRC (`a=ssrc`) from the remote description when it is known.
    ///
    /// The forwarded track's stream/track ids are stamped with the publishing client's id
    /// (`peer-<client_id>`), so the msid the SFU forwards carries the publisher's identity to
    /// every subscriber — the browser never has to embed it. The publisher's original track id is
    /// kept as a suffix to keep each track's id unique within the peer's stream.
    ///
    /// This keeps only codecs the SFU side already matched as supported instead of copying
    /// every raw offered codec from the browser m-line, which can include codecs the
    /// forwarding sender cannot advertise. If receiver parameters have not exposed any
    /// codec yet, fall back to whatever coding metadata the track already carries.
    ///
    /// A simulcast track (negotiated RID codings) is returned unchanged — its per-layer
    /// SSRCs are only knowable at packet time (`OnTrack(OnOpen)`).
    /// TODO: merge codecs into simulcast RID codings.
    fn track_with_codings_from_media_description(
        &self,
        track: MediaStreamTrack,
        parameters: &RTCRtpReceiveParameters,
        media: &MediaDescription,
    ) -> MediaStreamTrack {
        // `a=ssrc:<ssrc> ...` — primary (first) SSRC; `None` for a bare m-line, whose
        // wire SSRC is bound later from OnTrack(OnOpen).
        let ssrc = media
            .attribute("ssrc")
            .flatten()
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u32>().ok());

        let codings = parameters
            .rtp_parameters
            .codecs
            .iter()
            .map(|codec| RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc,
                    ..Default::default()
                },
                active: true,
                codec: codec.rtp_codec.clone(),
                ..Default::default()
            })
            .collect();

        MediaStreamTrack::new(
            format!("peer-{}-{}", self.id, track.stream_id()),
            format!("peer-{}-{}", self.id, track.track_id()),
            format!("peer-{}-{}", self.id, track.label()),
            track.kind(),
            codings,
        )
    }

    fn codec_for_payload_type(
        parameters: &RTCRtpReceiveParameters,
        payload_type: u8,
    ) -> Option<RTCRtpCodec> {
        parameters
            .rtp_parameters
            .codecs
            .iter()
            .find(|codec| codec.payload_type == payload_type)
            .map(|codec| codec.rtp_codec.clone())
    }

    fn payload_type_for_codec(
        parameters: &RTCRtpSendParameters,
        codec: &RTCRtpCodec,
    ) -> Option<u8> {
        parameters
            .rtp_parameters
            .codecs
            .iter()
            .find(|candidate| {
                candidate
                    .rtp_codec
                    .mime_type
                    .eq_ignore_ascii_case(&codec.mime_type)
                    && candidate.rtp_codec.sdp_fmtp_line == codec.sdp_fmtp_line
            })
            .or_else(|| {
                parameters.rtp_parameters.codecs.iter().find(|candidate| {
                    candidate
                        .rtp_codec
                        .mime_type
                        .eq_ignore_ascii_case(&codec.mime_type)
                })
            })
            .map(|matched| matched.payload_type)
    }

    /// The mid of the m-line a transceiver belongs to — used by `Room` to bind a
    /// packet-time SSRC (from `OnTrack(OnOpen)`) to its `ForwardKey`.
    pub(crate) fn transceiver_mid(
        &mut self,
        transceiver_id: impl Into<RTCRtpTransceiverId>,
    ) -> Option<Mid> {
        self.rtp_transceiver(transceiver_id.into())?.mid().clone()
    }

    /// Whether the forwarding sender's transceiver direction is active for sending media
    /// (e.g. `Sendonly` or `Sendrecv`, not `Inactive` or `Recvonly`).
    pub(crate) fn is_sender_active(&mut self, sender_id: RTCRtpSenderId) -> bool {
        self.rtp_transceiver(sender_id.into())
            .is_some_and(|t| t.current_direction().has_send())
    }

    /// Add a forwarding sender for another client's publish track. Uses a dedicated
    /// `Sendonly` transceiver (a new m-line per forwarded source, mirroring the old SFU)
    /// rather than `add_track`, which would recycle the client's own receive transceiver.
    /// Adding it triggers `OnNegotiationNeededEvent` → a subscribe offer.
    pub(crate) fn add_forward_track(&mut self, track: MediaStreamTrack) -> Result<RTCRtpSenderId> {
        let transceiver_id = self.peer_connection.add_transceiver_from_track(
            track,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Sendonly,
                streams: Vec::new(),
                send_encodings: Vec::new(),
            }),
        )?;
        Ok(RTCRtpSenderId::from(transceiver_id))
    }

    /// Tear down a forwarding sender (publisher gone / track no longer published). Also
    /// triggers renegotiation.
    pub(crate) fn remove_forward_track(&mut self, sender_id: RTCRtpSenderId) -> Result<()> {
        self.peer_connection.remove_track(sender_id)
    }

    /// Forward a subscriber's keyframe request (PLI/FIR) upstream: write the RTCP on this
    /// publisher's receiver whose track carries `media_ssrc`, so its encoder emits a
    /// keyframe. No-op if no receiver carries that SSRC (nothing to ask). The forwarded
    /// SSRC is preserved end to end, so the request's media SSRC already matches this leg.
    pub(crate) fn request_keyframe(
        &mut self,
        media_ssrc: SSRC,
        packets: Vec<Box<dyn rtcp::Packet>>,
    ) -> Result<()> {
        let receiver_ids: Vec<RTCRtpReceiverId> = self.peer_connection.get_receivers().collect();
        let receiver_id = receiver_ids.into_iter().find(|id| {
            self.peer_connection
                .rtp_receiver(*id)
                .is_some_and(|receiver| receiver.track().ssrcs().any(|ssrc| ssrc == media_ssrc))
        });
        match receiver_id {
            Some(receiver_id) => {
                trace!(
                    "[{}/{}] forwarding {} keyframe request(s) to publisher receiver {:?} for ssrc {}",
                    self.room_id,
                    self.id,
                    packets.len(),
                    receiver_id,
                    media_ssrc
                );
                self.peer_connection
                    .rtp_receiver(receiver_id)
                    .ok_or(Error::ErrRTPReceiverNotExisted)?
                    .write_rtcp(packets)?;
            }
            None => {
                trace!(
                    "[{}/{}] no receiver carries ssrc {} — keyframe request dropped",
                    self.room_id, self.id, media_ssrc
                );
            }
        }
        Ok(())
    }

    /// A renegotiation transaction (an offer of ours, or an answer we sent to the subscriber's
    /// publish) just finished. Re-drive any renegotiation that was deferred while it was in
    /// flight.
    fn mark_curr_negotiation_complete(&mut self) -> Result<()> {
        self.drive_pending_renegotiation()
    }

    /// The peer connection reported that renegotiation is needed (reconcile added or removed a
    /// forwarding transceiver). Record it and drive it if the connection is ready.
    fn on_negotiation_needed(&mut self) -> Result<()> {
        self.renegotiation_pending = true;
        self.drive_pending_renegotiation()
    }

    /// Create and emit the SFU's subscribe-renegotiation offer — but only when it is safe:
    ///
    ///   * the client's first offer has been answered (`client_negotiated`) — the SFU never
    ///     makes the *first* offer, so the client's own m-lines (and their codec / extension-id
    ///     assignments) are established before any forward m-line is offered,
    ///   * a renegotiation is actually pending,
    ///   * no offer of ours is already in flight (`curr_request_id`), and
    ///   * the peer connection is in a **stable** signaling state — i.e. it is not concurrently
    ///     answering the subscriber's publish (`HaveRemoteOffer`) or awaiting an answer to a
    ///     prior offer (`HaveLocalOffer`).
    ///
    /// Otherwise this is a no-op; the renegotiation is re-driven when the client's first offer is
    /// answered ([`Client::handle_session_description`]), when the connection returns to stable
    /// (see [`Client::poll_event`]), or when the in-flight offer completes
    /// ([`Client::mark_curr_negotiation_complete`]). This mirrors the browser's rule that a
    /// `negotiationneeded` offer is only created from a stable state.
    fn drive_pending_renegotiation(&mut self) -> Result<()> {
        if !self.client_negotiated
            || !self.renegotiation_pending
            || self.curr_request_id.is_some()
            || self.signaling_state != RTCSignalingState::Stable
        {
            return Ok(());
        }
        self.renegotiation_pending = false;

        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.curr_request_id = Some((
            self.next_request_id,
            Instant::now() + ONGOING_NEGOTIATION_TIMEOUT_IN_SECOND,
        ));

        let offer = self.peer_connection.create_offer(None)?;
        self.peer_connection.set_local_description(offer)?;
        let sdp = self
            .peer_connection
            .local_description()
            .ok_or(Error::ErrPeerConnLocalDescriptionNil)?;

        trace!(
            "{}:[{}/{}] creates SDP {}:\n{}",
            self.next_request_id, self.room_id, self.id, sdp.sdp_type, sdp.sdp
        );

        self.events
            .push_back(ClientEvent::SFUEvent(SFUEvent::SessionDescription {
                request_id: self.next_request_id,
                room_id: self.room_id,
                client_id: self.id,
                sdp,
            }));
        Ok(())
    }

    /// Update forwarding transceivers according to directions requested in a remote offer
    /// (e.g. client subscribing with `recvonly` or unsubscribing with `inactive`).
    fn update_transceiver_directions_from_remote_offer(&mut self) {
        if let Some(remote_sdp) = self.peer_connection.remote_description()
            && let Ok(parsed) = remote_sdp.unmarshal()
        {
            for media in &parsed.media_descriptions {
                if let Some(mid) = media.attribute("mid").flatten() {
                    let is_recvonly = media.attribute("recvonly").is_some();
                    let is_inactive = media.attribute("inactive").is_some();
                    let transceiver_ids: Vec<RTCRtpTransceiverId> =
                        self.peer_connection.get_transceivers().collect();
                    for id in transceiver_ids {
                        if let Some(mut t) = self.peer_connection.rtp_transceiver(id) {
                            if t.mid().as_deref() == Some(mid.as_ref()) {
                                if is_recvonly {
                                    t.set_direction(RTCRtpTransceiverDirection::Sendonly);
                                } else if is_inactive {
                                    t.set_direction(RTCRtpTransceiverDirection::Inactive);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_session_description(
        &mut self,
        request_id: RequestId,
        sdp: RTCSessionDescription,
    ) -> Result<()> {
        let sdp_type = sdp.sdp_type;

        if sdp_type == RTCSdpType::Answer {
            if self.curr_request_id.is_none() {
                return Err(Error::ErrTransactionNotExists);
            } else if let Some((current_request_id, _)) = self.curr_request_id.as_ref()
                && *current_request_id != request_id
            {
                return Err(Error::ErrTransactionNotExists);
            }
        } else if sdp_type == RTCSdpType::Offer && self.curr_request_id.is_some() {
            return Err(Error::ErrTransactionExists);
        }

        self.peer_connection.set_remote_description(sdp)?;

        if sdp_type == RTCSdpType::Offer {
            self.update_transceiver_directions_from_remote_offer();

            let candidate = CandidateHostConfig {
                base_config: CandidateConfig {
                    network: "udp".to_owned(),
                    address: self.local_addr.ip().to_string(),
                    port: self.local_addr.port(),
                    component: 1,
                    ..Default::default()
                },
                ..Default::default()
            }
            .new_candidate_host()?;
            let local_candidate_init = RTCIceCandidate::from(&candidate).to_json()?;
            self.peer_connection
                .add_local_candidate(local_candidate_init)?;

            let answer = self.peer_connection.create_answer(None)?;

            self.peer_connection.set_local_description(answer)?;

            let sdp_answer = self
                .peer_connection
                .local_description()
                .ok_or(Error::ErrPeerConnLocalDescriptionNil)?;

            trace!(
                "[{}/{}] creates SDP {}:\n{}",
                self.room_id, self.id, sdp_answer.sdp_type, sdp_answer.sdp
            );

            self.events
                .push_back(ClientEvent::SFUEvent(SFUEvent::SessionDescription {
                    request_id,
                    room_id: self.room_id,
                    client_id: self.id,
                    sdp: sdp_answer,
                }));

            // The client's first offer is now answered: the initial SDP round is complete and the
            // client's codecs / extension ids are learned. Only now may the SFU start re-offering
            // (any forward that was deferred while waiting is driven once the state settles back
            // to stable, see poll_event).
            self.client_negotiated = true;
        } else if sdp_type == RTCSdpType::Answer {
            // mark current negotiation done
            self.curr_request_id = None;
            self.mark_curr_negotiation_complete()?;
        }

        Ok(())
    }

    fn handle_sfu_event(&mut self, evt: SFUEvent) -> Result<()> {
        if let Some(room_id) = evt.room_id() {
            if room_id != self.room_id {
                return Err(Error::Other(format!("invalid room id: {}", room_id)));
            }
        } else {
            return Err(Error::Other("empty room id".to_string()));
        };

        if let Some(client_id) = evt.client_id() {
            if client_id != self.id {
                return Err(Error::Other(format!("invalid client id: {}", client_id)));
            }
        } else {
            return Err(Error::Other("empty client id".to_string()));
        }

        match evt {
            SFUEvent::Ok { request_id, .. } => {
                warn!("{}:[{}/{}] receives ok", request_id, self.room_id, self.id,);
            }
            SFUEvent::Err {
                request_id, reason, ..
            } => {
                warn!(
                    "{}:[{}/{}] receives err due to {}",
                    request_id, self.room_id, self.id, reason
                );
            }
            SFUEvent::Join {
                request_id,
                room_id,
                client_id,
            } => {
                warn!(
                    "{}:[{}/{}] has already joined",
                    request_id, room_id, client_id
                );
            }
            SFUEvent::SessionDescription {
                request_id,
                room_id,
                client_id,
                sdp,
            } => {
                trace!(
                    "{}:[{}/{}] receives SDP {}:\n{}",
                    request_id, room_id, client_id, sdp.sdp_type, sdp.sdp
                );
                self.handle_session_description(request_id, sdp)?;
            }
            SFUEvent::IceCandidate {
                request_id,
                room_id,
                client_id,
                candidate,
            } => {
                trace!(
                    "{}:[{}/{}] receives ice candidate {}",
                    request_id, room_id, client_id, candidate.candidate
                );
                self.peer_connection.add_remote_candidate(candidate)?;
            }
            SFUEvent::Leave {
                request_id,
                room_id,
                client_id,
                reason,
            } => {
                warn!(
                    "{}:[{}/{}] has already left due to {}",
                    request_id, room_id, client_id, reason
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc::peer_connection::configuration::RTCConfigurationBuilder;
    use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpParameters, RTCRtpSendParameters,
    };

    #[test]
    fn builds_default_peer_connection_client() {
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");

        let client = ClientBuilder::new(10, RoomId::from_u128(20), "0.0.0.0:0".parse().unwrap())
            .with_media_engine(media_engine)
            .build()
            .expect("default client should build");

        assert_eq!(client.id, 10);
        assert_eq!(client.room_id, RoomId::from_u128(20));
    }

    #[test]
    fn builds_client_with_custom_media_engine() {
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");

        let _ = ClientBuilder::new(1, RoomId::from_u128(2), "0.0.0.0:0".parse().unwrap())
            .with_media_engine(media_engine)
            .build()
            .expect("client should build");
    }

    #[test]
    fn builds_client_with_custom_setting_engine() {
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");

        let _ = ClientBuilder::new(3, RoomId::from_u128(4), "0.0.0.0:0".parse().unwrap())
            .with_media_engine(media_engine)
            .with_setting_engine(SettingEngine::default())
            .build()
            .expect("client should build");
    }

    #[test]
    fn builds_client_with_interceptor_registry() {
        let configuration = RTCConfigurationBuilder::new().build();
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");

        let _ = ClientBuilder::new(5, RoomId::from_u128(6), "0.0.0.0:0".parse().unwrap())
            .with_configuration(configuration)
            .with_media_engine(media_engine)
            .with_setting_engine(SettingEngine::default())
            .with_interceptor_registry(Registry::new())
            .build()
            .expect("client should build");
    }

    #[test]
    fn forwarding_track_uses_receiver_parameters_when_track_codings_are_empty() {
        let track = MediaStreamTrack::new(
            "stream".into(),
            "track".into(),
            "label".into(),
            RtpCodecKind::Video,
            vec![],
        );
        let parameters = RTCRtpReceiveParameters {
            rtp_parameters: RTCRtpParameters {
                codecs: vec![RTCRtpCodecParameters {
                    rtp_codec: RTCRtpCodec {
                        mime_type: "video/VP8".into(),
                        clock_rate: 90_000,
                        channels: 0,
                        sdp_fmtp_line: String::new(),
                        rtcp_feedback: vec![],
                    },
                    payload_type: 96,
                }],
                ..Default::default()
            },
        };
        let media = MediaDescription::default()
            .with_value_attribute("ssrc".to_owned(), "424242 cname:test".to_owned());

        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");
        let client = ClientBuilder::new(42, RoomId::from_u128(20), "0.0.0.0:0".parse().unwrap())
            .with_media_engine(media_engine)
            .build()
            .expect("client should build");

        let rebuilt = client.track_with_codings_from_media_description(track, &parameters, &media);

        assert_eq!(rebuilt.codings().len(), 1);
        assert_eq!(rebuilt.codings()[0].codec.mime_type, "video/VP8");
        assert_eq!(
            rebuilt.codings()[0].rtp_coding_parameters.ssrc,
            Some(424242)
        );
        // The forwarded track's identity is stamped with the publishing client's id (42) so
        // subscribers can recover the publisher from the msid.
        assert_eq!(rebuilt.stream_id(), "peer-42-stream");
        assert_eq!(rebuilt.track_id(), "peer-42-track");
        assert_eq!(rebuilt.label(), "peer-42-label");
    }

    #[test]
    fn outgoing_payload_type_maps_codec_across_legs() {
        let codec = RTCRtpCodec {
            mime_type: "video/H265".into(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-id=186;profile-id=1;tier-flag=0;tx-mode=SRST".into(),
            rtcp_feedback: vec![],
        };
        let parameters = RTCRtpSendParameters {
            rtp_parameters: RTCRtpParameters {
                codecs: vec![
                    RTCRtpCodecParameters {
                        rtp_codec: RTCRtpCodec {
                            mime_type: "video/ulpfec".into(),
                            clock_rate: 90_000,
                            channels: 0,
                            sdp_fmtp_line: String::new(),
                            rtcp_feedback: vec![],
                        },
                        payload_type: 116,
                    },
                    RTCRtpCodecParameters {
                        rtp_codec: codec.clone(),
                        payload_type: 126,
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(
            Client::payload_type_for_codec(&parameters, &codec),
            Some(126)
        );
    }
}
