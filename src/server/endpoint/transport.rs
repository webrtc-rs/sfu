use crate::server::endpoint::candidate::Candidate;
use crate::server::endpoint::Endpoint;
use crate::types::FourTuple;
use srtp::context::Context;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub struct Transport {
    four_tuple: FourTuple,
    endpoint: Weak<Endpoint>,

    // ICE
    candidate: Rc<Candidate>,

    // SRTP
    local_srtp_context: RefCell<Option<Context>>,
    remote_srtp_context: RefCell<Option<Context>>,
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

            local_srtp_context: RefCell::new(None),
            remote_srtp_context: RefCell::new(None),
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

    pub(crate) fn local_srtp_context(&self) -> &RefCell<Option<Context>> {
        &self.local_srtp_context
    }

    pub(crate) fn remote_srtp_context(&self) -> &RefCell<Option<Context>> {
        &self.remote_srtp_context
    }
}
