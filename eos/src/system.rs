#![allow(unused)]

use std::{
    collections::{HashMap, VecDeque},
    env::VarError,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    client,
    common::{Message, Props, SYSTEM, teleplot},
};
use bytes::Bytes;
use lazy_static::lazy_static;
use nanoid::nanoid;
use rune::{
    BuildError, Context, ContextError, Diagnostics, Module, Source, Sources, ToValue, Value, Vm,
    diagnostics::EmitError,
    from_value,
    runtime::{Object, RuntimeError, VmError},
    source::FromPathError,
    termcolor::{ColorChoice, StandardStream},
    to_value,
};
use serde_json::Value as JsonValue;
use std::sync::RwLock;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EosError {
    #[error("Actor with ID '{0}' already exists")]
    IdAlreadyExists(String),
    #[error("Rune allocation error {0}")]
    RuneAlloc(#[from] rune::alloc::Error),
    #[error("Rune VM error {0}")]
    VmError(#[from] VmError),
    #[error("Context error {0}")]
    ContextError(#[from] ContextError),
    #[error("From path error {0}")]
    FromPathError(#[from] FromPathError),
    #[error("Emit error {0}")]
    EmitError(#[from] EmitError),
    #[error("Build error {0}")]
    BuildError(#[from] BuildError),
    #[error("JSON error {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Runtime error {0}")]
    RuntimeError(#[from] RuntimeError),
    #[error("Shell error {0}")]
    ShellError(#[from] shellexpand::LookupError<VarError>),
    #[error("IO error {0}")]
    Io(#[from] std::io::Error),
}

pub type EosResult<T> = Result<T, EosError>;

#[derive(ToValue)]
pub struct InternalMessage {
    pub sender: Option<String>,
    pub payload: Value,
}

#[derive(Debug)]
pub struct Actor {
    pub id: String,
    pub mailbox: VecDeque<Message>,
    pub send_queue: VecDeque<Message>,
    pub script: String,
    pub state: JsonValue,
    pub paused: bool,
}

impl From<Message> for InternalMessage {
    fn from(value: Message) -> Self {
        InternalMessage {
            sender: value.from,
            payload: serde_json::from_value(value.payload).unwrap(),
        }
    }
}

impl Actor {
    pub async fn new(id: &str, script: &str) -> EosResult<Self> {
        let state = init(id, &script).await?;
        Ok(Actor {
            id: id.to_string(),
            script: script.to_owned(),
            state: serde_json::to_value(state)?,
            mailbox: VecDeque::new(),
            send_queue: VecDeque::new(),
            paused: false,
        })
    }

    pub async fn run(
        &mut self,
        spawn_queue: &mut Vec<Props>,
        message: Message,
    ) -> EosResult<Option<Message>> {
        let mut vm = make_vm(&self.id, &self.script).await?;
        log::info!("{message:?}");
        let result = vm.call(
            ["handle"],
            (
                serde_json::from_value::<rune::Value>(self.state.clone())?,
                serde_json::from_value::<rune::Value>(serde_json::to_value(&message.payload)?)?,
            ),
        )?;
        if let Ok((state, response)) = from_value::<(Object, Object)>(&result) {
            self.state = serde_json::to_value(rune::Value::new(state)?)?;
            if let Some(from) = message.from {
                return Ok(Some(Message {
                    from: message.to.into(),
                    payload: serde_json::to_value(rune::Value::new(response)?)?,
                    to: from,
                }));
            }
        } else if let Ok(state) = from_value::<Object>(&result) {
            self.state = serde_json::to_value(rune::Value::new(state)?)?;
        }
        Ok(None)
    }
}

#[derive(Debug)]
pub struct System {
    pub spawn_queue: Vec<Props>,
    pub actors: HashMap<String, Actor>,
    pub paused: bool,
}

impl System {
    pub fn new() -> Self {
        System {
            spawn_queue: Vec::new(),
            actors: HashMap::new(),
            paused: false,
        }
    }
    pub async fn kill_actor(&mut self, id: &str) -> EosResult<()> {
        if let Some(_) = self.actors.remove(id) {
            log::info!("killed: id:{id:?}");
        }
        Ok(())
    }

    pub async fn spawn_actor(&mut self, Props { script, id }: Props) -> EosResult<String> {
        log::info!("spawn: id:{id:?}");
        let id = id.unwrap_or_else(|| nanoid!());
        let actor = Actor::new(&id, &script).await?;
        if self.actors.contains_key(&id) {
            return Err(EosError::IdAlreadyExists(id));
        }
        self.actors.insert(id.clone(), actor);
        Ok(id)
    }

    pub async fn tick(&mut self) -> EosResult<()> {
        if self.paused {
            return Ok(());
        }
        while let Some(request) = self.spawn_queue.pop() {
            self.spawn_actor(request).await?;
        }
        let mut actor_messages = Vec::new();
        let mut spawn_requests = Vec::new();
        for actor in self.actors.values_mut() {
            if actor.paused {
                continue;
            }
            if let Some(msg) = actor.send_queue.pop_front() {
                actor_messages.push(msg);
            }
            if let Some(message) = actor.mailbox.pop_front()
                && let Some(response) = actor.run(&mut spawn_requests, message).await?
            {
                actor_messages.push(response);
            }
        }
        for msg in actor_messages {
            if let Some(actor) = self.actors.get_mut(&msg.to) {
                actor.mailbox.push_back(msg);
            }
        }
        self.spawn_queue = spawn_requests;
        Ok(())
    }
}

async fn init(id: &str, script: &str) -> EosResult<rune::Value> {
    let vm = make_vm(id, script).await?;
    if let Ok(init) = vm.lookup_function(["init"]) {
        Ok(init.call(()).into_result()?)
    } else {
        empty_state()
    }
}

fn empty_state() -> EosResult<rune::Value> {
    Ok(rune::Value::new(Object::new())?)
}

async fn make_vm(id: &str, script: &str) -> EosResult<rune::Vm> {
    let mut m = Module::new();
    {
        let id = id.to_owned();
        m.function("send", move |to: &str, value: rune::Value| {
            if let Some(this) = SYSTEM.write().unwrap().actors.get_mut(&id) {
                this.send_queue.push_back(Message {
                    from: Some(id.to_owned()),
                    to: to.to_owned(),
                    payload: serde_json::to_value(value).unwrap(),
                });
            } else {
                log::warn!("Actor died after sending a message");
            }
        })
        .build()?;
    }
    {
        m.function("plot", |value: &str| teleplot(value)).build()?;
    }

    let mut context = Context::with_default_modules()?;
    context.install(m)?;

    let runtime = Arc::new(context.runtime()?);
    let mut sources = Sources::new();
    sources.insert(Source::memory(&script)?)?;

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
