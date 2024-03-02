use crate::common::{HOST, SIGNAL_PORT};
use log::error;
use rand::random;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;

// importing common module.
mod common;

#[tokio::test]
async fn test_data_channel() -> anyhow::Result<()> {
    // Prepare the configuration
    let session_id: u64 = random::<u64>();
    let endpoint_id = 0;
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    let peer_connection = match common::setup_peer_connection(config, endpoint_id).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("error: {}", err);
            return Err(err.into());
        }
    };

    match common::connect(HOST, SIGNAL_PORT, session_id, endpoint_id, &peer_connection).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("error: {}", err);
            return Err(err.into());
        }
    };

    match common::teardown_peer_connection(peer_connection).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("error: {}", err);
            return Err(err.into());
        }
    }
    Ok(())
}

#[tokio::test]
async fn test_data_channels() -> anyhow::Result<()> {
    // Prepare the configuration
    let endpoint_count: usize = 3;
    let session_id: u64 = random::<u64>();
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut configs = vec![];
    let mut endpoint_ids = vec![];
    for endpoint_id in 0..endpoint_count {
        configs.push(config.clone());
        endpoint_ids.push(endpoint_id);
    }

    let peer_connections = match common::setup_peer_connections(configs, &endpoint_ids).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}: error {}", session_id, err);
            return Err(err.into());
        }
    };

    for (endpoint_id, peer_connection) in peer_connections.iter().enumerate() {
        match common::connect(
            HOST,
            SIGNAL_PORT,
            session_id,
            endpoint_id as u64,
            peer_connection,
        )
        .await
        {
            Ok(ok) => ok,
            Err(err) => {
                error!("{}/{}: error {}", session_id, endpoint_id, err);
                return Err(err.into());
            }
        };
    }

    match common::teardown_peer_connections(peer_connections).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}: error {}", session_id, err);
            return Err(err.into());
        }
    }
    Ok(())
}
