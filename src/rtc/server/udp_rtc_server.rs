use retty::bootstrap::bootstrap_udp_server::BootstrapUdpServer;
use retty::channel::pipeline::Pipeline;
use retty::runtime::default_runtime;
use retty::transport::async_transport_udp::AsyncTransportUdp;
use retty::transport::{AsyncTransportWrite, TransportContext};
use std::sync::Arc;

use crate::rtc::server::server_states::ServerStates;
use crate::rtc::server::udp_demuxer::UDPDemuxer;

use log::{error, info};
use tokio::sync::{broadcast, mpsc, Mutex};

/// udp_rtc_server starts a RTC Server on top of UDP
pub async fn udp_rtc_server(
    host: String,
    port: u16,
    _sdp_rx: mpsc::Receiver<String>,
    mut cancel_rx: broadcast::Receiver<()>,
) -> broadcast::Receiver<()> {
    let (done_tx, done_rx) = broadcast::channel(1);
    let server_states = Arc::new(Mutex::new(ServerStates {}));

    tokio::spawn(async move {
        let mut bootstrap = BootstrapUdpServer::new(default_runtime().unwrap());
        if let Err(err) = bootstrap
            .pipeline(Box::new(
                move |sock: Box<dyn AsyncTransportWrite + Send + Sync>| {
                    let mut pipeline = Pipeline::new(TransportContext {
                        local_addr: sock.local_addr().unwrap(),
                        peer_addr: sock.peer_addr().ok(),
                    });

                    let async_transport_handler = AsyncTransportUdp::new(sock);
                    let udp_demuxer_handler = UDPDemuxer::new(server_states.clone());

                    pipeline.add_back(async_transport_handler);
                    pipeline.add_back(udp_demuxer_handler);

                    Box::pin(async move { pipeline.finalize().await })
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
