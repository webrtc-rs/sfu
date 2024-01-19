pub mod candidate;
pub mod transport;

use crate::server::endpoint::transport::Transport;
use crate::server::session::Session;
use crate::types::{EndpointId, FourTuple};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

#[derive(Default, Debug, Clone)]
pub struct Endpoint {
    session: Weak<Session>,
    endpoint_id: EndpointId,
    transports: RefCell<HashMap<FourTuple, Rc<Transport>>>,
}

impl Endpoint {
    pub fn new(session: Weak<Session>, endpoint_id: EndpointId) -> Self {
        Self {
            session,
            endpoint_id,
            transports: RefCell::new(HashMap::new()),
        }
    }

    pub fn session(&self) -> &Weak<Session> {
        &self.session
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    pub fn add_transport(&self, transport: Rc<Transport>) {
        let mut transports = self.transports.borrow_mut();
        transports.insert(*transport.four_tuple(), transport);
    }

    pub fn get_transport(&self, four_tuple: &FourTuple) -> Option<Rc<Transport>> {
        self.transports.borrow().get(four_tuple).cloned()
    }
}
