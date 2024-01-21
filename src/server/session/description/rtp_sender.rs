use crate::server::session::description::rtp_codec::RTPCodecType;
use crate::server::session::description::rtp_transceiver::SSRC;

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
    //pub(crate) payload_type: PayloadType,
    pub(crate) ssrc: SSRC,
    //pub(crate) receive_mtu: usize,
    /// a transceiver sender since we can just check the
    /// transceiver negotiation status
    negotiated: bool,

    //pub(crate) media_engine: Arc<MediaEngine>,
    //pub(crate) interceptor: Arc<dyn Interceptor + Send + Sync>,
    //pub(crate) id: String,
    /// The id of the initial track, even if we later change to a different
    /// track id should be use when negotiating.
    pub(crate) initial_track_id: Option<String>,
    /// AssociatedMediaStreamIds from the WebRTC specifications
    pub(crate) associated_media_stream_ids: Vec<String>,
    //pub(crate) rtp_transceiver: Option<RTCRtpTransceiver>,
    paused: bool,
}

impl RTCRtpSender {
    pub(crate) fn new() -> Self {
        Self {
            track: None,
            ssrc: rand::random::<u32>(),
            negotiated: false,
            initial_track_id: None,
            associated_media_stream_ids: vec![],
            paused: false,
        }
    }

    pub(crate) fn is_negotiated(&self) -> bool {
        self.negotiated
    }

    pub(crate) fn set_negotiated(&mut self) {
        self.negotiated = true;
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }
}
