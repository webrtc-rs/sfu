use crate::room::RoomId;
use crate::{RequestId, SFUEvent};
use log::{trace, warn};
use rtc::ice::candidate::CandidateConfig;
use rtc::interceptor::{Interceptor, NoopInterceptor, Registry};
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::RTCPeerConnection;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::peer_connection::configuration::RTCConfiguration;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::setting_engine::SettingEngine;
use rtc::peer_connection::configuration::{RTCAnswerOptions, RTCOfferOptions};
use rtc::peer_connection::event::{RTCEvent, RTCPeerConnectionEvent};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
use rtc::peer_connection::transport::{CandidateHostConfig, RTCIceCandidate, RTCIceCandidateInit};
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RTCRtpReceiveParameters,
    RTCRtpSendParameters, RtpCodecKind,
};
use rtc::rtp_transceiver::{
    RTCRtpReceiverId, RTCRtpSenderId, RTCRtpTransceiverDirection, RTCRtpTransceiverId,
    RTCRtpTransceiverInit,
};
use rtc::sdp::MediaDescription;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::{Error, Result, flatten_errs};
use rtc::statistics::StatsSelector;
use rtc::statistics::report::RTCStatsReport;
use rtc::{rtcp, rtp};
use sansio::Protocol;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::time::{Duration, Instant};

pub(crate) trait PeerConnection:
    Protocol<
        TaggedBytesMut,
        RTCMessage,
        RTCEvent,
        Rout = RTCMessage,
        Wout = TaggedBytesMut,
        Eout = RTCPeerConnectionEvent,
        Error = Error,
        Time = Instant,
    > + Send
{
    fn create_offer(&mut self, options: Option<RTCOfferOptions>) -> Result<RTCSessionDescription>;
    fn create_answer(&mut self, options: Option<RTCAnswerOptions>)
    -> Result<RTCSessionDescription>;
    fn set_local_description(&mut self, local_description: RTCSessionDescription) -> Result<()>;
    fn local_description(&self) -> Option<RTCSessionDescription>;
    fn current_local_description(&self) -> Option<RTCSessionDescription>;
    fn pending_local_description(&self) -> Option<RTCSessionDescription>;
    fn can_trickle_ice_candidates(&self) -> Option<bool>;
    fn set_remote_description(&mut self, remote_description: RTCSessionDescription) -> Result<()>;
    fn remote_description(&self) -> Option<&RTCSessionDescription>;
    fn current_remote_description(&self) -> Option<&RTCSessionDescription>;
    fn pending_remote_description(&self) -> Option<&RTCSessionDescription>;
    fn add_local_candidate(&mut self, local_candidate: RTCIceCandidateInit) -> Result<()>;
    fn add_remote_candidate(&mut self, remote_candidate: RTCIceCandidateInit) -> Result<()>;
    fn restart_ice(&mut self);
    fn get_configuration(&self) -> &RTCConfiguration;
    fn set_configuration(&mut self, configuration: RTCConfiguration) -> Result<()>;
    /*fn create_data_channel(
        &mut self,
        label: &str,
        options: Option<RTCDataChannelInit>,
    ) -> Result<RTCDataChannelId>;*/
    fn get_senders(&self) -> Vec<RTCRtpSenderId>;
    fn get_receivers(&self) -> Vec<RTCRtpReceiverId>;
    fn get_transceivers(&self) -> Vec<RTCRtpTransceiverId>;

    fn add_track(&mut self, track: MediaStreamTrack) -> Result<RTCRtpSenderId>;
    fn remove_track(&mut self, sender_id: RTCRtpSenderId) -> Result<()>;
    fn add_transceiver_from_track(
        &mut self,
        track: MediaStreamTrack,
        init: Option<RTCRtpTransceiverInit>,
    ) -> Result<RTCRtpTransceiverId>;
    fn add_transceiver_from_kind(
        &mut self,
        kind: RtpCodecKind,
        init: Option<RTCRtpTransceiverInit>,
    ) -> Result<RTCRtpTransceiverId>;
    fn get_stats(&mut self, now: Instant, selector: StatsSelector) -> RTCStatsReport;

    fn write_rtp(&mut self, sender_id: RTCRtpSenderId, packet: rtp::Packet) -> Result<()>;
    fn write_rtcp(
        &mut self,
        sender_id: RTCRtpSenderId,
        packets: Vec<Box<dyn rtcp::Packet>>,
    ) -> Result<()>;

    /// The mid of a transceiver, if assigned — the stable forwarding key (it is not
    /// carried on the `MediaStreamTrack`).
    fn transceiver_mid(&mut self, transceiver_id: RTCRtpTransceiverId) -> Option<Mid>;

    /// The negotiated receive track for a receiver (`receiver.track()`, cloned): kind,
    /// stream/track ids, and any track-level coding metadata already available.
    fn receiver_track(&mut self, receiver_id: RTCRtpReceiverId) -> Option<MediaStreamTrack>;

    /// The receiver's negotiated RTP parameters, including the filtered codec list that
    /// is valid for this peer connection's receive side before `OnTrack` populates
    /// deferred track codings.
    fn receiver_parameters(
        &mut self,
        receiver_id: RTCRtpReceiverId,
    ) -> Option<RTCRtpReceiveParameters>;

    /// The sender's negotiated RTP parameters, including the subscriber leg payload
    /// types currently valid for that sender.
    fn sender_parameters(&mut self, sender_id: RTCRtpSenderId) -> Option<RTCRtpSendParameters>;
}

