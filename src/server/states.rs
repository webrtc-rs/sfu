use crate::server::config::ServerConfig;
use crate::server::endpoint::candidate::{Candidate, ConnectionCredentials};
use crate::server::endpoint::Endpoint;
use crate::server::session::description::RTCSessionDescription;
use crate::server::session::Session;
use crate::types::{EndpointId, FourTuple, SessionId, UserName};
use shared::error::{Error, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug)]
pub struct ServerStates {
    config: Arc<ServerConfig>,
    local_addr: SocketAddr,
    sessions: RefCell<HashMap<SessionId, Rc<Session>>>,

    // Thread-local map
    endpoints: RefCell<HashMap<FourTuple, Rc<Endpoint>>>,
    candidates: RefCell<HashMap<UserName, Rc<Candidate>>>,
}

impl ServerStates {
    pub fn new(config: Arc<ServerConfig>, local_addr: SocketAddr) -> Result<Self> {
        let _ = config
            .certificates
            .first()
            .ok_or(Error::ErrInvalidCertificate)?
            .get_fingerprints()
            .first()
            .ok_or(Error::ErrInvalidCertificate)?;

        Ok(Self {
            config,
            local_addr,
            sessions: RefCell::new(HashMap::new()),

            endpoints: RefCell::new(HashMap::new()),
            candidates: RefCell::new(HashMap::new()),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn create_or_get_session(&self, session_id: SessionId) -> Rc<Session> {
        let mut sessions = self.sessions.borrow_mut();
        if let Some(session) = sessions.get(&session_id) {
            session.clone()
        } else {
            let session = Rc::new(Session::new(
                session_id,
                self.local_addr,
                self.config.certificates.clone(),
            ));
            sessions.insert(session_id, Rc::clone(&session));
            session
        }
    }

    // set pending offer and return answer
    pub fn accept_pending_offer(
        &self,
        session_id: SessionId,
        endpoint_id: EndpointId,
        mut offer: RTCSessionDescription,
    ) -> Result<RTCSessionDescription> {
        offer.unmarshal()?;
        let parsed = offer.unmarshal()?;
        let remote_conn_cred = ConnectionCredentials::from_sdp(&parsed)?;
        offer.parsed = Some(parsed);

        let local_conn_cred = ConnectionCredentials::new(
            &self.config.certificates,
            remote_conn_cred.dtls_params.role,
        );

        let session = self.create_or_get_session(session_id);
        let answer =
            session.create_pending_answer(endpoint_id, &offer, &local_conn_cred.ice_params)?;

        self.add_candidate(Rc::new(Candidate::new(
            session.session_id(),
            endpoint_id,
            remote_conn_cred,
            local_conn_cred,
            offer,
            answer.clone(),
        )));

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
