#![allow(dead_code)]

use bytes::BytesMut;
use log::{error, info, trace, warn};
use rand::random;
use rouille::{Request, Response, ResponseBody};
use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use sansio::Protocol;
use sfu::{ClientId, RequestId, RoomId, SFUEvent, Sfu};
use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read};
use std::net::UdpSocket;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// How long a `POST /offer` will wait for the SFU's answer before giving up.
const OFFER_ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// Messages the web (rouille) threads hand to a media port's run loop.
pub enum Command {
    /// A request/response signaling message (`/join`, `/offer`, `/answer`, `/leave`).
    Signal(SignalingMessage),
    /// A browser subscribing to server-initiated push (the `GET /events/...` SSE stream).
    /// The run loop keeps `event_tx` and pushes server-initiated SDP (e.g. subscribe
    /// re-offers) to it; the SSE handler streams whatever arrives to the browser.
    Subscribe {
        room_id: RoomId,
        client_id: ClientId,
        event_tx: SyncSender<String>,
    },
}

pub struct SignalingMessage {
    pub request: SFUEvent,
    pub response_tx: SyncSender<SFUEvent>,
}

/// Streaming body for a Server-Sent Events response. Blocks on the per-client
/// receiver and emits each payload as an SSE `data:` frame, holding the HTTP
/// connection open. Returns EOF once the run loop drops the sender (client left).
struct SseReader {
    rx: Receiver<String>,
    pending: VecDeque<u8>,
}

impl SseReader {
    fn new(rx: Receiver<String>) -> Self {
        // A comment keeps EventSource reconnects from looking like errors, and nudges
        // the browser's retry interval.
        let mut pending = VecDeque::new();
        pending.extend(b": connected\nretry: 2000\n\n");
        Self { rx, pending }
    }
}

impl Read for SseReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            match self.rx.recv() {
                // SSE frame: "data: <json>\n\n"
                Ok(data) => self
                    .pending
                    .extend(format!("data: {}\n\n", data).into_bytes()),
                // Sender dropped (client left / run loop gone) => end the stream.
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len());
        for slot in buf.iter_mut().take(n) {
            *slot = self.pending.pop_front().unwrap();
        }
        Ok(n)
    }
}

/// Pick the media port (run loop) that owns a given room.
fn port_for_room(map: &HashMap<u16, SyncSender<Command>>, room_id: RoomId) -> Option<u16> {
    let mut ports: Vec<u16> = map.keys().copied().collect();
    ports.sort();
    if ports.is_empty() {
        return None;
    }
    Some(ports[(room_id as usize) % ports.len()])
}

