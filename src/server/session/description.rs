use sdp::SessionDescription;
use serde::{Deserialize, Serialize};
use shared::error::{Error, Result};
use std::fmt;
use std::io::Cursor;

/// SessionDescription is used to expose local and remote session descriptions.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct RTCSessionDescription {
    #[serde(rename = "type")]
    pub sdp_type: RTCSdpType,

    pub sdp: String,

    /// This will never be initialized by callers, internal use only
    #[serde(skip)]
    pub(crate) parsed: Option<SessionDescription>,
}

impl RTCSessionDescription {
    /// Given SDP representing an answer, wrap it in an RTCSessionDescription
    /// that can be given to an RTCPeerConnection.
    pub fn answer(sdp: String) -> Result<RTCSessionDescription> {
        let mut desc = RTCSessionDescription {
            sdp,
            sdp_type: RTCSdpType::Answer,
            parsed: None,
        };

        let parsed = desc.unmarshal()?;
        desc.parsed = Some(parsed);

        Ok(desc)
    }

    /// Given SDP representing an offer, wrap it in an RTCSessionDescription
    /// that can be given to an RTCPeerConnection.
    pub fn offer(sdp: String) -> Result<RTCSessionDescription> {
        let mut desc = RTCSessionDescription {
            sdp,
            sdp_type: RTCSdpType::Offer,
            parsed: None,
        };

        let parsed = desc.unmarshal()?;
        desc.parsed = Some(parsed);

        Ok(desc)
    }

    /// Given SDP representing an answer, wrap it in an RTCSessionDescription
    /// that can be given to an RTCPeerConnection. `pranswer` is used when the
    /// answer may not be final, or when updating a previously sent pranswer.
    pub fn pranswer(sdp: String) -> Result<RTCSessionDescription> {
        let mut desc = RTCSessionDescription {
            sdp,
            sdp_type: RTCSdpType::Pranswer,
            parsed: None,
        };

        let parsed = desc.unmarshal()?;
        desc.parsed = Some(parsed);

        Ok(desc)
    }

    /// Unmarshal is a helper to deserialize the sdp
    pub fn unmarshal(&self) -> Result<SessionDescription> {
        let mut reader = Cursor::new(self.sdp.as_bytes());
        let parsed = SessionDescription::unmarshal(&mut reader)
            .map_err(|err| Error::Other(err.to_string()))?;
        Ok(parsed)
    }
}

/// SDPType describes the type of an SessionDescription.
#[derive(Default, Debug, PartialEq, Eq, Copy, Clone, Serialize, Deserialize)]
pub enum RTCSdpType {
    #[default]
    Unspecified = 0,

    /// indicates that a description MUST be treated as an SDP offer.
    #[serde(rename = "offer")]
    Offer,

    /// indicates that a description MUST be treated as an
    /// SDP answer, but not a final answer. A description used as an SDP
    /// pranswer may be applied as a response to an SDP offer, or an update to
    /// a previously sent SDP pranswer.
    #[serde(rename = "pranswer")]
    Pranswer,

    /// indicates that a description MUST be treated as an SDP
    /// final answer, and the offer-answer exchange MUST be considered complete.
    /// A description used as an SDP answer may be applied as a response to an
    /// SDP offer or as an update to a previously sent SDP pranswer.    
    #[serde(rename = "answer")]
    Answer,

    /// indicates that a description MUST be treated as
    /// canceling the current SDP negotiation and moving the SDP offer and
    /// answer back to what it was in the previous stable state. Note the
    /// local or remote SDP descriptions in the previous stable state could be
    /// null if there has not yet been a successful offer-answer negotiation.
    #[serde(rename = "rollback")]
    Rollback,
}

const SDP_TYPE_OFFER_STR: &str = "offer";
const SDP_TYPE_PRANSWER_STR: &str = "pranswer";
const SDP_TYPE_ANSWER_STR: &str = "answer";
const SDP_TYPE_ROLLBACK_STR: &str = "rollback";

