use clap::Parser;
use sfu::signal;
use std::io::Write;

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

    println!(
        "listening {}@{}(signal)/{}(media)...",
        cli.host, cli.signal_port, cli.media_port
    );

    let mut _sdp_chan_rx = signal::http_sdp_server(cli.signal_port).await;

    println!("Press ctrl-c to stop");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {

        }
    };

    Ok(())
}
