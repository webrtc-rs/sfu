pub struct Endpoint {}

impl Default for Endpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl Endpoint {
    pub fn new() -> Self {
        Self {}
    }

    pub fn accept_offer(&mut self, offer: &str) -> Result<String, std::io::Error> {
        Ok(offer.to_string())
    }

    pub fn accept_answer(&mut self, _answer: &str) -> Result<(), std::io::Error> {
        Ok(())
    }
}
