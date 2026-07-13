#![allow(dead_code)]

//! Integration-test client for the SFU `chat` server.
//!
//! Mirrors `examples/chat.html`, but as a headless Rust client built on the async `webrtc`
//! API. Signaling is the chat server's AppRTC/Collider protocol over one TLS WebSocket
//! (`wss://host/ws`): `register {roomid, clientid}` once, then SDP as `{cmd:"offer"|"answer",
//! sdp, request_id?}` frames. The server answers our publish offers and pushes
//! server-initiated subscribe re-offers on the same socket, which we auto-answer (echoing the
//! `request_id`) exactly like the browser does.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::timeout;
use tokio_tungstenite::Connector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_OPUS, MIME_TYPE_VP8};
use rtc::rtp;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RTCRtpHeaderExtensionCapability, RtpCodecKind,
};
use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};

use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCPeerConnectionState, RTCSdpType, RTCSessionDescription, Registry,
    register_default_interceptors,
};
use webrtc::runtime::default_runtime;

/// The chat server binds its signaling + media on the loopback address when run with `-f`
/// (`--force_local_loop`), which is how the integration harness / Dockerfile launch it.
pub const HOST: &str = "127.0.0.1";
pub const SIGNAL_PORT: u16 = 8080;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

// ───────────────────────────────── signaling wire types ─────────────────────────────────

/// Browser → SFU frame (`WsClientMsg` on the server). `sdp` carries a full
/// `RTCSessionDescription` (`{type, sdp}`); `request_id` is echoed back only when answering a
/// server-initiated re-offer.
#[derive(Serialize)]
struct ClientMsg<'a> {
    cmd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    roomid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clientid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdp: Option<RTCSessionDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<u64>,
}

/// SFU → browser frame: a flattened `RTCSessionDescription` plus an optional `request_id`
/// (present on subscribe re-offers).
#[derive(Deserialize)]
struct ServerMsg {
    #[serde(flatten)]
    sdp: RTCSessionDescription,
    #[serde(default)]
    request_id: Option<u64>,
}

// ─────────────────────────────────────── TLS ────────────────────────────────────────────

/// The chat server presents a self-signed certificate; the test client trusts it blindly.
mod danger {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub struct NoCertVerification;

    impl ServerCertVerifier for NoCertVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }
}

fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub fn init_logging() {
    use std::io::Write;
    let _ = env_logger::Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{}:{} [{}] {} - {}",
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.level(),
                chrono::Local::now().format("%H:%M:%S.%6f"),
                record.args()
            )
        })
        .filter(None, log::LevelFilter::Info)
        .is_test(true)
        .try_init();
}

// ─────────────────────────────── peer connection event handler ──────────────────────────

#[derive(Clone)]
struct Handler {
    conn_state_tx: UnboundedSender<RTCPeerConnectionState>,
    track_tx: UnboundedSender<Arc<dyn TrackRemote>>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let _ = self.conn_state_tx.send(state);
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let _ = self.track_tx.send(track);
    }
}

/// The non-default payload types the "custom" publisher registers for VP8 / Opus. Both are
/// picked from the dynamic range but differ from every payload type `register_default_codecs`
/// assigns, so a subscriber that negotiates the defaults only receives these packets if the SFU
/// rewrote the payload type while forwarding. See [`custom_media_engine`].
pub const CUSTOM_VP8_PAYLOAD_TYPE: u8 = 120;
pub const CUSTOM_OPUS_PAYLOAD_TYPE: u8 = 118;

/// A media engine carrying the browser-default codec set (VP8 at PT 96, Opus at PT 111, …).
fn default_media_engine() -> Result<MediaEngine> {
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    Ok(media)
}

/// A media engine that registers VP8 and Opus at [`CUSTOM_VP8_PAYLOAD_TYPE`] /
/// [`CUSTOM_OPUS_PAYLOAD_TYPE`] instead of their default payload types. The codec definitions
/// otherwise mirror `register_default_codecs` (same clock rate, channels, fmtp) so they still
/// negotiate against the SFU's default codecs — only the payload type differs, which is exactly
/// what forces the SFU's inbound→outbound payload-type translation to kick in.
fn custom_media_engine() -> Result<MediaEngine> {
    let mut media = MediaEngine::default();
    media.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: CUSTOM_OPUS_PAYLOAD_TYPE,
        },
        RtpCodecKind::Audio,
    )?;
    media.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_VP8.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: String::new(),
                rtcp_feedback: vec![],
            },
            payload_type: CUSTOM_VP8_PAYLOAD_TYPE,
        },
        RtpCodecKind::Video,
    )?;
    Ok(media)
}

