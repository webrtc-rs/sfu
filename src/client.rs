use crate::Event;
use crate::room::RoomId;
use rtc::interceptor::{Interceptor, NoopInterceptor, Registry};
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::RTCPeerConnection;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::peer_connection::configuration::RTCAnswerOptions;
use rtc::peer_connection::configuration::RTCConfiguration;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::configuration::setting_engine::SettingEngine;
use rtc::peer_connection::event::{RTCEvent, RTCPeerConnectionEvent};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;
use rtc::rtp::packet::Packet;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use rtc::rtp_transceiver::{RTCRtpReceiverId, RTCRtpSenderId};
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::{Error, Result};
use sansio::Protocol;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Instant;

pub(crate) trait PeerConnection: Send {
    fn set_remote_description(&mut self, remote_description: RTCSessionDescription) -> Result<()>;
    fn create_answer(&mut self, options: Option<RTCAnswerOptions>)
    -> Result<RTCSessionDescription>;
    fn set_local_description(&mut self, local_description: RTCSessionDescription) -> Result<()>;
    fn local_description(&self) -> Option<RTCSessionDescription>;
    fn add_local_candidate(&mut self, local_candidate: RTCIceCandidateInit) -> Result<()>;
    fn add_remote_candidate(&mut self, remote_candidate: RTCIceCandidateInit) -> Result<()>;
    fn add_track(&mut self, track: MediaStreamTrack) -> Result<RTCRtpSenderId>;
    fn rtp_receiver_kind(&mut self, receiver_id: RTCRtpReceiverId) -> Option<RtpCodecKind>;
    fn rtp_sender_ssrc(&mut self, sender_id: RTCRtpSenderId) -> Option<u32>;
    fn write_rtp(&mut self, sender_id: RTCRtpSenderId, packet: Packet) -> Result<()>;

    // sansio::Protocol
    fn handle_read(&mut self, packet: TaggedBytesMut) -> Result<()>;
    fn poll_read(&mut self) -> Option<RTCMessage>;
    fn handle_write(&mut self, msg: RTCMessage) -> Result<()>;
    fn poll_write(&mut self) -> Option<TaggedBytesMut>;
    fn handle_event(&mut self, evt: RTCEvent) -> Result<()>;
    fn poll_event(&mut self) -> Option<RTCPeerConnectionEvent>;
    fn handle_timeout(&mut self, now: Instant) -> Result<()>;
    fn poll_timeout(&mut self) -> Option<Instant>;
    fn close(&mut self) -> Result<()>;
}

impl<I> PeerConnection for RTCPeerConnection<I>
where
    I: Interceptor + Send + 'static,
{
    fn set_remote_description(&mut self, remote_description: RTCSessionDescription) -> Result<()> {
        RTCPeerConnection::set_remote_description(self, remote_description)
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

    fn add_local_candidate(&mut self, local_candidate: RTCIceCandidateInit) -> Result<()> {
        RTCPeerConnection::add_local_candidate(self, local_candidate)
    }

    fn add_remote_candidate(&mut self, remote_candidate: RTCIceCandidateInit) -> Result<()> {
        RTCPeerConnection::add_remote_candidate(self, remote_candidate)
    }

    fn add_track(&mut self, track: MediaStreamTrack) -> Result<RTCRtpSenderId> {
        RTCPeerConnection::add_track(self, track)
    }

    fn rtp_receiver_kind(&mut self, receiver_id: RTCRtpReceiverId) -> Option<RtpCodecKind> {
        self.rtp_receiver(receiver_id)
            .map(|receiver| receiver.track().kind())
    }

    fn rtp_sender_ssrc(&mut self, sender_id: RTCRtpSenderId) -> Option<u32> {
        self.rtp_sender(sender_id)
            .and_then(|sender| sender.track().ssrcs().last())
    }

    fn write_rtp(&mut self, sender_id: RTCRtpSenderId, packet: Packet) -> Result<()> {
        let mut sender = self
            .rtp_sender(sender_id)
            .ok_or(Error::ErrRTPSenderNotExisted)?;
        sender.write_rtp(packet)
    }

    // sansio::Protocol
    fn handle_read(&mut self, packet: TaggedBytesMut) -> Result<()> {
        Protocol::handle_read(self, packet)
    }

    fn poll_read(&mut self) -> Option<RTCMessage> {
        Protocol::poll_read(self)
    }

    fn poll_write(&mut self) -> Option<TaggedBytesMut> {
        Protocol::poll_write(self)
    }

    fn handle_write(&mut self, msg: RTCMessage) -> Result<()> {
        Protocol::handle_write(self, msg)
    }

    fn handle_event(&mut self, evt: RTCEvent) -> Result<()> {
        Protocol::handle_event(self, evt)
    }

    fn poll_event(&mut self) -> Option<RTCPeerConnectionEvent> {
        Protocol::poll_event(self)
    }

    fn poll_timeout(&mut self) -> Option<Instant> {
        Protocol::poll_timeout(self)
    }

    fn handle_timeout(&mut self, now: Instant) -> Result<()> {
        Protocol::handle_timeout(self, now)
    }

    fn close(&mut self) -> Result<()> {
        Protocol::close(self)
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
    pub(crate) fn client_id(&self) -> ClientId {
        self.id
    }

    pub(crate) fn room_id(&self) -> RoomId {
        self.room_id
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
        let pc = self.peer_connection_builder.build()?;
        Ok(Client {
            id: self.id,
            room_id: self.room_id,
            peer_connection: Box::new(pc),

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
            if *room_id != self.room_id {
                return Err(Error::Other(format!("invalid room id: {}", room_id)));
            }
        } else {
            return Err(Error::Other("empty room id".to_string()));
        }

        if let Some(client_id) = evt.client_id() {
            if *client_id != self.id {
                return Err(Error::Other(format!("invalid client id: {}", client_id)));
            }
        } else {
            return Err(Error::Other("empty client id".to_string()));
        }

        /*
        match evt {
            Event::Join {
                request_id,
                room_id,
                client_id,
            } => {}
            Event::SessionDescription {
                request_id,
                room_id,
                client_id,
                sdp,
            } => {}
            Event::IceCandidate {
                request_id,
                room_id,
                client_id,
                candidate,
            } => {}
            Event::Leave {
                request_id,
                room_id,
                client_id,
                reason,
            } => {}
            Event::Error {
                request_id,
                room_id,
                client_id,
                reason,
            } => {}
        }*/

        Ok(())
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
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
