use retty::bootstrap::BootstrapUdpServer;
use retty::channel::Pipeline;
use retty::runtime::default_runtime;
use retty::transport::{AsyncTransportUdp, AsyncTransportWrite, TaggedBytesMut};

use crate::rtc::server::{
    codec::{demuxer_handler::DemuxerHandler, ice_handler::ICEHandler},
    ServerStates,
};

use log::{error, info};
use std::sync::Arc;
use tokio::sync::broadcast;

/// udp_rtc_server starts a RTC Server on top of UDP
pub async fn udp_rtc_server(
    host: String,
    port: u16,
    server_states: Arc<ServerStates>,
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

                        pipeline.add_back(AsyncTransportUdp::new(sock)).await;
                        pipeline
                            .add_back(DemuxerHandler::new(server_states.clone()))
                            .await;
                        pipeline
                            .add_back(ICEHandler::new(server_states.clone()))
                            .await;
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
