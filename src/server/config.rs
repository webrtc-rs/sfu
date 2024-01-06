use crate::server::certificate::RTCCertificate;

#[derive(Debug)]
pub struct ServerConfig {
    pub certificate: RTCCertificate,
}
