use retty::transport::TransportContext;
use sdp::description::session::Origin;
use sdp::util::ConnectionRole;
use sdp::SessionDescription;
use shared::error::{Error, Result};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub mod description;

use crate::server::config::SessionConfig;
use crate::server::endpoint::candidate::{Candidate, DTLSRole, RTCIceParameters};
use crate::server::endpoint::transport::Transport;
use crate::server::endpoint::Endpoint;
use crate::server::session::description::rtp_codec::RTPCodecType;
use crate::server::session::description::rtp_transceiver::RTCRtpTransceiver;
use crate::server::session::description::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use crate::server::session::description::sdp_type::RTCSdpType;
use crate::server::session::description::{
    get_mid_value, get_peer_direction, get_rids, populate_sdp, update_sdp_origin, MediaSection,
    RTCSessionDescription, MEDIA_SECTION_APPLICATION,
};
use crate::types::{EndpointId, Mid, SessionId};

#[derive(Debug)]
pub(crate) struct Session {
    session_config: SessionConfig,
    session_id: SessionId,
    endpoints: RefCell<HashMap<EndpointId, Rc<Endpoint>>>,
    transceivers: RefCell<HashMap<Mid, RTCRtpTransceiver>>,
}

impl Session {
    pub(crate) fn new(session_config: SessionConfig, session_id: SessionId) -> Self {
        Self {
            session_config,
            session_id,
            endpoints: RefCell::new(HashMap::new()),
            transceivers: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn session_id(&self) -> u64 {
        self.session_id
    }

    pub(crate) fn session_config(&self) -> &SessionConfig {
        &self.session_config
    }

    pub(crate) fn add_endpoint(
        self: &Rc<Self>,
        candidate: &Rc<Candidate>,
        transport_context: &TransportContext,
    ) -> Result<(bool, Rc<Endpoint>, Rc<Transport>)> {
        let endpoint_id = candidate.endpoint_id();
        let endpoint = self.get_endpoint(&endpoint_id);
        let four_tuple = transport_context.into();
        if let Some(endpoint) = endpoint {
            if let Some(transport) = endpoint.get_transport(&four_tuple) {
                Ok((true, endpoint, transport))
            } else {
                let transport = Rc::new(Transport::new(
                    four_tuple,
                    Rc::downgrade(&endpoint),
                    Rc::clone(candidate),
                ));
                endpoint.add_transport(Rc::clone(&transport));
                Ok((true, endpoint, transport))
            }
        } else {
            let endpoint = Rc::new(Endpoint::new(Rc::downgrade(self), endpoint_id));
            let transport = Rc::new(Transport::new(
                four_tuple,
                Rc::downgrade(&endpoint),
                Rc::clone(candidate),
            ));
            endpoint.add_transport(Rc::clone(&transport));
            Ok((false, endpoint, transport))
        }
    }

    pub(crate) fn get_endpoint(&self, endpoint_id: &EndpointId) -> Option<Rc<Endpoint>> {
        self.endpoints.borrow().get(endpoint_id).cloned()
    }

    pub(crate) fn has_endpoint(&self, endpoint_id: &EndpointId) -> bool {
        self.endpoints.borrow().contains_key(endpoint_id)
    }

    pub(crate) fn create_answer(
        &self,
        endpoint_id: EndpointId,
        remote_description: &RTCSessionDescription,
        local_ice_params: &RTCIceParameters,
    ) -> Result<RTCSessionDescription> {
        let use_identity = false; //TODO: self.config.idp_login_url.is_some();
        let mut d = self.generate_matched_sdp(
            endpoint_id,
            remote_description,
            local_ice_params,
            use_identity,
            false, /*includeUnmatched */
            DTLSRole::Server.to_connection_role(),
        )?;

        let mut sdp_origin = Origin::default();
        update_sdp_origin(&mut sdp_origin, &mut d);

        let sdp = d.marshal();

        let answer = RTCSessionDescription {
            sdp_type: RTCSdpType::Answer,
            sdp,
            parsed: Some(d),
        };

        Ok(answer)
    }

    /// generate_matched_sdp generates a SDP and takes the remote state into account
    /// this is used everytime we have a remote_description
    pub(crate) fn generate_matched_sdp(
        &self,
        _endpoint_id: EndpointId,
        remote_description: &RTCSessionDescription,
        local_ice_params: &RTCIceParameters,
        use_identity: bool,
        include_unmatched: bool,
        connection_role: ConnectionRole,
    ) -> Result<SessionDescription> {
        let d = SessionDescription::new_jsep_session_description(use_identity);

        let media_sections = {
            let mut local_transceivers = self.transceivers.borrow_mut();

            let mut media_sections = vec![];
            let mut already_have_application_media_section = false;
            let mut matched = HashSet::new();
            if let Some(parsed) = remote_description.parsed.as_ref() {
                for media in &parsed.media_descriptions {
                    if let Some(mid_value) = get_mid_value(media) {
                        if mid_value.is_empty() {
                            return Err(Error::Other(
                                "ErrPeerConnRemoteDescriptionWithoutMidValue".to_string(),
                            ));
                        }

                        if media.media_name.media == MEDIA_SECTION_APPLICATION {
                            media_sections.push(MediaSection {
                                mid: mid_value.to_owned(),
                                data: true,
                                ..Default::default()
                            });
                            already_have_application_media_section = true;
                            continue;
                        }

                        let kind = RTPCodecType::from(media.media_name.media.as_str());
                        let direction = get_peer_direction(media);
                        if kind == RTPCodecType::Unspecified
                            || direction == RTCRtpTransceiverDirection::Unspecified
                        {
                            continue;
                        }

                        if let Some(t) = local_transceivers.get_mut(mid_value) {
                            t.sender.set_negotiated();
                            matched.insert(t.mid.clone());

                            media_sections.push(MediaSection {
                                mid: mid_value.to_owned(),
                                rid_map: get_rids(media),
                                offered_direction: (!include_unmatched).then_some(direction),
                                ..Default::default()
                            });
                        } else {
                            return Err(Error::Other("ErrPeerConnTransceiverMidNil".to_string()));
                        }
                    }
                }
            }

            // If we are offering also include unmatched local transceivers
            if include_unmatched {
                for (mid, t) in local_transceivers.iter_mut() {
                    if !matched.contains(mid) {
                        t.sender.set_negotiated();
                        media_sections.push(MediaSection {
                            mid: t.mid.clone(),
                            ..Default::default()
                        });
                    }
                }

                if !already_have_application_media_section {
                    media_sections.push(MediaSection {
                        mid: format!("{}", media_sections.len()),
                        data: true,
                        ..Default::default()
                    });
                }
            }

            media_sections
        };

        let dtls_fingerprints =
            if let Some(cert) = self.session_config.server_config.certificates.first() {
                cert.get_fingerprints()
            } else {
                return Err(Error::Other("ErrNonCertificate".to_string()));
            };

        populate_sdp(
            d,
            &dtls_fingerprints,
            &self.session_config.local_addr,
            local_ice_params,
            connection_role,
            &media_sections,
            &self.transceivers.borrow(),
            true,
        )
    }
}
