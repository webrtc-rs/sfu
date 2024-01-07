//use sdp::SessionDescription;
use shared::error::Result;
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;

pub mod description;

use crate::server::certificate::RTCDtlsFingerprint;
use crate::server::endpoint::{
    candidate::{Candidate, ConnectionCredentials},
    Endpoint,
};
use crate::server::session::description::RTCSessionDescription;
use crate::shared::types::{EndpointId, SessionId, UserName};

#[derive(Debug)]
pub struct Session {
    session_id: SessionId,
    local_addr: SocketAddr,
    fingerprint: RTCDtlsFingerprint,
    endpoints: RefCell<HashMap<EndpointId, Rc<Endpoint>>>,
    candidates: RefCell<HashMap<UserName, Rc<Candidate>>>,
}

impl Session {
    pub fn new(
        session_id: SessionId,
        local_addr: SocketAddr,
        fingerprint: RTCDtlsFingerprint,
    ) -> Self {
        Self {
            session_id,
            local_addr,
            fingerprint,
            endpoints: RefCell::new(HashMap::new()),
            candidates: RefCell::new(HashMap::new()),
        }
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub(crate) fn add_candidate(&self, candidate: Rc<Candidate>) -> bool {
        let username = candidate.username();
        let mut candidates = self.candidates.borrow_mut();
        candidates.insert(username, candidate).is_some()
    }

    //TODO: add idle timeout to remove candidate
    pub(crate) fn remove_candidate(&self, candidate: &Rc<Candidate>) -> bool {
        let username = candidate.username();
        let mut candidates = self.candidates.borrow_mut();
        candidates.remove(&username).is_some()
    }

    pub(crate) fn find_candidate(&self, username: &UserName) -> Option<Rc<Candidate>> {
        let candidates = self.candidates.borrow();
        candidates.get(username).cloned()
    }

    // set pending offer and return answer
    pub fn accept_pending_offer(
        &self,
        endpoint_id: EndpointId,
        mut offer: RTCSessionDescription,
    ) -> Result<RTCSessionDescription> {
        offer.unmarshal()?;
        let parsed = offer.unmarshal()?;
        let remote_conn_cred = ConnectionCredentials::from_sdp(&parsed)?;
        offer.parsed = Some(parsed);

        let answer_sdp = offer.sdp.clone();

        let mut candidate = Candidate::new(
            self.session_id,
            endpoint_id,
            &self.fingerprint,
            remote_conn_cred,
            offer,
        );

        //TODO: generate Answer SDP
        let answer = RTCSessionDescription::answer(answer_sdp)?;
        candidate.set_local_description(&answer);

        self.add_candidate(Rc::new(candidate));

        Ok(answer)
    }

    /*
        pub fn create_pending_answer(
            &self,
            candidate: &Rc<Candidate>,
        ) -> Result<RTCSessionDescription> {
            let d = SessionDescription::new_jsep_session_description(false /*use_identity*/);

            let local_connection_credentials = candidate.local_connection_credentials();

            let mut media_sections = vec![];
            let mut already_have_application_media_section = false;
            let remote_description = candidate
                .remote_description()
                .parsed
                .ok_or(Error::Other("Unparsed remoted description".to_string()))?;
            for media in &remote_description.media_descriptions {
                if let Some(mid_value) = get_mid_value(media) {
                    if mid_value.is_empty() {
                        return Err(Error::Other(
                            "ErrPeerConnRemoteDescriptionWithoutMidValue".to_string(),
                        ));
                    }

                    if media.media_name.media == MEDIA_SECTION_APPLICATION {
                        media_sections.push(MediaSection {
                            id: mid_value.to_owned(),
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

                    if let Some(t) = find_by_mid(mid_value, &mut local_transceivers).await {
                        t.sender().await.set_negotiated();
                        let media_transceivers = vec![t];

                        // NB: The below could use `then_some`, but with our current MSRV
                        // it's not possible to actually do this. The clippy version that
                        // ships with 1.64.0 complains about this so we disable it for now.
                        #[allow(clippy::unnecessary_lazy_evaluations)]
                        media_sections.push(MediaSection {
                            id: mid_value.to_owned(),
                            transceivers: media_transceivers,
                            rid_map: get_rids(media),
                            offered_direction: (!include_unmatched).then(|| direction),
                            ..Default::default()
                        });
                    } else {
                        return Err(Error::ErrPeerConnTransceiverMidNil);
                    }
                }
            }

            Ok(RTCSessionDescription::default())
        }
    */
    pub fn create_answer(&self, _endpoint_id: EndpointId) -> Result<RTCSessionDescription> {
        Ok(RTCSessionDescription::default())
    }
}
