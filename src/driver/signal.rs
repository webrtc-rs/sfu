use crate::driver::UdpDriver;
use crate::engine::{ClientId, RoomId};
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;
use rtc::shared::error::Result;
use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct OfferRequest {
    pub request_id: u64,
    pub room_id: RoomId,
    pub client_id: ClientId,
    pub offer: RTCSessionDescription,
    pub local_addr: SocketAddr,
}

#[derive(Debug, Clone)]
pub struct OfferResponse {
    pub request_id: u64,
    pub client_id: ClientId,
    pub answer: RTCSessionDescription,
    pub local_candidate: RTCIceCandidateInit,
}

#[derive(Debug, Default)]
pub struct SignalAdapter;

impl SignalAdapter {
    pub fn handle_offer(
        &self,
        driver: &mut UdpDriver,
        request: OfferRequest,
    ) -> Result<OfferResponse> {
        let (answer, local_candidate) = driver.accept_offer(
            request.room_id,
            request.client_id,
            request.offer,
            request.local_addr,
        )?;

        Ok(OfferResponse {
            request_id: request.request_id,
            client_id: request.client_id,
            answer,
            local_candidate,
        })
    }
}
