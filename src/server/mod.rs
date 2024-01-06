use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod certificate;
pub mod config;
pub mod room;
pub mod states;

use crate::shared::types::RoomId;
use room::Room;

pub struct ServerStates {
    rooms: RefCell<HashMap<RoomId, Rc<Room>>>,
}

impl Default for ServerStates {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerStates {
    pub fn new() -> Self {
        Self {
            rooms: RefCell::new(HashMap::new()),
        }
    }
}
