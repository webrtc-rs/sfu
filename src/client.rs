use crate::room::RoomId;
use crate::{Event, RequestId};
use log::{info, warn};
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
use rtc::peer_connection::transport::RTCIceCandidateInit;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use rtc::rtp_transceiver::{
    RTCRtpReceiverId, RTCRtpSenderId, RTCRtpTransceiverId, RTCRtpTransceiverInit,
};
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::{Error, Result};
use rtc::statistics::StatsSelector;
use rtc::statistics::report::RTCStatsReport;
use sansio::Protocol;
use std::collections::VecDeque;
use std::convert::Infallible;
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
    peer_connection_builder: RTCPeerConnectionBuilder<I>,
}

impl ClientBuilder<NoopInterceptor> {
    pub(crate) fn new(id: ClientId, room_id: RoomId) -> Self {
        Self {
            id,
            room_id,
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
            peer_connection: Box::new(self.peer_connection_builder.build()?),

            transmits: Default::default(),
            events: Default::default(),
        })
    }
}

pub type ClientId = u64;

pub(crate) struct Client {
    id: ClientId,
    room_id: RoomId,
    peer_connection: Box<dyn PeerConnection>,

    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<Event>,
}

impl Protocol<TaggedBytesMut, Infallible, Event> for Client {
    type Rout = Infallible;
    type Wout = TaggedBytesMut;
    type Eout = Event;
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, _msg: TaggedBytesMut) -> std::result::Result<(), Self::Error> {
        Ok(())
    }

    fn poll_read(&mut self) -> Option<Self::Rout> {
        None
    }

    fn handle_write(&mut self, _msg: Infallible) -> std::result::Result<(), Self::Error> {
        match _msg {}
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        self.transmits.pop_front()
    }

    fn handle_event(&mut self, evt: Event) -> std::result::Result<(), Self::Error> {
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
            Event::Ok { request_id, .. } => {
                warn!("{}:{}:{} receives ok", request_id, self.room_id, self.id,);
            }
            Event::Err {
                request_id, reason, ..
            } => {
                warn!(
                    "{}:{}:{} receives err due to {}",
                    request_id, self.room_id, self.id, reason
                );
            }
            Event::Join {
                request_id,
                room_id,
                client_id,
            } => {
                warn!(
                    "{}:{}:{} has already joined",
                    request_id, room_id, client_id
                );
            }
            Event::SessionDescription {
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
            Event::IceCandidate {
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
            Event::Leave {
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

    fn poll_event(&mut self) -> Option<Self::Eout> {
        while let Some(evt) = self.peer_connection.poll_event() {
            //TODO: process peer_connection's event
            info!("TODO: process peer_connection's event {:?}", evt);
        }

        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> std::result::Result<(), Self::Error> {
        let _ = self.peer_connection.handle_timeout(now);
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        let mut eto: Option<Instant> = None;
        if let Some(next) = self.peer_connection.poll_timeout() {
            eto = Some(eto.map_or(next, |curr| std::cmp::min(curr, next)));
        }
        eto
    }

    fn close(&mut self) -> std::result::Result<(), Self::Error> {
        self.transmits.clear();
        self.events.clear();
        self.peer_connection.close()
    }
}

impl Client {
    fn handle_session_description(
        &mut self,
        request_id: RequestId,
        sdp: RTCSessionDescription,
    ) -> Result<()> {
        let sdp_type = sdp.sdp_type;

        self.peer_connection.set_remote_description(sdp)?;

        if sdp_type == RTCSdpType::Offer {
            let answer = self.peer_connection.create_answer(None)?;
            self.peer_connection.set_local_description(answer)?;
            self.events.push_back(Event::SessionDescription {
                request_id,
                room_id: self.room_id,
                client_id: self.id,
                sdp: self
                    .peer_connection
                    .local_description()
                    .ok_or(Error::ErrPeerConnLocalDescriptionNil)?,
            })
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

        let client = ClientBuilder::new(10, 20)
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

        let _ = ClientBuilder::new(1, 2)
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

        let _ = ClientBuilder::new(3, 4)
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

        let _ = ClientBuilder::new(5, 6)
            .with_configuration(configuration)
            .with_media_engine(media_engine)
            .with_setting_engine(SettingEngine::default())
            .with_interceptor_registry(Registry::new())
            .build()
            .expect("client should build");
    }
}
