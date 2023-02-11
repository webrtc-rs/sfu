use retty::bootstrap::BootstrapUdpServer;
use retty::channel::Pipeline;
use retty::runtime::default_runtime;
use retty::transport::{AsyncTransportUdp, AsyncTransportWrite, TaggedBytesMut};

use crate::rtc::server::{server_states::ServerStates, udp_demuxer_handler::UDPDemuxerHandler};

use log::{error, info};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

/// udp_rtc_server starts a RTC Server on top of UDP
pub async fn udp_rtc_server(
    host: String,
    port: u16,
    server_states: Arc<ServerStates>,
    _sdp_rx: mpsc::Receiver<String>,
    mut cancel_rx: broadcast::Receiver<()>,
) -> broadcast::Receiver<()> {
    let (done_tx, done_rx) = broadcast::channel(1);

    tokio::spawn(async move {
        let mut bootstrap = BootstrapUdpServer::new(default_runtime().unwrap());
        if let Err(err) = bootstrap
            .pipeline(Box::new(
                move |sock: Box<dyn AsyncTransportWrite + Send + Sync>| {
                    let server_states = server_states.clone();
                    Box::pin(async move {
                        let pipeline: Pipeline<TaggedBytesMut, TaggedBytesMut> = Pipeline::new();

                        let async_transport_handler = AsyncTransportUdp::new(sock);
                        let udp_demuxer_handler = UDPDemuxerHandler::new(server_states);

                        pipeline.add_back(async_transport_handler).await;
                        pipeline.add_back(udp_demuxer_handler).await;
                        pipeline.finalize().await;
                        Arc::new(pipeline)
                    })
                },
            ))
            .bind(format!("{}:{}", host, port))
            .await
        {
            error!("udp_rtc_server error: {}", err);
            let _ = done_tx.send(());
        } else {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    bootstrap.stop().await;
                    info!("udp_rtc_server receives cancel signal");
                    let _ = done_tx.send(());
                }
            };
        }
    });

    done_rx
}
