use crate::ids::ClientId;
use rtc::shared::FourTuple;
use rtc::shared::TaggedBytesMut;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Router {
    by_ufrag: HashMap<String, ClientId>,
    by_tuple: HashMap<FourTuple, ClientId>,
}

impl Router {
    pub fn bind_ufrag(&mut self, local_ufrag: impl Into<String>, client_id: ClientId) {
        self.by_ufrag.insert(local_ufrag.into(), client_id);
    }

    pub fn bind_tuple(&mut self, four_tuple: FourTuple, client_id: ClientId) {
        self.by_tuple.insert(four_tuple, client_id);
    }

    pub fn route(&self, pkt: &TaggedBytesMut) -> Option<ClientId> {
        let four_tuple = FourTuple::from(&pkt.transport);
        self.by_tuple.get(&four_tuple).copied()
    }
}
