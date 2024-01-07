use crate::server::certificate::RTCCertificate;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub certificate: RTCCertificate,
}
