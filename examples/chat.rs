//use async_broadcast::broadcast;
use clap::Parser;
use log::info;
use std::io::Write;

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

    info!(
        "listening {}@{}(signal)/[{}-{}](media)...",
        cli.host, cli.signal_port, cli.media_port_min, cli.media_port_max
    );

    let _media_ports: Vec<u16> = (cli.media_port_min..=cli.media_port_max).collect();
    //let (_stop_tx, mut _stop_rx) = broadcast::<()>(1);
    //let mut port2thread_map = HashMap::new();

    //let key_pair = rcgen::KeyPair::generate(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    // let certificate = RTCCertificate::from_key_pair(key_pair)?;
    // fingerprints = certificate.get_fingerprints();

    /*let (cancel_tx, signal_cancel_rx) = broadcast::channel(1);
    let rtc_cancel_rx = cancel_tx.subscribe();

    let server_states = Arc::new(ServerStates::new());

    let mut signal_done_rx = signal::http_sdp_server(
        cli.host.clone(),
        cli.signal_port,
        server_states.clone(),
        signal_cancel_rx,
    )
    .await;

    /*let mut rtc_done_rx = udp_rtc_server(
        cli.host,
        cli.media_port,
        server_states.clone(),
        rtc_cancel_rx,
    )
    .await;*/

    info!("Press ctrl-c to stop");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            let _ = cancel_tx.send(());
            let _ = rtc_done_rx.recv().await;
            let _ = signal_done_rx.recv().await;
        }
    };*/

    Ok(())
}
