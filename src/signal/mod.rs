use crate::rtc::server::server_states::ServerStates;
use crate::rtc::session::endpoint::Endpoint;
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
    match (req.method(), req.uri().path()) {
        // A HTTP handler that processes a SessionDescription given to us from the other WebRTC-rs or Pion process
        (&Method::POST, "/join") => {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "/offer") => {
            debug!("remote_handler receive from /offer");
            let bytes = hyper::body::to_bytes(req.into_body()).await?;
            let offer = std::str::from_utf8(&bytes).unwrap(); //TODO
            let mut endpoint = Endpoint::new();
            let answer = endpoint.accept_offer(offer).unwrap(); //TODO
            server_states.insert(endpoint).await;
            let mut response = Response::new(Body::from(answer));
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "/answer") => {
            debug!("remote_handler receive from /answer");
            let bytes = hyper::body::to_bytes(req.into_body()).await?;
            let _answer = std::str::from_utf8(&bytes).unwrap(); //TODO

            //endpoint.accept_answer(answer).unwrap(); //TODO

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "/trickle") => {
            debug!("remote_handler receive from /trickle");
            let bytes = hyper::body::to_bytes(req.into_body()).await?;
            let _trickle = std::str::from_utf8(&bytes).unwrap(); //TODO

            //endpoint.accept_answer(answer).unwrap(); //TODO

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }
        (&Method::POST, "/leave") => {
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
