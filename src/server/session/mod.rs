use shared::error::Result;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod description;

use crate::server::certificate::RTCDtlsFingerprint;
use crate::server::endpoint::{
    candidate::{Candidate, ConnectionCredentials},
    Endpoint,
};
use crate::server::session::description::RTCSessionDescription;
use crate::shared::types::{EndpointId, SessionId, UserName};

#[derive(Default, Debug)]
pub struct Session {
    session_id: SessionId,
    fingerprint: RTCDtlsFingerprint,
    endpoints: RefCell<HashMap<EndpointId, Rc<Endpoint>>>,
    candidates: RefCell<HashMap<UserName, Rc<Candidate>>>,
}

impl Session {
    pub fn new(session_id: SessionId, fingerprint: RTCDtlsFingerprint) -> Self {
        Self {
            session_id,
            fingerprint,
            endpoints: RefCell::new(HashMap::new()),
            candidates: RefCell::new(HashMap::new()),
        }
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub fn accept_offer(
        &self,
        endpoint_id: EndpointId,
        mut offer: RTCSessionDescription,
    ) -> Result<RTCSessionDescription> {
        offer.unmarshal()?;
        let parsed = offer.unmarshal()?;
        let remote_conn_cred = ConnectionCredentials::from_sdp(&parsed)?;
        offer.parsed = Some(parsed);

        let answer_sdp = offer.sdp.clone();

        let mut candidate = Candidate::new(
            self.session_id,
            endpoint_id,
            &self.fingerprint,
            remote_conn_cred,
            offer,
        );

        //TODO: generate Answer SDP
        let answer = RTCSessionDescription::answer(answer_sdp)?;
        candidate.set_local_description(&answer);

        self.add_candidate(Rc::new(candidate));

        Ok(answer)
    }

    pub(crate) fn add_candidate(&self, candidate: Rc<Candidate>) -> bool {
        let username = candidate.username();
        let mut candidates = self.candidates.borrow_mut();
        candidates.insert(username, candidate).is_some()
    }

    //TODO: add idle timeout to remove candidate
    pub(crate) fn remove_candidate(&self, candidate: &Rc<Candidate>) -> bool {
        let username = candidate.username();
        let mut candidates = self.candidates.borrow_mut();
        candidates.remove(&username).is_some()
    }

    pub(crate) fn find_candidate(&self, username: &UserName) -> Option<Rc<Candidate>> {
        let candidates = self.candidates.borrow();
        candidates.get(username).cloned()
    }
}