// Handle a web request.
pub fn web_request(
    request: &Request,
    media_port_thread_map: Arc<HashMap<u16, SyncSender<Command>>>,
) -> Response {
    // "/offer/433774451/456773342", "/leave/433774451/456773342", "/events/.../..."
    let path: Vec<String> = request.url().split('/').map(|s| s.to_owned()).collect();

    if request.method() == "GET" {
        // GET /events/{room}/{client} => open a Server-Sent Events stream for
        // server-initiated push (subscribe re-offers). Any other GET serves the page.
        if path.len() == 4 && path[1] == "events" {
            return match (path[2].parse::<RoomId>(), path[3].parse::<ClientId>()) {
                (Ok(room_id), Ok(client_id)) => {
                    subscribe_events(room_id, client_id, &media_port_thread_map)
                }
                _ => Response::empty_400(),
            };
        }
        return Response::html(include_str!("../chat.html"));
    }

    if path.len() != 4 || path[2].parse::<u64>().is_err() || path[3].parse::<u64>().is_err() {
        return Response::empty_400();
    }

    let room_id = path[2].parse::<RoomId>().unwrap();
    let client_id = path[3].parse::<ClientId>().unwrap();

    let Some(port) = port_for_room(&media_port_thread_map, room_id) else {
        return Response::empty_406();
    };
    let Some(tx) = media_port_thread_map.get(&port) else {
        return Response::empty_406();
    };

    // Read the POST body. Only `/offer` and `/answer` carry an SDP; `/join` and `/leave`
    // have none, so an empty/absent body is fine here and is rejected later only if the
    // handler actually needs it.
    let mut sdp = vec![];
    if let Some(mut data) = request.data()
        && let Err(err) = data.read_to_end(&mut sdp)
    {
        warn!("{}: failed to read request body: {}", path[1], err);
        return Response::text("failed to read body").with_status_code(400);
    }

    match path[1].as_str() {
        "join" => {
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            if let Err(resp) = send_signal(
                tx,
                SFUEvent::Join {
                    request_id: random(),
                    room_id,
                    client_id,
                },
                response_tx,
            ) {
                return resp;
            }
            recv_ack(&response_rx, "join")
        }
        "offer" => {
            let sdp = match parse_sdp("offer", sdp) {
                Ok(sdp) => sdp,
                Err(resp) => return resp,
            };
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            if let Err(resp) = send_signal(
                tx,
                SFUEvent::SessionDescription {
                    request_id: random(),
                    room_id,
                    client_id,
                    sdp,
                },
                response_tx,
            ) {
                return resp;
            }
            // The answer is produced by the SFU and delivered asynchronously by the
            // run loop (see `poll_event`), so wait for it here.
            match response_rx.recv_timeout(OFFER_ANSWER_TIMEOUT) {
                Ok(SFUEvent::SessionDescription { sdp, .. })
                    if sdp.sdp_type == RTCSdpType::Answer =>
                {
                    match serde_json::to_string(&sdp) {
                        Ok(json) => Response::from_data("application/json", json),
                        Err(err) => {
                            error!("offer: failed to serialize answer: {}", err);
                            Response::text("failed to serialize answer").with_status_code(500)
                        }
                    }
                }
                Ok(SFUEvent::Err { reason, .. }) => {
                    warn!("offer for {}/{} rejected: {}", room_id, client_id, reason);
                    Response::text(reason).with_status_code(422)
                }
                Ok(other) => {
                    warn!("offer for {}/{} got unexpected {:?}", room_id, client_id, other);
                    Response::empty_404()
                }
                Err(err) => {
                    error!("offer for {}/{} timed out: {}", room_id, client_id, err);
                    Response::text("timeout waiting for answer").with_status_code(504)
                }
            }
        }
        "answer" => {
            let sdp = match parse_sdp("answer", sdp) {
                Ok(sdp) => sdp,
                Err(resp) => return resp,
            };
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            if let Err(resp) = send_signal(
                tx,
                SFUEvent::SessionDescription {
                    request_id: random(),
                    room_id,
                    client_id,
                    sdp,
                },
                response_tx,
            ) {
                return resp;
            }
            recv_ack(&response_rx, "answer")
        }
        "leave" => {
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            if let Err(resp) = send_signal(
                tx,
                SFUEvent::Leave {
                    request_id: random(),
                    room_id,
                    client_id,
                    reason: "user left".to_string(),
                },
                response_tx,
            ) {
                return resp;
            }
            recv_ack(&response_rx, "leave")
        }
        _ => Response::empty_404(),
    }
}

fn subscribe_events(
    room_id: RoomId,
    client_id: ClientId,
    media_port_thread_map: &HashMap<u16, SyncSender<Command>>,
) -> Response {
    let Some(port) = port_for_room(media_port_thread_map, room_id) else {
        return Response::empty_406();
    };
    let Some(tx) = media_port_thread_map.get(&port) else {
        return Response::empty_406();
    };

    let (event_tx, event_rx) = mpsc::sync_channel::<String>(16);
    if tx
        .send(Command::Subscribe {
            room_id,
            client_id,
            event_tx,
        })
        .is_err()
    {
        return Response::empty_406();
    }

    Response {
        status_code: 200,
        headers: vec![
            ("Content-Type".into(), "text/event-stream".into()),
            ("Cache-Control".into(), "no-cache".into()),
        ],
        data: ResponseBody::from_reader(SseReader::new(event_rx)),
        upgrade: None,
    }
}

