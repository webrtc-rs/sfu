use crate::rtc::room::Room;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) mod udp_demuxer_handler;
pub mod udp_rtc_server;

pub struct ServerStates {
    sessions: Mutex<HashMap<usize, Arc<Room>>>,
}

impl Default for ServerStates {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerStates {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn insert(&self, session: Arc<Room>) {
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session.room_id(), session);
    }

    pub async fn get(&self, session_id: usize) -> Option<Arc<Room>> {
        let sessions = self.sessions.lock().await;
        sessions.get(&session_id).cloned()
    }
}
