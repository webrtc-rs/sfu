use crate::server::config::{ServerConfig, SessionConfig};
use crate::server::endpoint::candidate::{Candidate, ConnectionCredentials};
use crate::server::endpoint::Endpoint;
use crate::server::session::description::RTCSessionDescription;
use crate::server::session::Session;
use crate::types::{EndpointId, FourTuple, SessionId, UserName};
use sctp::{Association, AssociationHandle};
use shared::error::{Error, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

pub struct ServerStates {
    server_config: Arc<ServerConfig>,
    local_addr: SocketAddr,
    sessions: RefCell<HashMap<SessionId, Rc<Session>>>,

    //TODO: add idle timeout cleanup logic to remove idle endpoint and candidates
    candidates: RefCell<HashMap<UserName, Rc<Candidate>>>,
    endpoints: RefCell<HashMap<FourTuple, Rc<Endpoint>>>,

    sctp_associations: RefCell<HashMap<AssociationHandle, Association>>,
}

impl ServerStates {
    /// create new server states
    pub fn new(server_config: Arc<ServerConfig>, local_addr: SocketAddr) -> Result<Self> {
        let _ = server_config
            .certificates
            .first()
            .ok_or(Error::ErrInvalidCertificate)?
            .get_fingerprints()
            .first()
            .ok_or(Error::ErrInvalidCertificate)?;

        Ok(Self {
            server_config,
            local_addr,
            sessions: RefCell::new(HashMap::new()),

            candidates: RefCell::new(HashMap::new()),
            endpoints: RefCell::new(HashMap::new()),

            sctp_associations: RefCell::new(HashMap::new()),
        })
    }

    /// accept offer and return answer
    pub fn accept_offer(
        &self,
        session_id: SessionId,
        endpoint_id: EndpointId,
        four_tuple: Option<FourTuple>,
        mut offer: RTCSessionDescription,
    ) -> Result<RTCSessionDescription> {
        let parsed = offer.unmarshal()?;
        let remote_conn_cred = ConnectionCredentials::from_sdp(&parsed)?;
        offer.parsed = Some(parsed);

        let session = self.create_or_get_session(session_id);
        let endpoint = session.get_endpoint(&endpoint_id);

        let local_conn_cred = if let Some(endpoint) = endpoint.as_ref() {
            session.set_remote_description(endpoint, &offer)?;

            let four_tuple = four_tuple.ok_or(Error::Other("missing FourTuple".to_string()))?;
            let transports = endpoint.transports().borrow();
            let transport = transports.get(&four_tuple).ok_or(Error::Other(format!(
                "can't find transport for endpoint id {} with {:?}",
                endpoint_id, four_tuple
            )))?;
            transport.candidate().local_connection_credentials().clone()
        } else {
            ConnectionCredentials::new(
                &self.server_config.certificates,
                remote_conn_cred.dtls_params.role,
            )
        };

        let answer = session.create_answer(&endpoint, &offer, &local_conn_cred.ice_params)?;

        if endpoint.is_none() {
            self.add_candidate(Rc::new(Candidate::new(
                session.session_id(),
                endpoint_id,
                remote_conn_cred,
                local_conn_cred,
                offer,
                answer.clone(),
                Instant::now() + self.server_config.candidate_idle_timeout,
            )));
        }

        Ok(answer)
    }

    pub(crate) fn accept_answer(
        &self,
        session_id: SessionId,
        endpoint_id: EndpointId,
        _four_tuple: FourTuple,
        mut answer: RTCSessionDescription,
    ) -> Result<()> {
        let parsed = answer.unmarshal()?;
        answer.parsed = Some(parsed);

        let session = self.create_or_get_session(session_id);
        if let Some(endpoint) = session.get_endpoint(&endpoint_id) {
            session.set_remote_description(&endpoint, &answer)?;
        };

        Ok(())
    }

    pub(crate) fn server_config(&self) -> &Arc<ServerConfig> {
        &self.server_config
    }

    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub(crate) fn sctp_associations(&self) -> &RefCell<HashMap<AssociationHandle, Association>> {
        &self.sctp_associations
    }

    pub(crate) fn create_or_get_session(&self, session_id: SessionId) -> Rc<Session> {
        let mut sessions = self.sessions.borrow_mut();
        if let Some(session) = sessions.get(&session_id) {
            session.clone()
        } else {
            let session = Rc::new(Session::new(
                SessionConfig::new(Arc::clone(&self.server_config), self.local_addr),
                session_id,
            ));
            sessions.insert(session_id, Rc::clone(&session));
            session
        }
    }

    pub(crate) fn get_session(&self, session_id: &SessionId) -> Option<Rc<Session>> {
        self.sessions.borrow().get(session_id).cloned()
    }

    pub(crate) fn add_candidate(&self, candidate: Rc<Candidate>) -> Option<Rc<Candidate>> {
        let username = candidate.username();
        let mut candidates = self.candidates.borrow_mut();
        candidates.insert(username, candidate)
    }

    pub(crate) fn remove_candidate(&self, username: &UserName) -> Option<Rc<Candidate>> {
        let mut candidates = self.candidates.borrow_mut();
        candidates.remove(username)
    }

    pub(crate) fn find_candidate(&self, username: &UserName) -> Option<Rc<Candidate>> {
        let candidates = self.candidates.borrow();
        candidates.get(username).cloned()
    }

    pub(crate) fn add_endpoint(
        &self,
        four_tuple: FourTuple,
        endpoint: Rc<Endpoint>,
    ) -> Option<Rc<Endpoint>> {
        let mut endpoints = self.endpoints.borrow_mut();
        endpoints.insert(four_tuple, endpoint)
    }

    pub(crate) fn remove_endpoint(&self, four_tuple: &FourTuple) -> Option<Rc<Endpoint>> {
        let mut endpoints = self.endpoints.borrow_mut();
        endpoints.remove(four_tuple)
    }

    pub(crate) fn find_endpoint(&self, four_tuple: &FourTuple) -> Option<Rc<Endpoint>> {
        let endpoints = self.endpoints.borrow();
        endpoints.get(four_tuple).cloned()
    }
}