/// The transport-wide congestion control RTP header extension URI. `register_default_interceptors`
/// registers it (for both audio and video) on every peer, including the SFU.
use rtc::sdp::extmap::TRANSPORT_CC_URI;

/// A media engine that registers the transport-cc header extension *before* the default
/// interceptor extensions, so it negotiates a low extension id (1) instead of the id the SFU
/// assigns (4, after the simulcast MID/RID/RRID extensions `register_default_interceptors`
/// registers first). Codecs are the browser defaults. A publisher built from this engine sends
/// transport-cc at id 1, but a default-codec subscriber negotiates id 4 — forcing the SFU's
/// inbound→outbound header-extension-id translation to kick in. See [`connect_ext_custom`].
fn custom_ext_media_engine() -> Result<MediaEngine> {
    let mut media = MediaEngine::default();
    media.register_default_codecs()?;
    // Registered first → lands at index 0 of the extension table → negotiates as id 1.
    // `register_default_interceptors` (run in `build_peer_connection`) later appends MID/RID/RRID
    // and re-registers transport-cc by uri (a no-op that preserves this ordering).
    for kind in [RtpCodecKind::Video, RtpCodecKind::Audio] {
        media.register_header_extension(
            RTCRtpHeaderExtensionCapability {
                uri: TRANSPORT_CC_URI.to_owned(),
            },
            kind,
            None,
        )?;
    }
    Ok(media)
}

async fn build_peer_connection(
    handler: Handler,
    mut media: MediaEngine,
) -> Result<Arc<dyn PeerConnection>> {
    let registry = register_default_interceptors(Registry::new(), &mut media)?;
    let runtime = default_runtime().ok_or_else(|| anyhow!("no async runtime found"))?;

    let config = RTCConfigurationBuilder::new().build();

    let pc = PeerConnectionBuilder::new()
        .with_configuration(config)
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(Arc::new(handler) as Arc<dyn PeerConnectionEventHandler>)
        .with_runtime(runtime)
        .with_udp_addrs(vec![format!("{HOST}:0")])
        .build()
        .await?;

    Ok(Arc::new(pc) as Arc<dyn PeerConnection>)
}

// ──────────────────────────────────── WebSocket ─────────────────────────────────────────

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_connect(host: &str, port: u16) -> Result<WsStream> {
    ensure_crypto_provider();
    let request = format!("wss://{host}:{port}/ws").into_client_request()?;
    let tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(danger::NoCertVerification))
        .with_no_client_auth();
    let connector = Connector::Rustls(Arc::new(tls_config));
    let (ws, _resp) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
            .await?;
    Ok(ws)
}

fn frame(msg: &ClientMsg) -> Result<Message> {
    Ok(Message::text(serde_json::to_string(msg)?))
}

// ─────────────────────────────────────── Peer ───────────────────────────────────────────

/// A connected test peer: its `PeerConnection`, bootstrap data channel, and the receive ends
/// for server-pushed SDP (renegotiation answers + subscribe re-offers) and remote tracks.
pub struct Peer {
    pub pc: Arc<dyn PeerConnection>,
    pub data_channel: Arc<dyn DataChannel>,
    out_tx: UnboundedSender<Message>,
    sdp_rx: UnboundedReceiver<RTCSessionDescription>,
    track_rx: UnboundedReceiver<Arc<dyn TrackRemote>>,
    pub room_id: u64,
    pub client_id: u64,
}

