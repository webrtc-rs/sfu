#![allow(dead_code)]

use anyhow::Result;
use async_broadcast::{broadcast, Receiver};
use bytes::{Buf, Bytes};
use futures::channel::oneshot::Sender;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use log::{error, info};
use sfu::{RTCSessionDescription, ServerStates};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use str0m::change::SdpOffer;
use str0m::{Candidate, Rtc};
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};

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
    Join {
        session_id: u64,
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
    Trickle {
        session_id: u64,
        endpoint_id: u64,
        trickle_sdp: Bytes,
    },
    Leave {
        session_id: u64,
        endpoint_id: u64,
    },
}

pub struct SignalingMessage {
    pub request: SignalingProtocolMessage,
    pub response_tx: Sender<SignalingProtocolMessage>,
}

pub struct SignalingServer {
    signal_addr: SocketAddr,
    media_port_thread_map: Arc<HashMap<u16, SyncSender<Rtc>>>,
}

impl SignalingServer {
    pub fn new(
        signal_addr: SocketAddr,
        media_port_thread_map: HashMap<u16, SyncSender<Rtc>>,
    ) -> Self {
        Self {
            signal_addr,
            media_port_thread_map: Arc::new(media_port_thread_map),
        }
    }

    /// http_sdp_server starts a HTTP Server that consumes SDPs
    pub async fn run(&self, mut stop_rx: Receiver<()>) -> Receiver<()> {
        let (done_tx, done_rx) = broadcast(1);
        let signal_addr = self.signal_addr;
        let host_ip = signal_addr.ip();
        let media_port_thread_map = self.media_port_thread_map.clone();
        tokio::spawn(async move {
            let service = make_service_fn(move |_| {
                let media_port_thread_map = media_port_thread_map.clone();
                let host_ip = host_ip.clone();
                async move {
                    Ok::<_, hyper::Error>(service_fn(move |req| {
                        let media_port_thread_map = media_port_thread_map.clone();
                        let host_ip = host_ip.clone();
                        async move {
                            let resp = remote_handler(req, host_ip, media_port_thread_map).await?;
                            Ok::<_, hyper::Error>(resp)
                        }
                    }))
                }
            });
            let server = Server::bind(&signal_addr).serve(service);
            println!(
                "signaling server http://{}:{} is running...",
                signal_addr.ip(),
                signal_addr.port()
            );
            let graceful = server.with_graceful_shutdown(async {
                let _ = stop_rx.recv().await;
                info!("signaling server receives stop signal");
                let _ = done_tx.try_broadcast(());
            });

            // Run this server for forever until ctrl+c!
            if let Err(err) = graceful.await {
                error!("signaling server error: {}", err);
            }
        });

        done_rx
    }
}

// HTTP Listener to get sdp
async fn remote_handler(
    req: Request<Body>,
    host_ip: IpAddr,
    media_port_thread_map: Arc<HashMap<u16, SyncSender<Rtc>>>,
) -> Result<Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") | (&Method::GET, "/index.html") => {
            // Open file for reading
            if let Ok(file) = File::open("examples/chat.html").await {
                let stream = FramedRead::new(file, BytesCodec::new());
                let body = Body::wrap_stream(stream);
                return Ok(Response::new(body));
            } else {
                eprintln!("ERROR: Unable to open file.");
                let mut not_found = Response::default();
                *not_found.status_mut() = StatusCode::NOT_FOUND;
                return Ok(not_found);
            }
        }
        _ => {}
    };

    let path: Vec<&str> = req.uri().path().split('/').collect();
    if path.len() < 3
        || path[2].parse::<u64>().is_err()
        || ((path[1] == "offer" || path[1] == "answer" || path[1] == "leave")
            && (path.len() < 4 || path[3].parse::<u64>().is_err()))
    {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return Ok(response);
    }
    let session_id = path[2].parse::<u64>().unwrap();
    let mut sorted_ports: Vec<u16> = media_port_thread_map.keys().map(|x| *x).collect();
    sorted_ports.sort();
    assert!(!sorted_ports.is_empty());
    let port = sorted_ports[(session_id as usize) % sorted_ports.len()];
    let tx = media_port_thread_map.get(&port).unwrap();

    //let endpoint_id = path[3].parse::<u64>().unwrap();
    let offer_sdp = hyper::body::to_bytes(req.into_body()).await?;

    let offer: SdpOffer = serde_json::from_reader(offer_sdp.reader()).expect("serialized offer");
    let mut rtc = Rtc::builder()
        // Uncomment this to see statistics
        // .set_stats_interval(Some(Duration::from_secs(1)))
        // .set_ice_lite(true)
        .build();

    // Add the shared UDP socket as a host candidate
    let candidate =
        Candidate::host(SocketAddr::new(host_ip, port), "udp").expect("a host candidate");
    rtc.add_local_candidate(candidate);

    // Create an SDP Answer.
    let answer = rtc
        .sdp_api()
        .accept_offer(offer)
        .expect("offer to be accepted");

    // The Rtc instance is shipped off to the main run loop.
    tx.send(rtc).expect("to send Rtc instance");

    let body = serde_json::to_vec(&answer).expect("answer to serialize");

    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::OK;
    return Ok(response);
}

pub fn handle_signaling_message(
    server_states: &Rc<RefCell<ServerStates>>,
    signaling_msg: SignalingMessage,
) -> Result<()> {
    match signaling_msg.request {
        SignalingProtocolMessage::Join { session_id } => {
            let endpoint_id: u64 = rand::random();
            Ok(signaling_msg
                .response_tx
                .send(SignalingProtocolMessage::Ok {
                    session_id,
                    endpoint_id,
                })
                .map_err(|_| {
                    Error::new(
                        ErrorKind::Other,
                        "failed to send back signaling message response".to_string(),
                    )
                })?)
        }
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
        SignalingProtocolMessage::Answer {
            session_id,
            endpoint_id,
            answer_sdp,
        } => handle_answer_message(
            server_states,
            session_id,
            endpoint_id,
            answer_sdp,
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
        | SignalingProtocolMessage::Trickle {
            session_id,
            endpoint_id,
            trickle_sdp: _,
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
    response_tx: Sender<SignalingProtocolMessage>,
) -> Result<()> {
    let try_handle = || -> Result<Bytes> {
        let offer_str = String::from_utf8(offer.to_vec())?;
        info!(
            "handle_offer_message: {}/{}/{}",
            session_id, endpoint_id, offer_str,
        );
        let mut server_states = server_states.borrow_mut();

        let offer_sdp = serde_json::from_str::<RTCSessionDescription>(&offer_str)?;
        let answer = server_states.accept_offer(session_id, endpoint_id, None, offer_sdp)?;
        let answer_str = serde_json::to_string(&answer)?;
        info!("generate answer sdp: {}", answer_str);
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

fn handle_answer_message(
    _server_states: &Rc<RefCell<ServerStates>>,
    session_id: u64,
    endpoint_id: u64,
    answer_sdp: Bytes,
    response_tx: Sender<SignalingProtocolMessage>,
) -> Result<()> {
    let try_handle = || -> Result<()> {
        info!(
            "handle_answer_message: {}/{}/{}",
            session_id,
            endpoint_id,
            String::from_utf8(answer_sdp.to_vec())?
        );
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

fn handle_leave_message(
    _server_states: &Rc<RefCell<ServerStates>>,
    session_id: u64,
    endpoint_id: u64,
    response_tx: Sender<SignalingProtocolMessage>,
) -> Result<()> {
    let try_handle = || -> Result<()> {
        info!("handle_leave_message: {}/{}", session_id, endpoint_id,);
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
