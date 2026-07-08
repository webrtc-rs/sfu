use crate::forward::ForwardTrack;
use crate::room::RoomId;
use crate::{RequestId, SFUEvent};
use log::{info, warn};
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
    RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RtpCodecKind,
};
use rtc::rtp_transceiver::{
    RTCRtpReceiverId, RTCRtpSenderId, RTCRtpTransceiverDirection, RTCRtpTransceiverId,
    RTCRtpTransceiverInit,
};
use rtc::sdp::MediaDescription;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::{Error, Result};
use rtc::statistics::StatsSelector;
use rtc::statistics::report::RTCStatsReport;
use sansio::Protocol;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::Instant;

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

pub(crate) struct Client {
    id: ClientId,
    room_id: RoomId,
    local_addr: SocketAddr,
    peer_connection: Box<dyn PeerConnection>,
    next_request_id: RequestId,
    curr_request_id: Option<RequestId>,
    reads: VecDeque<RTCMessage>,
    writes: VecDeque<TaggedBytesMut>,
    events: VecDeque<ClientEvent>,
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
        self.peer_connection.handle_timeout(now)
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        self.peer_connection.poll_timeout()
    }

    fn close(&mut self) -> std::result::Result<(), Self::Error> {
        self.peer_connection.close()
    }
}

impl Client {
    /// The tracks this client is sending toward the SFU, keyed by m-line `mid`, ready to
    /// be forwarded. Built **directly from the remote description's media sections** once
    /// the offer/answer is complete — `Room::reconcile` runs it right after
    /// `handle_session_description` applies the offer and sets the local answer, so
    /// forwarding is wired before the first RTP packet (no wait for `OnTrack`).
    ///
    /// Only m-lines the remote is *sending* on are publish tracks: in the browser's remote
    /// description `sendrecv`/`sendonly` (and a bare m-line, which defaults to `sendrecv`)
    /// means it publishes here, while `recvonly`/`inactive` are the SFU's own subscribe
    /// senders echoed back. Data-channel (`application`) sections and m-lines without an
    /// SSRC or RID to identify the stream are skipped.
    pub(crate) fn get_forward_tracks(&mut self) -> HashMap<Mid, ForwardTrack> {
        let mut tracks = HashMap::new();

        let Some(remote) = self.peer_connection.remote_description() else {
            return tracks;
        };
        let Ok(parsed) = remote.unmarshal() else {
            return tracks;
        };

        for media in &parsed.media_descriptions {
            let kind = RtpCodecKind::from(media.media_name.media.as_str());
            if kind == RtpCodecKind::Unspecified {
                // application/data-channel m-line — nothing to forward.
                continue;
            }
            if media.has_attribute("recvonly") || media.has_attribute("inactive") {
                continue;
            }
            let Some(mid) = media.attribute("mid").flatten().map(str::to_owned) else {
                continue;
            };
            let Some(track) = self.track_from_media_description(&mid, kind, media) else {
                continue;
            };

            tracks.insert(mid, track);
        }
        tracks
    }

    /// Construct the `ForwardTrack` for one sending m-line: its SSRC
    /// (`a=ssrc`) / RID (`a=rid`), stream & track ids (`a=msid`), and primary codec. `None`
    /// if the m-line carries neither an SSRC nor a RID
    fn track_from_media_description(
        &self,
        mid: &str,
        kind: RtpCodecKind,
        media: &MediaDescription,
    ) -> Option<ForwardTrack> {
        // `a=ssrc:<ssrc> ...` — primary (first) SSRC
        let ssrc = media
            .attribute("ssrc")
            .flatten()
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u32>().ok());
        /*TODO: Simulcast:
        let rid = media
            .attribute("rid")
            .flatten()
            .and_then(|value| value.split_whitespace().next())
            .map(str::to_owned)
            .unwrap_or_default();
        */

        // `a=msid:<stream_id> <track_id>`, synthesizing from the mid when absent.
        let (mut stream_id, mut track_id) = media
            .attribute("msid")
            .flatten()
            .map(|value| {
                let mut it = value.split_whitespace();
                (
                    it.next().unwrap_or_default().to_owned(),
                    it.next().unwrap_or_default().to_owned(),
                )
            })
            .unwrap_or_default();
        if stream_id.is_empty() {
            stream_id = format!("stream-{}-{}", self.id, mid);
        }
        if track_id.is_empty() {
            track_id = format!("track-{}-{}", self.id, mid);
        }
        let label = format!("{}-{}", self.id, mid);

        let mut codecs = vec![];
        for (payload_type, codec) in media.codecs() {
            codecs.push(RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: format!("{}/{}", media.media_name.media, codec.name),
                    clock_rate: codec.clock_rate,
                    channels: codec.encoding_parameters.parse().unwrap_or(0),
                    sdp_fmtp_line: codec.fmtp.to_string(),
                    rtcp_feedback: codec
                        .rtcp_feedback
                        .iter()
                        .map(|raw| {
                            let mut parts = raw.splitn(2, ' ');
                            RTCPFeedback {
                                typ: parts.next().unwrap_or_default().to_owned(),
                                parameter: parts.next().unwrap_or_default().to_owned(),
                            }
                        })
                        .collect(),
                },
                payload_type,
            });
        }

        Some(ForwardTrack {
            mid: mid.to_string(),
            ssrc,
            stream_id,
            track_id,
            label,
            kind,
            codecs,
        })
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

    /// Generate the SFU's offer for a subscribe renegotiation and emit it upward.
    fn on_negotiation_needed(&mut self) -> Result<()> {
        if self.curr_request_id.is_some() {
            // negotiation is ongoing ...
            return Ok(());
        }

        let offer = self.peer_connection.create_offer(None)?;
        self.peer_connection.set_local_description(offer)?;
        let sdp = self
            .peer_connection
            .local_description()
            .ok_or(Error::ErrPeerConnLocalDescriptionNil)?;

        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.curr_request_id = Some(self.next_request_id);

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
            if self.curr_request_id.is_none() || self.curr_request_id != Some(request_id) {
                return Err(Error::ErrTransactionNotExists);
            }
            // mark current negotiation done
            self.curr_request_id = None;
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

            self.events
                .push_back(ClientEvent::SFUEvent(SFUEvent::SessionDescription {
                    request_id,
                    room_id: self.room_id,
                    client_id: self.id,
                    sdp: self
                        .peer_connection
                        .local_description()
                        .ok_or(Error::ErrPeerConnLocalDescriptionNil)?,
                }))
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
                warn!("{}:{}:{} receives ok", request_id, self.room_id, self.id,);
            }
            SFUEvent::Err {
                request_id, reason, ..
            } => {
                warn!(
                    "{}:{}:{} receives err due to {}",
                    request_id, self.room_id, self.id, reason
                );
            }
            SFUEvent::Join {
                request_id,
                room_id,
                client_id,
            } => {
                warn!(
                    "{}:{}:{} has already joined",
                    request_id, room_id, client_id
                );
            }
            SFUEvent::SessionDescription {
                request_id,
                room_id,
                client_id,
                sdp,
            } => {
                info!(
                    "{}:{}:{} receives sdp type {} and {}",
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
                info!(
                    "{}:{}:{} receives ice candidate {}",
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
                    "{}:{}:{} has already left due to {}",
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
}
