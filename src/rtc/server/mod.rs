use std::cell::RefCell;
use crate::rtc::room::Room;
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) mod handlers;
pub mod udp_rtc_server;

pub struct ServerStates {
    rooms: RefCell<HashMap<u64, Rc<Room>>>,
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

    pub async fn insert(&self, room: Rc<Room>) {
        let mut rooms = self.rooms.lock().await;
        rooms.insert(room.room_id(), room);
    }

    pub async fn get(&self, room_id: u64) -> Option<Rc<Room>> {
        let rooms = self.rooms.lock().await;
        rooms.get(&room_id).cloned()
    }
}
