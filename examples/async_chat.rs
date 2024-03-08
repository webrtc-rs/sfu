extern crate num_cpus;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_broadcast::{broadcast, Receiver};
use clap::Parser;
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use log::{error, info};
use opentelemetry::{/*global,*/ metrics::MeterProvider, KeyValue};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::{runtime, Resource};
use opentelemetry_stdout::MetricsExporterBuilder;
use retty::bootstrap::BootstrapUdpServer;
use retty::channel::Pipeline;
use retty::executor::LocalExecutorBuilder;
use retty::transport::TaggedBytesMut;
use waitgroup::{WaitGroup, Worker};

use sfu::{
    DataChannelHandler, DemuxerHandler, DtlsHandler, ExceptionHandler, GatewayHandler,
    InterceptorHandler, RTCCertificate, SctpHandler, ServerConfig, ServerStates, SrtpHandler,
    StunHandler,
};

mod async_signal;

use async_signal::*;

#[derive(Default, Debug, Copy, Clone, clap::ValueEnum)]
enum Level {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl From<Level> for log::LevelFilter {
    fn from(level: Level) -> Self {
        match level {
            Level::Error => log::LevelFilter::Error,
            Level::Warn => log::LevelFilter::Warn,
            Level::Info => log::LevelFilter::Info,
            Level::Debug => log::LevelFilter::Debug,
            Level::Trace => log::LevelFilter::Trace,
        }
    }
}

#[derive(Parser)]
#[command(name = "SFU Server")]
#[command(author = "Rusty Rain <y@ngr.tc>")]
#[command(version = "0.1.0")]
#[command(about = "An example of SFU Server", long_about = None)]
struct Cli {
    #[arg(long, default_value_t = format!("127.0.0.1"))]
    host: String,
    #[arg(short, long, default_value_t = 8080)]
    signal_port: u16,
    #[arg(long, default_value_t = 3478)]
    media_port_min: u16,
    #[arg(long, default_value_t = 3495)]
    media_port_max: u16,

