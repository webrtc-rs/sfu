use crate::server::endpoint::candidate::Candidate;
use crate::server::endpoint::Endpoint;
use crate::types::FourTuple;
use std::rc::{Rc, Weak};

#[derive(Debug, Clone)]
pub struct Transport {
    four_tuple: FourTuple,
    endpoint: Weak<Endpoint>,
    candidate: Rc<Candidate>,
}

impl Transport {
    pub(crate) fn new(
        four_tuple: FourTuple,
        endpoint: Weak<Endpoint>,
        candidate: Rc<Candidate>,
    ) -> Self {
        Self {
            four_tuple,
            endpoint,
            candidate,
        }
    }

    pub(crate) fn four_tuple(&self) -> &FourTuple {
        &self.four_tuple
    }

    pub(crate) fn endpoint(&self) -> &Weak<Endpoint> {
        &self.endpoint
    }

    pub(crate) fn candidate(&self) -> &Rc<Candidate> {
        &self.candidate
    }
}
