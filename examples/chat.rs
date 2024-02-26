#[macro_use]
extern crate tracing;

use clap::Parser;
use rouille::Server;
use sfu::{RTCCertificate, ServerConfig};
use std::collections::HashMap;
use std::io::Write;
use std::net::UdpSocket;
use std::sync::mpsc::{self};
use std::sync::Arc;

mod sfu_impl;
mod str0m_impl;
mod util;

use sfu_impl::*;
use str0m_impl::*;

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
    //#[arg(long, default_value_t = format!("127.0.0.1"))]
    //host: String,
    #[arg(long, default_value_t = 8080)]
    signal_port: u16,
    #[arg(long, default_value_t = 3478)]
    media_port_min: u16,
    #[arg(long, default_value_t = 3495)]
    media_port_max: u16,

    #[arg(short, long)]
    sfu_impl: bool,
    #[arg(short, long)]
    debug: bool,
    #[arg(short, long, default_value_t = Level::Info)]
    #[clap(value_enum)]
    level: Level,
}

pub fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    init_log(cli.debug, cli.level);

    let certificate = include_bytes!("cer.pem").to_vec();
    let private_key = include_bytes!("key.pem").to_vec();

    // Figure out some public IP address, since Firefox will not accept 127.0.0.1 for WebRTC traffic.
    let host_addr = util::select_host_address();

    // Figure out some public IP address, since Firefox will not accept 127.0.0.1 for WebRTC traffic.
    let media_ports: Vec<u16> = (cli.media_port_min..=cli.media_port_max).collect();
    let mut media_port_thread_map = HashMap::new();
    let key_pair = rcgen::KeyPair::generate(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let server_config = Arc::new(ServerConfig::new(vec![RTCCertificate::from_key_pair(
        key_pair,
    )?]));

    for port in media_ports {
        //let worker = wait_group.worker();
        //let host = host_addr; //cli.host.clone();
        //let mut stop_rx = stop_rx.clone();
        let (signaling_tx, signaling_rx) = mpsc::sync_channel(1);
        let (signaling_msg_tx, signaling_msg_rx) = mpsc::sync_channel(1);

        // Spin up a UDP socket for the RTC. All WebRTC traffic is going to be multiplexed over this single
        // server socket. Clients are identified via their respective remote (UDP) socket address.
        let socket = UdpSocket::bind(format!("{host_addr}:{port}"))
            .expect(&format!("binding to {host_addr}:{port}"));

        if cli.sfu_impl {
            media_port_thread_map.insert(port, (None, Some(signaling_msg_tx)));
            let server_config = server_config.clone();
            // The run loop is on a separate thread to the web server.
            std::thread::spawn(move || run_sfu(socket, signaling_msg_rx, server_config));
        } else {
            media_port_thread_map.insert(port, (Some(signaling_tx), None));
            // The run loop is on a separate thread to the web server.
            std::thread::spawn(move || run_str0m(socket, signaling_rx));
        }
    }

    // The run loop is on a separate thread to the web server.
    //thread::spawn(move || run(socket, rx));
    let media_port_thread_map = Arc::new(media_port_thread_map);

    let signal_port = cli.signal_port;
    let use_sfu_impl = cli.sfu_impl;
    let server = Server::new_ssl(
        format!("{}:{}", host_addr, signal_port),
        move |request| {
            if use_sfu_impl {
                web_request_sfu(request, media_port_thread_map.clone())
            } else {
                web_request_str0m(request, host_addr, media_port_thread_map.clone())
            }
        },
        certificate,
        private_key,
    )
    .expect("starting the web server");

    let port = server.server_addr().port();
    info!("Connect a browser to https://{}:{}", host_addr, port);

    server.run();

    Ok(())
}

fn init_log(debug: bool, level: Level) {
    use std::env;
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    if debug {
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
            .filter(None, level.into())
            .init();
    } else {
        if env::var("RUST_LOG").is_err() {
            env::set_var("RUST_LOG", "chat=info,str0m=info");
        }

        tracing_subscriber::registry()
            .with(fmt::layer())
            .with(EnvFilter::from_default_env())
            .init();
    }
}
