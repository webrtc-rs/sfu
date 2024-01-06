use retty::transport::TransportContext;
use std::net::SocketAddr;

pub type EndpointId = u64;
pub type RoomId = u64;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct FourTuple {
    pub local_addr: SocketAddr,
    pub peer_addr: SocketAddr,
}

impl From<&TransportContext> for FourTuple {
    fn from(value: &TransportContext) -> Self {
        Self {
            local_addr: value.local_addr,
            peer_addr: value.peer_addr,
        }
    }
}
