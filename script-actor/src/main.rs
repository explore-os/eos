use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use eos::{Db, Message, teleplot};
use nanoid::nanoid;
use rune::runtime::Object;
use rune::termcolor::{ColorChoice, StandardStream};
use rune::{Context, Diagnostics, Module, Source, Sources, Vm};
use serde::{Deserialize, Serialize};
use tokio::signal::unix::{SignalKind, signal};

#[derive(Debug, Parser, Serialize, Deserialize)]
struct Args {
    id: String,
    state_file: PathBuf,
    message_file: PathBuf,
    send_dir: PathBuf,
    script: PathBuf,
}

async fn make_vm(m: &Module, script: impl AsRef<Path>) -> anyhow::Result<rune::Vm> {
    let mut context = Context::with_default_modules()?;
    context.install(m)?;

    let runtime = Arc::new(context.runtime()?);
    let mut sources = Sources::new();
    sources.insert(Source::from_path(script)?)?;

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
    Ok(Vm::new(runtime, Arc::new(unit)))
}

async fn init(m: &Module, script: impl AsRef<Path>) -> anyhow::Result<rune::Value> {
    let vm = make_vm(m, script).await?;
    if let Ok(init) = vm.lookup_function(["init"]) {
        Ok(init.call(()).into_result()?)
    } else {
        empty_state()
    }
}

fn empty_state() -> anyhow::Result<rune::Value> {
    Ok(rune::Value::new(Object::new())?)
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
    let mut m = Module::new();
    {
        let id = id.clone();
        m.function("send", move |to: &str, value: rune::Value| {
            std::fs::write(
                send_dir.join(format!("{to}::{}", nanoid!())),
                serde_json::to_string_pretty(&Message {
                    sender: Some(id.to_string()),
                    payload: serde_json::to_value(value).unwrap(),
                })
                .unwrap(),
            )
        })
        .build()?;
    }
    let db = Db::new(&id)?;
    {
        let db = db.clone();
        m.function("store", move |key: &str, value: rune::Value| {
            db.store(key, value)
        })
        .build()?;
    }
    {
        let db = db.clone();
        m.function("load", move |key: &str| db.load::<rune::Value>(key))
            .build()?;
    }
    {
        let db = db.clone();
        m.function("delete", move |key: &str| db.delete(key))
            .build()?;
    }
    {
        m.function("plot", |value: &str| teleplot(value)).build()?;
    }
    tokio::fs::write(
        &state_file,
        serde_json::to_string_pretty(&init(&m, &script).await?)?,
    )
    .await?;
    let mut message_signal = signal(SignalKind::user_defined1())?;
    loop {
        message_signal.recv().await;
        let message_string = tokio::fs::read_to_string(&message_file).await?;
        let message: rune::Value = serde_json::from_str(&message_string)?;

        let state = if state_file.exists() {
            let state_string = tokio::fs::read_to_string(&state_file).await?;
            let state: rune::Value = serde_json::from_str(&state_string)?;
            state
        } else {
            empty_state()?
        };

        let mut vm = make_vm(&m, &script).await?;

        let output = vm.call(["handle"], (state, message))?;
        tokio::fs::write(&state_file, serde_json::to_string_pretty(&output)?).await?;

        tokio::fs::remove_file(&message_file).await?;
    }
}
