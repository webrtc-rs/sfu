pub mod candidate;
pub mod transport;

use crate::server::endpoint::transport::Transport;
use crate::server::session::description::rtp_transceiver::RTCRtpTransceiver;
use crate::server::session::Session;
use crate::types::{EndpointId, FourTuple, Mid};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

#[derive(Default, Debug, Clone)]
pub(crate) struct Endpoint {
    session: Weak<Session>,
    endpoint_id: EndpointId,
    transports: RefCell<HashMap<FourTuple, Rc<Transport>>>,
    pub(crate) transceivers: RefCell<HashMap<Mid, RTCRtpTransceiver>>,
}

impl Endpoint {
    pub(crate) fn new(session: Weak<Session>, endpoint_id: EndpointId) -> Self {
        Self {
            session,
            endpoint_id,
            transports: RefCell::new(HashMap::new()),
            transceivers: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn session(&self) -> &Weak<Session> {
        &self.session
    }

    pub(crate) fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    pub(crate) fn add_transport(&self, transport: Rc<Transport>) {
        let mut transports = self.transports.borrow_mut();
        transports.insert(*transport.four_tuple(), transport);
    }

    pub(crate) fn get_transport(&self, four_tuple: &FourTuple) -> Option<Rc<Transport>> {
        self.transports.borrow().get(four_tuple).cloned()
    }
}
