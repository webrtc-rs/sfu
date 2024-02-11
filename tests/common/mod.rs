use log::info;
use log::LevelFilter::Debug;
use std::io::Write;

pub fn setup() -> anyhow::Result<()> {
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
        .filter(None, Debug)
        .try_init()?;

    // some setup code, like creating required files/directories, starting
    // servers, etc.
    info!("common setup");

    Ok(())
}
