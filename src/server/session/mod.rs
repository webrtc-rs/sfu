use sdp::SessionDescription;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub mod endpoint;

use crate::server::certificate::RTCDtlsFingerprint;
use crate::server::session::endpoint::candidate::{Candidate, ConnectionCredentials};
use crate::shared::types::{EndpointId, SessionId, UserName};
use endpoint::Endpoint;

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
        session_id: SessionId,
        endpoint_id: EndpointId,
        peer_conn_cred: ConnectionCredentials,
        offer_sdp: SessionDescription,
    ) -> SessionDescription {
        let mut candidate = Candidate::new(
            session_id,
            endpoint_id,
            &self.fingerprint,
            peer_conn_cred,
            offer_sdp,
        );

        //TODO: generate Answer SDP
        let answer_sdp = SessionDescription::default();
        candidate.set_answer_sdp(&answer_sdp);

        self.add_candidate(Rc::new(candidate));

        answer_sdp
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
