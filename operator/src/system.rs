#![allow(unused)]

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

use nanoid::nanoid;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EosError {
    #[error("Actor with ID '{0}' already exists")]
    IdAlreadyExists(String),
}

pub type EosResult<T> = Result<T, EosError>;

pub enum MessageKind {
    Request,
    Response,
    Notification,
}

pub struct Message {
    pub kind: MessageKind,
    pub from: Option<String>,
    pub to: String,
    pub payload: Value,
}

#[derive(Default)]
pub struct Actor {
    pub mailbox: VecDeque<Message>,
    pub send_queue: VecDeque<Message>,
    pub script: PathBuf,
    pub state: Value,
}

impl Actor {
    pub fn new(script: PathBuf, state: Option<Value>) -> Self {
        Actor {
            script,
            state: state.unwrap_or_default(),
            ..Default::default()
        }
    }

    pub async fn run(
        &mut self,
        spawn_queue: &mut Vec<Props>,
        message: Message,
    ) -> EosResult<Option<Message>> {
        todo!()
    }
}

pub struct Props {
    pub script: PathBuf,
    pub id: Option<String>,
    pub state: Option<serde_json::Value>,
}

pub struct System {
    pub nats: String,
    pub spawn_queue: Vec<Props>,
    pub actors: HashMap<String, Actor>,
}

impl System {
    pub fn new(nats: &str) -> Self {
        System {
            nats: nats.to_string(),
            spawn_queue: Vec::new(),
            actors: HashMap::new(),
        }
    }

    pub fn spawn_actor(&mut self, Props { script, id, state }: Props) -> EosResult<String> {
        let actor = Actor {
            script,
            state: state.unwrap_or_default(),
            ..Default::default()
        };
        let id = id.unwrap_or_else(|| nanoid!());
        if self.actors.contains_key(&id) {
            return Err(EosError::IdAlreadyExists(id));
        }
        self.actors.insert(id.clone(), actor);
        Ok(id)
    }

    pub async fn tick(&mut self) -> EosResult<()> {
        while let Some(request) = self.spawn_queue.pop() {
            self.spawn_actor(request)?;
        }
        let mut actor_messages = Vec::new();
        let mut spawn_requests = Vec::new();
        for actor in self.actors.values_mut() {
            if let Some(message) = actor.mailbox.pop_front()
                && let Some(response) = actor.run(&mut spawn_requests, message).await?
            {
                actor_messages.push(response);
            }
            if let Some(msg) = actor.send_queue.pop_front() {
                actor_messages.push(msg);
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
