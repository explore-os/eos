mod file_overlay;
mod system;

use clap::Parser;
use env_logger::Env;
use rs9p::srv::srv_async_unix;

use crate::{file_overlay::FsOverlay, system::System};

// const DEFAULT_TICK: u64 = 2000;

#[derive(Parser)]
struct Cli {
    #[arg(short, long, default_value = "nats://msgbus:4222")]
    nats: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let Cli { nats } = Cli::parse();

    srv_async_unix(FsOverlay::new(System::new(&nats)), "/tmp/eos-operator:0").await?;

    Ok(())
}
