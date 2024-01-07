use crate::server::config::ServerConfig;
use crate::server::session::{endpoint::Endpoint, Session};
use crate::shared::types::{FourTuple, SessionId};
use shared::error::{Error, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug)]
pub struct ServerStates {
    config: Arc<ServerConfig>,
    sessions: RefCell<HashMap<SessionId, Rc<Session>>>,
    endpoints: RefCell<HashMap<FourTuple, Rc<Endpoint>>>,
}

impl ServerStates {
    pub fn new(config: Arc<ServerConfig>) -> Result<Self> {
        let _ = config
            .certificate
            .get_fingerprints()
            .first()
            .ok_or(Error::ErrInvalidCertificate)?;

        Ok(Self {
            config,
            sessions: RefCell::new(HashMap::new()),
            endpoints: RefCell::new(HashMap::new()),
        })
    }

    pub fn create_or_get_session(&self, session_id: SessionId) -> Rc<Session> {
        let mut sessions = self.sessions.borrow_mut();
        if let Some(session) = sessions.get(&session_id) {
            session.clone()
        } else {
            let session = Rc::new(Session::new(
                session_id,
                self.config
                    .certificate
                    .get_fingerprints()
                    .first()
                    .unwrap()
                    .clone(),
            ));
            sessions.insert(session_id, Rc::clone(&session));
            session
        }
    }
}