impl<I> PeerConnection for RTCPeerConnection<I>
where
    I: Interceptor + Send + 'static,
{
    fn create_offer(&mut self, options: Option<RTCOfferOptions>) -> Result<RTCSessionDescription> {
        RTCPeerConnection::create_offer(self, options)
    }

    fn create_answer(
        &mut self,
        options: Option<RTCAnswerOptions>,
    ) -> Result<RTCSessionDescription> {
        RTCPeerConnection::create_answer(self, options)
    }

    fn set_local_description(&mut self, local_description: RTCSessionDescription) -> Result<()> {
        RTCPeerConnection::set_local_description(self, local_description)
    }

    fn local_description(&self) -> Option<RTCSessionDescription> {
        RTCPeerConnection::local_description(self)
    }

    fn current_local_description(&self) -> Option<RTCSessionDescription> {
        RTCPeerConnection::current_local_description(self)
    }

    fn pending_local_description(&self) -> Option<RTCSessionDescription> {
        RTCPeerConnection::pending_local_description(self)
    }

    fn can_trickle_ice_candidates(&self) -> Option<bool> {
        RTCPeerConnection::can_trickle_ice_candidates(self)
    }

    fn set_remote_description(&mut self, remote_description: RTCSessionDescription) -> Result<()> {
        RTCPeerConnection::set_remote_description(self, remote_description)
    }

    fn remote_description(&self) -> Option<&RTCSessionDescription> {
        RTCPeerConnection::remote_description(self)
    }

    fn current_remote_description(&self) -> Option<&RTCSessionDescription> {
        RTCPeerConnection::current_remote_description(self)
    }

    fn pending_remote_description(&self) -> Option<&RTCSessionDescription> {
        RTCPeerConnection::pending_remote_description(self)
    }

    fn add_local_candidate(&mut self, local_candidate: RTCIceCandidateInit) -> Result<()> {
        RTCPeerConnection::add_local_candidate(self, local_candidate)
    }

    fn add_remote_candidate(&mut self, remote_candidate: RTCIceCandidateInit) -> Result<()> {
        RTCPeerConnection::add_remote_candidate(self, remote_candidate)
    }

    fn restart_ice(&mut self) {
        RTCPeerConnection::restart_ice(self)
    }

    fn get_configuration(&self) -> &RTCConfiguration {
        RTCPeerConnection::get_configuration(self)
    }
    fn set_configuration(&mut self, configuration: RTCConfiguration) -> Result<()> {
        RTCPeerConnection::set_configuration(self, configuration)
    }

    /*fn create_data_channel(
        &mut self,
        label: &str,
        options: Option<RTCDataChannelInit>,
    ) -> Result<RTCDataChannelId> {
        let dc = RTCPeerConnection::create_data_channel(self, label, options)?;
        Ok(dc.id())
    }*/

    fn get_senders(&self) -> Vec<RTCRtpSenderId> {
        RTCPeerConnection::get_senders(self).collect()
    }
    fn get_receivers(&self) -> Vec<RTCRtpReceiverId> {
        RTCPeerConnection::get_receivers(self).collect()
    }
    fn get_transceivers(&self) -> Vec<RTCRtpTransceiverId> {
        RTCPeerConnection::get_transceivers(self).collect()
    }

    fn add_track(&mut self, track: MediaStreamTrack) -> Result<RTCRtpSenderId> {
        RTCPeerConnection::add_track(self, track)
    }

    fn remove_track(&mut self, sender_id: RTCRtpSenderId) -> Result<()> {
        RTCPeerConnection::remove_track(self, sender_id)
    }

    fn add_transceiver_from_track(
        &mut self,
        track: MediaStreamTrack,
        init: Option<RTCRtpTransceiverInit>,
    ) -> Result<RTCRtpTransceiverId> {
        RTCPeerConnection::add_transceiver_from_track(self, track, init)
    }

    fn add_transceiver_from_kind(
        &mut self,
        kind: RtpCodecKind,
        init: Option<RTCRtpTransceiverInit>,
    ) -> Result<RTCRtpTransceiverId> {
        RTCPeerConnection::add_transceiver_from_kind(self, kind, init)
    }

    fn get_stats(&mut self, now: Instant, selector: StatsSelector) -> RTCStatsReport {
        RTCPeerConnection::get_stats(self, now, selector)
    }

    fn write_rtp(&mut self, sender_id: RTCRtpSenderId, packet: rtp::Packet) -> Result<()> {
        RTCPeerConnection::rtp_sender(self, sender_id)
            .ok_or(Error::ErrRTPSenderNotExisted)?
            .write_rtp(packet)
    }

    fn write_rtcp(
        &mut self,
        sender_id: RTCRtpSenderId,
        packets: Vec<Box<dyn rtcp::Packet>>,
    ) -> Result<()> {
        RTCPeerConnection::rtp_sender(self, sender_id)
            .ok_or(Error::ErrRTPSenderNotExisted)?
            .write_rtcp(packets)
    }

    fn transceiver_mid(&mut self, transceiver_id: RTCRtpTransceiverId) -> Option<Mid> {
        self.rtp_transceiver(transceiver_id)?.mid().clone()
    }

    fn receiver_track(&mut self, receiver_id: RTCRtpReceiverId) -> Option<MediaStreamTrack> {
        Some(self.rtp_receiver(receiver_id)?.track().clone())
    }

    fn receiver_parameters(
        &mut self,
        receiver_id: RTCRtpReceiverId,
    ) -> Option<RTCRtpReceiveParameters> {
        Some(self.rtp_receiver(receiver_id)?.get_parameters().clone())
    }

    fn sender_parameters(&mut self, sender_id: RTCRtpSenderId) -> Option<RTCRtpSendParameters> {
        Some(self.rtp_sender(sender_id)?.get_parameters().clone())
    }
}

