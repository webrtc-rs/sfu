use anyhow::Result;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

lazy_static! {
    static ref SDP_CHAN_TX_MUTEX: Arc<Mutex<Option<mpsc::Sender<String>>>> =
        Arc::new(Mutex::new(None));
}

// HTTP Listener to get sdp
async fn remote_handler(req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        // A HTTP handler that processes a SessionDescription given to us from the other WebRTC-rs or Pion process
        (&Method::POST, "/sdp") => {
            //println!("remote_handler receive from /sdp");
            let sdp_str = match std::str::from_utf8(&hyper::body::to_bytes(req.into_body()).await?)
            {
                Ok(s) => s.to_owned(),
                Err(err) => panic!("{}", err),
            };

            {
                let sdp_chan_tx = SDP_CHAN_TX_MUTEX.lock().await;
                if let Some(tx) = &*sdp_chan_tx {
                    let _ = tx.send(sdp_str).await;
                }
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
pub async fn http_sdp_server(port: u16) -> mpsc::Receiver<String> {
    let (sdp_chan_tx, sdp_chan_rx) = mpsc::channel::<String>(1);
    {
        let mut tx = SDP_CHAN_TX_MUTEX.lock().await;
        *tx = Some(sdp_chan_tx);
    }

    tokio::spawn(async move {
        let addr = SocketAddr::from_str(&format!("0.0.0.0:{}", port)).unwrap();
        let service =
            make_service_fn(|_| async { Ok::<_, hyper::Error>(service_fn(remote_handler)) });
        let server = Server::bind(&addr).serve(service);
        // Run this server for... forever!
        if let Err(e) = server.await {
            eprintln!("server error: {}", e);
        }
    });

    sdp_chan_rx
}