impl Peer {
    /// Renegotiate: create an offer, apply it locally, and publish it to the SFU. Used after
    /// adding a track. The SFU's answer (and any resulting subscribe re-offers to other peers)
    /// arrive asynchronously via [`Peer::next_sdp`] on the respective peers.
    pub async fn renegotiate(&self) -> Result<()> {
        let offer = self.pc.create_offer(None).await?;
        self.pc.set_local_description(offer.clone()).await?;
        log::info!(
            "test client: client_id={} room_id={} sends renegotiation offer:\n{}",
            self.client_id,
            self.room_id,
            offer.sdp
        );
        self.out_tx.send(frame(&ClientMsg {
            cmd: "offer",
            roomid: None,
            clientid: None,
            sdp: Some(offer),
            request_id: None,
        })?)?;
        Ok(())
    }

    /// Add a send track and return it together with the ssrc and payload type it was created
    /// / negotiated with. The sender validates (does not rewrite) written packets, so callers
    /// must stamp both `ssrc` and `payload_type` onto every packet they write.
    pub async fn add_track(
        &self,
        mime: &str,
        track_id: &str,
        direction: RTCRtpTransceiverDirection,
    ) -> Result<SendTrack> {
        self.add_track_inner(mime, track_id, direction, None).await
    }

    /// Like [`Peer::add_track`], but the returned [`SendTrack`] also stamps a one-byte RTP header
    /// extension (`uri`, carrying `payload`) onto every packet, at the id this sender negotiated
    /// for `uri`. The sender must have that extension registered (e.g. via
    /// [`connect_ext_custom`]); the negotiated send id is captured so callers can assert the SFU
    /// rewrites it to the subscriber's id on forward.
    pub async fn add_track_with_extension(
        &self,
        mime: &str,
        track_id: &str,
        direction: RTCRtpTransceiverDirection,
        uri: &str,
        payload: bytes::Bytes,
    ) -> Result<SendTrack> {
        self.add_track_inner(mime, track_id, direction, Some((uri, payload)))
            .await
    }

