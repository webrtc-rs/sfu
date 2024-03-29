#![allow(dead_code)]

use anyhow::Result;
use hyper::{Body, Client, Method, Request};
use log::LevelFilter::Debug;
use log::{error, info};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::Notify;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::sdp_type::RTCSdpType;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_remote::TrackRemote;

pub const HOST: &'static str = "127.0.0.1";
pub const SIGNAL_PORT: u16 = 8080;

fn pretty_sdp(input: &str) -> String {
    input.replace("\\r\\n", "\n")
}

pub async fn setup_peer_connection(
    config: RTCConfiguration,
    endpoint_id: u64,
) -> Result<Arc<RTCPeerConnection>> {
    let _ = env_logger::Builder::new()
        .format(|buf, record| {
            writeln!(
                buf,
                "{}:{} [{}] {} - {}",
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.level(),
                chrono::Local::now().format("%H:%M:%S.%6f"),
                record.args()
            )
        })
        .filter(None, Debug)
        .try_init();

    // some setup code, like creating required files/directories, starting
    // servers, etc.
    info!("setup_peer_connection {}", endpoint_id);

    // Create a MediaEngine object to configure the supported codec
    let mut m = MediaEngine::default();

    // Register default codecs
    m.register_default_codecs()?;

    // Create a InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
    // This provides NACKs, RTCP Reports and other features. If you use `webrtc.NewPeerConnection`
    // this is enabled by default. If you are manually managing You MUST create a InterceptorRegistry
    // for each PeerConnection.
    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut m)?;

    // Create the API object with the MediaEngine
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    // Create a new RTCPeerConnection
    let peer_connection = Arc::new(api.new_peer_connection(config).await?);

    // Set the handler for Peer connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        info!("Peer Connection State has changed: {s}");

        if s == RTCPeerConnectionState::Failed {
            // Wait until PeerConnection has had no network activity for 30 seconds or another failure. It may be reconnected using an ICE Restart.
            // Use webrtc.PeerConnectionStateDisconnected if you are interested in detecting faster timeout.
            // Note that the PeerConnection may come back from PeerConnectionStateDisconnected.
            error!("Peer Connection has gone to failed exiting");
            assert!(false);
        }

        Box::pin(async {})
    }));

    Ok(peer_connection)
}

pub async fn setup_peer_connections(
    configs: Vec<RTCConfiguration>,
    endpoint_ids: &[usize],
) -> Result<Vec<Arc<RTCPeerConnection>>> {
    assert_eq!(configs.len(), endpoint_ids.len());

    let mut peer_connections = Vec::with_capacity(configs.len());

    for (config, endpoint_id) in configs.into_iter().zip(endpoint_ids) {
        let peer_connection = setup_peer_connection(config, *endpoint_id as u64).await?;
        peer_connections.push(peer_connection);
    }

    Ok(peer_connections)
}

pub async fn teardown_peer_connection(pc: Arc<RTCPeerConnection>) -> Result<()> {
    pc.close().await?;

    Ok(())
}

pub async fn teardown_peer_connections(pcs: Vec<Arc<RTCPeerConnection>>) -> Result<()> {
    for pc in pcs {
        teardown_peer_connection(pc).await?;
    }

    Ok(())
}

async fn signaling(
    host: &str,
    signal_port: u16,
    session_id: u64,
    endpoint_id: u64,
    offer_payload: String,
) -> Result<RTCSessionDescription> {
    info!("connecting to signaling server http://{host}:{signal_port}/offer/{session_id}/{endpoint_id}");
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!(
            "http://{host}:{signal_port}/offer/{session_id}/{endpoint_id}"
        ))
        .header("content-type", "application/json; charset=utf-8")
        .body(Body::from(offer_payload))?;

    let resp = Client::new().request(req).await?;
    let answer_payload =
        std::str::from_utf8(&hyper::body::to_bytes(resp.into_body()).await?)?.to_string();
    info!(
        "{}/{}: answer sdp {}",
        session_id,
        endpoint_id,
        pretty_sdp(&answer_payload)
    );
    let answer = serde_json::from_str::<RTCSessionDescription>(&answer_payload)?;

    Ok(answer)
}

pub async fn renegotiate(
    host: &str,
    signal_port: u16,
    session_id: u64,
    endpoint_id: u64,
    peer_connection: &Arc<RTCPeerConnection>,
    data_channel: Option<&Arc<RTCDataChannel>>,
) -> Result<()> {
    // Create an offer to send to the other process
    let offer = peer_connection.create_offer(None).await?;

    // Send our offer to the HTTP server listening in the other process
    let offer_payload = serde_json::to_string(&offer)?;
    info!(
        "{}/{}: offer sdp {}",
        session_id,
        endpoint_id,
        pretty_sdp(&offer_payload)
    );

    // Sets the LocalDescription, and starts our UDP listeners
    // Note: this will start the gathering of ICE candidates
    peer_connection.set_local_description(offer).await?;

    if let Some(data_channel) = data_channel {
        data_channel.send_text(offer_payload).await?;
    } else {
        let answer = signaling(host, signal_port, session_id, endpoint_id, offer_payload).await?;
        peer_connection.set_remote_description(answer).await?;
    }

    Ok(())
}

