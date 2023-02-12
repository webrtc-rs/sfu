use crate::rtc::session::endpoint::Endpoint;
use tokio::sync::Mutex;

pub struct ServerStates {
    endpoints: Mutex<Vec<Endpoint>>,
}

impl Default for ServerStates {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerStates {
    pub fn new() -> Self {
        Self {
            endpoints: Mutex::new(vec![]),
        }
    }

    pub async fn insert(&self, endpoint: Endpoint) {
        let mut endpoints = self.endpoints.lock().await;
        endpoints.push(endpoint);
    }
}