/// Hand a signaling command to the media run loop. Returns a `503` response (instead of
/// panicking) if that run loop is gone, so a dead media thread surfaces as an HTTP error.
fn send_signal(
    tx: &SyncSender<Command>,
    request: SFUEvent,
    response_tx: SyncSender<SFUEvent>,
) -> Result<(), Response> {
    tx.send(Command::Signal(SignalingMessage {
        request,
        response_tx,
    }))
    .map_err(|err| {
        error!("signaling run loop unavailable: {}", err);
        Response::text("signaling unavailable").with_status_code(503)
    })
}

/// Parse a POSTed SDP body into an `RTCSessionDescription`, logging and returning a `400`
/// (with the reason) on bad UTF-8 or bad JSON — so a malformed `/offer` or `/answer`
/// surfaces as a diagnosable error instead of an opaque `500` panic.
fn parse_sdp(kind: &str, body: Vec<u8>) -> Result<RTCSessionDescription, Response> {
    let text = String::from_utf8(body).map_err(|err| {
        warn!("{}: request body is not valid UTF-8: {}", kind, err);
        Response::text("body is not valid UTF-8").with_status_code(400)
    })?;

    serde_json::from_str::<RTCSessionDescription>(&text).map_err(|err| {
        warn!("{}: failed to parse SDP JSON: {}; body was:\n{}", kind, err, text);
        Response::text(format!("invalid SDP JSON: {err}")).with_status_code(400)
    })
}

/// Wait for the SFU's `Ok`/`Err` reply to an applied answer (or any request that only
/// needs an ack), turning it into an HTTP response and never blocking forever.
fn recv_ack(response_rx: &Receiver<SFUEvent>, kind: &str) -> Response {
    match response_rx.recv_timeout(OFFER_ANSWER_TIMEOUT) {
        Ok(SFUEvent::Ok { .. }) => ok_200(),
        Ok(SFUEvent::Err { reason, .. }) => {
            warn!("{}: SFU rejected request: {}", kind, reason);
            Response::text(reason).with_status_code(422)
        }
        Ok(other) => {
            warn!("{}: unexpected SFU reply {:?}", kind, other);
            Response::empty_404()
        }
        Err(err) => {
            error!("{}: timed out waiting for SFU reply: {}", kind, err);
            Response::text("timeout waiting for SFU").with_status_code(504)
        }
    }
}

fn ok_200() -> Response {
    Response {
        status_code: 200,
        headers: vec![],
        data: ResponseBody::empty(),
        upgrade: None,
    }
}

/// Log one signaling SDP exchange: a concise summary at `info!` and the full SDP at
/// `trace!`. `label` names the exchange (Offer / Answer / Re-Offer / Re-Answer) and
/// `direction` its flow (`browser->SFU` or `SFU->browser`).
fn log_sdp(
    label: &str,
    direction: &str,
    room_id: RoomId,
    client_id: ClientId,
    request_id: RequestId,
    sdp: &RTCSessionDescription,
) {
    trace!(
        "{label:<9} {direction:<11} room={room_id} client={client_id} req={request_id} SDP {}:\n{}",
        sdp.sdp_type, sdp.sdp
    );
}

