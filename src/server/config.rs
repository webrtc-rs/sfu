use crate::server::certificate::RTCCertificate;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub certificates: Vec<RTCCertificate>,
    pub(crate) sctp_server_config: Arc<sctp::ServerConfig>,
    pub(crate) endpoint_idle_timeout: Duration,
    pub(crate) candidate_idle_timeout: Duration,
}

impl ServerConfig {
    /// create new server config
    pub fn new(certificates: Vec<RTCCertificate>) -> Self {
        Self {
            certificates,
            sctp_server_config: Arc::new(sctp::ServerConfig::default()),
            endpoint_idle_timeout: Duration::from_secs(30),
            candidate_idle_timeout: Duration::from_secs(30),
        }
    }

    pub fn with_sctp_server_config(mut self, sctp_server_config: Arc<sctp::ServerConfig>) -> Self {
        self.sctp_server_config = sctp_server_config;
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

#[derive(Debug, Clone)]
pub(crate) struct SessionConfig {
    pub(crate) server_config: Arc<ServerConfig>,
    pub(crate) local_addr: SocketAddr,
}

impl SessionConfig {
    pub(crate) fn new(server_config: Arc<ServerConfig>, local_addr: SocketAddr) -> Self {
        Self {
            server_config,
            local_addr,
        }
    }
}
