use crate::server::endpoint::candidate::Candidate;
use crate::server::endpoint::Endpoint;
use crate::types::FourTuple;
use srtp::context::Context;
use std::rc::{Rc, Weak};

pub struct Transport {
    four_tuple: FourTuple,
    endpoint: Weak<Endpoint>,

    // ICE
    candidate: Rc<Candidate>,

    // SRTP
    local_srtp_context: Option<Context>,
    remote_srtp_context: Option<Context>,
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

            local_srtp_context: None,
            remote_srtp_context: None,
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

    pub(crate) fn local_srtp_context(&mut self) -> Option<&mut Context> {
        self.local_srtp_context.as_mut()
    }

    pub(crate) fn remote_srtp_context(&mut self) -> Option<&mut Context> {
        self.remote_srtp_context.as_mut()
    }

    pub(crate) fn set_local_srtp_context(&mut self, local_srtp_context: Context) {
        self.local_srtp_context = Some(local_srtp_context);
    }

    pub(crate) fn set_remote_srtp_context(&mut self, remote_srtp_context: Context) {
        self.remote_srtp_context = Some(remote_srtp_context);
    }
}
