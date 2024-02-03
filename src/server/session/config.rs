use crate::server::config::ServerConfig;
use std::net::SocketAddr;
use std::sync::Arc;

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
