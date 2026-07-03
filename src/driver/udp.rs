use crate::engine::{Client, ClientBuilder, ClientId, RoomId, SfuCommand, SfuCore, SfuEvent};
use bytes::BytesMut;
use rtc::interceptor::Interceptor;
use rtc::interceptor::Registry;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::peer_connection::event::RTCPeerConnectionEvent;
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::{
    CandidateConfig, CandidateHostConfig, RTCIceCandidate, RTCIceCandidateInit,
};
use rtc::shared::error::{Error, Result};
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use sansio::Protocol;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Instant;

pub struct UdpDriver {
    pub core: SfuCore,
    outbound: VecDeque<TaggedBytesMut>,
    peer_events: VecDeque<RTCPeerConnectionEvent>,
    reads: VecDeque<RTCMessage>,
}

impl Default for UdpDriver {
    fn default() -> Self {
        Self::new(SfuCore::default())
    }
}

impl UdpDriver {
    pub fn new(core: SfuCore) -> Self {
        Self {
            core,
            outbound: VecDeque::new(),
            peer_events: VecDeque::new(),
            reads: VecDeque::new(),
        }
    }

    pub fn handle_read(&mut self, client_id: ClientId, packet: TaggedBytesMut) -> Result<()> {
        let client = self
            .core
            .client_mut(client_id)
            .ok_or_else(|| Error::Other(format!("unknown client {client_id}")))?;
        let pc = client
            .pc
            .as_mut()
            .ok_or_else(|| Error::Other(format!("client {client_id} has no peer connection")))?;
        pc.handle_read(packet)?;
        self.drain_client(client_id);
        Ok(())
    }

    pub fn handle_timeout(&mut self, now: Instant) -> Result<()> {
        let client_ids: Vec<_> = self.core.clients_with_peer_connections().collect();
        for client_id in client_ids {
            self.core.handle_client_timeout(client_id, now)?;
            self.drain_client(client_id);
        }
        Ok(())
    }

    pub fn poll_timeout(&mut self) -> Option<Instant> {
        self.core
            .clients_with_peer_connections()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|client_id| self.core.client_timeout(client_id))
            .min()
    }

    pub fn poll_write(&mut self) -> Option<TaggedBytesMut> {
        self.outbound.pop_front()
    }

    pub fn poll_event(&mut self) -> Option<RTCPeerConnectionEvent> {
        self.peer_events.pop_front()
    }

    pub fn poll_read(&mut self) -> Option<RTCMessage> {
        self.reads.pop_front()
    }

    pub fn poll_sfu_event(&mut self) -> Option<SfuEvent> {
        self.core.poll_event()
    }

    fn drain_client(&mut self, client_id: ClientId) {
        self.core.drain_client(
            client_id,
            &mut self.outbound,
            &mut self.peer_events,
            &mut self.reads,
        );
    }

    pub fn create_client(&mut self, room_id: RoomId, client_id: ClientId) -> Result<()> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;

        self.create_client_with_builder(
            Client::builder(client_id, room_id)
                .with_media_engine(media_engine)
                .with_interceptor_registry(registry),
        )
    }

    pub fn upsert_client(&mut self, client: Client) -> Result<()> {
        let client_id = client.id;
        if self.core.client(client_id).is_some() {
            return Ok(());
        }

        self.core.upsert_client(client);
        self.drain_client(client_id);
        Ok(())
    }

    pub fn create_client_with_builder<I>(&mut self, builder: ClientBuilder<I>) -> Result<()>
    where
        I: Interceptor + Send + 'static,
    {
        let client_id = builder.client_id();
        if self.core.client(client_id).is_some() {
            return Ok(());
        }

        let client = builder.build()?;
        self.core.upsert_client(client);
        self.drain_client(client_id);
        Ok(())
    }

    pub fn handle_command(&mut self, command: SfuCommand, client_id: ClientId) -> Result<bool> {
        let should_stop =
            matches!(command, SfuCommand::CloseClient { client } if client == client_id);
        self.core.handle_event(command)?;
        self.drain_client(client_id);
        Ok(should_stop)
    }

    pub fn accept_offer(
        &mut self,
        room_id: RoomId,
        client_id: ClientId,
        offer: RTCSessionDescription,
        local_addr: SocketAddr,
    ) -> Result<(RTCSessionDescription, RTCIceCandidateInit)> {
        if self.core.client(client_id).is_none() {
            self.create_client(room_id, client_id)?;
        }

        let local_candidate = host_candidate(local_addr)?;
        let answer = {
            let client = self
                .core
                .client_mut(client_id)
                .ok_or_else(|| Error::Other(format!("unknown client {client_id}")))?;
            let pc = client.pc.as_mut().ok_or_else(|| {
                Error::Other(format!("client {client_id} has no peer connection"))
            })?;

            pc.set_remote_description(offer)?;
            pc.add_local_candidate(local_candidate.clone())?;
            let answer = pc.create_answer(None)?;
            pc.set_local_description(answer.clone())?;
            pc.local_description().unwrap_or(answer)
        };

        self.drain_client(client_id);
        Ok((answer, local_candidate))
    }
}

