use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::signal::unix::{SignalKind, signal};

#[derive(Debug, Parser, Serialize, Deserialize)]
struct Args {
    id: String,
    state_file: PathBuf,
    message_file: PathBuf,
    send_dir: PathBuf,
    rest: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct LastMessage {
    last_message: serde_json::Value,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tokio::fs::write(&args.state_file, serde_json::to_string_pretty(&args)?).await?;
    let Args {
        id: _,
        state_file,
        message_file,
        send_dir: _,
        rest: _,
    } = args;
    let mut message_signal = signal(SignalKind::user_defined1())?;
    loop {
        message_signal.recv().await;
        let content = tokio::fs::read_to_string(&message_file).await?;
        let message: serde_json::Value = serde_json::from_str(&content)?;
        tokio::fs::write(
            &state_file,
            serde_json::to_string_pretty(&LastMessage {
                last_message: message,
            })?,
        )
        .await?;
        tokio::fs::remove_file(&message_file).await?;
    }
}
