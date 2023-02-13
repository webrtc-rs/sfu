use crate::rtc::room::Room;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) mod demuxer_handler;
pub(crate) mod ice_handler;
pub mod udp_rtc_server;

pub struct ServerStates {
    rooms: Mutex<HashMap<u64, Arc<Room>>>,
}

impl Default for ServerStates {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerStates {
    pub fn new() -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
        }
    }

    pub async fn insert(&self, room: Arc<Room>) {
        let mut rooms = self.rooms.lock().await;
        rooms.insert(room.room_id(), room);
    }

    pub async fn get(&self, room_id: u64) -> Option<Arc<Room>> {
        let rooms = self.rooms.lock().await;
        rooms.get(&room_id).cloned()
    }
}
