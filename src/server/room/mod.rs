use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod endpoint;

use crate::shared::types::{EndpointId, RoomId};
use endpoint::Endpoint;

#[derive(Default, Debug)]
pub struct Room {
    room_id: RoomId,
    endpoints: RefCell<HashMap<EndpointId, Rc<Endpoint>>>,
}

impl Room {
    pub fn new(room_id: RoomId) -> Self {
        Self {
            room_id,
            endpoints: RefCell::new(HashMap::new()),
        }
    }

    pub fn room_id(&self) -> u64 {
        self.room_id
    }
}