    #[arg(short, long)]
    debug: bool,
    #[arg(short, long, default_value_t = Level::Info)]
    #[clap(value_enum)]
    level: Level,
}

fn init_meter_provider(mut stop_rx: Receiver<()>, worker: Worker) -> SdkMeterProvider {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();

        rt.block_on(async move {
            let _worker = worker;
            let exporter = MetricsExporterBuilder::default()
                .with_encoder(|writer, data| {
                    Ok(serde_json::to_writer_pretty(writer, &data).unwrap())
                })
                .build();
            let reader = PeriodicReader::builder(exporter, runtime::TokioCurrentThread)
                .with_interval(Duration::from_secs(30))
                .build();
            let meter_provider = SdkMeterProvider::builder()
                .with_reader(reader)
                .with_resource(Resource::new(vec![KeyValue::new("chat", "metrics")]))
                .build();
            let _ = tx.send(meter_provider.clone());

            let _ = stop_rx.recv().await;
            let _ = meter_provider.shutdown();
            info!("meter provider is gracefully down");
        });
    });

    let meter_provider = rx.recv().unwrap();
    meter_provider
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.debug {
        env_logger::Builder::new()
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
            .filter(None, cli.level.into())
            .init();
    }

    println!(
        "listening {}:{}(signal)/[{}-{}](media)...",
        cli.host, cli.signal_port, cli.media_port_min, cli.media_port_max
    );

    let media_ports: Vec<u16> = (cli.media_port_min..=cli.media_port_max).collect();
    let (stop_tx, mut stop_rx) = broadcast::<()>(1);
    let mut media_port_thread_map = HashMap::new();

    let key_pair = rcgen::KeyPair::generate(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let certificates = vec![RTCCertificate::from_key_pair(key_pair)?];
    let dtls_handshake_config = Arc::new(
        dtls::config::ConfigBuilder::default()
            .with_certificates(
                certificates
                    .iter()
                    .map(|c| c.dtls_certificate.clone())
                    .collect(),
            )
            .with_srtp_protection_profiles(vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80])
            .with_extended_master_secret(dtls::config::ExtendedMasterSecretType::Require)
            .build(false, None)?,
    );
    let sctp_endpoint_config = Arc::new(sctp::EndpointConfig::default());
    let sctp_server_config = Arc::new(sctp::ServerConfig::default());
    let server_config = Arc::new(
        ServerConfig::new(certificates)
            .with_dtls_handshake_config(dtls_handshake_config)
            .with_sctp_endpoint_config(sctp_endpoint_config)
            .with_sctp_server_config(sctp_server_config),
    );
    let core_num = num_cpus::get();
    let wait_group = WaitGroup::new();
    let meter_provider = init_meter_provider(stop_rx.clone(), wait_group.worker());

    for port in media_ports {
        let worker = wait_group.worker();
        let host = cli.host.clone();
        let meter_provider = meter_provider.clone();
        let mut stop_rx = stop_rx.clone();
        let (signaling_tx, signaling_rx) = smol::channel::unbounded::<SignalingMessage>();
        media_port_thread_map.insert(port, signaling_tx);

        let server_config = server_config.clone();
        LocalExecutorBuilder::new()
            .name(format!("media_port_{}", port).as_str())
            .core_id(core_affinity::CoreId {
                id: (port as usize) % core_num,
            })
            .spawn(move || async move {
                let _worker = worker;
                let local_addr = SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap();
                let server_states = Rc::new(RefCell::new(
						ServerStates::new(server_config, local_addr,
						meter_provider.meter(format!("{}:{}", host, port)),
						).unwrap()
					));

                info!("listening {}:{}...", host, port);

                let server_states_moved = server_states.clone();
                let mut bootstrap = BootstrapUdpServer::new();
                bootstrap.pipeline(Box::new(
                    move || {
                        let pipeline: Pipeline<TaggedBytesMut, TaggedBytesMut> = Pipeline::new();

                        let demuxer_handler = DemuxerHandler::new();
                        let stun_handler = StunHandler::new();
                        // DTLS
                        let dtls_handler = DtlsHandler::new(local_addr, Rc::clone(&server_states_moved));
                        let sctp_handler = SctpHandler::new(local_addr, Rc::clone(&server_states_moved));
                        let data_channel_handler = DataChannelHandler::new();
                        // SRTP
                        let srtp_handler = SrtpHandler::new(Rc::clone(&server_states_moved));
                        let interceptor_handler = InterceptorHandler::new(Rc::clone(&server_states_moved));
                        // Gateway
                        let gateway_handler = GatewayHandler::new(Rc::clone(&server_states_moved));
                        let exception_handler = ExceptionHandler::new();

                        pipeline.add_back(demuxer_handler);
                        pipeline.add_back(stun_handler);
                        // DTLS
                        pipeline.add_back(dtls_handler);
                        pipeline.add_back(sctp_handler);
                        pipeline.add_back(data_channel_handler);
                        // SRTP
                        pipeline.add_back(srtp_handler);
                        pipeline.add_back(interceptor_handler);
                        // Gateway
                        pipeline.add_back(gateway_handler);
                        pipeline.add_back(exception_handler);

                        pipeline.finalize()
                    },
                ));

                if let Err(err) = bootstrap.bind(format!("{}:{}", host, port)).await {
                    error!("bootstrap binding error: {}", err);
                    return;
                }

                loop {
                    tokio::select! {
                        _ = stop_rx.recv() => {
                            info!("media server on {}:{} receives stop signal", host, port);
                            break;
                        }
                        recv = signaling_rx.recv() => {
                            match recv {
                                Ok(signaling_msg) => {
                                    if let Err(err) = handle_signaling_message(&server_states, signaling_msg) {
                                        error!("handle_signaling_message error: {}", err);
                                    }
                                }
                                Err(err) => {
                                    error!("signal_rx recv error: {}", err);
                                    break;
                                }
                            }
                        }
                    }
                }

                bootstrap.graceful_stop().await;
                info!("media server on {}:{} is gracefully down", host, port);
            })?;
    }

    let signaling_addr = SocketAddr::from_str(&format!("{}:{}", cli.host, cli.signal_port))?;
    let signaling_stop_rx = stop_rx.clone();
    let signaling_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();

        rt.block_on(async {
            let signaling_server = SignalingServer::new(signaling_addr, media_port_thread_map);
            let mut done_rx = signaling_server.run(signaling_stop_rx).await;
            let _ = done_rx.recv().await;
            wait_group.wait().await;
            info!("signaling server is gracefully down");
        })
    });

    LocalExecutorBuilder::default().run(async move {
        println!("Press Ctrl-C to stop");
        std::thread::spawn(move || {
            let mut stop_tx = Some(stop_tx);
            ctrlc::set_handler(move || {
                if let Some(stop_tx) = stop_tx.take() {
                    let _ = stop_tx.try_broadcast(());
                }
            })
            .expect("Error setting Ctrl-C handler");
        });
        let _ = stop_rx.recv().await;
        println!("Wait for Signaling Sever and Media Server Gracefully Shutdown...");
    });

    let _ = signaling_handle.join();

    Ok(())
}
