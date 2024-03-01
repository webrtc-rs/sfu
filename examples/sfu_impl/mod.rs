#![allow(dead_code)]

mod sync_transport;

use crate::sfu_impl::sync_transport::SyncTransport;
use bytes::{Bytes, BytesMut};
use retty::channel::{InboundPipeline, Pipeline};
use retty::transport::{TaggedBytesMut, TransportContext};
use rouille::{Request, Response, ResponseBody};
use sfu::{
    DataChannelHandler, DemuxerHandler, DtlsHandler, ExceptionHandler, GatewayHandler,
    InterceptorHandler, RTCSessionDescription, SctpHandler, ServerConfig, ServerStates,
    SrtpHandler, StunHandler,
};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::{Error, ErrorKind, Read};
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

// Handle a web request.
pub fn web_request_sfu(
    request: &Request,
    media_port_thread_map: Arc<
        HashMap<
            u16,
            (
                Option<SyncSender<str0m::Rtc>>,
                Option<SyncSender<SignalingMessage>>,
            ),
        >,
    >,
) -> Response {
    if request.method() == "GET" {
        return Response::html(include_str!("../chat.html"));
    }

    // "/offer/433774451/456773342" or "/leave/433774451/456773342"
    let path: Vec<String> = request.url().split('/').map(|s| s.to_owned()).collect();
    if path.len() != 4 || path[2].parse::<u64>().is_err() || path[3].parse::<u64>().is_err() {
        return Response::empty_400();
    }

    let session_id = path[2].parse::<u64>().unwrap();
    let mut sorted_ports: Vec<u16> = media_port_thread_map.keys().map(|x| *x).collect();
    sorted_ports.sort();
    assert!(!sorted_ports.is_empty());
    let port = sorted_ports[(session_id as usize) % sorted_ports.len()];
    let (_, tx) = media_port_thread_map.get(&port).unwrap();

    // Expected POST SDP Offers.
    let mut offer_sdp = vec![];
    request
        .data()
        .expect("body to be available")
        .read_to_end(&mut offer_sdp)
        .unwrap();

    // The Rtc instance is shipped off to the main run loop.
    if let Some(tx) = tx {
        let endpoint_id = path[3].parse::<u64>().unwrap();
        if path[1] == "offer" {
            let (response_tx, response_rx) = mpsc::sync_channel(1);

            tx.send(SignalingMessage {
                request: SignalingProtocolMessage::Offer {
                    session_id,
                    endpoint_id,
                    offer_sdp: Bytes::from(offer_sdp),
                },
                response_tx,
            })
            .expect("to send SignalingMessage instance");

            let response = response_rx.recv().expect("receive answer offer");
            match response {
                SignalingProtocolMessage::Answer {
                    session_id: _,
                    endpoint_id: _,
                    answer_sdp,
                } => Response::from_data("application/json", answer_sdp),
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
pub fn run_sfu(
    stop_rx: crossbeam_channel::Receiver<()>,
    socket: UdpSocket,
    rx: Receiver<SignalingMessage>,
    server_config: Arc<ServerConfig>,
) -> anyhow::Result<()> {
    let server_states = Rc::new(RefCell::new(ServerStates::new(
        server_config,
        socket.local_addr()?,
    )?));

    println!("listening {}...", socket.local_addr()?);

    let outgoing_queue = Rc::new(RefCell::new(VecDeque::new()));

    let pipeline = build_pipeline(
        socket.local_addr()?,
        outgoing_queue.clone(),
        server_states.clone(),
    );

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

        write_socket_output(&socket, &outgoing_queue);

        // Spawn new incoming signal message from the signaling server thread.
        if let Ok(signal_message) = rx.try_recv() {
            if let Err(err) = handle_signaling_message(&server_states, signal_message) {
                error!("handle_signaling_message got error:{}", err);
                continue;
            }
        }

        // Poll clients until they return timeout
        let mut eto = Instant::now() + Duration::from_millis(100);
        pipeline.poll_timeout(&mut eto);

        let delay_from_now = eto
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::from_secs(0));
        if delay_from_now.is_zero() {
            pipeline.handle_timeout(Instant::now());
            continue;
        }

        socket
            .set_read_timeout(Some(delay_from_now))
            .expect("setting socket read timeout");

        if let Some(input) = read_socket_input(&socket, &mut buf) {
            pipeline.read(input);
        }

        // Drive time forward in all clients.
        pipeline.handle_timeout(Instant::now());
    }

    println!(
        "media server on {} is gracefully down",
        socket.local_addr()?
    );
    Ok(())
}

fn write_socket_output(socket: &UdpSocket, outgoing_queue: &Rc<RefCell<VecDeque<TaggedBytesMut>>>) {
    let mut queue = outgoing_queue.borrow_mut();
    while let Some(transmit) = queue.pop_front() {
        socket
            .send_to(&transmit.message, transmit.transport.peer_addr)
            .expect("sending UDP data");
    }
}

fn read_socket_input(socket: &UdpSocket, buf: &mut [u8]) -> Option<TaggedBytesMut> {
    match socket.recv_from(buf) {
        Ok((n, peer_addr)) => {
            return Some(TaggedBytesMut {
                now: Instant::now(),
                transport: TransportContext {
                    local_addr: socket.local_addr().unwrap(),
                    peer_addr,
                    ecn: None,
                },
                message: BytesMut::from(&buf[..n]),
            });
        }

        Err(e) => match e.kind() {
            // Expected error for set_read_timeout(). One for windows, one for the rest.
            ErrorKind::WouldBlock | ErrorKind::TimedOut => None,
            _ => panic!("UdpSocket read failed: {e:?}"),
        },
    }
}

fn build_pipeline(
    local_addr: SocketAddr,
    writer: Rc<RefCell<VecDeque<TaggedBytesMut>>>,
    server_states: Rc<RefCell<ServerStates>>,
) -> Rc<Pipeline<TaggedBytesMut, TaggedBytesMut>> {
    let pipeline: Pipeline<TaggedBytesMut, TaggedBytesMut> = Pipeline::new();

    let sync_transport_handler = SyncTransport::new(writer);
    let demuxer_handler = DemuxerHandler::new();
    let write_exception_handler = ExceptionHandler::new();
    let stun_handler = StunHandler::new();
    // DTLS
    let dtls_handler = DtlsHandler::new(local_addr, Rc::clone(&server_states));
    let sctp_handler = SctpHandler::new(local_addr, Rc::clone(&server_states));
    let data_channel_handler = DataChannelHandler::new();
    // SRTP
    let srtp_handler = SrtpHandler::new(Rc::clone(&server_states));
    let interceptor_handler = InterceptorHandler::new(Rc::clone(&server_states));
    // Gateway
    let gateway_handler = GatewayHandler::new(Rc::clone(&server_states));
    let read_exception_handler = ExceptionHandler::new();

    pipeline.add_back(sync_transport_handler);
    pipeline.add_back(demuxer_handler);
    pipeline.add_back(write_exception_handler);
    pipeline.add_back(stun_handler);
    // DTLS
    pipeline.add_back(dtls_handler);
    pipeline.add_back(sctp_handler);
    pipeline.add_back(data_channel_handler);
    // SRTP
    pipeline.add_back(srtp_handler);
    pipeline.add_back(interceptor_handler);
    // Gateway
    pipeline.add_back(gateway_handler);
    pipeline.add_back(read_exception_handler);

    pipeline.finalize()
}

pub enum SignalingProtocolMessage {
    Ok {
        session_id: u64,
        endpoint_id: u64,
    },
    Err {
        session_id: u64,
        endpoint_id: u64,
        reason: Bytes,
    },
    Offer {
        session_id: u64,
        endpoint_id: u64,
        offer_sdp: Bytes,
    },
    Answer {
        session_id: u64,
        endpoint_id: u64,
        answer_sdp: Bytes,
    },
    Leave {
        session_id: u64,
        endpoint_id: u64,
    },
}

pub struct SignalingMessage {
    pub request: SignalingProtocolMessage,
    pub response_tx: SyncSender<SignalingProtocolMessage>,
}

pub fn handle_signaling_message(
    server_states: &Rc<RefCell<ServerStates>>,
    signaling_msg: SignalingMessage,
) -> anyhow::Result<()> {
    match signaling_msg.request {
        SignalingProtocolMessage::Offer {
            session_id,
            endpoint_id,
            offer_sdp,
        } => handle_offer_message(
            server_states,
            session_id,
            endpoint_id,
            offer_sdp,
            signaling_msg.response_tx,
        ),
        SignalingProtocolMessage::Leave {
            session_id,
            endpoint_id,
        } => handle_leave_message(
            server_states,
            session_id,
            endpoint_id,
            signaling_msg.response_tx,
        ),
        SignalingProtocolMessage::Ok {
            session_id,
            endpoint_id,
        }
        | SignalingProtocolMessage::Err {
            session_id,
            endpoint_id,
            reason: _,
        }
        | SignalingProtocolMessage::Answer {
            session_id,
            endpoint_id,
            answer_sdp: _,
        } => Ok(signaling_msg
            .response_tx
            .send(SignalingProtocolMessage::Err {
                session_id,
                endpoint_id,
                reason: Bytes::from("Invalid Request"),
            })
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    "failed to send back signaling message response".to_string(),
                )
            })?),
    }
}

