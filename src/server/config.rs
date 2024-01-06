use crate::server::certificate::RTCCertificate;

#[derive(Default, Debug)]
pub struct ServerConfig {
    pub certificate: Option<RTCCertificate>,
}
