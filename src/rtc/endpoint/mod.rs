#[derive(Default)]
pub(crate) struct UfragPwd {
    pub(crate) local_ufrag: String,
    pub(crate) local_pwd: String,
    pub(crate) remote_ufrag: String,
    pub(crate) remote_pwd: String,
}

#[derive(Default)]
pub struct Endpoint {
    room_id: u64,
    endpoint_id: u64,
    ufrag_pwd: UfragPwd,
}

impl Endpoint {
    pub fn new(room_id: u64, endpoint_id: u64) -> Self {
        Self {
            room_id,
            endpoint_id,
            ufrag_pwd: UfragPwd::default(),
        }
    }

    pub fn room_id(&self) -> u64 {
        self.room_id
    }

    pub fn endpoint_id(&self) -> u64 {
        self.endpoint_id
    }

    pub(crate) fn ufrag_pwd(&self) -> &UfragPwd {
        &self.ufrag_pwd
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
