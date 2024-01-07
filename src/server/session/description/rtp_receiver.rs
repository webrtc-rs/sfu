use crate::server::session::description::rtp_codec::{RTCRtpCodecParameters, RTPCodecType};
use std::fmt;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    /// We haven't started yet.
    Unstarted = 0,
    /// We haven't started yet and additionally we've been paused.
    UnstartedPaused = 1,

    /// We have started and are running.
    Started = 2,

    /// We have been paused after starting.
    Paused = 3,

    /// We have been stopped.
    Stopped = 4,
}

impl From<u8> for State {
    fn from(value: u8) -> Self {
        match value {
            v if v == State::Unstarted as u8 => State::Unstarted,
            v if v == State::UnstartedPaused as u8 => State::UnstartedPaused,
            v if v == State::Started as u8 => State::Started,
            v if v == State::Paused as u8 => State::Paused,
            v if v == State::Stopped as u8 => State::Stopped,
            _ => unreachable!(
                "Invalid serialization of {}: {}",
                std::any::type_name::<Self>(),
                value
            ),
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            State::Unstarted => write!(f, "Unstarted"),
            State::UnstartedPaused => write!(f, "UnstartedPaused"),
            State::Started => write!(f, "Running"),
            State::Paused => write!(f, "Paused"),
            State::Stopped => write!(f, "Closed"),
        }
    }
}

/// RTPReceiver allows an application to inspect the receipt of a TrackRemote
#[derive(Debug, Clone)]
pub struct RTCRtpReceiver {
    pub(crate) receive_mtu: usize,
    pub(crate) kind: RTPCodecType,
    //transport: Arc<RTCDtlsTransport>,

    // State is stored within the channel
    //state_tx: watch::Sender<State>,
    //state_rx: watch::Receiver<State>,

    //tracks: RwLock<Vec<TrackStreams>>,
    pub(crate) transceiver_codecs: Vec<RTCRtpCodecParameters>,
    //media_engine: Arc<MediaEngine>,
    //interceptor: Arc<dyn Interceptor + Send + Sync>,
}