/// This is the "main run loop" that handles all clients, reads and writes UdpSocket traffic,
/// and forwards media data between clients.
pub fn run(
    stop_rx: crossbeam_channel::Receiver<()>,
    socket: UdpSocket,
    rx: Receiver<Command>,
) -> anyhow::Result<()> {
    println!("listening {}...", socket.local_addr()?);

    let mut sfu = Sfu::new(random(), socket.local_addr()?);

    // Per-client SSE push channels (server -> browser), and the answer channels of
    // in-flight `POST /offer` requests awaiting the SFU's answer.
    let mut subscribers: HashMap<ClientId, SyncSender<String>> = HashMap::new();
    let mut pending_offers: HashMap<RequestId, SyncSender<SFUEvent>> = HashMap::new();

    let mut buf = vec![0; 2000];
    loop {
        match stop_rx.try_recv() {
            Ok(_) => break,
            Err(err) => {
                if err.is_disconnected() {
                    break;
                }
            }
        };

        while let Some(transmit) = sfu.poll_write() {
            socket.send_to(&transmit.message, transmit.transport.peer_addr)?;
        }

        // Spawn new incoming command from the signaling server thread.
        if let Ok(command) = rx.try_recv() {
            if let Err(err) =
                handle_command(&mut sfu, command, &mut pending_offers, &mut subscribers)
            {
                error!("handle_command got error:{}", err);
                continue;
            }
        }

        // Drain SFU-emitted events: answers -> the pending POST /offer; server-initiated
        // offers -> the target client's SSE stream.
        poll_event(&mut sfu, &mut pending_offers, &mut subscribers);

        while let Some(msg) = sfu.poll_read() {
            info!("process sfu's poll_read {:?}, should always be None", msg);
        }

        // Poll clients until they return timeout
        let eto = sfu
            .poll_timeout()
            .unwrap_or_else(|| Instant::now() + Duration::from_millis(100)); //TODO: DEFAULT_TIMEOUT_DURATION
        let delay_from_now = eto
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::from_secs(0));
        if delay_from_now.is_zero() {
            sfu.handle_timeout(Instant::now())?;
            continue;
        }

        socket
            .set_read_timeout(Some(delay_from_now))
            .expect("setting socket read timeout");
        if let Some(packet) = match socket.recv_from(&mut buf) {
            Ok((n, peer_addr)) => Some(TaggedBytesMut {
                now: Instant::now(),
                transport: TransportContext {
                    local_addr: socket.local_addr().unwrap(),
                    peer_addr,
                    transport_protocol: TransportProtocol::UDP,
                    ecn: None,
                },
                message: BytesMut::from(&buf[..n]),
            }),

            Err(e) => match e.kind() {
                // Expected error for set_read_timeout(). One for windows, one for the rest.
                ErrorKind::WouldBlock | ErrorKind::TimedOut => None,
                _ => panic!("UdpSocket read failed: {e:?}"),
            },
        } {
            sfu.handle_read(packet)?;
        }

        // Drive time forward in all clients.
        sfu.handle_timeout(Instant::now())?;
    }

    println!(
        "media server on {} is gracefully down",
        socket.local_addr()?
    );

    Ok(())
}

/// Drain every event the SFU has produced and route it:
/// - a `SessionDescription` **answer** completes an in-flight `POST /offer`;
/// - a `SessionDescription` **offer** is server-initiated (e.g. a subscribe re-offer)
///   and is pushed to that client's SSE stream, so the browser can answer it via
///   `POST /answer` — no data channel involved.
fn poll_event(
    sfu: &mut Sfu,
    pending_offers: &mut HashMap<RequestId, SyncSender<SFUEvent>>,
    subscribers: &mut HashMap<ClientId, SyncSender<String>>,
) {
    while let Some(evt) = sfu.poll_event() {
        match evt {
            SFUEvent::SessionDescription {
                request_id,
                room_id,
                client_id,
                sdp,
            } => {
                if sdp.sdp_type == RTCSdpType::Answer {
                    // The SFU's Answer to a browser Offer, returned over the pending
                    // `POST /offer` HTTP response.
                    log_sdp(
                        "Answer",
                        "SFU->browser",
                        room_id,
                        client_id,
                        request_id,
                        &sdp,
                    );
                    match pending_offers.remove(&request_id) {
                        Some(response_tx) => {
                            let _ = response_tx.send(SFUEvent::SessionDescription {
                                request_id,
                                room_id,
                                client_id,
                                sdp,
                            });
                        }
                        None => warn!("no pending POST /offer for answer request {}", request_id),
                    }
                } else {
                    // Server-initiated subscribe Re-Offer -> push to the client's SSE
                    // stream; the browser answers it via `POST /answer` (the Re-Answer).
                    log_sdp(
                        "Re-Offer",
                        "SFU->browser",
                        room_id,
                        client_id,
                        request_id,
                        &sdp,
                    );
                    push_to_subscriber(subscribers, client_id, &sdp);
                }
            }
            // The SFU does not yet emit trickle ICE toward signaling; when it does,
            // route Event::IceCandidate here to `push_to_subscriber` as well.
            other => warn!("run loop dropped unroutable SFU event {:?}", other),
        }
    }
}

