use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;

use crate::client::ClientId;
use crate::room::RoomId;

pub type RequestId = u64;

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    Ok {
        request_id: RequestId,
        room_id: Option<RoomId>,
        client_id: Option<ClientId>,
    },
    Err {
        request_id: RequestId,
        room_id: Option<RoomId>,
        client_id: Option<ClientId>,
        reason: String,
    },
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
}

impl Event {
    pub fn request_id(&self) -> RequestId {
        match self {
            Event::Ok { request_id, .. } => *request_id,
            Event::Err { request_id, .. } => *request_id,
            Event::Join { request_id, .. } => *request_id,
            Event::SessionDescription { request_id, .. } => *request_id,
            Event::IceCandidate { request_id, .. } => *request_id,
            Event::Leave { request_id, .. } => *request_id,
        }
    }
    pub fn room_id(&self) -> Option<RoomId> {
        match self {
            Event::Ok { room_id, .. } => *room_id,
            Event::Err { room_id, .. } => *room_id,
            Event::Join { room_id, .. } => Some(*room_id),
            Event::SessionDescription { room_id, .. } => Some(*room_id),
            Event::IceCandidate { room_id, .. } => Some(*room_id),
            Event::Leave { room_id, .. } => Some(*room_id),
        }
    }

    pub fn client_id(&self) -> Option<ClientId> {
        match self {
            Event::Ok { client_id, .. } => *client_id,
            Event::Err { client_id, .. } => *client_id,
            Event::Join { client_id, .. } => Some(*client_id),
            Event::SessionDescription { client_id, .. } => Some(*client_id),
            Event::IceCandidate { client_id, .. } => Some(*client_id),
            Event::Leave { client_id, .. } => Some(*client_id),
        }
    }
}
