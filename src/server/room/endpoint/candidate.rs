use crate::server::certificate::RTCDtlsFingerprint;
use crate::shared::types::{EndpointId, RoomId};
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
    pub(crate) fn new(fingerprint: &RTCDtlsFingerprint, peer_role: ConnectionRole) -> Self {
        let rng = SystemRandom::new();

        let mut user = [0u8; 9];
        let _ = rng.fill(&mut user);
        let mut password = [0u8; 18];
        let _ = rng.fill(&mut password);

        Self {
            ice_ufrag: BASE64_STANDARD.encode(&user[..]),
            ice_pwd: BASE64_STANDARD.encode(&password[..]),
            fingerprint: fingerprint.clone(),
            role: if peer_role == ConnectionRole::Active {
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
    room_id: RoomId,
    endpoint_id: EndpointId,
    local_conn_cred: ConnectionCredentials,
    peer_conn_cred: ConnectionCredentials,
    offer_sdp: SessionDescription,
    answer_sdp: Option<SessionDescription>,
}

impl Candidate {
    pub(crate) fn new(
        room_id: RoomId,
        endpoint_id: EndpointId,
        fingerprint: &RTCDtlsFingerprint,
        peer_conn_cred: ConnectionCredentials,
        offer_sdp: SessionDescription,
    ) -> Self {
        Self {
            room_id,
            endpoint_id,
            local_conn_cred: ConnectionCredentials::new(fingerprint, peer_conn_cred.role),
            peer_conn_cred,
            offer_sdp,
            answer_sdp: None,
        }
    }

    pub(crate) fn peer_connection_credentials(&self) -> &ConnectionCredentials {
        &self.peer_conn_cred
    }

    pub(crate) fn local_connection_credentials(&self) -> &ConnectionCredentials {
        &self.local_conn_cred
    }

    pub(crate) fn room_id(&self) -> RoomId {
        self.room_id
    }

    pub(crate) fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    pub(crate) fn username(&self) -> String {
        format!(
            "{}:{}",
            self.local_conn_cred.ice_ufrag, self.peer_conn_cred.ice_ufrag
        )
    }

    pub(crate) fn offer_sdp(&self) -> &SessionDescription {
        &self.offer_sdp
    }

    pub(crate) fn set_answer_sdp(&mut self, answer_sdp: &SessionDescription) {
        self.answer_sdp = Some(answer_sdp.clone());
    }

    pub(crate) fn answer_sdp(&mut self) -> Option<&SessionDescription> {
        self.answer_sdp.as_ref()
    }
}
