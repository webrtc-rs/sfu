use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::media_engine::{MIME_TYPE_OPUS, MIME_TYPE_VP8};
use rtc::peer_connection::event::{RTCPeerConnectionEvent, RTCTrackEvent, RTCTrackEventInit};
use rtc::peer_connection::message::RTCMessage;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::rtp_transceiver::RTCRtpSenderId;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use rtc::shared::error::{Error, Result as RtcResult};
use sfu::driver::udp::tagged_udp_packet;
use sfu::{OfferRequest, SignalAdapter, UdpDriver};
use std::collections::HashMap;
use std::env;
use std::error::Error as StdError;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

const DEFAULT_HTTP_BIND_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_UDP_BIND_ADDR: &str = "127.0.0.1:3478";
const DEFAULT_TIMEOUT_DURATION: Duration = Duration::from_secs(86400);
const ROOM_ID: u64 = 1;
const CLIENT_ID: u64 = 1;

type DynError = Box<dyn StdError + Send + Sync + 'static>;

#[derive(Clone)]
struct HttpState {
    command_tx: mpsc::Sender<DriverCommand>,
}

#[derive(Default)]
struct EchoState {
    sender_by_kind: HashMap<RtpCodecKind, RTCRtpSenderId>,
    sender_by_track_id: HashMap<String, RTCRtpSenderId>,
}

enum DriverCommand {
    AcceptOffer {
        offer: RTCSessionDescription,
        response_tx: oneshot::Sender<std::result::Result<RTCSessionDescription, String>>,
    },
}

#[tokio::main]
async fn main() -> std::result::Result<(), DynError> {
    let http_bind_addr = socket_addr_from_env("SFU_HTTP_BIND_ADDR", DEFAULT_HTTP_BIND_ADDR)?;
    let udp_bind_addr = socket_addr_from_env("SFU_UDP_BIND_ADDR", DEFAULT_UDP_BIND_ADDR)?;

    let udp_socket = UdpSocket::bind(udp_bind_addr).await?;
    let udp_local_addr = udp_socket.local_addr()?;
    let udp_candidate_addr = advertised_udp_addr(udp_local_addr)?;

    let (command_tx, command_rx) = mpsc::channel::<DriverCommand>(16);
    let http_state = HttpState { command_tx };
    let make_svc = make_service_fn(move |_| {
        let http_state = http_state.clone();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                handle_http_request(req, http_state.clone())
            }))
        }
    });

    println!("HTTP signaling listening on http://{http_bind_addr}/offer");
    println!("UDP media listening on {udp_local_addr}");
    println!("Advertising ICE host candidate {udp_candidate_addr}");

    let http_server = Server::bind(&http_bind_addr).serve(make_svc);
    tokio::pin!(http_server);

    let driver_loop = run_driver_loop(
        UdpDriver::default(),
        SignalAdapter,
        udp_socket,
        udp_candidate_addr,
        command_rx,
    );
    tokio::pin!(driver_loop);

    tokio::select! {
        result = &mut http_server => result?,
        result = &mut driver_loop => result?,
        _ = tokio::signal::ctrl_c() => {
            println!("shutting down");
        }
    }

    Ok(())
}

async fn handle_http_request(
    req: Request<Body>,
    state: HttpState,
) -> std::result::Result<Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&Method::POST, "/offer") => handle_offer(req, state).await,
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap()),
    }
}

async fn handle_offer(
    req: Request<Body>,
    state: HttpState,
) -> std::result::Result<Response<Body>, hyper::Error> {
    let body = hyper::body::to_bytes(req.into_body()).await?;
    let offer = match serde_json::from_slice::<RTCSessionDescription>(&body) {
        Ok(offer) => offer,
        Err(err) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from(format!("invalid offer JSON: {err}")))
                .unwrap());
        }
    };

    let (response_tx, response_rx) = oneshot::channel();
    if state
        .command_tx
        .send(DriverCommand::AcceptOffer { offer, response_tx })
        .await
        .is_err()
    {
        return Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("driver loop unavailable"))
            .unwrap());
    }

    let answer = match response_rx.await {
        Ok(Ok(answer)) => answer,
        Ok(Err(err)) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(err))
                .unwrap());
        }
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("driver loop dropped response"))
                .unwrap());
        }
    };

    let payload = match serde_json::to_string(&answer) {
        Ok(payload) => payload,
        Err(err) => {
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!("failed to serialize answer: {err}")))
                .unwrap());
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap())
}

