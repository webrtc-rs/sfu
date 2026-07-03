use crate::forward::ForwardKey;
use crate::ids::{ClientId, RoomId};
use rtc::interceptor::NoopInterceptor;
use rtc::peer_connection::RTCPeerConnection;
use rtc::rtp_transceiver::{RTCRtpReceiverId, RTCRtpSenderId};
use std::collections::HashMap;

pub type ClientPeerConnection = RTCPeerConnection<NoopInterceptor>;

#[derive(Debug, Clone)]
pub struct InboundTrack {
    pub track_id: String,
}

pub struct Client {
    pub id: ClientId,
    pub room_id: RoomId,
    pub pending_request: Option<u64>,
    pub pc: Option<ClientPeerConnection>,
    pub inbound: HashMap<RTCRtpReceiverId, InboundTrack>,
    pub outbound: HashMap<ForwardKey, RTCRtpSenderId>,
}

impl Client {
    pub fn new(id: ClientId, room_id: RoomId) -> Self {
        Self {
            id,
            room_id,
            pending_request: None,
            pc: None,
            inbound: HashMap::new(),
            outbound: HashMap::new(),
        }
    }
}
