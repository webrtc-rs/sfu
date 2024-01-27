pub mod candidate;
pub mod transport;

use crate::server::endpoint::transport::Transport;
use crate::server::session::description::rtp_transceiver::RTCRtpTransceiver;
use crate::server::session::description::RTCSessionDescription;
use crate::server::session::Session;
use crate::types::{EndpointId, FourTuple, Mid};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Weak;

#[derive(Default)]
pub(crate) struct Endpoint {
    session: Weak<Session>,
    endpoint_id: EndpointId,

    remote_description: RefCell<Option<RTCSessionDescription>>,
    local_description: RefCell<Option<RTCSessionDescription>>,

    transports: RefCell<HashMap<FourTuple, Transport>>,

    mids: RefCell<Vec<Mid>>,
    transceivers: RefCell<HashMap<Mid, RTCRtpTransceiver>>,
}

impl Endpoint {
    pub(crate) fn new(session: Weak<Session>, endpoint_id: EndpointId) -> Self {
        Self {
            session,
            endpoint_id,

            remote_description: RefCell::new(None),
            local_description: RefCell::new(None),

            transports: RefCell::new(HashMap::new()),

            mids: RefCell::new(vec![]),
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

    pub(crate) fn mids(&self) -> &RefCell<Vec<Mid>> {
        &self.mids
    }

    pub(crate) fn transceivers(&self) -> &RefCell<HashMap<Mid, RTCRtpTransceiver>> {
        &self.transceivers
    }

    pub(crate) fn remote_description(&self) -> Option<RTCSessionDescription> {
        let remote_description = self.remote_description.borrow();
        remote_description.clone()
    }

    pub(crate) fn local_description(&self) -> Option<RTCSessionDescription> {
        let local_description = self.local_description.borrow();
        local_description.clone()
    }

    pub(crate) fn set_remote_description(&self, description: RTCSessionDescription) {
        let mut remote_description = self.remote_description.borrow_mut();
        *remote_description = Some(description);
    }

    pub(crate) fn set_local_description(&self, description: RTCSessionDescription) {
        let mut local_description = self.local_description.borrow_mut();
        *local_description = Some(description);
    }
}
