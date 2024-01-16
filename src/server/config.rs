use crate::server::certificate::RTCCertificate;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub certificates: Vec<RTCCertificate>,
}