pub async fn connect(
    host: &str,
    signal_port: u16,
    session_id: u64,
    endpoint_id: u64,
    peer_connection: &Arc<RTCPeerConnection>,
) -> Result<(
    Arc<RTCDataChannel>,
    UnboundedReceiver<RTCSessionDescription>,
)> {
    // Create a datachannel with label 'data'
    let data_channel = peer_connection.create_data_channel("data", None).await?;

    // Register channel opening handling
    let data_channel_opened_notify_tx = Arc::new(Notify::new());
    let data_channel_opened_ready_notify_rx = data_channel_opened_notify_tx.clone();
    data_channel.on_open(Box::new(move || {
        info!("DataChannel is opened");
        data_channel_opened_notify_tx.notify_waiters();
        Box::pin(async {})
    }));

    // Register SDP message handling
    let (data_channel_tx, data_channel_rx) =
        tokio::sync::mpsc::unbounded_channel::<RTCSessionDescription>();
    let peer_connection_clone = peer_connection.clone();
    let data_channel_clone = data_channel.clone();
    data_channel.on_message(Box::new(move |msg: DataChannelMessage| {
        let sdp_str = String::from_utf8(msg.data.to_vec()).unwrap();
        info!(
            "{}/{}: SDP from DataChannel: {}",
            session_id,
            endpoint_id,
            pretty_sdp(&sdp_str)
        );
        let sdp = match serde_json::from_str::<RTCSessionDescription>(&sdp_str) {
            Ok(sdp) => sdp,
            Err(err) => {
                error!("deserialize sdp str failed: {}", err);
                assert!(false);
                return Box::pin(async {});
            }
        };
        let pc = peer_connection_clone.clone();
        let dc = data_channel_clone.clone();
        let tx = data_channel_tx.clone();
        Box::pin(async move {
            match sdp.sdp_type {
                RTCSdpType::Offer => {
                    if let Err(err) = pc.set_remote_description(sdp.clone()).await {
                        error!("set_remote_description offer error {:?}", err);
                        assert!(false);
                        return;
                    }

                    // Create an answer to send to the other process
                    let answer = match pc.create_answer(None).await {
                        Ok(a) => a,
                        Err(err) => {
                            error!("create_answer error {:?}", err);
                            assert!(false);
                            return;
                        }
                    };

                    let answer_str = match serde_json::to_string(&answer) {
                        Ok(a) => a,
                        Err(err) => {
                            error!("serialize answer error {:?}", err);
                            assert!(false);
                            return;
                        }
                    };
                    info!(
                        "{}/{}: SDP to DataChannel: '{}'",
                        session_id,
                        endpoint_id,
                        pretty_sdp(&answer_str)
                    );

                    // Sets the LocalDescription, and starts our UDP listeners
                    if let Err(err) = pc.set_local_description(answer).await {
                        error!("create_answer error {:?}", err);
                        assert!(false);
                        return;
                    }

                    if let Err(err) = dc.send_text(answer_str).await {
                        error!("data channel send answer error {:?}", err);
                        assert!(false);
                        return;
                    }
                }
                RTCSdpType::Answer => {
                    if let Err(err) = pc.set_remote_description(sdp.clone()).await {
                        error!("set_remote_description answer error {:?}", err);
                        assert!(false);
                        return;
                    }
                }
                _ => {
                    error!("Unsupported SDP type {}", sdp.sdp_type);
                    assert!(false);
                }
            };
            if let Err(err) = tx.send(sdp) {
                error!("data_channel_tx send error {}", err);
                assert!(false);
            }
        })
    }));

    renegotiate(
        host,
        signal_port,
        session_id,
        endpoint_id,
        peer_connection,
        None,
    )
    .await?;

    let ice_ready_notify_tx = Arc::new(Notify::new());
    let ice_ready_notify_rx = ice_ready_notify_tx.clone();

    // Set the handler for ICE connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_ice_connection_state_change(Box::new(
        move |connection_state: RTCIceConnectionState| {
            info!("Connection State has changed {connection_state}");
            if connection_state == RTCIceConnectionState::Connected {
                ice_ready_notify_tx.notify_waiters();
            }
            Box::pin(async {})
        },
    ));

    // Wait for connection established
    ice_ready_notify_rx.notified().await;

    // Wait for data channel opened
    data_channel_opened_ready_notify_rx.notified().await;

    Ok((data_channel, data_channel_rx))
}

pub async fn add_track(
    peer_connection: &Arc<RTCPeerConnection>,
    mime_type: &str,
    track_id: &str,
    direction: RTCRtpTransceiverDirection,
) -> Result<(Arc<RTCRtpSender>, Arc<TrackLocalStaticRTP>)> {
    // Create a video track
    let track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: mime_type.to_owned(),
            ..Default::default()
        },
        track_id.to_owned(),
        "webrtc-rs".to_owned(),
    ));

    // Add this newly created track to the PeerConnection
    let rtp_transceiver = peer_connection
        .add_transceiver_from_track(
            Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>,
            Some(RTCRtpTransceiverInit {
                direction,
                send_encodings: vec![],
            }),
        )
        .await?;

    Ok((rtp_transceiver.sender().await, track))
}

pub async fn on_track(
    peer_connection: &Arc<RTCPeerConnection>,
) -> Result<UnboundedReceiver<Arc<TrackRemote>>> {
    let (track_tx, track_rx) = tokio::sync::mpsc::unbounded_channel::<Arc<TrackRemote>>();
    peer_connection.on_track(Box::new(move |track, _, _| {
        let tx = track_tx.clone();
        Box::pin(async move {
            if let Err(err) = tx.send(track) {
                error!("track_tx send error {}", err);
                assert!(false);
            }
        })
    }));

    Ok(track_rx)
}
