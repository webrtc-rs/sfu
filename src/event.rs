use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;

use crate::client::ClientId;
use crate::room::RoomId;

pub type RequestId = u64;

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    Join {
        request_id: RequestId,
        room_id: RoomId,
        client_id: ClientId,
    },
    SessionDescription {
        request_id: RequestId,
        room_id: RoomId,
        client_id: ClientId,
        sdp: RTCSessionDescription,
    },
    IceCandidate {
        request_id: RequestId,
        room_id: RoomId,
        client_id: ClientId,
        candidate: RTCIceCandidateInit,
    },
    Leave {
        request_id: RequestId,
        room_id: RoomId,
        client_id: ClientId,
        reason: String,
    },
    Error {
        request_id: RequestId,
        room_id: Option<RoomId>,
        client_id: Option<ClientId>,
        reason: String,
    },
}

impl Event {
    pub fn request_id(&self) -> &RequestId {
        match self {
            Event::Join { request_id, .. } => request_id,
            Event::SessionDescription { request_id, .. } => request_id,
            Event::IceCandidate { request_id, .. } => request_id,
            Event::Leave { request_id, .. } => request_id,
            Event::Error { request_id, .. } => request_id,
        }
    }
    pub fn room_id(&self) -> Option<&RoomId> {
        match self {
            Event::Join { room_id, .. } => Some(room_id),
            Event::SessionDescription { room_id, .. } => Some(room_id),
            Event::IceCandidate { room_id, .. } => Some(room_id),
            Event::Leave { room_id, .. } => Some(room_id),
            Event::Error { room_id, .. } => room_id.as_ref(),
        }
    }

    pub fn client_id(&self) -> Option<&ClientId> {
        match self {
            Event::Join { client_id, .. } => Some(client_id),
            Event::SessionDescription { client_id, .. } => Some(client_id),
            Event::IceCandidate { client_id, .. } => Some(client_id),
            Event::Leave { client_id, .. } => Some(client_id),
            Event::Error { client_id, .. } => client_id.as_ref(),
        }
    }
}
