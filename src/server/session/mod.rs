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
use crate::server::session::description::rtp_codec::{RTCRtpParameters, RTPCodecType};
use crate::server::session::description::rtp_transceiver::RTCRtpTransceiver;
use crate::server::session::description::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use crate::server::session::description::sdp_type::RTCSdpType;
use crate::server::session::description::{
    codecs_from_media_description, get_cname, get_mid_value, get_msid, get_peer_direction,
    get_rids, get_ssrc_groups, populate_sdp, rtp_extensions_from_media_description,
    update_sdp_origin, MediaSection, RTCSessionDescription, MEDIA_SECTION_APPLICATION,
};
use crate::types::{EndpointId, Mid, SessionId};

pub(crate) struct Session {
    session_config: SessionConfig,
    session_id: SessionId,
    endpoints: RefCell<HashMap<EndpointId, Rc<Endpoint>>>,
}

impl Session {
    pub(crate) fn new(session_config: SessionConfig, session_id: SessionId) -> Self {
        Self {
            session_config,
            session_id,
            endpoints: RefCell::new(HashMap::new()),
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
    ) -> Result<(bool, Rc<Endpoint>)> {
        let endpoint_id = candidate.endpoint_id();
        let endpoint = self.get_endpoint(&endpoint_id);
        let four_tuple = transport_context.into();
        if let Some(endpoint) = endpoint {
            if endpoint.has_transport(&four_tuple) {
                Ok((true, endpoint))
            } else {
                let transport =
                    Transport::new(four_tuple, Rc::downgrade(&endpoint), Rc::clone(candidate));
                endpoint.add_transport(transport);
                Ok((true, endpoint))
            }
        } else {
            let endpoint = Rc::new(Endpoint::new(Rc::downgrade(self), endpoint_id));
            let transport =
                Transport::new(four_tuple, Rc::downgrade(&endpoint), Rc::clone(candidate));
            endpoint.add_transport(transport);

            {
                let mut endpoints = self.endpoints.borrow_mut();
                endpoints.insert(endpoint_id, Rc::clone(&endpoint));
            }

            Ok((false, endpoint))
        }
    }

    pub(crate) fn get_endpoint(&self, endpoint_id: &EndpointId) -> Option<Rc<Endpoint>> {
        self.endpoints.borrow().get(endpoint_id).cloned()
    }

    pub(crate) fn has_endpoint(&self, endpoint_id: &EndpointId) -> bool {
        self.endpoints.borrow().contains_key(endpoint_id)
    }

    pub(crate) fn endpoints(&self) -> &RefCell<HashMap<EndpointId, Rc<Endpoint>>> {
        &self.endpoints
    }

    pub(crate) fn set_remote_description(
        &self,
        endpoint: &Rc<Endpoint>,
        remote_description: &RTCSessionDescription,
    ) -> Result<()> {
        let parsed = remote_description
            .parsed
            .as_ref()
            .ok_or(Error::Other("Unparsed remote description".to_string()))?;

        let mut local_transceivers = endpoint.transceivers().borrow_mut();
        let we_offer = remote_description.sdp_type == RTCSdpType::Answer;

        if !we_offer {
            // This is an offer from the remove
            for media in &parsed.media_descriptions {
                let mid_value = match get_mid_value(media) {
                    Some(m) => {
                        if m.is_empty() {
                            return Err(Error::Other(
                                "ErrPeerConnRemoteDescriptionWithoutMidValue".to_string(),
                            ));
                        } else {
                            m
                        }
                    }
                    None => continue,
                };

                if media.media_name.media == MEDIA_SECTION_APPLICATION {
                    continue;
                }

                let kind = RTPCodecType::from(media.media_name.media.as_str());
                let direction = get_peer_direction(media);
                if kind == RTPCodecType::Unspecified
                    || direction == RTCRtpTransceiverDirection::Unspecified
                {
                    continue;
                }

                let cname = get_cname(media).unwrap_or_default();
                let msid = get_msid(media).ok_or(Error::Other(
                    "ErrPeerConnRemoteDescriptionWithoutMsidValue".to_string(),
                ))?;
                let (ssrc_groups, ssrcs) = get_ssrc_groups(media)?;
                let codecs = codecs_from_media_description(media)?;
                let header_extensions = rtp_extensions_from_media_description(media)?;
                let rtp_params = RTCRtpParameters {
                    header_extensions,
                    codecs,
                };

                let transceiver = RTCRtpTransceiver {
                    mid: mid_value.to_string(),
                    kind,
                    direction,
                    cname,
                    msid,
                    rtp_params,
                    ssrcs,
                    ssrc_groups,
                };

                if let Some(local_transceiver) = local_transceivers.get_mut(mid_value) {
                    *local_transceiver = transceiver;
                } else {
                    local_transceivers.insert(mid_value.to_string(), transceiver);
                }
            }
        } else {
            // WebRTC Spec 1.0 https://www.w3.org/TR/webrtc/
            // 4.5.9.2
            // This is an answer from the remote.
            for media in &parsed.media_descriptions {
                let _mid_value = match get_mid_value(media) {
                    Some(m) => {
                        if m.is_empty() {
                            return Err(Error::Other(
                                "ErrPeerConnRemoteDescriptionWithoutMidValue".to_string(),
                            ));
                        } else {
                            m
                        }
                    }
                    None => continue,
                };

                if media.media_name.media == MEDIA_SECTION_APPLICATION {
                    continue;
                }
                let kind = RTPCodecType::from(media.media_name.media.as_str());
                let direction = get_peer_direction(media);
                if kind == RTPCodecType::Unspecified
                    || direction == RTCRtpTransceiverDirection::Unspecified
                {
                    continue;
                }

                /*TODO: if let Some(t) = local_transceivers.get_mut(mid_value) {
                    let previous_direction = t.current_direction();

                    // 4.5.9.2.9
                    // Let direction be an RTCRtpTransceiverDirection value representing the direction
                    // from the media description, but with the send and receive directions reversed to
                    // represent this peer's point of view. If the media description is rejected,
                    // set direction to "inactive".
                    let reversed_direction = direction.reverse();

                    // 4.5.9.2.13.2
                    // Set transceiver.[[CurrentDirection]] and transceiver.[[Direction]]s to direction.
                    t.set_current_direction(reversed_direction);
                    // TODO: According to the specification we should set
                    // transceiver.[[Direction]] here, however libWebrtc doesn't do this.
                    // NOTE: After raising this it seems like the specification might
                    // change to remove the setting of transceiver.[[Direction]].
                    // See https://github.com/w3c/webrtc-pc/issues/2751#issuecomment-1185901962
                    // t.set_direction_internal(reversed_direction);
                    t.process_new_current_direction(previous_direction)?;
                }*/
            }
        }

        Ok(())
    }

    pub(crate) fn create_answer(
        &self,
        endpoint: &Option<Rc<Endpoint>>,
        remote_description: &RTCSessionDescription,
        local_ice_params: &RTCIceParameters,
    ) -> Result<RTCSessionDescription> {
        let use_identity = false; //TODO: self.config.idp_login_url.is_some();
        let mut d = self.generate_matched_sdp(
            endpoint,
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
        endpoint: &Option<Rc<Endpoint>>,
        remote_description: &RTCSessionDescription,
        local_ice_params: &RTCIceParameters,
        use_identity: bool,
        include_unmatched: bool,
        connection_role: ConnectionRole,
    ) -> Result<SessionDescription> {
        let d = SessionDescription::new_jsep_session_description(use_identity);
        let empty_transceivers = RefCell::new(HashMap::new());

        let media_sections = {
            let mut local_transceivers = if let Some(endpoint) = endpoint.as_ref() {
                endpoint.transceivers().borrow_mut()
            } else {
                empty_transceivers.borrow_mut()
            };

            let mut media_sections = vec![];
            let mut already_have_application_media_section = false;
            let mut matched: HashSet<Mid> = HashSet::new();
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

                        if let Some(_t) = local_transceivers.get_mut(mid_value) {
                            matched.insert(mid_value.to_string());

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
                for (mid, _t) in local_transceivers.iter_mut() {
                    if !matched.contains::<Mid>(mid) {
                        media_sections.push(MediaSection {
                            mid: mid.clone(),
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

        let local_transceiver = if let Some(endpoint) = endpoint.as_ref() {
            endpoint.transceivers().borrow()
        } else {
            empty_transceivers.borrow()
        };

        populate_sdp(
            d,
            &dtls_fingerprints,
            &self.session_config.local_addr,
            local_ice_params,
            connection_role,
            &media_sections,
            &local_transceiver,
            true,
        )
    }
}
