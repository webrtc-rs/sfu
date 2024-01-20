use crate::signal::{handle_signaling_message, SignalingMessage, SignalingServer};
use async_broadcast::broadcast;
use clap::Parser;
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use log::{error, info};
use retty::bootstrap::BootstrapUdpServer;
use retty::channel::Pipeline;
use retty::executor::LocalExecutorBuilder;
use retty::transport::{AsyncTransport, AsyncTransportWrite, TaggedBytesMut};
use sfu::handlers::demuxer::DemuxerHandler;
use sfu::handlers::dtls::DtlsHandler;
use sfu::handlers::gateway::GatewayHandler;
use sfu::handlers::stun::StunHandler;
use sfu::server::certificate::RTCCertificate;
use sfu::server::config::ServerConfig;
use sfu::server::states::ServerStates;
use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use waitgroup::WaitGroup;

mod signal;

extern crate num_cpus;

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
    #[arg(long, default_value_t = format!("0.0.0.0"))]
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
    let server_config = Arc::new(ServerConfig::new(vec![RTCCertificate::from_key_pair(
        key_pair,
    )?]));
    let core_num = num_cpus::get();
    let wait_group = WaitGroup::new();

    for port in media_ports {
        let worker = wait_group.worker();
        let host = cli.host.clone();
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
                let dtls_handshake_config = dtls::config::ConfigBuilder::default()
                    .with_certificates(server_config.certificates.iter().map(|c| c.dtls_certificate.clone()).collect())
                    .with_srtp_protection_profiles(vec![
                        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
                    ])
                    .with_extended_master_secret(dtls::config::ExtendedMasterSecretType::Require)
                    .build(false, None)
                    .unwrap();
                let server_states = Rc::new(ServerStates::new(server_config,
                                                              SocketAddr::from_str(&format!("{}:{}", host, port)).unwrap()).unwrap());

                info!("listening {}:{}...", host, port);

                let server_states_moved = server_states.clone();
                let dtls_handshake_config_moved = dtls_handshake_config.clone();
                let mut bootstrap = BootstrapUdpServer::new();
                bootstrap.pipeline(Box::new(
                    move |writer: AsyncTransportWrite<TaggedBytesMut>| {
                        let pipeline: Pipeline<TaggedBytesMut, TaggedBytesMut> = Pipeline::new();

                        let async_transport_handler = AsyncTransport::new(writer);
                        let demuxer_handler = DemuxerHandler::new();
                        let stun_handler = StunHandler::new();
                        let dtls_handler = DtlsHandler::new(Rc::clone(&server_states_moved), dtls_handshake_config_moved.clone());
                        //TODO: add DTLS and RTP handlers                        
                        let gateway_handler = GatewayHandler::new(Rc::clone(&server_states_moved));

                        pipeline.add_back(async_transport_handler);
                        pipeline.add_back(demuxer_handler);
                        pipeline.add_back(stun_handler);
                        pipeline.add_back(dtls_handler);
                        //TODO: add DTLS and RTP handlers
                        pipeline.add_back(gateway_handler);

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
