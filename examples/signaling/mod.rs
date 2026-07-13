#![allow(dead_code)]

//! WebSocket signaling for the SFU chat example.
//!
//! Borrowed from AppRTC's Collider: each browser opens **one** WebSocket and, over that
//! single full-duplex channel, `register`s `{room, client}` then exchanges SDP. The SFU
//! answers and pushes server-initiated subscribe re-offers back on the *same* socket —
//! so, unlike the old SSE + `POST` transport, server→browser frames flush immediately
//! (no buffering stall) and there is no request/response correlation to manage.
//!
//! We own the TCP + TLS listener directly (no rouille): a small hand-rolled HTTP layer
//! serves `GET /` (the page) and upgrades `/ws` to a `tungstenite` WebSocket.

use bytes::BytesMut;
use log::{error, info, trace, warn};
use rand::random;
use rtc::peer_connection::sdp::{RTCSdpType, RTCSessionDescription};
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use sansio::Protocol;
use sfu::{ClientId, RequestId, RoomId, SFUEvent, Sfu};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

/// A WebSocket `read()` blocks at most this long, so the connection thread can interleave
/// server→browser writes (a pushed re-offer reaches the browser within one interval).
const WS_READ_TIMEOUT: Duration = Duration::from_millis(50);

/// Commands a client's WebSocket thread hands to a media port's run loop.
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// The client registered on its socket: bind its server→browser channel and `Join`.
    Register {
        room_id: RoomId,
        client_id: ClientId,
        out_tx: SyncSender<String>,
    },
    /// A signaling event from the client's socket (offer / answer / leave).
    Signal(SFUEvent),
}

/// JSON envelope the browser sends over the WebSocket (AppRTC/Collider style):
/// `{cmd:"register", roomid, clientid}` once, then `{cmd:"offer"|"answer", sdp,
/// request_id?}` or `{cmd:"leave"}`. `request_id` is required only when answering an
/// SFU-initiated re-offer so the SFU can correlate that answer to its outstanding
/// transaction.
#[derive(serde::Deserialize)]
struct WsClientMsg {
    cmd: String,
    #[serde(default)]
    roomid: Option<RoomId>,
    #[serde(default)]
    clientid: Option<ClientId>,
    #[serde(default)]
    sdp: Option<RTCSessionDescription>,
    #[serde(default)]
    request_id: Option<RequestId>,
}

#[derive(serde::Serialize)]
struct WsServerSdp<'a> {
    #[serde(flatten)]
    sdp: &'a RTCSessionDescription,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<RequestId>,
}

/// What a WebSocket binds to once it has `register`ed: its room/client and the run loop
/// that owns that room.
type Bound = (RoomId, ClientId, SyncSender<Command>);

/// A blocking rustls TLS stream over a `TcpStream` (the underlying socket is reachable via
/// `.sock` for the WebSocket read timeout).
type Tls = StreamOwned<ServerConnection, TcpStream>;

// ───────────────────────────── TLS + HTTP + WebSocket ──────────────────────────────