    async fn add_track_inner(
        &self,
        mime: &str,
        track_id: &str,
        direction: RTCRtpTransceiverDirection,
        extension: Option<(&str, bytes::Bytes)>,
    ) -> Result<SendTrack> {
        let is_audio = mime.to_ascii_lowercase().starts_with("audio");
        let (kind, clock_rate, channels, fmtp) = if is_audio {
            (
                RtpCodecKind::Audio,
                48000,
                2,
                "minptime=10;useinbandfec=1".to_owned(),
            )
        } else {
            (RtpCodecKind::Video, 90000, 0, String::new())
        };
        let codec = RTCRtpCodec {
            mime_type: mime.to_owned(),
            clock_rate,
            channels,
            sdp_fmtp_line: fmtp,
            rtcp_feedback: vec![],
        };
        let ssrc = rand::random::<u32>();
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            format!("stream-{track_id}"),
            track_id.to_owned(),
            format!("label-{track_id}"),
            kind,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                active: true,
                codec: codec.clone(),
                ..Default::default()
            }],
        )));

        let transceiver = self
            .pc
            .add_transceiver_from_track(
                Arc::clone(&track) as Arc<dyn TrackLocal>,
                Some(RTCRtpTransceiverInit {
                    direction,
                    streams: vec![],
                    send_encodings: vec![],
                }),
            )
            .await?;

        let sender = transceiver
            .sender()
            .await?
            .ok_or_else(|| anyhow!("transceiver has no sender"))?;
        let params = sender.get_parameters().await?;
        let payload_type = params
            .rtp_parameters
            .codecs
            .iter()
            .find(|c| c.rtp_codec.mime_type.eq_ignore_ascii_case(mime))
            .or_else(|| params.rtp_parameters.codecs.first())
            .map(|c| c.payload_type)
            .ok_or_else(|| anyhow!("sender has no negotiated codec"))?;

        let send_extension = match extension {
            Some((uri, payload)) => {
                let id = params
                    .rtp_parameters
                    .header_extensions
                    .iter()
                    .find(|e| e.uri == uri)
                    .map(|e| e.id as u8)
                    .ok_or_else(|| anyhow!("sender did not negotiate header extension {uri}"))?;
                Some(SendExtension { id, payload })
            }
            None => None,
        };

        Ok(SendTrack {
            track,
            ssrc,
            payload_type,
            extension: send_extension,
        })
    }

    /// Await the next server-pushed SDP (a renegotiation answer to our own offer, or a
    /// subscribe re-offer the SFU sent us — the latter is auto-answered before it is surfaced).
    pub async fn next_sdp(&mut self) -> Result<RTCSessionDescription> {
        timeout(CONNECT_TIMEOUT, self.sdp_rx.recv())
            .await
            .map_err(|_| anyhow!("timed out waiting for SDP"))?
            .ok_or_else(|| anyhow!("signaling channel closed"))
    }

    /// The payload type this peer, acting as a subscriber, negotiated for `mime` — i.e. the
    /// payload type the SFU must rewrite forwarded packets to. Scans every transceiver's
    /// receiver parameters for the codec whose mime type matches. Must be called *after*
    /// [`Peer::next_track`] for the corresponding track: a transceiver's receiver is only
    /// attached once its first RTP packet arrives (when `on_track` fires).
    pub async fn negotiated_recv_payload_type(&self, mime: &str) -> Result<u8> {
        for transceiver in self.pc.get_transceivers().await {
            let Some(receiver) = transceiver.receiver().await? else {
                continue;
            };
            let params = receiver.get_parameters().await?;
            if let Some(codec) = params
                .rtp_parameters
                .codecs
                .iter()
                .find(|c| c.rtp_codec.mime_type.eq_ignore_ascii_case(mime))
            {
                return Ok(codec.payload_type);
            }
        }
        Err(anyhow!("no negotiated receiver codec for {mime}"))
    }

    /// The id this peer, acting as a subscriber, negotiated for the header extension `uri` — i.e.
    /// the id the SFU must rewrite forwarded packets' extension to. Scans every transceiver's
    /// receiver parameters for the extension whose uri matches. Must be called *after*
    /// [`Peer::next_track`] for the corresponding track (see [`Peer::negotiated_recv_payload_type`]).
    pub async fn negotiated_recv_extension_id(&self, uri: &str) -> Result<u8> {
        for transceiver in self.pc.get_transceivers().await {
            let Some(receiver) = transceiver.receiver().await? else {
                continue;
            };
            let params = receiver.get_parameters().await?;
            if let Some(ext) = params
                .rtp_parameters
                .header_extensions
                .iter()
                .find(|e| e.uri == uri)
            {
                return Ok(ext.id as u8);
            }
        }
        Err(anyhow!("no negotiated receiver header extension for {uri}"))
    }

    /// Await the next remote track the SFU forwards to this peer.
    pub async fn next_track(&mut self) -> Result<Arc<dyn TrackRemote>> {
        timeout(CONNECT_TIMEOUT, self.track_rx.recv())
            .await
            .map_err(|_| anyhow!("timed out waiting for remote track"))?
            .ok_or_else(|| anyhow!("track channel closed"))
    }

    pub async fn close(self) -> Result<()> {
        let _ = self.out_tx.send(frame(&ClientMsg {
            cmd: "leave",
            roomid: None,
            clientid: None,
            sdp: None,
            request_id: None,
        })?);
        self.pc.close().await?;
        Ok(())
    }
}

/// Read the next RTP packet from a forwarded remote track, skipping non-RTP events.
pub async fn read_rtp(track: &Arc<dyn TrackRemote>) -> Result<rtp::Packet> {
    loop {
        match timeout(CONNECT_TIMEOUT, track.poll()).await {
            Ok(Some(TrackRemoteEvent::OnRtpPacket(pkt))) => return Ok(pkt),
            Ok(Some(_)) => continue,
            Ok(None) => return Err(anyhow!("remote track ended")),
            Err(_) => return Err(anyhow!("timed out reading RTP")),
        }
    }
}

/// The fixed VP8 payload every writer sends; readers assert it round-trips byte-for-byte.
pub const RTP_PAYLOAD: &[u8] = &[0x98, 0x36, 0xbe, 0x88, 0x9e];

/// A one-byte RTP header extension the sender stamps onto every packet, at the id it negotiated.
pub struct SendExtension {
    pub id: u8,
    pub payload: bytes::Bytes,
}

/// A local send track plus the ssrc/payload-type the sender will accept for it, and an optional
/// header extension it stamps onto every packet.
pub struct SendTrack {
    pub track: Arc<TrackLocalStaticRTP>,
    pub ssrc: u32,
    pub payload_type: u8,
    pub extension: Option<SendExtension>,
}

