use crate::server::session::description::rtp_codec::RTPCodecType;
use crate::server::session::description::rtp_transceiver::{PayloadType, SSRC};
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub(crate) struct TrackLocal {
    pub(crate) id: String,
    pub(crate) stream_id: String,
    pub(crate) kind: RTPCodecType,
}

/// RTPSender allows an application to control how a given Track is encoded and transmitted to a remote peer
#[derive(Debug, Clone)]
pub struct RTCRtpSender {
    pub(crate) track: Option<TrackLocal>,

    //pub(crate) stream_info: Mutex<StreamInfo>,

    //pub(crate) context: Mutex<TrackLocalContext>,

    //pub(crate) transport: Arc<RTCDtlsTransport>,
    pub(crate) payload_type: PayloadType,
    pub(crate) ssrc: SSRC,
    pub(crate) receive_mtu: usize,

    /// a transceiver sender since we can just check the
    /// transceiver negotiation status
    pub(crate) negotiated: RefCell<bool>,

    //pub(crate) media_engine: Arc<MediaEngine>,
    //pub(crate) interceptor: Arc<dyn Interceptor + Send + Sync>,
    pub(crate) id: String,

    /// The id of the initial track, even if we later change to a different
    /// track id should be use when negotiating.
    pub(crate) initial_track_id: Option<String>,
    /// AssociatedMediaStreamIds from the WebRTC specifications
    pub(crate) associated_media_stream_ids: Vec<String>,

    //pub(crate) rtp_transceiver: Option<RTCRtpTransceiver>,
    pub(crate) paused: bool,
}

impl RTCRtpSender {
    pub(crate) fn is_negotiated(&self) -> bool {
        let negotiated = self.negotiated.borrow();
        *negotiated
    }

    pub(crate) fn set_negotiated(&self) {
        let mut negotiated = self.negotiated.borrow_mut();
        *negotiated = true;
    }
}
