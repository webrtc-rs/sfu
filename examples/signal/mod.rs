#![allow(dead_code)]

use anyhow::Result;
use bytes::Bytes;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use log::{debug, error, info};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;

pub enum SignalingProtocolMessage {
    Ok {
        room_id: u64,
        endpoint_id: u64,
    },
    Err {
        room_id: u64,
        endpoint_id: u64,
        reason: String,
    },
    Join {
        room_id: u64,
    },
    Offer {
        room_id: u64,
        endpoint_id: u64,
        offer_sdp: Bytes,
    },
    Answer {
        room_id: u64,
        endpoint_id: u64,
        answer_sdp: Bytes,
    },
    Trickle {
        room_id: u64,
        endpoint_id: u64,
        trickle_sdp: Bytes,
    },
    Leave {
        room_id: u64,
        endpoint_id: u64,
    },
}

pub struct SignalingMessage {
    pub request: SignalingProtocolMessage,
    pub response_tx: futures::channel::oneshot::Sender<SignalingProtocolMessage>,
}

pub struct SignalingServer {
    signal_addr: SocketAddr,
    media_port_thread_map: Arc<HashMap<u16, smol::channel::Sender<SignalingMessage>>>,
}

impl SignalingServer {
    pub fn new(
        signal_addr: SocketAddr,
        media_port_thread_map: HashMap<u16, smol::channel::Sender<SignalingMessage>>,
    ) -> Self {
        Self {
            signal_addr,
            media_port_thread_map: Arc::new(media_port_thread_map),
        }
    }

    /// http_sdp_server starts a HTTP Server that consumes SDPs
    pub async fn run(&self, mut cancel_rx: broadcast::Receiver<()>) -> broadcast::Receiver<()> {
        let (done_tx, done_rx) = broadcast::channel(1);
        let signal_addr = self.signal_addr;
        let media_port_thread_map = self.media_port_thread_map.clone();
        tokio::spawn(async move {
            let service = make_service_fn(move |_| {
                let media_port_thread_map = media_port_thread_map.clone();
                async move {
                    Ok::<_, hyper::Error>(service_fn(move |req| {
                        let media_port_thread_map = media_port_thread_map.clone();
                        async move {
                            let resp = remote_handler(req, media_port_thread_map).await?;
                            Ok::<_, hyper::Error>(resp)
                        }
                    }))
                }
            });
            let server = Server::bind(&signal_addr).serve(service);
            let graceful = server.with_graceful_shutdown(async {
                let _ = cancel_rx.recv().await;
                info!("signaling server receives cancel signal");
                let _ = done_tx.send(());
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
    media_port_thread_map: Arc<HashMap<u16, smol::channel::Sender<SignalingMessage>>>,
) -> Result<Response<Body>, hyper::Error> {
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
    let room_id = path[2].parse::<u64>().unwrap();
    let mut sorted_ports: Vec<u16> = media_port_thread_map.keys().map(|x| *x).collect();
    sorted_ports.sort();
    assert!(!sorted_ports.is_empty());
    let port = sorted_ports[(room_id as usize) % sorted_ports.len()];
    let event_base = media_port_thread_map.get(&port).unwrap();
    let (response_tx, response_rx) =
        futures::channel::oneshot::channel::<SignalingProtocolMessage>();

    match (req.method(), path[1]) {
        (&Method::POST, "join") => {
            debug!("remote_handler receive from /join/room_id");

            if event_base
                .send(SignalingMessage {
                    request: SignalingProtocolMessage::Join { room_id },
                    response_tx,
                })
                .await
                .is_ok()
            {
                if let Ok(response) = response_rx.await {
                    match response {
                        SignalingProtocolMessage::Ok {
                            room_id: _,
                            endpoint_id,
                        } => {
                            let mut response =
                                Response::new(Body::from(format!("{}", endpoint_id)));
                            *response.status_mut() = StatusCode::OK;
                            return Ok(response);
                        }
                        SignalingProtocolMessage::Err {
                            room_id: _,
                            endpoint_id: _,
                            reason,
                        } => {
                            let mut response = Response::new(Body::from(reason));
                            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                            return Ok(response);
                        }
                        _ => {}
                    }
                }
            }
        }
        (&Method::POST, "offer") => {
            debug!("remote_handler receive from /offer/room_id/endpoint_id");

            let endpoint_id = path[3].parse::<u64>().unwrap();
            let offer_sdp = hyper::body::to_bytes(req.into_body()).await?;

            if event_base
                .send(SignalingMessage {
                    request: SignalingProtocolMessage::Offer {
                        room_id,
                        endpoint_id,
                        offer_sdp,
                    },
                    response_tx,
                })
                .await
                .is_ok()
            {
                if let Ok(response) = response_rx.await {
                    match response {
                        SignalingProtocolMessage::Answer {
                            room_id: _,
                            endpoint_id: _,
                            answer_sdp,
                        } => {
                            let mut response = Response::new(Body::from(answer_sdp));
                            *response.status_mut() = StatusCode::OK;
                            return Ok(response);
                        }
                        SignalingProtocolMessage::Err {
                            room_id: _,
                            endpoint_id: _,
                            reason,
                        } => {
                            let mut response = Response::new(Body::from(reason));
                            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                            return Ok(response);
                        }
                        _ => {}
                    }
                }
            }
        }
        (&Method::POST, "answer") => {
            debug!("remote_handler receive from /answer/room_id/endpoint_id");

            let endpoint_id = path[3].parse::<u64>().unwrap();
            let answer_sdp = hyper::body::to_bytes(req.into_body()).await?;

            if event_base
                .send(SignalingMessage {
                    request: SignalingProtocolMessage::Answer {
                        room_id,
                        endpoint_id,
                        answer_sdp,
                    },
                    response_tx,
                })
                .await
                .is_ok()
            {
                if let Ok(response) = response_rx.await {
                    match response {
                        SignalingProtocolMessage::Ok {
                            room_id: _,
                            endpoint_id: _,
                        } => {
                            let mut response = Response::default();
                            *response.status_mut() = StatusCode::OK;
                            return Ok(response);
                        }
                        SignalingProtocolMessage::Err {
                            room_id: _,
                            endpoint_id: _,
                            reason,
                        } => {
                            let mut response = Response::new(Body::from(reason));
                            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                            return Ok(response);
                        }
                        _ => {}
                    }
                }
            }
        }
        (&Method::POST, "leave") => {
            debug!("remote_handler receive from /leave/room_id/endpoint_id");

            let endpoint_id = path[3].parse::<u64>().unwrap();

            if event_base
                .send(SignalingMessage {
                    request: SignalingProtocolMessage::Leave {
                        room_id,
                        endpoint_id,
                    },
                    response_tx,
                })
                .await
                .is_ok()
            {
                if let Ok(response) = response_rx.await {
                    match response {
                        SignalingProtocolMessage::Ok {
                            room_id: _,
                            endpoint_id: _,
                        } => {
                            let mut response = Response::default();
                            *response.status_mut() = StatusCode::OK;
                            return Ok(response);
                        }
                        SignalingProtocolMessage::Err {
                            room_id: _,
                            endpoint_id: _,
                            reason,
                        } => {
                            let mut response = Response::new(Body::from(reason));
                            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                            return Ok(response);
                        }
                        _ => {}
                    }
                }
            }
        }
        // Return the 404 Not Found for other routes.
        _ => {
            let mut not_found = Response::default();
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            return Ok(not_found);
        }
    };

    let mut response = Response::default();
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    Ok(response)
}
