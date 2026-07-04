use crate::{ClientId, RoomId};
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;

#[derive(Debug)]
pub enum SFUCommand {
    AcceptOffer {
        request_id: u64,
        room: RoomId,
        client: ClientId,
        offer: RTCSessionDescription,
    },
    AcceptAnswer {
        request_id: u64,
        room: RoomId,
        client: ClientId,
        answer: RTCSessionDescription,
    },
    AddRemoteCandidate {
        room: RoomId,
        client: ClientId,
        candidate: RTCIceCandidateInit,
    },
    CloseClient {
        room: RoomId,
        client: ClientId,
    },
}

#[derive(Debug)]
pub enum SFUEvent {
    Answer {
        request_id: u64,
        room: RoomId,
        client: ClientId,
        answer: RTCSessionDescription,
    },
    Offer {
        room: RoomId,
        client: ClientId,
        offer: RTCSessionDescription,
    },
    LocalCandidate {
        room: RoomId,
        client: ClientId,
        candidate: RTCIceCandidateInit,
    },
    ClientConnected {
        room: RoomId,
        client: ClientId,
    },
    ClientDisconnected {
        room: RoomId,
        client: ClientId,
    },
    Error {
        request_id: Option<u64>,
        room: Option<RoomId>,
        client: Option<ClientId>,
        error: String,
    },
}