async fn run_driver_loop(
    mut driver: UdpDriver,
    signal_adapter: SignalAdapter,
    socket: UdpSocket,
    local_candidate_addr: SocketAddr,
    mut command_rx: mpsc::Receiver<DriverCommand>,
) -> std::result::Result<(), DynError> {
    let mut next_request_id = 1_u64;
    let mut buffer = vec![0_u8; 2000];
    let mut echo_state = EchoState::default();

    loop {
        while let Some(packet) = driver.poll_write() {
            socket
                .send_to(&packet.message, packet.transport.peer_addr)
                .await?;
        }

        while let Some(event) = driver.poll_event() {
            handle_peer_event(&mut driver, &mut echo_state, event)?;
        }

        while let Some(_sfu_event) = driver.poll_sfu_event() {}

        while let Some(message) = driver.poll_read() {
            echo_peer_read(&mut driver, &mut echo_state, message)?;
        }

        let timeout_at = driver
            .poll_timeout()
            .unwrap_or_else(|| Instant::now() + DEFAULT_TIMEOUT_DURATION);
        let delay = timeout_at.saturating_duration_since(Instant::now());
        if delay.is_zero() {
            driver.handle_timeout(Instant::now())?;
            continue;
        }

        let timer = tokio::time::sleep(delay);
        tokio::pin!(timer);

        tokio::select! {
            maybe_command = command_rx.recv() => {
                let Some(command) = maybe_command else {
                    break;
                };

                match command {
                    DriverCommand::AcceptOffer { offer, response_tx } => {
                        let response = ensure_echo_client(&mut driver, &mut echo_state, &offer)
                            .and_then(|_| {
                                signal_adapter.handle_offer(
                                    &mut driver,
                                    OfferRequest {
                                        request_id: next_request_id,
                                        room_id: ROOM_ID,
                                        client_id: CLIENT_ID,
                                        offer,
                                        local_addr: local_candidate_addr,
                                    },
                                )
                            })
                            .map(|response| response.answer)
                            .map_err(|err| err.to_string());
                        next_request_id += 1;
                        let _ = response_tx.send(response);
                    }
                }
            }
            _ = timer.as_mut() => {
                driver.handle_timeout(Instant::now())?;
            }
            recv_result = socket.recv_from(&mut buffer) => {
                let (packet_len, peer_addr) = recv_result?;
                if driver.core.client(CLIENT_ID).is_none() {
                    continue;
                }

                driver.handle_read(
                    CLIENT_ID,
                    tagged_udp_packet(local_candidate_addr, peer_addr, &buffer[..packet_len]),
                )?;
            }
        }
    }

    Ok(())
}

fn socket_addr_from_env(
    var_name: &str,
    default_value: &str,
) -> std::result::Result<SocketAddr, DynError> {
    let raw = env::var(var_name).unwrap_or_else(|_| default_value.to_owned());
    Ok(raw.parse()?)
}

fn advertised_udp_addr(local_addr: SocketAddr) -> std::result::Result<SocketAddr, DynError> {
    let candidate_addr = match env::var("SFU_UDP_CANDIDATE_ADDR") {
        Ok(raw) => raw.parse()?,
        Err(_) => local_addr,
    };

    if candidate_addr.ip().is_unspecified() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "SFU_UDP_CANDIDATE_ADDR must be set to a concrete IP when SFU_UDP_BIND_ADDR uses 0.0.0.0",
        )
        .into());
    }

    Ok(candidate_addr)
}