impl SendTrack {
    fn packet(&self, sequence_number: u16) -> rtp::Packet {
        let mut header = rtp::header::Header {
            version: 2,
            marker: true,
            payload_type: self.payload_type,
            sequence_number,
            timestamp: 3653407706,
            ssrc: self.ssrc,
            ..Default::default()
        };
        if let Some(ext) = &self.extension {
            header.extension = true;
            header.extension_profile = rtp::header::EXTENSION_PROFILE_ONE_BYTE;
            header.extensions = vec![rtp::header::Extension {
                id: ext.id,
                payload: ext.payload.clone(),
            }];
        }
        rtp::Packet {
            header,
            payload: bytes::Bytes::from_static(RTP_PAYLOAD),
        }
    }

    /// Continuously write RTP with monotonically increasing sequence numbers until stopped,
    /// modelling a live sender. Returns a handle plus the stop flag.
    pub fn spawn_writer(
        self: Arc<Self>,
    ) -> (
        tokio::task::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = Arc::new(AtomicBool::new(false));
        let stop_signal = stop.clone();
        let handle = tokio::spawn(async move {
            let mut sequence_number: u16 = 1;
            while !stop_signal.load(Ordering::Relaxed) {
                if self
                    .track
                    .write_rtp(self.packet(sequence_number))
                    .await
                    .is_err()
                {
                    break;
                }
                sequence_number = sequence_number.wrapping_add(1);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        (handle, stop)
    }
}

/// Assert that a forwarded remote track delivers our payload in-order: read the first packet,
/// then verify the next `count` arrive with contiguous sequence numbers and identical payload.
/// Tolerates the leading packets a subscriber misses before its forward path is fully wired.
pub async fn verify_rtp_flow(track: &Arc<dyn TrackRemote>, count: usize) -> Result<()> {
    verify_rtp_flow_inner(track, count, None).await
}

/// Like [`verify_rtp_flow`], but additionally asserts every forwarded packet carries
/// `expected_payload_type` — the payload type the subscriber negotiated. When the publisher
/// sent a different payload type, this proves the SFU translated it on the way through.
pub async fn verify_rtp_flow_payload_type(
    track: &Arc<dyn TrackRemote>,
    count: usize,
    expected_payload_type: u8,
) -> Result<()> {
    verify_rtp_flow_inner(track, count, Some(expected_payload_type)).await
}

async fn verify_rtp_flow_inner(
    track: &Arc<dyn TrackRemote>,
    count: usize,
    expected_payload_type: Option<u8>,
) -> Result<()> {
    use std::collections::HashMap;

    // A subscriber's track can surface more than one forwarded source (ssrc); verify each
    // ssrc's own sub-stream arrives with contiguous sequence numbers and our payload.
    let mut next_seq: HashMap<u32, u16> = HashMap::new();
    for _ in 0..count {
        let pkt = read_rtp(track).await?;
        if pkt.payload.as_ref() != RTP_PAYLOAD {
            return Err(anyhow!("unexpected RTP payload {:?}", pkt.payload));
        }
        if let Some(expected) = expected_payload_type
            && pkt.header.payload_type != expected
        {
            return Err(anyhow!(
                "unexpected forwarded payload type on ssrc {}: expected {expected}, got {}",
                pkt.header.ssrc,
                pkt.header.payload_type
            ));
        }
        let seq = pkt.header.sequence_number;
        match next_seq.get(&pkt.header.ssrc) {
            None => {} // first packet seen for this ssrc; adopt whatever seq it starts at
            Some(&expected) if seq != expected => {
                return Err(anyhow!(
                    "sequence gap on ssrc {}: expected {expected}, got {seq}",
                    pkt.header.ssrc
                ));
            }
            Some(_) => {}
        }
        next_seq.insert(pkt.header.ssrc, seq.wrapping_add(1));
    }
    Ok(())
}

/// Like [`verify_rtp_flow`], but additionally asserts every forwarded packet carries the header
/// extension `expected_id` with byte-for-byte `expected_payload`. When the publisher sent the
/// same extension under a different id, this proves the SFU translated the id (and preserved the
/// payload) on the way through.
pub async fn verify_rtp_flow_extension(
    track: &Arc<dyn TrackRemote>,
    count: usize,
    expected_id: u8,
    expected_payload: &[u8],
) -> Result<()> {
    use std::collections::HashMap;

    let mut next_seq: HashMap<u32, u16> = HashMap::new();
    for _ in 0..count {
        let pkt = read_rtp(track).await?;
        if pkt.payload.as_ref() != RTP_PAYLOAD {
            return Err(anyhow!("unexpected RTP payload {:?}", pkt.payload));
        }
        let ext = pkt
            .header
            .extensions
            .iter()
            .find(|e| e.id == expected_id)
            .ok_or_else(|| {
                anyhow!(
                    "forwarded packet on ssrc {} missing header extension id {expected_id}; got ids {:?}",
                    pkt.header.ssrc,
                    pkt.header.extensions.iter().map(|e| e.id).collect::<Vec<_>>()
                )
            })?;
        if ext.payload.as_ref() != expected_payload {
            return Err(anyhow!(
                "unexpected header extension payload on ssrc {}: expected {expected_payload:?}, got {:?}",
                pkt.header.ssrc,
                ext.payload
            ));
        }
        let seq = pkt.header.sequence_number;
        match next_seq.get(&pkt.header.ssrc) {
            None => {}
            Some(&expected) if seq != expected => {
                return Err(anyhow!(
                    "sequence gap on ssrc {}: expected {expected}, got {seq}",
                    pkt.header.ssrc
                ));
            }
            Some(_) => {}
        }
        next_seq.insert(pkt.header.ssrc, seq.wrapping_add(1));
    }
    Ok(())
}

// ─────────────────────────────────────── connect ────────────────────────────────────────

/// Register `client_id` into `room_id` on the chat SFU, publish an initial data-channel-only
/// offer, and drive the WebSocket signaling loop until the peer connection is connected and
/// the bootstrap data channel is open.
pub async fn connect(host: &str, port: u16, room_id: u64, client_id: u64) -> Result<Peer> {
    connect_with_media_engine(host, port, room_id, client_id, default_media_engine()?).await
}

/// Like [`connect`], but the peer registers VP8 / Opus at the non-default payload types in
/// [`custom_media_engine`]. Used for publishers whose inbound payload types must differ from
/// what default-codec subscribers negotiate, forcing the SFU to translate on forward.
pub async fn connect_custom(host: &str, port: u16, room_id: u64, client_id: u64) -> Result<Peer> {
    connect_with_media_engine(host, port, room_id, client_id, custom_media_engine()?).await
}

/// Like [`connect`], but the peer negotiates the transport-cc header extension at a non-default
/// id (see [`custom_ext_media_engine`]). Used for publishers whose inbound header-extension id
/// must differ from what default subscribers negotiate, forcing the SFU to translate on forward.
pub async fn connect_ext_custom(
    host: &str,
    port: u16,
    room_id: u64,
    client_id: u64,
) -> Result<Peer> {
    connect_with_media_engine(host, port, room_id, client_id, custom_ext_media_engine()?).await
}

async fn connect_with_media_engine(
    host: &str,
    port: u16,
    room_id: u64,
    client_id: u64,
    media: MediaEngine,
) -> Result<Peer> {
    init_logging();

    let (conn_state_tx, mut conn_state_rx) = unbounded_channel();
    let (track_tx, track_rx) = unbounded_channel();
    let pc = build_peer_connection(
        Handler {
            conn_state_tx,
            track_tx,
        },
        media,
    )
    .await?;

    let ws = ws_connect(host, port).await?;
    let (mut ws_write, mut ws_read) = ws.split();

    // Single writer task owns the sink; both `connect` and the inbound task enqueue frames.
    let (out_tx, mut out_rx) = unbounded_channel::<Message>();
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_write.send(msg).await.is_err() {
                break;
            }
        }
    });