/// creates an SDPType from a string
impl From<&str> for RTCSdpType {
    fn from(raw: &str) -> Self {
        match raw {
            SDP_TYPE_OFFER_STR => RTCSdpType::Offer,
            SDP_TYPE_PRANSWER_STR => RTCSdpType::Pranswer,
            SDP_TYPE_ANSWER_STR => RTCSdpType::Answer,
            SDP_TYPE_ROLLBACK_STR => RTCSdpType::Rollback,
            _ => RTCSdpType::Unspecified,
        }
    }
}

impl fmt::Display for RTCSdpType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            RTCSdpType::Offer => write!(f, "{SDP_TYPE_OFFER_STR}"),
            RTCSdpType::Pranswer => write!(f, "{SDP_TYPE_PRANSWER_STR}"),
            RTCSdpType::Answer => write!(f, "{SDP_TYPE_ANSWER_STR}"),
            RTCSdpType::Rollback => write!(f, "{SDP_TYPE_ROLLBACK_STR}"),
            _ => write!(f, "Unspecified"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_new_sdp_type() {
        let tests = vec![
            ("Unspecified", RTCSdpType::Unspecified),
            ("offer", RTCSdpType::Offer),
            ("pranswer", RTCSdpType::Pranswer),
            ("answer", RTCSdpType::Answer),
            ("rollback", RTCSdpType::Rollback),
        ];

        for (sdp_type_string, expected_sdp_type) in tests {
            assert_eq!(RTCSdpType::from(sdp_type_string), expected_sdp_type);
        }
    }

    #[test]
    fn test_sdp_type_string() {
        let tests = vec![
            (RTCSdpType::Unspecified, "Unspecified"),
            (RTCSdpType::Offer, "offer"),
            (RTCSdpType::Pranswer, "pranswer"),
            (RTCSdpType::Answer, "answer"),
            (RTCSdpType::Rollback, "rollback"),
        ];

        for (sdp_type, expected_string) in tests {
            assert_eq!(sdp_type.to_string(), expected_string);
        }
    }

    #[test]
    fn test_session_description_json() {
        let tests = vec![
            (
                RTCSessionDescription {
                    sdp_type: RTCSdpType::Offer,
                    sdp: "sdp".to_owned(),
                    parsed: None,
                },
                r#"{"type":"offer","sdp":"sdp"}"#,
            ),
            (
                RTCSessionDescription {
                    sdp_type: RTCSdpType::Pranswer,
                    sdp: "sdp".to_owned(),
                    parsed: None,
                },
                r#"{"type":"pranswer","sdp":"sdp"}"#,
            ),
            (
                RTCSessionDescription {
                    sdp_type: RTCSdpType::Answer,
                    sdp: "sdp".to_owned(),
                    parsed: None,
                },
                r#"{"type":"answer","sdp":"sdp"}"#,
            ),
            (
                RTCSessionDescription {
                    sdp_type: RTCSdpType::Rollback,
                    sdp: "sdp".to_owned(),
                    parsed: None,
                },
                r#"{"type":"rollback","sdp":"sdp"}"#,
            ),
            (
                RTCSessionDescription {
                    sdp_type: RTCSdpType::Unspecified,
                    sdp: "sdp".to_owned(),
                    parsed: None,
                },
                r#"{"type":"Unspecified","sdp":"sdp"}"#,
            ),
        ];

        for (desc, expected_string) in tests {
            let result = serde_json::to_string(&desc);
            assert!(result.is_ok(), "testCase: marshal err: {result:?}");
            let desc_data = result.unwrap();
            assert_eq!(desc_data, expected_string, "string is not expected");

            let result = serde_json::from_str::<RTCSessionDescription>(&desc_data);
            assert!(result.is_ok(), "testCase: unmarshal err: {result:?}");
            if let Ok(sd) = result {
                assert!(sd.sdp == desc.sdp && sd.sdp_type == desc.sdp_type);
            }
        }
    }
}
