use crate::server::certificate::RTCDtlsFingerprint;
use crate::server::session::description::RTCSessionDescription;
use crate::shared::types::{EndpointId, SessionId, UserName};
use base64::{prelude::BASE64_STANDARD, Engine};
use ring::rand::{SecureRandom, SystemRandom};
use sdp::util::ConnectionRole;
use sdp::SessionDescription;
use shared::error::{Error, Result};

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct ConnectionCredentials {
    ice_ufrag: String,
    ice_pwd: String,
    fingerprint: RTCDtlsFingerprint,
    role: ConnectionRole,
}

impl ConnectionCredentials {
    pub(crate) fn new(fingerprint: &RTCDtlsFingerprint, remote_role: ConnectionRole) -> Self {
        let rng = SystemRandom::new();

        let mut user = [0u8; 9];
        let _ = rng.fill(&mut user);
        let mut password = [0u8; 18];
        let _ = rng.fill(&mut password);

        Self {
            ice_ufrag: BASE64_STANDARD.encode(&user[..]),
            ice_pwd: BASE64_STANDARD.encode(&password[..]),
            fingerprint: fingerprint.clone(),
            role: if remote_role == ConnectionRole::Active {
                ConnectionRole::Passive
            } else {
                ConnectionRole::Active
            },
        }
    }

    pub fn from_sdp(sdp: &SessionDescription) -> Result<Self> {
        let media_description = sdp
            .media_descriptions
            .first()
            .ok_or(Error::ErrAttributeNotFound)?;
        let ice_ufrag = media_description
            .attribute("ice-ufrag")
            .ok_or(Error::ErrAttributeNotFound)?
            .ok_or(Error::ErrAttributeNotFound)?
            .to_string();
        let ice_pwd = media_description
            .attribute("ice-pwd")
            .ok_or(Error::ErrAttributeNotFound)?
            .ok_or(Error::ErrAttributeNotFound)?
            .to_string();
        let fingerprint = media_description
            .attribute("fingerprint")
            .ok_or(Error::ErrAttributeNotFound)?
            .ok_or(Error::ErrAttributeNotFound)?
            .try_into()?;
        let role = media_description
            .attribute("setup")
            .ok_or(Error::ErrAttributeNotFound)?
            .ok_or(Error::ErrAttributeNotFound)?
            .into();

        Ok(Self {
            ice_ufrag,
            ice_pwd,
            fingerprint,
            role,
        })
    }

    pub fn valid(&self) -> bool {
        self.ice_ufrag.len() >= 4
            && self.ice_ufrag.len() <= 256
            && self.ice_pwd.len() >= 22
            && self.ice_pwd.len() <= 256
    }
}

#[derive(Default, Debug)]
pub struct Candidate {
    session_id: SessionId,
    endpoint_id: EndpointId,
    remote_conn_cred: ConnectionCredentials,
    local_conn_cred: ConnectionCredentials,
    remote_description: RTCSessionDescription,
    local_description: Option<RTCSessionDescription>,
}

impl Candidate {
    pub(crate) fn new(
        session_id: SessionId,
        endpoint_id: EndpointId,
        fingerprint: &RTCDtlsFingerprint,
        remote_conn_cred: ConnectionCredentials,
        offer: RTCSessionDescription,
    ) -> Self {
        Self {
            session_id,
            endpoint_id,
            local_conn_cred: ConnectionCredentials::new(fingerprint, remote_conn_cred.role),
            remote_conn_cred,
            remote_description: offer,
            local_description: None,
        }
    }

    pub(crate) fn remote_connection_credentials(&self) -> &ConnectionCredentials {
        &self.remote_conn_cred
    }

    pub(crate) fn local_connection_credentials(&self) -> &ConnectionCredentials {
        &self.local_conn_cred
    }

    pub(crate) fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub(crate) fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    pub(crate) fn username(&self) -> UserName {
        format!(
            "{}:{}",
            self.local_conn_cred.ice_ufrag, self.remote_conn_cred.ice_ufrag
        )
    }

    pub(crate) fn set_remote_description(&mut self, remote_description: &RTCSessionDescription) {
        self.remote_description = remote_description.clone();
    }

    pub(crate) fn remote_description(&self) -> &RTCSessionDescription {
        &self.remote_description
    }

    pub(crate) fn set_local_description(&mut self, local_description: &RTCSessionDescription) {
        self.local_description = Some(local_description.clone());
    }

    pub(crate) fn local_description(&mut self) -> Option<&RTCSessionDescription> {
        self.local_description.as_ref()
    }
}
