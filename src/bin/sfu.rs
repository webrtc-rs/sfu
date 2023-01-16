use clap::Parser;
use log::info;
use std::io::Write;
use tokio::sync::{broadcast, mpsc};

use sfu::{rtc, signal};

#[derive(Parser)]
#[command(name = "SFU Server")]
#[command(author = "Rusty Rain <y@liu.mx>")]
#[command(version = "0.1.0")]
#[command(about = "An example of SFU media", long_about = None)]
struct Cli {
    #[arg(short, long)]
    debug: bool,
    #[arg(long, default_value_t = format!("0.0.0.0"))]
    host: String,
    #[arg(short, long, default_value_t = 8080)]
    signal_port: u16,
    #[arg(short, long, default_value_t = 3478)]
    media_port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
            .filter(None, log::LevelFilter::Trace)
            .init();
    }

    info!(
        "listening {}@{}(signal)/{}(media)...",
        cli.host, cli.signal_port, cli.media_port
    );

    let (sdp_tx, sdp_rx) = mpsc::channel::<String>(1);
    let (cancel_tx, signal_cancel_rx) = broadcast::channel(1);
    let rtc_cancel_rx = cancel_tx.subscribe();
    let mut signal_done_rx =
        signal::http_sdp_server(cli.host.clone(), cli.signal_port, sdp_tx, signal_cancel_rx).await;
    let mut rtc_done_rx =
        rtc::udp_rtc_server(cli.host, cli.media_port, sdp_rx, rtc_cancel_rx).await;

    info!("Press ctrl-c to stop");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            let _ = cancel_tx.send(());
            let _ = rtc_done_rx.recv().await;
            let _ = signal_done_rx.recv().await;
        }
    };

    Ok(())
}
