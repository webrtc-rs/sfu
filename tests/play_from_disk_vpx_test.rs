use crate::common::{HOST, SIGNAL_PORT};
use log::{error, info};
use rand::random;
use sfu::SessionId;
use webrtc::api::media_engine::MIME_TYPE_VP8;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::sdp_type::RTCSdpType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;

// importing common module.
mod common;

#[tokio::test]
async fn test_play_from_disk_vpx_1to1() -> anyhow::Result<()> {
    // Prepare the configuration
    let session_id: SessionId = random::<u64>();
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    let peer_connections = match common::setup_peer_connections(vec![config.clone(), config]).await
    {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}: error {}", session_id, err);
            return Err(err.into());
        }
    };

    let mut data_channels = vec![];
    for (endpoint_id, peer_connection) in peer_connections.iter() {
        let (data_channel_tx, data_channel_rx) =
            match common::connect(HOST, SIGNAL_PORT, session_id, *endpoint_id, peer_connection)
                .await
            {
                Ok(ok) => ok,
                Err(err) => {
                    error!("{}/{}: error {}", session_id, endpoint_id, err);
                    return Err(err.into());
                }
            };
        data_channels.push((data_channel_tx, data_channel_rx));
    }

    let rtp_transceiver = match common::add_track(
        &peer_connections[0].1,
        MIME_TYPE_VP8,
        "video_track",
        RTCRtpTransceiverDirection::Sendonly,
    )
    .await
    {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}/{}: error {}", session_id, peer_connections[0].0, err);
            return Err(err.into());
        }
    };

    // Read incoming RTCP packets
    // Before these packets are returned they are processed by interceptors. For things
    // like NACK this needs to be called.
    tokio::spawn(async move {
        let rtp_sender = rtp_transceiver.sender().await;
        while let Ok((rtcp_packets, _)) = rtp_sender.read_rtcp().await {
            info!("received RTCP packets {:?}", rtcp_packets);
            //TODO: check RTCP report and handle cancel
        }
    });

    match common::renegotiate(
        HOST,
        SIGNAL_PORT,
        session_id,
        peer_connections[0].0,
        &peer_connections[0].1,
        Some(&data_channels[0].0),
    )
    .await
    {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}/{}: error {}", session_id, peer_connections[0].0, err);
            return Err(err.into());
        }
    };

    // waiting for answer SDP from data channel of endpoint 0
    let answer_sdp = data_channels[0].1.recv().await;
    if let Some(answer_sdp) = answer_sdp {
        assert_eq!(RTCSdpType::Answer, answer_sdp.sdp_type);
    } else {
        assert!(false);
    }

    // waiting for offer SDP from data channel of endpoint 1
    let offer_sdp = data_channels[1].1.recv().await;
    if let Some(offer_sdp) = offer_sdp {
        assert_eq!(RTCSdpType::Offer, offer_sdp.sdp_type);
    } else {
        assert!(false);
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
