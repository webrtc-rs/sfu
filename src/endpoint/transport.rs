use crate::endpoint::candidate::Candidate;
use crate::types::FourTuple;
use srtp::context::Context;
use std::rc::Rc;
use std::sync::Arc;

pub(crate) struct Transport {
    four_tuple: FourTuple,

    // ICE
    candidate: Rc<Candidate>,

    // DTLS
    dtls_endpoint: dtls::endpoint::Endpoint,

    // SCTP
    sctp_endpoint: sctp::Endpoint,

    // DataChannel
    association_handle: Option<usize>,
    stream_id: Option<u16>,

    // SRTP
    local_srtp_context: Option<Context>,
    remote_srtp_context: Option<Context>,
}

impl Transport {
    pub(crate) fn new(
        four_tuple: FourTuple,
        candidate: Rc<Candidate>,
        dtls_handshake_config: Arc<dtls::config::HandshakeConfig>,
        sctp_endpoint_config: Arc<sctp::EndpointConfig>,
        sctp_server_config: Arc<sctp::ServerConfig>,
    ) -> Self {
        Self {
            four_tuple,

            candidate,

            dtls_endpoint: dtls::endpoint::Endpoint::new(Some(dtls_handshake_config)),

            sctp_endpoint: sctp::Endpoint::new(sctp_endpoint_config, Some(sctp_server_config)),

            association_handle: None,
            stream_id: None,

            local_srtp_context: None,
            remote_srtp_context: None,
        }
    }

    pub(crate) fn four_tuple(&self) -> &FourTuple {
        &self.four_tuple
    }

    pub(crate) fn candidate(&self) -> &Rc<Candidate> {
        &self.candidate
    }

    pub(crate) fn get_mut_dtls_endpoint(&mut self) -> &mut dtls::endpoint::Endpoint {
        &mut self.dtls_endpoint
    }

    pub(crate) fn get_dtls_endpoint(&self) -> &dtls::endpoint::Endpoint {
        &self.dtls_endpoint
    }

    pub(crate) fn get_mut_sctp_endpoint(&mut self) -> &mut sctp::Endpoint {
        &mut self.sctp_endpoint
    }

    pub(crate) fn get_sctp_endpoint(&self) -> &sctp::Endpoint {
        &self.sctp_endpoint
    }

    pub(crate) fn local_srtp_context(&mut self) -> Option<&mut Context> {
        self.local_srtp_context.as_mut()
    }

    pub(crate) fn remote_srtp_context(&mut self) -> Option<&mut Context> {
        self.remote_srtp_context.as_mut()
    }

    pub(crate) fn set_local_srtp_context(&mut self, local_srtp_context: Context) {
        self.local_srtp_context = Some(local_srtp_context);
    }

    pub(crate) fn set_remote_srtp_context(&mut self, remote_srtp_context: Context) {
        self.remote_srtp_context = Some(remote_srtp_context);
    }

    pub(crate) fn set_association_handle_and_stream_id(
        &mut self,
        association_handle: usize,
        stream_id: u16,
    ) {
        self.association_handle = Some(association_handle);
        self.stream_id = Some(stream_id)
    }

    pub(crate) fn association_handle_and_stream_id(&self) -> (Option<usize>, Option<u16>) {
        (self.association_handle, self.stream_id)
    }
}
