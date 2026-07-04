use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;

use crate::client::ClientId;
use crate::room::RoomId;

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    SessionDescription {
        request_id: u64,
        room_id: RoomId,
        client_id: ClientId,
        sdp: RTCSessionDescription,
    },
    IceCandidate {
        request_id: u64,
        room_id: RoomId,
        client_id: ClientId,
        candidate: RTCIceCandidateInit,
    },
    ClientConnected {
        request_id: u64,
        room_id: RoomId,
        client_id: ClientId,
    },
    ClientDisconnected {
        request_id: u64,
        room_id: RoomId,
        client_id: ClientId,
    },
    Error {
        request_id: u64,
        room_id: RoomId,
        client_id: ClientId,
        reason: String,
    },
}