fn ensure_echo_client(
    driver: &mut UdpDriver,
    echo_state: &mut EchoState,
    offer: &RTCSessionDescription,
) -> RtcResult<()> {
    if driver.core.client(CLIENT_ID).is_some() {
        return Ok(());
    }

    driver.create_client(ROOM_ID, CLIENT_ID)?;
    let offered_kinds = offered_media_kinds(offer)?;
    let client = driver
        .core
        .client_mut(CLIENT_ID)
        .ok_or_else(|| Error::Other(format!("unknown client {CLIENT_ID}")))?;
    let pc = client
        .pc
        .as_mut()
        .ok_or_else(|| Error::Other(format!("client {CLIENT_ID} has no peer connection")))?;

    for kind in offered_kinds {
        let sender_id = pc.add_track(echo_track(kind))?;
        echo_state.sender_by_kind.insert(kind, sender_id);
    }

    Ok(())
}

fn offered_media_kinds(offer: &RTCSessionDescription) -> RtcResult<Vec<RtpCodecKind>> {
    let mut kinds = Vec::new();
    let parsed = offer.unmarshal()?;
    for media in parsed.media_descriptions {
        match media.media_name.media.as_str() {
            "audio" if !kinds.contains(&RtpCodecKind::Audio) => kinds.push(RtpCodecKind::Audio),
            "video" if !kinds.contains(&RtpCodecKind::Video) => kinds.push(RtpCodecKind::Video),
            _ => {}
        }
    }
    Ok(kinds)
}

fn echo_track(kind: RtpCodecKind) -> MediaStreamTrack {
    match kind {
        RtpCodecKind::Audio => MediaStreamTrack::new(
            "m2-echo-stream".to_owned(),
            "m2-echo-audio".to_owned(),
            "M2 Echo Audio".to_owned(),
            kind,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(0x1000_0001),
                    ..Default::default()
                },
                codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: 48_000,
                    channels: 2,
                    sdp_fmtp_line: String::new(),
                    rtcp_feedback: vec![],
                },
                ..Default::default()
            }],
        ),
        RtpCodecKind::Video => MediaStreamTrack::new(
            "m2-echo-stream".to_owned(),
            "m2-echo-video".to_owned(),
            "M2 Echo Video".to_owned(),
            kind,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(0x1000_0002),
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
        ),
        _ => MediaStreamTrack::default(),
    }
}

fn handle_peer_event(
    driver: &mut UdpDriver,
    echo_state: &mut EchoState,
    event: RTCPeerConnectionEvent,
) -> RtcResult<()> {
    match event {
        RTCPeerConnectionEvent::OnIceConnectionStateChangeEvent(state) => {
            println!("ICE connection state changed: {state}");
        }
        RTCPeerConnectionEvent::OnConnectionStateChangeEvent(state) => {
            println!("Peer connection state changed: {state}");
        }
        RTCPeerConnectionEvent::OnTrack(track_event) => match track_event {
            RTCTrackEvent::OnOpen(init) => {
                bind_track_echo_sender(driver, echo_state, &init)?;
                println!("Track event: {init:?}");
            }
            other => {
                println!("Track event: {other:?}");
            }
        },
        _ => {}
    }

    Ok(())
}

fn bind_track_echo_sender(
    driver: &mut UdpDriver,
    echo_state: &mut EchoState,
    init: &RTCTrackEventInit,
) -> RtcResult<()> {
    let client = driver
        .core
        .client_mut(CLIENT_ID)
        .ok_or_else(|| Error::Other(format!("unknown client {CLIENT_ID}")))?;
    let pc = client
        .pc
        .as_mut()
        .ok_or_else(|| Error::Other(format!("client {CLIENT_ID} has no peer connection")))?;
    let kind = pc
        .rtp_receiver_kind(init.receiver_id)
        .ok_or(Error::ErrRTPReceiverNotExisted)?;
    if let Some(sender_id) = echo_state.sender_by_kind.get(&kind).copied() {
        echo_state
            .sender_by_track_id
            .insert(init.track_id.to_string(), sender_id);
    }

    Ok(())
}

