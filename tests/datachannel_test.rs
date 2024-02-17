use log::error;
use rand::random;
use sfu::SessionId;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;

// importing common module.
mod common;

#[tokio::test]
async fn test_datachannel() -> anyhow::Result<()> {
    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let host = "127.0.0.1".to_string();
    let signal_port = 8080u16;
    let session_id: SessionId = random::<u64>();

    let (peer_connection, endpoint_id) = match common::setup(config).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("error: {}", err);
            return Err(err.into());
        }
    };

    match common::connect(host, signal_port, session_id, endpoint_id, &peer_connection).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("error: {}", err);
            return Err(err.into());
        }
    };

    match common::teardown(peer_connection).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("error: {}", err);
            return Err(err.into());
        }
    }
    Ok(())
}
