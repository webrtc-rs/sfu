use crate::ClientId;
use std::collections::HashSet;

pub type RoomId = u64;

#[derive(Debug, Default)]
pub struct Room {
    pub id: RoomId,
    pub clients: HashSet<ClientId>,
}

impl Room {
    pub fn new(id: RoomId) -> Self {
        Self {
            id,
            clients: HashSet::new(),
        }
    }
}
