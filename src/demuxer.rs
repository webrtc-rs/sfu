use crate::client::ClientId;
use crate::room::{RoomId, decode_local_ufrag};
use rtc::shared::FourTuple;
use rtc::shared::TaggedBytesMut;
use rtc::stun::attributes::ATTR_USERNAME;
use rtc::stun::message::{Message, is_stun_message};
use rtc::stun::textattrs::Username;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub(crate) struct Demuxer {
    //TODO: handle expiry or eviction
    affinity: HashMap<FourTuple, (RoomId, ClientId)>,
    reverse: HashMap<(RoomId, ClientId), HashSet<FourTuple>>,
}

impl Demuxer {
    pub(crate) fn demux(&mut self, pkt: &TaggedBytesMut) -> Option<(RoomId, ClientId)> {
        let four_tuple = FourTuple::from(&pkt.transport);
        if let Some(room_client) = self.affinity.get(&four_tuple) {
            return Some(*room_client);
        }

        self.demux_stun_username(pkt)
    }

    fn demux_stun_username(&mut self, pkt: &TaggedBytesMut) -> Option<(RoomId, ClientId)> {
        if !is_stun_message(pkt.message.as_ref()) {
            return None;
        }

        let mut stun = Message::new();
        stun.unmarshal_binary(pkt.message.as_ref()).ok()?;

        // USERNAME = local_ufrag ":" remote_ufrag; the local half is what the SFU issued,
        // so it carries the room and client (see `room::encode_local_ufrag`).
        let username = Username::get_from_as(&stun, ATTR_USERNAME).ok()?;
        let local_ufrag = username.text.split_once(':')?.0;
        let (room_id, client_id) = decode_local_ufrag(local_ufrag)?;

        let four_tuple = pkt.transport.into();
        self.affinity.insert(four_tuple, (room_id, client_id));
        self.reverse
            .entry((room_id, client_id))
            .or_default()
            .insert(four_tuple);

        Some((room_id, client_id))
    }
}
