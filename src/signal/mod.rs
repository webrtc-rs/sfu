use anyhow::Result;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use log::{debug, error, info};
use std::net::SocketAddr;
use std::str::FromStr;
use tokio::sync::{mpsc, oneshot};

// HTTP Listener to get sdp
async fn remote_handler(
    req: Request<Body>,
    tx: mpsc::Sender<String>,
) -> Result<Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        // A HTTP handler that processes a SessionDescription given to us from the other WebRTC-rs or Pion process
        (&Method::POST, "/sdp") => {
            debug!("remote_handler receive from /sdp");
            let sdp_str = match std::str::from_utf8(&hyper::body::to_bytes(req.into_body()).await?)
            {
                Ok(s) => s.to_owned(),
                Err(err) => panic!("{}", err),
            };

            let _ = tx.send(sdp_str).await;

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
    port: u16,
    sdp_tx: mpsc::Sender<String>,
    cancel_rx: oneshot::Receiver<()>,
) -> oneshot::Receiver<()> {
    let (done_tx, done_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let addr = SocketAddr::from_str(&format!("0.0.0.0:{}", port)).unwrap();
        let service = make_service_fn(move |_| {
            let tx = sdp_tx.clone();
            async move {
                Ok::<_, hyper::Error>(service_fn(move |req| {
                    let tx = tx.clone();
                    async move {
                        let resp = remote_handler(req, tx).await?;
                        Ok::<_, hyper::Error>(resp)
                    }
                }))
            }
        });
        let server = Server::bind(&addr).serve(service);
        let graceful = server.with_graceful_shutdown(async {
            cancel_rx.await.ok();
            info!("http_sdp_server receives cancel signal");
            let _ = done_tx.send(());
        });

        // Run this server for forever until ctrl+c!
        if let Err(e) = graceful.await {
            error!("server error: {}", e);
        }
    });

    done_rx
}