fn handle_offer_message(
    server_states: &Rc<RefCell<ServerStates>>,
    session_id: u64,
    endpoint_id: u64,
    offer: Bytes,
    response_tx: SyncSender<SignalingProtocolMessage>,
) -> anyhow::Result<()> {
    let try_handle = || -> anyhow::Result<Bytes> {
        let offer_str = String::from_utf8(offer.to_vec())?;
        log::info!(
            "handle_offer_message: {}/{}/{}",
            session_id,
            endpoint_id,
            offer_str,
        );
        let mut server_states = server_states.borrow_mut();

        let offer_sdp = serde_json::from_str::<RTCSessionDescription>(&offer_str)?;
        let answer = server_states.accept_offer(session_id, endpoint_id, None, offer_sdp)?;
        let answer_str = serde_json::to_string(&answer)?;
        log::info!("generate answer sdp: {}", answer_str);
        Ok(Bytes::from(answer_str))
    };

    match try_handle() {
        Ok(answer_sdp) => Ok(response_tx
            .send(SignalingProtocolMessage::Answer {
                session_id,
                endpoint_id,
                answer_sdp,
            })
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    "failed to send back signaling message response".to_string(),
                )
            })?),
        Err(err) => Ok(response_tx
            .send(SignalingProtocolMessage::Err {
                session_id,
                endpoint_id,
                reason: Bytes::from(err.to_string()),
            })
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    "failed to send back signaling message response".to_string(),
                )
            })?),
    }
}

fn handle_leave_message(
    _server_states: &Rc<RefCell<ServerStates>>,
    session_id: u64,
    endpoint_id: u64,
    response_tx: SyncSender<SignalingProtocolMessage>,
) -> anyhow::Result<()> {
    let try_handle = || -> anyhow::Result<()> {
        log::info!("handle_leave_message: {}/{}", session_id, endpoint_id,);
        Ok(())
    };

    match try_handle() {
        Ok(_) => Ok(response_tx
            .send(SignalingProtocolMessage::Ok {
                session_id,
                endpoint_id,
            })
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    "failed to send back signaling message response".to_string(),
                )
            })?),
        Err(err) => Ok(response_tx
            .send(SignalingProtocolMessage::Err {
                session_id,
                endpoint_id,
                reason: Bytes::from(err.to_string()),
            })
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    "failed to send back signaling message response".to_string(),
                )
            })?),
    }
}
