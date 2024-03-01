use crate::description::config::MediaConfig;
use crate::server::certificate::RTCCertificate;
use std::sync::Arc;
use std::time::Duration;

pub struct ServerConfig {
    pub certificates: Vec<RTCCertificate>,
    pub dtls_handshake_config: Arc<dtls::config::HandshakeConfig>,
    pub sctp_endpoint_config: Arc<sctp::EndpointConfig>,
    pub sctp_server_config: Arc<sctp::ServerConfig>,
    pub media_config: MediaConfig,

    pub(crate) endpoint_idle_timeout: Duration,
    pub(crate) candidate_idle_timeout: Duration,
}

impl ServerConfig {
    /// create new server config
    pub fn new(certificates: Vec<RTCCertificate>) -> Self {
        Self {
            certificates,
            media_config: MediaConfig::default(),
            sctp_endpoint_config: Arc::new(sctp::EndpointConfig::default()),
            sctp_server_config: Arc::new(sctp::ServerConfig::default()),
            dtls_handshake_config: Arc::new(dtls::config::HandshakeConfig::default()),
            endpoint_idle_timeout: Duration::from_secs(30),
            candidate_idle_timeout: Duration::from_secs(30),
        }
    }

    pub fn with_media_config(mut self, media_config: MediaConfig) -> Self {
        self.media_config = media_config;
        self
    }

    pub fn with_sctp_server_config(mut self, sctp_server_config: Arc<sctp::ServerConfig>) -> Self {
        self.sctp_server_config = sctp_server_config;
        self
    }

    pub fn with_sctp_endpoint_config(
        mut self,
        sctp_endpoint_config: Arc<sctp::EndpointConfig>,
    ) -> Self {
        self.sctp_endpoint_config = sctp_endpoint_config;
        self
    }

    pub fn with_dtls_handshake_config(
        mut self,
        dtls_handshake_config: Arc<dtls::config::HandshakeConfig>,
    ) -> Self {
        self.dtls_handshake_config = dtls_handshake_config;
        self
    }

    pub fn with_endpoint_idle_timeout(mut self, endpoint_idle_timeout: Duration) -> Self {
        self.endpoint_idle_timeout = endpoint_idle_timeout;
        self
    }

    pub fn with_candidate_idle_timeout(mut self, candidate_idle_timeout: Duration) -> Self {
        self.candidate_idle_timeout = candidate_idle_timeout;
        self
    }
}
