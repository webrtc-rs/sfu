#![allow(dead_code)]

use anyhow::anyhow;
use bytes::BytesMut;
use log::error;
use rand::random;
use rouille::{Request, Response, ResponseBody};
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use sansio::Protocol;
use sfu::{ClientId, Event, RequestId, RoomId, Sfu};
use std::collections::HashMap;
use std::io::{ErrorKind, Read};
use std::net::UdpSocket;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

// Handle a web request.
pub fn web_request(
    request: &Request,
    media_port_thread_map: Arc<HashMap<u16, SyncSender<SignalingMessage>>>,
) -> Response {
    if request.method() == "GET" {
        return Response::html(include_str!("../chat.html"));
    }

    // "/offer/433774451/456773342" or "/leave/433774451/456773342"
    let path: Vec<String> = request.url().split('/').map(|s| s.to_owned()).collect();
    if path.len() != 4 || path[2].parse::<u64>().is_err() || path[3].parse::<u64>().is_err() {
        return Response::empty_400();
    }

    let room_id = path[2].parse::<u64>().unwrap();
    let mut sorted_ports: Vec<u16> = media_port_thread_map.keys().map(|x| *x).collect();
    sorted_ports.sort();
    assert!(!sorted_ports.is_empty());
    let port = sorted_ports[(room_id as usize) % sorted_ports.len()];
    let tx = media_port_thread_map.get(&port);

    // Expected POST SDP Offers.
    let mut offer_sdp = vec![];
    request
        .data()
        .expect("body to be available")
        .read_to_end(&mut offer_sdp)
        .unwrap();

    // The Rtc instance is shipped off to the main run loop.
    if let Some(tx) = tx {
        let client_id = path[3].parse::<u64>().unwrap();
        if path[1] == "join" {
            let (response_tx, response_rx) = mpsc::sync_channel(1);

            tx.send(SignalingMessage {
                request: Event::Join {
                    request_id: random(),
                    room_id,
                    client_id,
                },
                response_tx,
            })
            .expect("to send Join");

            let response = response_rx.recv().expect("receive ok");
            match response {
                Event::Ok {
                    request_id: _,
                    room_id: _,
                    client_id: _,
                } => Response {
                    status_code: 200,
                    headers: vec![],
                    data: ResponseBody::empty(),
                    upgrade: None,
                },
                _ => Response::empty_404(),
            }
        } else if path[1] == "offer" {
            let (response_tx, response_rx) = mpsc::sync_channel(1);

            tx.send(SignalingMessage {
                request: Event::SessionDescription {
                    request_id: random(),
                    room_id,
                    client_id,
                    sdp: RTCSessionDescription::offer(String::from_utf8(offer_sdp).unwrap())
                        .unwrap(),
                },
                response_tx,
            })
            .expect("to send Offer");

            let response = response_rx.recv().expect("receive Answer");
            match response {
                Event::SessionDescription {
                    request_id: _,
                    room_id: _,
                    client_id: _,
                    sdp,
                } => Response::from_data("application/json", sdp.sdp),
                _ => Response::empty_404(),
            }
        } else {
            // leave
            Response {
                status_code: 200,
                headers: vec![],
                data: ResponseBody::empty(),
                upgrade: None,
            }
        }
    } else {
        Response::empty_406()
    }
}

/// This is the "main run loop" that handles all clients, reads and writes UdpSocket traffic,
/// and forwards media data between clients.
pub fn run(
    stop_rx: crossbeam_channel::Receiver<()>,
    socket: UdpSocket,
    rx: Receiver<SignalingMessage>,
) -> anyhow::Result<()> {
    println!("listening {}...", socket.local_addr()?);

    let mut sfu = Sfu::new(random());

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

        // Spawn new incoming signal message from the signaling server thread.
        if let Ok(signal_message) = rx.try_recv() {
            if let Err(err) = handle_signaling_message(&mut sfu, signal_message) {
                error!("handle_signaling_message got error:{}", err);
                continue;
            }
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

pub struct SignalingMessage {
    pub request: Event,
    pub response_tx: SyncSender<Event>,
}

pub fn handle_signaling_message(
    sfu: &mut Sfu,
    signaling_msg: SignalingMessage,
) -> anyhow::Result<()> {
    match signaling_msg.request {
        Event::Join {
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
        Event::SessionDescription {
            request_id,
            room_id,
            client_id,
            sdp,
        } => handle_offer_message(
            sfu,
            request_id,
            room_id,
            client_id,
            sdp,
            signaling_msg.response_tx,
        ),
        Event::Leave {
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
        Event::IceCandidate {
            request_id,
            room_id,
            client_id,
            candidate: _,
        } => Ok(signaling_msg.response_tx.send(Event::Err {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
            reason: "Unsupported Request yet".to_string(),
        })?),
        Event::Ok {
            request_id,
            room_id,
            client_id,
            ..
        }
        | Event::Err {
            request_id,
            room_id,
            client_id,
            reason: _,
            ..
        } => Ok(signaling_msg.response_tx.send(Event::Err {
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
    response_tx: SyncSender<Event>,
) -> anyhow::Result<()> {
    let mut try_handle = || -> anyhow::Result<()> {
        log::info!("handle_join_message: {}/{}", room_id, client_id);
        Ok(sfu.handle_event(Event::Join {
            request_id,
            room_id,
            client_id,
        })?)
    };

    match try_handle() {
        Ok(_) => Ok(response_tx.send(Event::Ok {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
        })?),
        Err(err) => Ok(response_tx.send(Event::Err {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
            reason: err.to_string(),
        })?),
    }
}

fn handle_offer_message(
    sfu: &mut Sfu,
    request_id: RequestId,
    room_id: RoomId,
    client_id: ClientId,
    sdp: RTCSessionDescription,
    response_tx: SyncSender<Event>,
) -> anyhow::Result<()> {
    let try_handle = || -> anyhow::Result<Event> {
        log::info!(
            "handle_offer_message: {}/{}/{}",
            room_id,
            client_id,
            sdp.sdp,
        );
        sfu.handle_event(Event::SessionDescription {
            request_id,
            room_id,
            client_id,
            sdp,
        })?;
        if let Some(evt) = sfu.poll_event()
            && let Event::SessionDescription { .. } = &evt
        {
            Ok(evt)
        } else {
            Err(anyhow!("SessionDescription answer failed"))
        }
    };

    match try_handle() {
        Ok(evt) => Ok(response_tx.send(evt)?),
        Err(err) => Ok(response_tx.send(Event::Err {
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
    response_tx: SyncSender<Event>,
) -> anyhow::Result<()> {
    let try_handle = || -> anyhow::Result<()> {
        log::info!("handle_leave_message: {}/{}", room_id, client_id);
        Ok(sfu.handle_event(Event::Leave {
            request_id,
            room_id,
            client_id,
            reason,
        })?)
    };

    match try_handle() {
        Ok(_) => Ok(response_tx.send(Event::Ok {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
        })?),
        Err(err) => Ok(response_tx.send(Event::Err {
            request_id,
            room_id: Some(room_id),
            client_id: Some(client_id),
            reason: err.to_string(),
        })?),
    }
}
