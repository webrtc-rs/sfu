use crate::ids::{ClientId, RoomId};
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::transport::RTCIceCandidateInit;

#[derive(Debug)]
pub enum SfuCommand {
    AcceptOffer {
        request_id: u64,
        room: RoomId,
        client: ClientId,
        offer: RTCSessionDescription,
    },
    AcceptAnswer {
        request_id: u64,
        client: ClientId,
        answer: RTCSessionDescription,
    },
    AddLocalCandidate {
        client: ClientId,
        candidate: RTCIceCandidateInit,
    },
    CloseClient {
        client: ClientId,
    },
}

#[derive(Debug)]
pub enum SfuEvent {
    Answer {
        request_id: u64,
        client: ClientId,
        answer: RTCSessionDescription,
    },
    Offer {
        client: ClientId,
        offer: RTCSessionDescription,
    },
    LocalCandidate {
        client: ClientId,
        candidate: RTCIceCandidateInit,
    },
    ClientConnected {
        client: ClientId,
    },
    ClientDisconnected {
        client: ClientId,
    },
    Error {
        request_id: Option<u64>,
        client: Option<ClientId>,
        error: String,
    },
}
