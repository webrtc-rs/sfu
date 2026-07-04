use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;

use crate::client::ClientId;
use crate::room::RoomId;

pub type RequestId = u64;

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
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
    ClientConnected {
        request_id: RequestId,
        room_id: RoomId,
        client_id: ClientId,
    },
    ClientDisconnected {
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
