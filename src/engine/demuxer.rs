use crate::ClientId;
use rtc::shared::FourTuple;
use rtc::shared::TaggedBytesMut;
use rtc::stun::attributes::ATTR_USERNAME;
use rtc::stun::message::{Message, is_stun_message};
use rtc::stun::textattrs::Username;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Demuxer {
    by_ufrag: HashMap<String, ClientId>,
    by_tuple: HashMap<FourTuple, ClientId>,
}

impl Demuxer {
    pub fn bind_ufrag(&mut self, local_ufrag: impl Into<String>, client_id: ClientId) {
        self.by_ufrag.insert(local_ufrag.into(), client_id);
    }

    pub fn bind_tuple(&mut self, four_tuple: FourTuple, client_id: ClientId) {
        self.by_tuple.insert(four_tuple, client_id);
    }

    pub fn demux(&self, pkt: &TaggedBytesMut) -> Option<ClientId> {
        let four_tuple = FourTuple::from(&pkt.transport);
        if let Some(client_id) = self.by_tuple.get(&four_tuple) {
            return Some(*client_id);
        }

        self.demux_stun_username(pkt.message.as_ref())
    }

    fn demux_stun_username(&self, message: &[u8]) -> Option<ClientId> {
        if !is_stun_message(message) {
            return None;
        }

        let mut stun = Message::new();
        stun.unmarshal_binary(message).ok()?;

        let username = Username::get_from_as(&stun, ATTR_USERNAME).ok()?;
        let local_ufrag = username.text.split_once(':')?.0;
        self.by_ufrag.get(local_ufrag).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use rtc::shared::{TransportContext, TransportProtocol};
    use rtc::stun::message::{BINDING_REQUEST, Message, Setter, TransactionId};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::Instant;

    fn tagged_packet(message: Vec<u8>, local_port: u16, peer_port: u16) -> TaggedBytesMut {
        TaggedBytesMut {
            now: Instant::now(),
            transport: TransportContext {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), local_port),
                peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), peer_port),
                transport_protocol: TransportProtocol::UDP,
                ecn: None,
            },
            message: BytesMut::from(message.as_slice()),
        }
    }

    fn build_stun_binding(username: &str) -> Vec<u8> {
        let mut msg = Message::new();
        let setters: [Box<dyn Setter>; 3] = [
            Box::new(BINDING_REQUEST),
            Box::new(TransactionId([1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2])),
            Box::new(Username::new(ATTR_USERNAME, username.to_owned())),
        ];
        msg.build(&setters)
            .expect("stun binding request should build");
        msg.raw
    }

    #[test]
    fn routes_by_four_tuple_before_stun_parsing() {
        let mut demuxer = Demuxer::default();
        let packet = tagged_packet(vec![0u8; 4], 5000, 6000);
        demuxer.bind_tuple(FourTuple::from(&packet.transport), 7);
        demuxer.bind_ufrag("local", 9);

        assert_eq!(demuxer.demux(&packet), Some(7));
    }

    #[test]
    fn routes_stun_by_local_ufrag_from_username() {
        let mut demuxer = Demuxer::default();
        demuxer.bind_ufrag("localufrag", 42);

        let packet = tagged_packet(build_stun_binding("localufrag:remoteufrag"), 5001, 6001);

        assert_eq!(demuxer.demux(&packet), Some(42));
    }

    #[test]
    fn returns_none_for_unknown_or_invalid_packets() {
        let mut demuxer = Demuxer::default();
        demuxer.bind_ufrag("known", 11);

        let unknown = tagged_packet(build_stun_binding("other:remote"), 5002, 6002);
        let invalid = tagged_packet(vec![1, 2, 3, 4, 5], 5003, 6003);

        assert_eq!(demuxer.demux(&unknown), None);
        assert_eq!(demuxer.demux(&invalid), None);
    }
}