fn push_to_subscriber(
    subscribers: &mut HashMap<ClientId, SyncSender<String>>,
    client_id: ClientId,
    sdp: &RTCSessionDescription,
) {
    let Some(tx) = subscribers.get(&client_id) else {
        warn!(
            "server-initiated offer for client {} but it has no SSE stream",
            client_id
        );
        return;
    };
    match serde_json::to_string(sdp) {
        Ok(payload) => {
            if tx.send(payload).is_err() {
                // Browser closed the SSE stream.
                warn!(
                    "SSE stream for client {} closed; dropping Re-Offer",
                    client_id
                );
                subscribers.remove(&client_id);
            } else {
                trace!("Re-Offer pushed to SSE stream for client {}", client_id);
            }
        }
        Err(err) => error!("failed to serialize server-initiated offer: {}", err),
    }
}

fn handle_command(
    sfu: &mut Sfu,
    command: Command,
    pending_offers: &mut HashMap<RequestId, SyncSender<SFUEvent>>,
    subscribers: &mut HashMap<ClientId, SyncSender<String>>,
) -> anyhow::Result<()> {
    match command {
        Command::Subscribe {
            room_id: _,
            client_id,
            event_tx,
        } => {
            // Replace any stale stream (EventSource auto-reconnect).
            subscribers.insert(client_id, event_tx);
            Ok(())
        }
        Command::Signal(msg) => {
            // Tear down a client's SSE stream when it leaves.
            let leaving = match &msg.request {
                SFUEvent::Leave { client_id, .. } => Some(*client_id),
                _ => None,
            };
            let result = handle_signaling_message(sfu, msg, pending_offers);
            if let Some(client_id) = leaving {
                subscribers.remove(&client_id);
            }
            result
        }
    }
}

pub fn handle_signaling_message(
    sfu: &mut Sfu,
    signaling_msg: SignalingMessage,
    pending_offers: &mut HashMap<RequestId, SyncSender<SFUEvent>>,
) -> anyhow::Result<()> {
    match signaling_msg.request {
        SFUEvent::Join {
            request_id,
            room_id,
            client_id,
        } => handle_join_message(
            sfu,
            request_id,
            room_id,
            client_id,
            signaling_msg.response_tx,
        ),
        SFUEvent::SessionDescription {
            request_id,
            room_id,
            client_id,
            sdp,
        } => handle_session_description(
            sfu,
            request_id,
            room_id,
            client_id,
            sdp,
            signaling_msg.response_tx,
            pending_offers,
        ),
        SFUEvent::Leave {
            request_id,
            room_id,
            client_id,
            reason,
        } => handle_leave_message(
            sfu,
            request_id,
            room_id,
            client_id,
            reason,
            signaling_msg.response_tx,
        ),
        SFUEvent::IceCandidate {
            request_id,
            room_id,
            client_id,
            candidate: _,
        } => Ok(signaling_msg.response_tx.send(SFUEvent::Err {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
            reason: "Unsupported Request yet".to_string(),
        })?),
        SFUEvent::Ok {
            request_id,
            room_id,
            client_id,
            ..
        }
        | SFUEvent::Err {
            request_id,
            room_id,
            client_id,
            reason: _,
            ..
        } => Ok(signaling_msg.response_tx.send(SFUEvent::Err {
            request_id,
            room_id,
            client_id,
            reason: "Invalid Request".to_string(),
        })?),
    }
}

