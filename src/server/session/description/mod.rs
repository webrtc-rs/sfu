mod fmtp;
mod rtp_codec;
mod rtp_transceiver;
mod rtp_transceiver_direction;
mod sdp_type;

use crate::server::session::description::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use crate::server::session::description::sdp_type::RTCSdpType;
use sdp::{MediaDescription, SessionDescription};
use serde::{Deserialize, Serialize};
use shared::error::{Error, Result};
use std::collections::HashMap;
use std::io::Cursor;

const UNSPECIFIED_STR: &str = "Unspecified";

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

pub(crate) const MEDIA_SECTION_APPLICATION: &str = "application";

pub(crate) fn get_mid_value(media: &MediaDescription) -> Option<&String> {
    for attr in &media.attributes {
        if attr.key == "mid" {
            return attr.value.as_ref();
        }
    }
    None
}

/// RTPTransceiver represents a combination of an RTPSender and an RTPReceiver that share a common mid.
pub(crate) struct RTCRtpTransceiver {}

#[derive(Default)]
pub(crate) struct MediaSection {
    pub(crate) id: String,
    pub(crate) transceivers: Vec<RTCRtpTransceiver>,
    pub(crate) data: bool,
    pub(crate) rid_map: HashMap<String, String>,
    pub(crate) offered_direction: Option<RTCRtpTransceiverDirection>,
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

    #[test]
    fn test_new_rtp_transceiver_direction() {
        let tests = vec![
            ("Unspecified", RTCRtpTransceiverDirection::Unspecified),
            ("sendrecv", RTCRtpTransceiverDirection::Sendrecv),
            ("sendonly", RTCRtpTransceiverDirection::Sendonly),
            ("recvonly", RTCRtpTransceiverDirection::Recvonly),
            ("inactive", RTCRtpTransceiverDirection::Inactive),
        ];

        for (ct_str, expected_type) in tests {
            assert_eq!(RTCRtpTransceiverDirection::from(ct_str), expected_type);
        }
    }

    #[test]
    fn test_rtp_transceiver_direction_string() {
        let tests = vec![
            (RTCRtpTransceiverDirection::Unspecified, "Unspecified"),
            (RTCRtpTransceiverDirection::Sendrecv, "sendrecv"),
            (RTCRtpTransceiverDirection::Sendonly, "sendonly"),
            (RTCRtpTransceiverDirection::Recvonly, "recvonly"),
            (RTCRtpTransceiverDirection::Inactive, "inactive"),
        ];

        for (d, expected_string) in tests {
            assert_eq!(d.to_string(), expected_string);
        }
    }

    #[test]
    fn test_rtp_transceiver_has_send() {
        let tests = vec![
            (RTCRtpTransceiverDirection::Unspecified, false),
            (RTCRtpTransceiverDirection::Sendrecv, true),
            (RTCRtpTransceiverDirection::Sendonly, true),
            (RTCRtpTransceiverDirection::Recvonly, false),
            (RTCRtpTransceiverDirection::Inactive, false),
        ];

        for (d, expected_value) in tests {
            assert_eq!(d.has_send(), expected_value);
        }
    }

    #[test]
    fn test_rtp_transceiver_has_recv() {
        let tests = vec![
            (RTCRtpTransceiverDirection::Unspecified, false),
            (RTCRtpTransceiverDirection::Sendrecv, true),
            (RTCRtpTransceiverDirection::Sendonly, false),
            (RTCRtpTransceiverDirection::Recvonly, true),
            (RTCRtpTransceiverDirection::Inactive, false),
        ];

        for (d, expected_value) in tests {
            assert_eq!(d.has_recv(), expected_value);
        }
    }

    #[test]
    fn test_rtp_transceiver_from_send_recv() {
        let tests = vec![
            (RTCRtpTransceiverDirection::Sendrecv, (true, true)),
            (RTCRtpTransceiverDirection::Sendonly, (true, false)),
            (RTCRtpTransceiverDirection::Recvonly, (false, true)),
            (RTCRtpTransceiverDirection::Inactive, (false, false)),
        ];

        for (expected_value, (send, recv)) in tests {
            assert_eq!(
                RTCRtpTransceiverDirection::from_send_recv(send, recv),
                expected_value
            );
        }
    }

    #[test]
    fn test_rtp_transceiver_intersect() {
        use RTCRtpTransceiverDirection::*;

        let tests = vec![
            ((Sendrecv, Recvonly), Recvonly),
            ((Sendrecv, Sendonly), Sendonly),
            ((Sendrecv, Inactive), Inactive),
            ((Sendonly, Inactive), Inactive),
            ((Recvonly, Inactive), Inactive),
            ((Recvonly, Sendrecv), Recvonly),
            ((Sendonly, Sendrecv), Sendonly),
            ((Sendonly, Recvonly), Inactive),
            ((Recvonly, Recvonly), Recvonly),
        ];

        for ((a, b), expected_direction) in tests {
            assert_eq!(a.intersect(b), expected_direction);
        }
    }
}
