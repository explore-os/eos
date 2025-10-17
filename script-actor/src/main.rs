use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use nanoid::nanoid;
use rune::termcolor::{ColorChoice, StandardStream};
use rune::{Context, ContextError, Diagnostics, Module, Source, Sources, Vm};
use serde::{Deserialize, Serialize};
use supervisor::Message;
use tokio::signal::unix::{SignalKind, signal};

#[derive(Debug, Parser, Serialize, Deserialize)]
struct Args {
    id: String,
    state_file: PathBuf,
    message_file: PathBuf,
    send_dir: PathBuf,
    script: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args {
        id,
        state_file,
        message_file,
        send_dir,
        script,
    } = Args::parse();
    let mut message_signal = signal(SignalKind::user_defined1())?;
    loop {
        message_signal.recv().await;
        let state = if state_file.exists() {
            let state_string = tokio::fs::read_to_string(&state_file).await?;
            let state: rune::Value = serde_json::from_str(&state_string)?;
            state
        } else {
            rune::Value::empty()
        };
        let message_string = tokio::fs::read_to_string(&message_file).await?;
        let message: rune::Value = serde_json::from_str(&message_string)?;

        let mut context = Context::with_default_modules()?;
        let m = module(&send_dir, &id)?;
        context.install(m)?;

        let runtime = Arc::new(context.runtime()?);
        let mut sources = Sources::new();
        sources.insert(Source::from_path(&script)?)?;

        let mut diagnostics = Diagnostics::new();

        let result = rune::prepare(&mut sources)
            .with_context(&context)
            .with_diagnostics(&mut diagnostics)
            .build();

        if !diagnostics.is_empty() {
            let mut writer = StandardStream::stderr(ColorChoice::Always);
            diagnostics.emit(&mut writer, &sources)?;
        }

        let unit = result?;
        let mut vm = Vm::new(runtime, Arc::new(unit));

        let output = vm.call(["handle"], (state, message))?;
        tokio::fs::write(&state_file, serde_json::to_string_pretty(&output)?).await?;

        tokio::fs::remove_file(&message_file).await?;
    }
}

fn module(send_dir: impl AsRef<Path>, id: &str) -> Result<Module, ContextError> {
    let mut m = Module::new();
    let send_dir = send_dir.as_ref().to_path_buf();
    let id = id.to_owned();
    m.function("send", move |to: &str, value: rune::Value| {
        std::fs::write(
            send_dir.join(format!("{to}::{}", nanoid!())),
            serde_json::to_string_pretty(&Message {
                sender: id.to_string(),
                payload: serde_json::to_value(value).unwrap(),
            })
            .unwrap(),
        )
    })
    .build()?;
    Ok(m)
}
