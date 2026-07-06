use rtc::shared::FourTuple;
use rtc::shared::TaggedBytesMut;
use rtc::stun::attributes::ATTR_USERNAME;
use rtc::stun::message::{Message, is_stun_message};
use rtc::stun::textattrs::Username;
use std::collections::{HashMap, HashSet};

use crate::client::ClientId;
use crate::room::RoomId;

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

        let username = Username::get_from_as(&stun, ATTR_USERNAME).ok()?;
        let local_ufrag = username.text.split_once(':')?.0;
        let room_client = local_ufrag.split_once('+')?.0;
        let fields = room_client.split('/').collect::<Vec<&str>>();
        if fields.len() >= 2 {
            let room_id = fields[0].parse().ok()?;
            let client_id = fields[1].parse().ok()?;
            let four_tuple = pkt.transport.into();
            self.affinity.insert(four_tuple, (room_id, client_id));
            self.reverse
                .entry((room_id, client_id))
                .or_default()
                .insert(four_tuple);

            Some((room_id, client_id))
        } else {
            None
        }
    }
}
