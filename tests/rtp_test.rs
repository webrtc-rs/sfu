use crate::common::{HOST, SIGNAL_PORT};
use bytes::Bytes;
use log::{error, info};
use rand::random;
use shared::error::Error;
use webrtc::api::media_engine::MIME_TYPE_VP8;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::sdp_type::RTCSdpType;
use webrtc::rtp::header::{Extension, Header};
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::TrackLocalWriter;

// importing common module.
mod common;

#[tokio::test]
async fn test_rtp_uni_direction_0sendonly_1recvonly() -> anyhow::Result<()> {
    // Prepare the configuration
    let endpoint_count: usize = 2;
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

    let mut data_channels = vec![];
    for (endpoint_id, peer_connection) in peer_connections.iter().enumerate() {
        let (data_channel_tx, data_channel_rx) = match common::connect(
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
        data_channels.push((data_channel_tx, data_channel_rx));
    }

    let (rtp_sender, track_local) = match common::add_track(
        &peer_connections[0],
        MIME_TYPE_VP8,
        "video_track",
        RTCRtpTransceiverDirection::Sendonly,
    )
    .await
    {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}/{}: error {}", session_id, endpoint_ids[0], err);
            return Err(err.into());
        }
    };

    // Read incoming RTCP packets
    // Before these packets are returned they are processed by interceptors. For things
    // like NACK this needs to be called.
    tokio::spawn(async move {
        while let Ok((rtcp_packets, _)) = rtp_sender.read_rtcp().await {
            info!("received RTCP packets {:?}", rtcp_packets);
            //TODO: check RTCP report and handle cancel
        }
    });

    let mut track_remote_rx = match common::on_track(&peer_connections[1]).await {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}/{}: error {}", session_id, endpoint_ids[1], err);
            return Err(err.into());
        }
    };

    match common::renegotiate(
        HOST,
        SIGNAL_PORT,
        session_id,
        endpoint_ids[0] as u64,
        &peer_connections[0],
        Some(&data_channels[0].0),
    )
    .await
    {
        Ok(ok) => ok,
        Err(err) => {
            error!("{}/{}: error {}", session_id, endpoint_ids[0], err);
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

    let mut send_rtp_packet = webrtc::rtp::packet::Packet {
        header: Header {
            version: 2,
            padding: false,
            extension: true,
            marker: true,
            payload_type: 96,
            sequence_number: 0,
            timestamp: 3653407706,
            ssrc: 476325762,
            csrc: vec![],
            extension_profile: 1,
            extensions: vec![Extension {
                id: 0,
                payload: Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF]),
            }],
            ..Default::default()
        },
        payload: Bytes::from_static(&[0x98, 0x36, 0xbe, 0x88, 0x9e]),
    };

    if let Err(err) = track_local.write_rtp(&send_rtp_packet).await {
        error!("write_sample: {err}");
        assert!(false);
        return Err(err.into());
    }

    // waiting for track_remote for endpoint 1
    let track_remote = match track_remote_rx.recv().await {
        Some(track_remote) => track_remote,
        None => {
            assert!(false);
            return Err(Error::Other("track remote rx close".to_string()).into());
        }
    };

    match track_remote.read_rtp().await {
        Ok((recv_rtp_packet, _)) => {
            assert_eq!(
                send_rtp_packet.header.sequence_number,
                recv_rtp_packet.header.sequence_number
            );
            assert_eq!(send_rtp_packet.payload, recv_rtp_packet.payload);
        }
        Err(err) => {
            error!("read_rtp error {:?}", err);
            assert!(false);
            return Err(err.into());
        }
    }

    // Verify track_local and track_remote match
    for i in 1..100 {
        send_rtp_packet.header.sequence_number = i;
        if let Err(err) = track_local.write_rtp(&send_rtp_packet).await {
            error!("write_sample: {err}");
            assert!(false);
            return Err(err.into());
        }

        match track_remote.read_rtp().await {
            Ok((recv_rtp_packet, _)) => {
                assert_eq!(
                    send_rtp_packet.header.sequence_number,
                    recv_rtp_packet.header.sequence_number
                );
                assert_eq!(send_rtp_packet.payload, recv_rtp_packet.payload);
            }
            Err(err) => {
                error!("read_rtp error {:?}", err);
                assert!(false);
                return Err(err.into());
            }
        }
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

async fn test_rtp_bi_direction_sendrecv(endpoint_count: usize) -> anyhow::Result<()> {
    // Prepare the configuration
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

    let mut data_channels = vec![];
    for (endpoint_id, peer_connection) in peer_connections.iter().enumerate() {
        let (data_channel_tx, data_channel_rx) = match common::connect(
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
        data_channels.push((data_channel_tx, data_channel_rx));
    }

    let mut tracks = vec![];
    for (endpoint_id, peer_connection) in peer_connections.iter().enumerate() {
        let (rtp_sender, track_local) = match common::add_track(
            peer_connection,
            MIME_TYPE_VP8,
            "video_track",
            RTCRtpTransceiverDirection::Sendonly,
        )
        .await
        {
            Ok(ok) => ok,
            Err(err) => {
                error!("{}/{}: error {}", session_id, endpoint_id, err);
                return Err(err.into());
            }
        };
        // Read incoming RTCP packets
        // Before these packets are returned they are processed by interceptors. For things
        // like NACK this needs to be called.
        tokio::spawn(async move {
            while let Ok((rtcp_packets, _)) = rtp_sender.read_rtcp().await {
                info!(
                    "{}/{}: received RTCP packets {:?}",
                    session_id, endpoint_id, rtcp_packets
                );
                //TODO: check RTCP report and handle cancel
            }
        });

        let track_remote_rx = match common::on_track(peer_connection).await {
            Ok(ok) => ok,
            Err(err) => {
                error!("{}/{}: error {}", session_id, endpoint_id, err);
                return Err(err.into());
            }
        };

        tracks.push((track_local, track_remote_rx));

        match common::renegotiate(
            HOST,
            SIGNAL_PORT,
            session_id,
            endpoint_id as u64,
            peer_connection,
            Some(&data_channels[endpoint_id].0),
        )
        .await
        {
            Ok(ok) => ok,
            Err(err) => {
                error!("{}/{}: error {}", session_id, endpoint_id, err);
                return Err(err.into());
            }
        };

        // waiting for answer SDP from data channel of endpoint_id
        let answer_sdp = data_channels[endpoint_id].1.recv().await;
        if let Some(answer_sdp) = answer_sdp {
            assert_eq!(RTCSdpType::Answer, answer_sdp.sdp_type);
        } else {
            assert!(false);
        }

        // waiting for offer SDP from data channel of other endpoint_ids
        for &other_endpoint_id in endpoint_ids.iter() {
            if other_endpoint_id == endpoint_id {
                continue;
            }

            let offer_sdp = data_channels[other_endpoint_id].1.recv().await;
            if let Some(offer_sdp) = offer_sdp {
                assert_eq!(RTCSdpType::Offer, offer_sdp.sdp_type);
            } else {
                assert!(false);
            }
        }
    }

    let mut send_rtp_packet = webrtc::rtp::packet::Packet {
        header: Header {
            version: 2,
            padding: false,
            extension: true,
            marker: true,
            payload_type: 96,
            sequence_number: 0,
            timestamp: 3653407706,
            ssrc: 476325762,
            csrc: vec![],
            extension_profile: 1,
            extensions: vec![Extension {
                id: 0,
                payload: Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF]),
            }],
            ..Default::default()
        },
        payload: Bytes::from_static(&[0x98, 0x36, 0xbe, 0x88, 0x9e]),
    };

    let mut track_remotes = vec![];
    for _ in endpoint_ids.iter() {
        let mut track_remote = vec![];
        for _ in endpoint_ids.iter() {
            track_remote.push(None);
        }
        track_remotes.push(track_remote);
    }

    for &endpoint_id in endpoint_ids.iter() {
        if let Err(err) = tracks[endpoint_id].0.write_rtp(&send_rtp_packet).await {
            error!("write_sample: {err}");
            assert!(false);
            return Err(err.into());
        }

        for &other_endpoint_id in endpoint_ids.iter() {
            if other_endpoint_id == endpoint_id {
                continue;
            }

            // waiting for track_remote for other endpoint id
            let track_remote = match tracks[other_endpoint_id].1.recv().await {
                Some(track_remote) => track_remote,
                None => {
                    assert!(false);
                    return Err(Error::Other("track remote rx close".to_string()).into());
                }
            };

            match track_remote.read_rtp().await {
                Ok((recv_rtp_packet, _)) => {
                    assert_eq!(
                        send_rtp_packet.header.sequence_number,
                        recv_rtp_packet.header.sequence_number
                    );
                    assert_eq!(send_rtp_packet.payload, recv_rtp_packet.payload);
                }
                Err(err) => {
                    error!("read_rtp error {:?}", err);
                    assert!(false);
                    return Err(err.into());
                }
            }

            track_remotes[other_endpoint_id][endpoint_id] = Some(track_remote);
        }
    }

    // Verify track_local and track_remote match
    for sequence_number in 1..100 {
        send_rtp_packet.header.sequence_number = sequence_number;
        for &endpoint_id in endpoint_ids.iter() {
            if let Err(err) = tracks[endpoint_id].0.write_rtp(&send_rtp_packet).await {
                error!("write_sample: {err}");
                assert!(false);
                return Err(err.into());
            }

            for &other_endpoint_id in endpoint_ids.iter() {
                if other_endpoint_id == endpoint_id {
                    continue;
                }
                match track_remotes[other_endpoint_id][endpoint_id]
                    .as_ref()
                    .unwrap()
                    .read_rtp()
                    .await
                {
                    Ok((recv_rtp_packet, _)) => {
                        assert_eq!(
                            send_rtp_packet.header.sequence_number,
                            recv_rtp_packet.header.sequence_number
                        );
                        assert_eq!(send_rtp_packet.payload, recv_rtp_packet.payload);
                    }
                    Err(err) => {
                        error!("read_rtp error {:?}", err);
                        assert!(false);
                        return Err(err.into());
                    }
                }
            }
        }
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

#[tokio::test]
async fn test_rtp_2p_bi_direction_sendrecv() -> anyhow::Result<()> {
    let endpoint_count: usize = 2;
    test_rtp_bi_direction_sendrecv(endpoint_count).await?;
    Ok(())
}

#[tokio::test]
async fn test_rtp_3p_bi_direction_sendrecv() -> anyhow::Result<()> {
    let endpoint_count: usize = 3;
    test_rtp_bi_direction_sendrecv(endpoint_count).await?;
    Ok(())
}