    // register {room, client}
    out_tx.send(frame(&ClientMsg {
        cmd: "register",
        roomid: Some(room_id),
        clientid: Some(client_id),
        sdp: None,
        request_id: None,
    })?)?;

    // Bootstrap data channel + initial offer (mirrors chat.html: send the offer immediately;
    // ICE candidates are learned peer-reflexively, no candidate signaling in this protocol).
    let data_channel = pc.create_data_channel("bootstrap", None).await?;
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer.clone()).await?;
    log::info!(
        "test client: client_id={} room_id={} sends initial bootstrap offer:\n{}",
        client_id,
        room_id,
        offer.sdp
    );
    out_tx.send(frame(&ClientMsg {
        cmd: "offer",
        roomid: None,
        clientid: None,
        sdp: Some(offer),
        request_id: None,
    })?)?;

    // Inbound task: apply answers, auto-answer subscribe re-offers, and surface every SDP.
    let (sdp_tx, mut sdp_rx) = unbounded_channel::<RTCSessionDescription>();
    let pc_in = pc.clone();
    let out_in = out_tx.clone();
    tokio::spawn(async move {
        while let Some(Ok(message)) = ws_read.next().await {
            let text = match message {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            let server: ServerMsg = match serde_json::from_str(&text) {
                Ok(server) => server,
                Err(err) => {
                    log::warn!("test client: bad server frame: {err}");
                    continue;
                }
            };
            let sdp = server.sdp;
            match sdp.sdp_type {
                RTCSdpType::Answer => {
                    log::info!(
                        "test client: client_id={} room_id={} receives Answer:\n{}",
                        client_id,
                        room_id,
                        sdp.sdp
                    );
                    if let Err(err) = pc_in.set_remote_description(sdp.clone()).await {
                        log::error!("test client: set_remote_description(answer): {err}");
                    }
                }
                RTCSdpType::Offer => {
                    log::info!(
                        "test client: client_id={} room_id={} receives Offer:\n{}",
                        client_id,
                        room_id,
                        sdp.sdp
                    );
                    if let Err(err) = pc_in.set_remote_description(sdp.clone()).await {
                        log::error!("test client: set_remote_description(offer): {err}");
                        continue;
                    }
                    match pc_in.create_answer(None).await {
                        Ok(answer) => {
                            log::info!(
                                "test client: client_id={} room_id={} sends auto-Answer:\n{}",
                                client_id,
                                room_id,
                                answer.sdp
                            );
                            if let Err(err) = pc_in.set_local_description(answer.clone()).await {
                                log::error!("test client: set_local_description(answer): {err}");
                                continue;
                            }
                            let payload = ClientMsg {
                                cmd: "answer",
                                roomid: None,
                                clientid: None,
                                sdp: Some(answer),
                                request_id: server.request_id,
                            };
                            if let Ok(msg) = frame(&payload) {
                                let _ = out_in.send(msg);
                            }
                        }
                        Err(err) => log::error!("test client: create_answer: {err}"),
                    }
                }
                _ => {}
            }
            let _ = sdp_tx.send(sdp);
        }
    });

    wait_connected(&mut conn_state_rx).await?;
    wait_data_channel_open(&data_channel).await?;

    // The initial data-channel-only answer has been applied; drain it so tests observe only
    // SDP produced by post-connect renegotiation.
    while sdp_rx.try_recv().is_ok() {}

    Ok(Peer {
        pc,
        data_channel,
        out_tx,
        sdp_rx,
        track_rx,
        room_id,
        client_id,
    })
}

async fn wait_connected(
    conn_state_rx: &mut UnboundedReceiver<RTCPeerConnectionState>,
) -> Result<()> {
    let fut = async {
        while let Some(state) = conn_state_rx.recv().await {
            match state {
                RTCPeerConnectionState::Connected => return Ok(()),
                RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
                    return Err(anyhow!("peer connection {state}"));
                }
                _ => {}
            }
        }
        Err(anyhow!("connection state channel closed"))
    };
    timeout(CONNECT_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow!("timed out waiting for connection"))?
}

async fn wait_data_channel_open(data_channel: &Arc<dyn DataChannel>) -> Result<()> {
    let fut = async {
        loop {
            match data_channel.poll().await {
                Some(DataChannelEvent::OnOpen) => return Ok(()),
                Some(DataChannelEvent::OnClose) | None => {
                    return Err(anyhow!("data channel closed before opening"));
                }
                Some(_) => {}
            }
        }
    };
    timeout(CONNECT_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow!("timed out waiting for data channel open"))?
}