/// Accept TLS connections until `stop_rx` fires; serve `GET /` and upgrade `/ws`.
pub fn serve(
    stop_rx: crossbeam_channel::Receiver<()>,
    listener: TcpListener,
    tls_config: Arc<ServerConfig>,
    media_port_thread_map: Arc<HashMap<u16, SyncSender<Command>>>,
    web_root: Arc<Option<String>>,
) {
    listener
        .set_nonblocking(true)
        .expect("set signaling listener non-blocking");

    loop {
        // Stop on an explicit signal *or* when the sender drops (the single `()` is
        // consumed by one receiver, so everyone else sees disconnect on shutdown).
        match stop_rx.try_recv() {
            Ok(_) => break,
            Err(err) if err.is_disconnected() => break,
            Err(_) => {}
        }
        match listener.accept() {
            Ok((tcp, _peer)) => {
                let tls_config = tls_config.clone();
                let media_port_thread_map = media_port_thread_map.clone();
                let web_root = web_root.clone();
                std::thread::spawn(move || {
                    if let Err(err) =
                        handle_connection(tcp, tls_config, &media_port_thread_map, &web_root)
                    {
                        trace!("connection ended: {}", err);
                    }
                });
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => warn!("accept error: {}", err),
        }
    }
    info!("signaling server stopped");
}

/// One accepted connection: TLS handshake, read the HTTP request head, then either serve
/// the page or upgrade to a WebSocket.
fn handle_connection(
    tcp: TcpStream,
    tls_config: Arc<ServerConfig>,
    media_port_thread_map: &HashMap<u16, SyncSender<Command>>,
    web_root: &Option<String>,
) -> anyhow::Result<()> {
    // The accepted stream can inherit the listener's non-blocking flag; the TLS handshake
    // and request-head read need blocking I/O (the per-frame read timeout is set later,
    // only for the WebSocket poll loop).
    tcp.set_nonblocking(false)?;
    // rustls performs the handshake lazily on the first read/write.
    let conn = ServerConnection::new(tls_config)?;
    let mut tls: Tls = StreamOwned::new(conn, tcp);
    let head = read_http_head(&mut tls)?;
    let request = HttpRequest::parse(&head);

    let is_ws = request
        .header("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && request.header("sec-websocket-key").is_some();

    if is_ws {
        // Complete the RFC 6455 handshake ourselves (tiny_http's opaque upgrade stream
        // can't expose a read timeout), then hand the raw stream to tungstenite.
        let key = request.header("sec-websocket-key").unwrap();
        let accept = derive_accept_key(key.as_bytes());
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        tls.write_all(response.as_bytes())?;
        tls.flush()?;

        tls.sock.set_read_timeout(Some(WS_READ_TIMEOUT))?;
        let ws = WebSocket::from_raw_socket(tls, Role::Server, None);
        ws_session(ws, media_port_thread_map);
    } else {
        serve_static(&mut tls, &request.path, web_root)?;
    }
    Ok(())
}

/// Read bytes until the end of the HTTP request head (`\r\n\r\n`). WebSocket clients send
/// nothing after the head until they receive the `101`, and `GET /` has no body, so this
/// consumes exactly the head.
fn read_http_head(tls: &mut Tls) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = tls.read(&mut chunk)?;
        if n == 0 {
            anyhow::bail!("connection closed before request head");
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        anyhow::ensure!(buf.len() <= 64 * 1024, "request head too large");
    }
    Ok(buf)
}

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl HttpRequest {
    fn parse(head: &[u8]) -> Self {
        let text = String::from_utf8_lossy(head);
        let mut lines = text.split("\r\n");
        let mut request_line = lines.next().unwrap_or("").split_whitespace();
        let method = request_line.next().unwrap_or("").to_owned();
        let path = request_line.next().unwrap_or("/").to_owned();
        let headers = lines
            .take_while(|line| !line.is_empty())
            .filter_map(|line| {
                let (key, value) = line.split_once(':')?;
                Some((key.trim().to_owned(), value.trim().to_owned()))
            })
            .collect();
        HttpRequest {
            method,
            path,
            headers,
        }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Serve the chat page for `GET /`, `404` otherwise.
fn serve_static(tls: &mut Tls, path: &str, web_root: &Option<String>) -> anyhow::Result<()> {
    let (status, content_type, body) = if path == "/" {
        let content = if let Some(path) = web_root {
            std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path, e))?
        } else {
            include_str!("../chat.html").to_string()
        };
        ("200 OK", "text/html; charset=utf-8", content)
    } else {
        (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found".to_string(),
        )
    };
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    tls.write_all(response.as_bytes())?;
    tls.flush()?;
    Ok(())
}

/// Drive one client's WebSocket for its lifetime: read client frames → run loop, and
/// drain server→browser pushes → client.
fn ws_session(mut ws: WebSocket<Tls>, media_port_thread_map: &HashMap<u16, SyncSender<Command>>) {
    // Server→browser channel; its sender is handed to the run loop on `register`.
    let (out_tx, out_rx) = std::sync::mpsc::sync_channel::<String>(64);
    let mut bound: Option<Bound> = None;

    loop {
        // 1. Read one client frame (blocks up to WS_READ_TIMEOUT).
        match ws.read() {
            Ok(Message::Text(text)) => {
                handle_ws_text(text.as_str(), &mut bound, &out_tx, media_port_thread_map)
            }
            Ok(Message::Ping(payload)) => {
                let _ = ws.send(Message::Pong(payload));
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(err))
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => {}
            Err(err) => {
                trace!("ws read ended: {}", err);
                break;
            }
        }

        // 2. Flush server→browser pushes (answers and subscribe re-offers).
        loop {
            match out_rx.try_recv() {
                Ok(payload) => {
                    if let Some((room_id, client_id, _)) = &bound {
                        trace!(
                            "ws->browser room={} client={} pushing {} bytes",
                            room_id,
                            client_id,
                            payload.len()
                        );
                    }
                    if let Err(err) = ws.send(Message::text(payload)) {
                        trace!("ws send failed: {}", err);
                        teardown(&bound);
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    teardown(&bound);
}

/// Dispatch one client `{cmd:...}` message.
fn handle_ws_text(
    text: &str,
    bound: &mut Option<Bound>,
    out_tx: &SyncSender<String>,
    media_port_thread_map: &HashMap<u16, SyncSender<Command>>,
) {
    let msg: WsClientMsg = match serde_json::from_str(text) {
        Ok(msg) => msg,
        Err(err) => {
            warn!("bad ws message: {}; text was:\n{}", err, text);
            return;
        }
    };

    match msg.cmd.as_str() {
        "register" => {
            let (Some(room_id), Some(client_id)) = (msg.roomid, msg.clientid) else {
                warn!("register missing roomid/clientid");
                return;
            };
            if bound.is_some() {
                warn!("duplicate register for {}/{}", room_id, client_id);
                return;
            }
            let Some(tx) = port_tx_for_room(media_port_thread_map, room_id) else {
                warn!("no media port for room {}", room_id);
                return;
            };
            info!(
                "Register  browser->SFU room={} client={}",
                room_id, client_id
            );
            if tx
                .send(Command::Register {
                    room_id,
                    client_id,
                    out_tx: out_tx.clone(),
                })
                .is_err()
            {
                error!(
                    "run loop unavailable for register {}/{}",
                    room_id, client_id
                );
                return;
            }
            *bound = Some((room_id, client_id, tx));
        }
        "offer" | "answer" => {
            let Some((room_id, client_id, tx)) = bound.as_ref() else {
                warn!("{} before register", msg.cmd);
                return;
            };
            let Some(sdp) = msg.sdp else {
                warn!("{} missing sdp", msg.cmd);
                return;
            };
            // Offer: browser publishes/renegotiates. Answer (Re-Answer): browser answers a
            // server-initiated subscribe re-offer.
            let label = if msg.cmd == "offer" {
                "Offer"
            } else {
                "Re-Answer"
            };
            log_sdp(label, "browser->SFU", *room_id, *client_id, &sdp);
            let _ = tx.send(Command::Signal(SFUEvent::SessionDescription {
                request_id: msg.request_id.unwrap_or_else(random),
                room_id: *room_id,
                client_id: *client_id,
                sdp,
            }));
        }
        "leave" => {
            if let Some((room_id, client_id, tx)) = bound.as_ref() {
                info!(
                    "Leave     browser->SFU room={} client={}",
                    room_id, client_id
                );
                let _ = tx.send(Command::Signal(SFUEvent::Leave {
                    request_id: random(),
                    room_id: *room_id,
                    client_id: *client_id,
                    reason: "user left".to_owned(),
                }));
            }
            // The client is now gone; drop the binding so the imminent socket close (the browser
            // closes the ws right after sending `leave`) doesn't fire a duplicate `Leave` via
            // `teardown`. Ungraceful disconnects leave `bound` set, so teardown still covers them.
            *bound = None;
        }
        other => warn!("unknown ws cmd: {}", other),
    }
}

/// A closed socket means the client is gone: tell the run loop to `Leave` it.
fn teardown(bound: &Option<Bound>) {
    if let Some((room_id, client_id, tx)) = bound {
        info!(
            "Leave     browser->SFU room={} client={} (ws closed)",
            room_id, client_id
        );
        let _ = tx.send(Command::Signal(SFUEvent::Leave {
            request_id: random(),
            room_id: *room_id,
            client_id: *client_id,
            reason: "ws closed".to_owned(),
        }));
    }
}

/// The run loop that owns the room, keyed by room id (mirrors the old `port_for_room`).
fn port_tx_for_room(
    map: &HashMap<u16, SyncSender<Command>>,
    room_id: RoomId,
) -> Option<SyncSender<Command>> {
    let mut ports: Vec<u16> = map.keys().copied().collect();
    ports.sort();
    let port = *ports.get((room_id as usize) % ports.len().max(1))?;
    map.get(&port).cloned()
}

// ───────────────────────────────── media run loop ──────────────────────────────────

/// The media run loop for one UDP port: drives the SFU, forwards RTP/RTCP, and pushes
/// SFU-emitted SDP (answers and subscribe re-offers) back to each client's WebSocket.
pub fn run(
    stop_rx: crossbeam_channel::Receiver<()>,
    socket: UdpSocket,
    rx: Receiver<Command>,
) -> anyhow::Result<()> {
    let local_addr = socket.local_addr()?;
    println!("listening {}...", local_addr);

    let mut sfu = Sfu::new(random(), local_addr);

    // Per-client server→browser channels (the WebSocket writers).
    let mut subscribers: HashMap<(RoomId, ClientId), SyncSender<String>> = HashMap::new();

    let mut buf = vec![0; 2000];
    loop {
        match stop_rx.try_recv() {
            Ok(_) => break,
            Err(err) if err.is_disconnected() => break,
            Err(_) => {}
        }

        while let Some(transmit) = sfu.poll_write() {
            if let Err(err) = socket.send_to(&transmit.message, transmit.transport.peer_addr) {
                warn!("failed to send message to socket with error: {}", err);
            }
        }

        if let Ok(command) = rx.try_recv() {
            handle_command(&mut sfu, command, &mut subscribers);
        }

        // Drain SFU-emitted SDP: every answer and subscribe re-offer is pushed to the
        // target client's WebSocket (no request/response correlation with WS transport).
        drain_sfu_events(&mut sfu, &mut subscribers);

        while let Some(msg) = sfu.poll_read() {
            info!("process sfu's poll_read {:?}, should always be None", msg);
        }

        let eto = sfu
            .poll_timeout()
            .unwrap_or_else(|| Instant::now() + Duration::from_millis(100));
        // Cap the socket wait so incoming WebSocket commands (offers/answers arrive
        // out-of-band, not on this UDP socket) are picked up within one tick even when the
        // SFU's next timer is far away.
        let delay_from_now = eto
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::from_secs(0))
            .min(Duration::from_millis(50));
        if delay_from_now.is_zero() {
            if let Err(err) = sfu.handle_timeout(Instant::now()) {
                warn!("failed to handle timeout, {}", err);
            }
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
            Err(err) => match err.kind() {
                ErrorKind::WouldBlock | ErrorKind::TimedOut => None,
                _ => panic!("UdpSocket read failed: {err:?}"),
            },
        } {
            // A single client's transient read error must not kill the media loop that
            // serves every client on this port. E.g. a DTLS packet that races ahead of the
            // peer connection's start_transports, or a stale packet for a client that is
            // mid-teardown, returns an error here — DTLS/ICE simply retransmit, so log and
            // keep serving the other clients rather than tearing the whole thread down.
            if let Err(err) = sfu.handle_read(packet) {
                warn!("sfu.handle_read dropped a packet: {err}");
            }
        }

        if let Err(err) = sfu.handle_timeout(Instant::now()) {
            warn!("sfu.handle_timeout: {err}");
        }
    }

    println!("media server on {} is gracefully down", local_addr);
    Ok(())
}

fn handle_command(
    sfu: &mut Sfu,
    command: Command,
    subscribers: &mut HashMap<(RoomId, ClientId), SyncSender<String>>,
) {
    match command {
        Command::Register {
            room_id,
            client_id,
            out_tx,
        } => {
            // Replace any stale writer (browser reconnected on a fresh socket).
            subscribers.insert((room_id, client_id), out_tx);
            if let Err(err) = sfu.handle_event(SFUEvent::Join {
                request_id: random(),
                room_id,
                client_id,
            }) {
                error!("join failed for {}/{}: {}", room_id, client_id, err);
            }
        }
        Command::Signal(evt) => {
            let leaving = match &evt {
                SFUEvent::Leave {
                    room_id, client_id, ..
                } => Some((*room_id, *client_id)),
                _ => None,
            };
            if let Err(err) = sfu.handle_event(evt) {
                error!("handle_event failed: {}", err);
            }
            if let Some(key) = leaving {
                subscribers.remove(&key);
            }
        }
    }
}

/// Push every SFU-emitted SDP to the target client's WebSocket. An `Answer` is the reply
/// to that client's offer; an `Offer` is a server-initiated subscribe re-offer.
fn drain_sfu_events(
    sfu: &mut Sfu,
    subscribers: &mut HashMap<(RoomId, ClientId), SyncSender<String>>,
) {
    while let Some(evt) = sfu.poll_event() {
        match evt {
            SFUEvent::SessionDescription {
                request_id,
                room_id,
                client_id,
                sdp,
            } => {
                let label = if sdp.sdp_type == RTCSdpType::Answer {
                    "Answer"
                } else {
                    "Re-Offer"
                };
                log_sdp(label, "SFU->browser", room_id, client_id, &sdp);
                push_to_subscriber(
                    subscribers,
                    room_id,
                    client_id,
                    &sdp,
                    (sdp.sdp_type == RTCSdpType::Offer).then_some(request_id),
                );
            }
            other => warn!("run loop dropped unroutable SFU event {:?}", other),
        }
    }
}

fn push_to_subscriber(
    subscribers: &mut HashMap<(RoomId, ClientId), SyncSender<String>>,
    room_id: RoomId,
    client_id: ClientId,
    sdp: &RTCSessionDescription,
    request_id: Option<RequestId>,
) {
    let key = (room_id, client_id);
    let Some(tx) = subscribers.get(&key) else {
        warn!(
            "SFU emitted SDP for client {}/{} with no WebSocket",
            room_id, client_id
        );
        return;
    };
    match serde_json::to_string(&WsServerSdp { sdp, request_id }) {
        Ok(payload) => {
            if tx.try_send(payload).is_err() {
                warn!(
                    "WebSocket for client {}/{} is gone/full; dropping SDP",
                    room_id, client_id
                );
                subscribers.remove(&key);
            }
        }
        Err(err) => error!(
            "failed to serialize SDP for client {}/{}: {}",
            room_id, client_id, err
        ),
    }
}

// ───────────────────────────────────── logging ─────────────────────────────────────

/// Concise SDP summary for `info!` flow logs; the full SDP goes to `trace!`.
fn sdp_summary(sdp: &RTCSessionDescription) -> String {
    let m_lines = sdp
        .sdp
        .lines()
        .filter(|line| line.starts_with("m="))
        .count();
    format!(
        "{} ({} bytes, {} m-line{})",
        sdp.sdp_type,
        sdp.sdp.len(),
        m_lines,
        if m_lines == 1 { "" } else { "s" }
    )
}

/// Log one SDP exchange: a summary at `info!` and the full SDP at `trace!`. `label` names
/// the exchange (Offer / Answer / Re-Offer / Re-Answer), `direction` its flow.
fn log_sdp(
    label: &str,
    direction: &str,
    room_id: RoomId,
    client_id: ClientId,
    sdp: &RTCSessionDescription,
) {
    info!(
        "{label:<9} {direction:<11} room={room_id} client={client_id} full SDP:\n{}",
        sdp.sdp
    );
}