pub fn tagged_udp_packet(
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    payload: &[u8],
) -> TaggedBytesMut {
    TaggedBytesMut {
        now: Instant::now(),
        transport: TransportContext {
            local_addr,
            peer_addr,
            ecn: None,
            transport_protocol: TransportProtocol::UDP,
        },
        message: BytesMut::from(payload),
    }
}

fn host_candidate(local_addr: SocketAddr) -> Result<RTCIceCandidateInit> {
    RTCIceCandidate::from(
        &CandidateHostConfig {
            base_config: CandidateConfig {
                network: "udp".to_owned(),
                address: local_addr.ip().to_string(),
                port: local_addr.port(),
                component: 1,
                ..Default::default()
            },
            ..Default::default()
        }
        .new_candidate_host()?,
    )
    .to_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{OfferRequest, SignalAdapter};
    use rtc::peer_connection::RTCPeerConnectionBuilder;
    use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
    use rtc::peer_connection::configuration::media_engine::MediaEngine;

    #[test]
    fn create_client_registers_room_and_peer_connection() {
        let mut driver = UdpDriver::default();
        driver
            .create_client(100, 200)
            .expect("client creation should succeed");

        let client = driver.core.client(200).expect("client should exist");
        assert_eq!(client.room_id, 100);
        assert!(client.has_peer_connection());
        assert!(
            driver
                .core
                .rooms
                .get(&100)
                .is_some_and(|room| room.clients.contains(&200))
        );
    }

    #[test]
    fn create_client_is_idempotent() {
        let mut driver = UdpDriver::default();
        driver
            .create_client(100, 200)
            .expect("initial client creation should succeed");
        driver
            .create_client(999, 200)
            .expect("re-creating same client should be a no-op");

        let client = driver.core.client(200).expect("client should still exist");
        assert_eq!(client.room_id, 100);
    }

    #[test]
    fn close_client_command_requests_loop_stop() {
        let mut driver = UdpDriver::default();
        driver
            .create_client(100, 200)
            .expect("client creation should succeed");

        let should_stop = driver
            .handle_command(SfuCommand::CloseClient { client: 200 }, 200)
            .expect("close command should succeed");

        assert!(should_stop);
        assert!(driver.core.client(200).is_none());
    }

    #[test]
    fn accept_offer_creates_answer_and_local_candidate() {
        let mut driver = UdpDriver::default();
        let offer = create_offer().expect("offer should be created");

        let (answer, local_candidate) = driver
            .accept_offer(
                100,
                200,
                offer,
                "127.0.0.1:3478"
                    .parse()
                    .expect("socket address should parse"),
            )
            .expect("offer should be accepted");

        assert_eq!(answer.sdp_type.to_string(), "answer");
        assert!(local_candidate.candidate.starts_with("candidate:"));
        assert!(
            driver
                .core
                .client(200)
                .and_then(|client| client.pc.as_ref())
                .and_then(|pc| pc.local_description())
                .is_some()
        );
    }

    #[test]
    fn signal_adapter_handles_offer() {
        let mut driver = UdpDriver::default();
        let adapter = SignalAdapter;
        let request = OfferRequest {
            request_id: 7,
            room_id: 100,
            client_id: 200,
            offer: create_offer().expect("offer should be created"),
            local_addr: "127.0.0.1:3478"
                .parse()
                .expect("socket address should parse"),
        };

        let response = adapter
            .handle_offer(&mut driver, request)
            .expect("offer should be handled");

        assert_eq!(response.request_id, 7);
        assert_eq!(response.client_id, 200);
        assert_eq!(response.answer.sdp_type.to_string(), "answer");
        assert!(response.local_candidate.candidate.starts_with("candidate:"));
    }

    fn create_offer() -> Result<RTCSessionDescription> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
        let mut pc = RTCPeerConnectionBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build()?;
        let _ = pc.create_data_channel("test-channel", None)?;
        pc.add_local_candidate(host_candidate(
            "127.0.0.1:4000".parse().expect("socket address should parse"),
        )?)?;
        let offer = pc.create_offer(None)?;
        pc.set_local_description(offer.clone())?;
        pc.local_description()
            .ok_or_else(|| Error::Other("missing local offer description".to_owned()))
    }
}
