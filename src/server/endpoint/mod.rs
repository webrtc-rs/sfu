pub mod candidate;
pub mod transport;

use crate::server::endpoint::transport::Transport;
use crate::server::session::description::rtp_transceiver::RTCRtpTransceiver;
use crate::server::session::Session;
use crate::types::{EndpointId, FourTuple, Mid};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Weak;

#[derive(Default)]
pub(crate) struct Endpoint {
    session: Weak<Session>,
    endpoint_id: EndpointId,
    transports: RefCell<HashMap<FourTuple, Transport>>,
    transceivers: RefCell<HashMap<Mid, RTCRtpTransceiver>>,
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

    pub(crate) fn add_transport(&self, transport: Transport) {
        let mut transports = self.transports.borrow_mut();
        transports.insert(*transport.four_tuple(), transport);
    }

    pub(crate) fn remove_transport(&self, four_tuple: &FourTuple) {
        let mut transports = self.transports.borrow_mut();
        transports.remove(four_tuple);
    }

    pub(crate) fn has_transport(&self, four_tuple: &FourTuple) -> bool {
        let transports = self.transports.borrow();
        transports.contains_key(four_tuple)
    }

    pub(crate) fn transports(&self) -> &RefCell<HashMap<FourTuple, Transport>> {
        &self.transports
    }

    pub(crate) fn transceivers(&self) -> &RefCell<HashMap<Mid, RTCRtpTransceiver>> {
        &self.transceivers
    }
}