fn handle_join_message(
    sfu: &mut Sfu,
    request_id: RequestId,
    room_id: RoomId,
    client_id: ClientId,
    response_tx: SyncSender<SFUEvent>,
) -> anyhow::Result<()> {
    info!("Join      browser->SFU room={room_id} client={client_id} req={request_id}");

    let mut try_handle = || -> anyhow::Result<()> {
        Ok(sfu.handle_event(SFUEvent::Join {
            request_id,
            room_id,
            client_id,
        })?)
    };

    match try_handle() {
        Ok(_) => Ok(response_tx.send(SFUEvent::Ok {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
        })?),
        Err(err) => Ok(response_tx.send(SFUEvent::Err {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
            reason: err.to_string(),
        })?),
    }
}

/// Feed an incoming SDP into the SFU.
///
/// - **Offer** (`POST /offer`): the SFU answers asynchronously. We stash `response_tx`
///   in `pending_offers` keyed by `request_id`; the run loop delivers the answer once
///   `poll_event` yields it (see `drain_events`).
/// - **Answer** (`POST /answer`, in reply to a server-initiated re-offer): the SFU only
///   applies it, so we reply `Ok` immediately.
fn handle_session_description(
    sfu: &mut Sfu,
    request_id: RequestId,
    room_id: RoomId,
    client_id: ClientId,
    sdp: RTCSessionDescription,
    response_tx: SyncSender<SFUEvent>,
    pending_offers: &mut HashMap<RequestId, SyncSender<SFUEvent>>,
) -> anyhow::Result<()> {
    let is_offer = sdp.sdp_type == RTCSdpType::Offer;
    // Offer (POST /offer): the browser publishes or renegotiates its send tracks.
    // Re-Answer (POST /answer): the browser answers a server-initiated subscribe Re-Offer.
    let label = if is_offer { "Offer" } else { "Re-Answer" };
    log_sdp(label, "browser->SFU", room_id, client_id, request_id, &sdp);

    match sfu.handle_event(SFUEvent::SessionDescription {
        request_id,
        room_id,
        client_id,
        sdp,
    }) {
        Ok(_) => {
            if is_offer {
                // Answer delivered later by drain_events, correlated by request_id.
                pending_offers.insert(request_id, response_tx);
                Ok(())
            } else {
                Ok(response_tx.send(SFUEvent::Ok {
                    request_id,
                    room_id: Some(room_id),
                    client_id: Some(client_id),
                })?)
            }
        }
        Err(err) => Ok(response_tx.send(SFUEvent::Err {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
            reason: err.to_string(),
        })?),
    }
}

fn handle_leave_message(
    sfu: &mut Sfu,
    request_id: RequestId,
    room_id: RoomId,
    client_id: ClientId,
    reason: String,
    response_tx: SyncSender<SFUEvent>,
) -> anyhow::Result<()> {
    info!(
        "Leave     browser->SFU room={room_id} client={client_id} req={request_id} reason={reason:?}"
    );

    let try_handle = || -> anyhow::Result<()> {
        Ok(sfu.handle_event(SFUEvent::Leave {
            request_id,
            room_id,
            client_id,
            reason,
        })?)
    };

    match try_handle() {
        Ok(_) => Ok(response_tx.send(SFUEvent::Ok {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
        })?),
        Err(err) => Ok(response_tx.send(SFUEvent::Err {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
            reason: err.to_string(),
        })?),
    }
}