fn echo_peer_read(
    driver: &mut UdpDriver,
    echo_state: &mut EchoState,
    message: RTCMessage,
) -> RtcResult<()> {
    match message {
        RTCMessage::RtpPacket(track_id, mut packet) => {
            let Some(sender_id) = echo_state
                .sender_by_track_id
                .get(track_id.as_str())
                .copied()
            else {
                return Ok(());
            };

            let client = driver
                .core
                .client_mut(CLIENT_ID)
                .ok_or_else(|| Error::Other(format!("unknown client {CLIENT_ID}")))?;
            let pc = client.pc.as_mut().ok_or_else(|| {
                Error::Other(format!("client {CLIENT_ID} has no peer connection"))
            })?;
            let sender_ssrc = pc
                .rtp_sender_ssrc(sender_id)
                .ok_or(Error::ErrSenderWithNoSSRCs)?;
            packet.header.ssrc = sender_ssrc;
            pc.write_rtp(sender_id, packet)?;
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn offer_handler_returns_answer_json() {
        let (command_tx, mut command_rx) = mpsc::channel(1);
        let answer = test_answer();

        tokio::spawn(async move {
            let Some(DriverCommand::AcceptOffer { offer, response_tx }) = command_rx.recv().await
            else {
                panic!("expected offer command");
            };

            assert_eq!(offer.sdp_type.to_string(), "offer");
            let _ = response_tx.send(Ok(answer));
        });

        let response = handle_http_request(
            Request::builder()
                .method(Method::POST)
                .uri("/offer")
                .body(Body::from(serde_json::to_string(&test_offer()).unwrap()))
                .unwrap(),
            HttpState { command_tx },
        )
        .await
        .expect("HTTP handler should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body())
            .await
            .expect("response body should be readable");
        let answer: RTCSessionDescription =
            serde_json::from_slice(&body).expect("response should contain answer JSON");
        assert_eq!(answer.sdp_type.to_string(), "answer");
    }

    #[tokio::test]
    async fn offer_handler_rejects_invalid_json() {
        let (command_tx, _command_rx) = mpsc::channel(1);

        let response = handle_http_request(
            Request::builder()
                .method(Method::POST)
                .uri("/offer")
                .body(Body::from("{not-json"))
                .unwrap(),
            HttpState { command_tx },
        )
        .await
        .expect("HTTP handler should succeed");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn ensure_echo_client_adds_senders_for_offered_audio_and_video() {
        let mut driver = UdpDriver::default();
        let mut echo_state = EchoState::default();

        ensure_echo_client(&mut driver, &mut echo_state, &media_offer())
            .expect("echo client setup should succeed");

        assert!(driver.core.client(CLIENT_ID).is_some());
        assert_eq!(echo_state.sender_by_kind.len(), 2);
        assert!(echo_state.sender_by_kind.contains_key(&RtpCodecKind::Audio));
        assert!(echo_state.sender_by_kind.contains_key(&RtpCodecKind::Video));
    }

    fn test_offer() -> RTCSessionDescription {
        RTCSessionDescription::offer(
            "v=0\r\no=- 123456 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 127.0.0.1\r\na=mid:0\r\n".to_owned(),
        )
        .expect("offer SDP should be valid")
    }

    fn test_answer() -> RTCSessionDescription {
        RTCSessionDescription::answer(
            "v=0\r\no=- 654321 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 127.0.0.1\r\na=mid:0\r\n".to_owned(),
        )
        .expect("answer SDP should be valid")
    }

    fn media_offer() -> RTCSessionDescription {
        RTCSessionDescription::offer(
            concat!(
                "v=0\r\n",
                "o=- 123456 2 IN IP4 127.0.0.1\r\n",
                "s=-\r\n",
                "t=0 0\r\n",
                "a=group:BUNDLE 0 1\r\n",
                "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
                "c=IN IP4 0.0.0.0\r\n",
                "a=mid:0\r\n",
                "m=video 9 UDP/TLS/RTP/SAVPF 96\r\n",
                "c=IN IP4 0.0.0.0\r\n",
                "a=mid:1\r\n"
            )
            .to_owned(),
        )
        .expect("media offer SDP should be valid")
    }
}
