#[derive(Default)]
pub struct Endpoint {
    room_id: usize,
    endpoint_id: usize,
}

impl Endpoint {
    pub fn new(room_id: usize, endpoint_id: usize) -> Self {
        Self {
            room_id,
            endpoint_id,
        }
    }

    pub fn room_id(&self) -> usize {
        self.room_id
    }

    pub fn endpoint_id(&self) -> usize {
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
