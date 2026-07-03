use hyper::body::to_bytes;
use hyper::{Body, Client, Method, Request};
use rtc::interceptor::Registry;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::RTCPeerConnectionBuilder;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_VP8, MediaEngine};
use rtc::peer_connection::event::{RTCPeerConnectionEvent, RTCTrackEvent};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::peer_connection::state::{RTCIceConnectionState, RTCPeerConnectionState};
use rtc::peer_connection::transport::{CandidateConfig, CandidateHostConfig, RTCIceCandidate};
use rtc::rtp;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use rtc::sansio::Protocol;
use rtc::shared::error::Error;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use std::env;
use std::error::Error as StdError;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const DEFAULT_SIGNALING_URL: &str = "http://127.0.0.1:8080/offer";
const DEFAULT_TIMEOUT_DURATION: Duration = Duration::from_secs(86400);
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

type DynError = Box<dyn StdError + Send + Sync + 'static>;

#[tokio::main]
async fn main() -> std::result::Result<(), DynError> {
    let signaling_url =
        env::var("SFU_SIGNALING_URL").unwrap_or_else(|_| DEFAULT_SIGNALING_URL.to_owned());
    let timeout = env::var("SFU_PROBE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_WAIT_TIMEOUT);

    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    let local_addr = socket.local_addr()?;

    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs()?;
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;

    let mut pc = RTCPeerConnectionBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .build()?;

    let local_candidate = RTCIceCandidate::from(
        &CandidateHostConfig {
            base_config: CandidateConfig {
                network: "udp".to_owned(),
                address: local_addr.ip().to_string(),
                port: local_addr.port(),
                component: 1,
                ..Default::default()
            },
            ..Default::default()
        }
        .new_candidate_host()?,
    )
    .to_json()?;
    pc.add_local_candidate(local_candidate)?;

    let video_track = MediaStreamTrack::new(
        "probe-stream".to_owned(),
        "probe-video".to_owned(),
        "Probe Video".to_owned(),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(0x2000_0001),
                ..Default::default()
            },
            codec: RTCRtpCodec {
                mime_type: MIME_TYPE_VP8.to_owned(),
                clock_rate: 90_000,
                channels: 0,
                sdp_fmtp_line: String::new(),
                rtcp_feedback: vec![],
            },
            ..Default::default()
        }],
    );
    let sender_id = pc.add_track(video_track)?;

    let offer = pc.create_offer(None)?;
    pc.set_local_description(offer)?;
    let local_offer = pc
        .local_description()
        .ok_or_else(|| Error::Other("missing local offer".to_owned()))?;

    let request = Request::builder()
        .method(Method::POST)
        .uri(&signaling_url)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&local_offer)?))?;
    let response = Client::new().request(request).await?;
    if !response.status().is_success() {
        return Err(std::io::Error::other(format!(
            "offer request failed with status {}",
            response.status()
        ))
        .into());
    }

    let answer =
        serde_json::from_slice::<RTCSessionDescription>(&to_bytes(response.into_body()).await?)?;
    pc.set_remote_description(answer)?;

    let deadline = Instant::now() + timeout;
    let mut buffer = vec![0_u8; 2000];
    let mut connected = false;
    let mut sent = false;

    loop {
        while let Some(msg) = pc.poll_write() {
            socket
                .send_to(&msg.message, msg.transport.peer_addr)
                .await?;
        }

        while let Some(event) = pc.poll_event() {
            match event {
                RTCPeerConnectionEvent::OnIceConnectionStateChangeEvent(state) => {
                    println!("probe ICE state: {state}");
                    if state == RTCIceConnectionState::Failed {
                        return Err(std::io::Error::other("probe ICE failed").into());
                    }
                }
                RTCPeerConnectionEvent::OnConnectionStateChangeEvent(state) => {
                    println!("probe PC state: {state}");
                    if state == RTCPeerConnectionState::Connected {
                        connected = true;
                    }
                    if state == RTCPeerConnectionState::Failed {
                        return Err(std::io::Error::other("probe peer connection failed").into());
                    }
                }
                RTCPeerConnectionEvent::OnTrack(RTCTrackEvent::OnOpen(init)) => {
                    println!("probe remote track opened: {:?}", init.track_id);
                }
                _ => {}
            }
        }

        while let Some(message) = pc.poll_read() {
            if let RTCMessage::RtpPacket(_, packet) = message {
                println!(
                    "probe received echoed RTP seq={} ts={} payload_len={}",
                    packet.header.sequence_number,
                    packet.header.timestamp,
                    packet.payload.len()
                );
                return Ok(());
            }
        }

        if connected && !sent {
            let mut sender = pc
                .rtp_sender(sender_id)
                .ok_or(Error::ErrRTPSenderNotExisted)?;
            let ssrc = sender
                .track()
                .ssrcs()
                .last()
                .ok_or(Error::ErrSenderWithNoSSRCs)?;
            sender.write_rtp(rtp::Packet {
                header: rtp::Header {
                    version: 2,
                    payload_type: 96,
                    sequence_number: 1,
                    timestamp: 1,
                    ssrc,
                    ..Default::default()
                },
                payload: vec![1, 2, 3, 4].into(),
            })?;
            sent = true;
            continue;
        }

        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "probe timed out waiting for RTP echo",
            )
            .into());
        }

        let timeout_at = pc
            .poll_timeout()
            .unwrap_or_else(|| Instant::now() + DEFAULT_TIMEOUT_DURATION);
        let delay = timeout_at.saturating_duration_since(Instant::now());
        if delay.is_zero() {
            pc.handle_timeout(Instant::now())?;
            continue;
        }

        let timer = tokio::time::sleep(delay.min(Duration::from_millis(100)));
        tokio::pin!(timer);

        tokio::select! {
            _ = timer.as_mut() => {
                pc.handle_timeout(Instant::now())?;
            }
            recv_result = socket.recv_from(&mut buffer) => {
                let (n, peer_addr) = recv_result?;
                pc.handle_read(TaggedBytesMut {
                    now: Instant::now(),
                    transport: TransportContext {
                        local_addr,
                        peer_addr,
                        ecn: None,
                        transport_protocol: TransportProtocol::UDP,
                    },
                    message: (&buffer[..n]).into(),
                })?;
            }
        }
    }
}
