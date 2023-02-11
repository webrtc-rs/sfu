use retty::bootstrap::BootstrapUdpServer;
use retty::channel::{
    Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler, Pipeline,
};
use retty::codec::{
    byte_to_message_decoder::{LineBasedFrameDecoder, TaggedByteToMessageCodec, TerminatorType},
    string_codec::{TaggedString, TaggedStringCodec},
};
use retty::runtime::default_runtime;
use retty::transport::{AsyncTransportUdp, AsyncTransportWrite, TaggedBytesMut, TransportContext};

use async_trait::async_trait;
use log::{error, info};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

struct TaggedEchoDecoder {
    interval: Duration,
    timeout: Instant,
    last_transport: Option<TransportContext>,
}
struct TaggedEchoEncoder;
struct TaggedEchoHandler {
    decoder: TaggedEchoDecoder,
    encoder: TaggedEchoEncoder,
}

impl TaggedEchoHandler {
    fn new(interval: Duration) -> Self {
        TaggedEchoHandler {
            decoder: TaggedEchoDecoder {
                timeout: Instant::now() + interval,
                interval,
                last_transport: None,
            },
            encoder: TaggedEchoEncoder,
        }
    }
}

#[async_trait]
impl InboundHandler for TaggedEchoDecoder {
    type Rin = TaggedString;
    type Rout = Self::Rin;

    async fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        println!(
            "handling {} from {:?}",
            msg.message, msg.transport.peer_addr
        );
        if msg.message == "bye" {
            self.last_transport.take();
        } else {
            self.last_transport = Some(msg.transport);
            ctx.fire_write(TaggedString {
                transport: msg.transport,
                message: format!("{}\r\n", msg.message),
            })
            .await;
        }
    }

    async fn read_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, now: Instant) {
        if self.last_transport.is_some() && self.timeout <= now {
            println!("TaggedEchoHandler timeout at: {:?}", self.timeout);
            self.timeout = now + self.interval;
            if let Some(transport) = &self.last_transport {
                ctx.fire_write(TaggedString {
                    transport: *transport,
                    message: format!("Keep-alive message: next one at {:?}\r\n", self.timeout),
                })
                .await;
            }
        }

        //last handler, no need to fire_read_timeout
    }
    async fn poll_timeout(
        &mut self,
        _ctx: &InboundContext<Self::Rin, Self::Rout>,
        eto: &mut Instant,
    ) {
        if self.last_transport.is_some() && self.timeout < *eto {
            *eto = self.timeout;
        }

        //last handler, no need to fire_poll_timeout
    }
}

#[async_trait]
impl OutboundHandler for TaggedEchoEncoder {
    type Win = TaggedString;
    type Wout = Self::Win;

    async fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg).await;
    }
}

impl Handler for TaggedEchoHandler {
    type Rin = TaggedString;
    type Rout = Self::Rin;
    type Win = TaggedString;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "TaggedEchoHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.decoder), Box::new(self.encoder))
    }
}

/// udp_echo_server starts a Echo Server on top of UDP
pub async fn udp_echo_server(
    host: String,
    port: u16,
    _sdp_rx: mpsc::Receiver<String>,
    mut cancel_rx: broadcast::Receiver<()>,
) -> broadcast::Receiver<()> {
    let (done_tx, done_rx) = broadcast::channel(1);
    tokio::spawn(async move {
        let mut bootstrap = BootstrapUdpServer::new(default_runtime().unwrap());
        if let Err(err) = bootstrap
            .pipeline(Box::new(
                move |sock: Box<dyn AsyncTransportWrite + Send + Sync>| {
                    Box::pin(async move {
                        let pipeline: Pipeline<TaggedBytesMut, TaggedString> = Pipeline::new();

                        let async_transport_handler = AsyncTransportUdp::new(sock);
                        let line_based_frame_decoder_handler = TaggedByteToMessageCodec::new(
                            Box::new(LineBasedFrameDecoder::new(8192, true, TerminatorType::BOTH)),
                        );
                        let string_codec_handler = TaggedStringCodec::new();
                        let echo_handler = TaggedEchoHandler::new(Duration::from_secs(5));

                        pipeline.add_back(async_transport_handler).await;
                        pipeline.add_back(line_based_frame_decoder_handler).await;
                        pipeline.add_back(string_codec_handler).await;
                        pipeline.add_back(echo_handler).await;
                        pipeline.finalize().await;

                        Arc::new(pipeline)
                    })
                },
            ))
            .bind(format!("{}:{}", host, port))
            .await
        {
            error!("udp_echo_server error: {}", err);
            let _ = done_tx.send(());
        } else {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    bootstrap.stop().await;
                    info!("udp_echo_server receives cancel signal");
                    let _ = done_tx.send(());
                }
            };
        }
    });

    done_rx
}
