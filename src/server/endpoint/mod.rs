pub mod candidate;
pub mod transport;

use crate::server::endpoint::transport::Transport;
use crate::server::session::description::rtp_transceiver::RTCRtpTransceiver;
use crate::server::session::description::RTCSessionDescription;
use crate::types::{EndpointId, FourTuple, Mid};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Endpoint {
    endpoint_id: EndpointId,

    remote_description: Option<RTCSessionDescription>,
    local_description: Option<RTCSessionDescription>,

    transports: HashMap<FourTuple, Transport>,

    mids: Vec<Mid>,
    transceivers: HashMap<Mid, RTCRtpTransceiver>,
}

impl Endpoint {
    pub(crate) fn new(endpoint_id: EndpointId) -> Self {
        Self {
            endpoint_id,

            remote_description: None,
            local_description: None,

            transports: HashMap::new(),

            mids: vec![],
            transceivers: HashMap::new(),
        }
    }

    pub(crate) fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    pub(crate) fn add_transport(&mut self, transport: Transport) {
        self.transports.insert(*transport.four_tuple(), transport);
    }

    pub(crate) fn remove_transport(&mut self, four_tuple: &FourTuple) {
        self.transports.remove(four_tuple);
    }

    pub(crate) fn has_transport(&self, four_tuple: &FourTuple) -> bool {
        self.transports.contains_key(four_tuple)
    }

    pub(crate) fn get_transports(&self) -> &HashMap<FourTuple, Transport> {
        &self.transports
    }

    pub(crate) fn get_mut_transports(&mut self) -> &mut HashMap<FourTuple, Transport> {
        &mut self.transports
    }

    pub(crate) fn get_mids(&self) -> &Vec<Mid> {
        &self.mids
    }

    pub(crate) fn get_mut_mids(&mut self) -> &mut Vec<Mid> {
        &mut self.mids
    }

    pub(crate) fn get_transceivers(&self) -> &HashMap<Mid, RTCRtpTransceiver> {
        &self.transceivers
    }

    pub(crate) fn get_mut_transceivers(&mut self) -> &mut HashMap<Mid, RTCRtpTransceiver> {
        &mut self.transceivers
    }

    pub(crate) fn get_mut_mids_and_transceivers(
        &mut self,
    ) -> (&mut Vec<Mid>, &mut HashMap<Mid, RTCRtpTransceiver>) {
        (&mut self.mids, &mut self.transceivers)
    }

    pub(crate) fn remote_description(&self) -> Option<&RTCSessionDescription> {
        self.remote_description.as_ref()
    }

    pub(crate) fn local_description(&self) -> Option<&RTCSessionDescription> {
        self.local_description.as_ref()
    }

    pub(crate) fn set_remote_description(&mut self, description: RTCSessionDescription) {
        self.remote_description = Some(description);
    }

    pub(crate) fn set_local_description(&mut self, description: RTCSessionDescription) {
        self.local_description = Some(description);
    }
}
