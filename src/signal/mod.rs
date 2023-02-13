use crate::rtc::{endpoint::Endpoint, server::ServerStates};

use crate::rtc::room::Room;
use anyhow::Result;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use log::{debug, error, info};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::broadcast;

// HTTP Listener to get sdp
async fn remote_handler(
    req: Request<Body>,
    server_states: Arc<ServerStates>,
) -> Result<Response<Body>, hyper::Error> {
    let path: Vec<&str> = req.uri().path().split('/').collect();
    if path.len() < 3
        || path[2].parse::<u64>().is_err()
        || ((path[1] == "offer"
            || path[1] == "answer"
            || path[1] == "trickle"
            || path[1] == "leave")
            && (path.len() < 4 || path[3].parse::<u64>().is_err()))
    {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return Ok(response);
    }
    let room_id = path[2].parse::<u64>().unwrap();

    match (req.method(), path[1]) {
        // A HTTP handler that processes a SessionDescription given to us from the other WebRTC-rs or Pion process
        (&Method::POST, "join") => {
            let room = if let Some(room) = server_states.get(room_id).await {
                room
            } else {
                let room = Arc::new(Room::new(room_id));
                server_states.insert(room.clone()).await;
                room
            };

            let endpoint_id = rand::random::<u64>();
            let endpoint = Arc::new(Endpoint::new(room_id, endpoint_id));
            room.insert(endpoint).await;

            info!("endpoint {} joined room {}", endpoint_id, room_id);

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "offer") => {
            debug!("remote_handler receive from /offer/room_id/endpoint_id");

            let room = if let Some(room) = server_states.get(room_id).await {
                room
            } else {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(response);
            };

            let endpoint_id = path[3].parse::<u64>().unwrap();
            let endpoint = if let Some(endpoint) = room.get(endpoint_id).await {
                endpoint
            } else {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(response);
            };

            let bytes = hyper::body::to_bytes(req.into_body()).await?;
            let offer = std::str::from_utf8(&bytes).unwrap(); //TODO
            let answer = endpoint.accept_offer(offer).unwrap(); //TODO

            info!(
                "endpoint {} sent offer {} in room {}",
                endpoint_id, offer, room_id
            );

            let mut response = Response::new(Body::from(answer));
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "answer") => {
            debug!("remote_handler receive from /answer/room_id/endpoint_id");

            let room = if let Some(room) = server_states.get(room_id).await {
                room
            } else {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(response);
            };

            let endpoint_id = path[3].parse::<u64>().unwrap();
            let endpoint = if let Some(endpoint) = room.get(endpoint_id).await {
                endpoint
            } else {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(response);
            };

            let bytes = hyper::body::to_bytes(req.into_body()).await?;
            let answer = std::str::from_utf8(&bytes).unwrap(); //TODO
            endpoint.accept_answer(answer).unwrap(); //TODO

            info!(
                "endpoint {} sent answer {} in room {}",
                endpoint_id, answer, room_id
            );

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "trickle") => {
            debug!("remote_handler receive from /trickle/room_id/endpoint_id");
            let room = if let Some(room) = server_states.get(room_id).await {
                room
            } else {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(response);
            };

            let endpoint_id = path[3].parse::<u64>().unwrap();
            let endpoint = if let Some(endpoint) = room.get(endpoint_id).await {
                endpoint
            } else {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(response);
            };

            let bytes = hyper::body::to_bytes(req.into_body()).await?;
            let trickle = std::str::from_utf8(&bytes).unwrap(); //TODO
            endpoint.accept_trickle(trickle).unwrap(); //TODO

            info!(
                "endpoint {} sent trickle {} in room {}",
                endpoint_id, trickle, room_id
            );

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "leave") => {
            debug!("remote_handler receive from /leave/room_id/endpoint_id");
            let room = if let Some(room) = server_states.get(room_id).await {
                room
            } else {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(response);
            };

            let endpoint_id = path[3].parse::<u64>().unwrap();
            if let Some(endpoint) = room.remove(endpoint_id).await {
                info!("endpoint {} left room {}", endpoint.endpoint_id(), room_id);
            }

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        // Return the 404 Not Found for other routes.
        _ => {
            let mut not_found = Response::default();
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

/// http_sdp_server starts a HTTP Server that consumes SDPs
pub async fn http_sdp_server(
    host: String,
    port: u16,
    server_states: Arc<ServerStates>,
    mut cancel_rx: broadcast::Receiver<()>,
) -> broadcast::Receiver<()> {
    let (done_tx, done_rx) = broadcast::channel(1);
    tokio::spawn(async move {
        let addr = SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap();
        let service = make_service_fn(move |_| {
            let server_states = server_states.clone();
            async move {
                Ok::<_, hyper::Error>(service_fn(move |req| {
                    let server_states = server_states.clone();
                    async move {
                        let resp = remote_handler(req, server_states).await?;
                        Ok::<_, hyper::Error>(resp)
                    }
                }))
            }
        });
        let server = Server::bind(&addr).serve(service);
        let graceful = server.with_graceful_shutdown(async {
            let _ = cancel_rx.recv().await;
            info!("http_sdp_server receives cancel signal");
            let _ = done_tx.send(());
        });

        // Run this server for forever until ctrl+c!
        if let Err(err) = graceful.await {
            error!("http_sdp_server error: {}", err);
        }
    });

    done_rx
}
