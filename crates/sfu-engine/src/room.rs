use crate::ids::{ClientId, RoomId};
use std::collections::HashSet;

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