pub(crate) struct ClientBuilder<I = NoopInterceptor>
where
    I: Interceptor,
{
    id: ClientId,
    room_id: RoomId,
    local_addr: SocketAddr,
    peer_connection_builder: RTCPeerConnectionBuilder<I>,
}

impl ClientBuilder<NoopInterceptor> {
    pub(crate) fn new(id: ClientId, room_id: RoomId, local_addr: SocketAddr) -> Self {
        Self {
            id,
            room_id,
            local_addr,
            peer_connection_builder: RTCPeerConnectionBuilder::new(),
        }
    }
}

impl<I> ClientBuilder<I>
where
    I: Interceptor,
{
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

    pub(crate) fn with_interceptor_registry<P>(
        self,
        interceptor_registry: Registry<P>,
    ) -> ClientBuilder<P>
    where
        P: Interceptor,
    {
        ClientBuilder {
            id: self.id,
            room_id: self.room_id,
            local_addr: self.local_addr,
            peer_connection_builder: self
                .peer_connection_builder
                .with_interceptor_registry(interceptor_registry),
        }
    }

    pub(crate) fn build(self) -> Result<Client>
    where
        I: Send + 'static,
    {
        Ok(Client {
            id: self.id,
            room_id: self.room_id,
            local_addr: self.local_addr,
            peer_connection: Box::new(self.peer_connection_builder.build()?),

            next_request_id: 0,
            curr_request_id: None,
            negotiation_needed_state: NegotiationNeededState::Empty,

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

#[derive(Default, Debug, Copy, Clone, PartialEq)]
enum NegotiationNeededState {
    /// NegotiationNeededStateEmpty not running and queue is empty
    #[default]
    Empty,
    /// NegotiationNeededStateEmpty running and queue is empty
    Run,
    /// NegotiationNeededStateEmpty running and queue
    Queue,
}

pub(crate) struct Client {
    id: ClientId,
    room_id: RoomId,
    local_addr: SocketAddr,
    peer_connection: Box<dyn PeerConnection>,

    next_request_id: RequestId,
    curr_request_id: Option<(RequestId, Instant)>,
    negotiation_needed_state: NegotiationNeededState,

    reads: VecDeque<RTCMessage>,
    writes: VecDeque<TaggedBytesMut>,
    events: VecDeque<ClientEvent>,
}

impl Deref for Client {
    type Target = Box<dyn PeerConnection>;

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
            ClientEvent::SFUEvent(evt) => {
                self.handle_sfu_event(evt)?;
            }
            ClientEvent::PeerConnectionEvent(_) => {
                //TODO:
            }
        }

        Ok(())
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
                other => self
                    .events
                    .push_back(ClientEvent::PeerConnectionEvent(other)),
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
    pub(crate) fn incoming_codec_for_rtp(
        &mut self,
        ssrc: u32,
        payload_type: u8,
    ) -> Option<RTCRtpCodec> {
        for receiver_id in self.peer_connection.get_receivers() {
            let Some(track) = self.peer_connection.receiver_track(receiver_id) else {
                continue;
            };
            if !track.ssrcs().any(|track_ssrc| track_ssrc == ssrc) {
                continue;
            }

            let Some(parameters) = self.peer_connection.receiver_parameters(receiver_id) else {
                continue;
            };
            if let Some(codec) = Client::codec_for_payload_type(&parameters, payload_type) {
                return Some(codec);
            }
        }

        None
    }

    pub(crate) fn outgoing_payload_type_for_codec(
        &mut self,
        sender_id: RTCRtpSenderId,
        codec: &RTCRtpCodec,
    ) -> Option<u8> {
        let parameters = self.peer_connection.sender_parameters(sender_id)?;
        Client::payload_type_for_codec(&parameters, codec)
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

        for receiver_id in self.peer_connection.get_receivers() {
            let Some(mid) = self.peer_connection.transceiver_mid(receiver_id.into()) else {
                continue;
            };
            let Some(mut track) = self.peer_connection.receiver_track(receiver_id) else {
                continue;
            };
            let Some(parameters) = self.peer_connection.receiver_parameters(receiver_id) else {
                continue;
            };

            if let Some(media) = parsed.as_ref().and_then(|parsed| {
                parsed
                    .media_descriptions
                    .iter()
                    .find(|media| media.attribute("mid").flatten() == Some(mid.as_str()))
            }) {
                track =
                    Client::track_with_codings_from_media_description(&track, &parameters, media);
            }

            tracks.insert(mid, track);
        }
        tracks
    }

    /// Rebuild `track` (keeping the negotiated receiver's identity: stream/track ids,
    /// label, kind) from the receiver's negotiated parameters, then fill in the
    /// publish-side primary SSRC (`a=ssrc`) from the remote description when it is known.
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
        track: &MediaStreamTrack,
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
            track.stream_id().clone(),
            track.track_id().clone(),
            track.label().clone(),
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

    /// The mid of the m-line a receiver belongs to — used by `Room` to bind a
    /// packet-time SSRC (from `OnTrack(OnOpen)`) to its `ForwardKey`.
    pub(crate) fn transceiver_mid(&mut self, receiver_id: RTCRtpReceiverId) -> Option<Mid> {
        self.peer_connection.transceiver_mid(receiver_id.into())
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

    pub(crate) fn is_negotiation_ongoing(&self) -> bool {
        self.curr_request_id.is_some()
    }

    pub(crate) fn is_negotiation_needed(&self) -> bool {
        self.negotiation_needed_state == NegotiationNeededState::Queue
    }

    fn do_negotiation_needed(&mut self) -> bool {
        if self.negotiation_needed_state == NegotiationNeededState::Run {
            self.negotiation_needed_state = NegotiationNeededState::Queue;
            false
        } else if self.negotiation_needed_state == NegotiationNeededState::Queue {
            false
        } else {
            self.negotiation_needed_state = NegotiationNeededState::Run;
            true
        }
    }

    fn mark_curr_negotiation_complete(&mut self) -> Result<()> {
        if self.negotiation_needed_state == NegotiationNeededState::Run {
            self.negotiation_needed_state = NegotiationNeededState::Empty;
        } else if self.negotiation_needed_state == NegotiationNeededState::Queue {
            self.negotiation_needed_state = NegotiationNeededState::Empty;
            return self.on_negotiation_needed();
        }

        Ok(())
    }

    /// Generate the SFU's offer for a subscribe renegotiation and emit it upward.
    fn on_negotiation_needed(&mut self) -> Result<()> {
        if !self.do_negotiation_needed() {
            if let Some((request_id, _)) = self.curr_request_id.as_ref() {
                // negotiation is ongoing ...
                trace!(
                    "{}:[{}/{}] negotiation is ongoing ...",
                    request_id, self.room_id, self.id
                );
                return Ok(());
            } else {
                return Err(Error::ErrNegotiatedWithoutID);
            }
        }

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
                }))
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
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpParameters, RTCRtpSendParameters,
    };

    #[test]
    fn builds_default_peer_connection_client() {
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");

        let client = ClientBuilder::new(10, 20, "0.0.0.0:0".parse().unwrap())
            .with_media_engine(media_engine)
            .build()
            .expect("default client should build");

        assert_eq!(client.id, 10);
        assert_eq!(client.room_id, 20);
    }

    #[test]
    fn builds_client_with_custom_media_engine() {
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .expect("default codecs should register");

        let _ = ClientBuilder::new(1, 2, "0.0.0.0:0".parse().unwrap())
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

        let _ = ClientBuilder::new(3, 4, "0.0.0.0:0".parse().unwrap())
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

        let _ = ClientBuilder::new(5, 6, "0.0.0.0:0".parse().unwrap())
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

        let rebuilt =
            Client::track_with_codings_from_media_description(&track, &parameters, &media);

        assert_eq!(rebuilt.codings().len(), 1);
        assert_eq!(rebuilt.codings()[0].codec.mime_type, "video/VP8");
        assert_eq!(
            rebuilt.codings()[0].rtp_coding_parameters.ssrc,
            Some(424242)
        );
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
