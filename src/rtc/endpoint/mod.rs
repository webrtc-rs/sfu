use crate::rtc::proto::ice;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct Endpoint {
    room_id: u64,
    endpoint_id: u64,
    pub(crate) ice_agent: Mutex<ice::Agent>,
}

impl Endpoint {
    pub fn new(room_id: u64, endpoint_id: u64) -> Self {
        Self {
            room_id,
            endpoint_id,
            ice_agent: Mutex::new(ice::Agent::default()),
        }
    }

    pub fn room_id(&self) -> u64 {
        self.room_id
    }

    pub fn endpoint_id(&self) -> u64 {
        self.endpoint_id
    }

    pub fn accept_offer(&self, offer: &str) -> Result<String, std::io::Error> {
        Ok(offer.to_string())
    }

    pub fn accept_answer(&self, _answer: &str) -> Result<(), std::io::Error> {
        Ok(())
    }

    pub fn accept_trickle(&self, _trickle: &str) -> Result<(), std::io::Error> {
        Ok(())
    }
}
