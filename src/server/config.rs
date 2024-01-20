use crate::server::certificate::RTCCertificate;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

//TODO: use builder pattern to build ServerConfig and SessionConfig

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub certificates: Vec<RTCCertificate>,

    pub(crate) endpoint_idle_timeout: Duration,
    pub(crate) candidate_idle_timeout: Duration,
}

impl ServerConfig {
    /// create new server config
    pub fn new(certificates: Vec<RTCCertificate>) -> Self {
        Self {
            certificates,
            endpoint_idle_timeout: Duration::from_secs(30), //TODO: be to configurable
            candidate_idle_timeout: Duration::from_secs(30), //TODO: be to configurable
        }
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
