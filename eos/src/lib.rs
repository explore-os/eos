use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const ROOT: &str = "/explore";
pub const PID_FILE: &str = ".pid";
pub const TICK_FILE: &str = ".tick";
pub const MAILBOX_DIR: &str = "spool";
pub const MAILBOX_HEAD: &str = "current";
pub const ACTOR_DIR: &str = "actors";
pub const SPAWN_DIR: &str = "spawn";
pub const SEND_DIR: &str = "send";
pub const PAUSE_FILE: &str = "paused";
pub const EOS_CTL: &str = "eos.ctl";

pub struct Dirs {
    pub root_dir: PathBuf,
    pub spawn_dir: PathBuf,
    pub actor_dir: PathBuf,
    pub send_dir: PathBuf,
}

impl Dirs {
    pub fn init() -> anyhow::Result<Self> {
        let Self {
            root_dir,
            actor_dir,
            spawn_dir,
            send_dir,
        } = Self::get();

        std::fs::create_dir_all(&actor_dir)?;
        std::fs::create_dir_all(&spawn_dir)?;
        std::fs::create_dir_all(&send_dir)?;
        Ok(Self {
            root_dir,
            actor_dir,
            spawn_dir,
            send_dir,
        })
    }
    pub fn get() -> Self {
        let root = PathBuf::from(ROOT);
        Self {
            root_dir: root.clone(),
            spawn_dir: root.join(SPAWN_DIR),
            actor_dir: root.join(ACTOR_DIR),
            send_dir: root.join(SEND_DIR),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Done,
    Failed { err: String },
    Spawned { id: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub session_id: String,
    pub cmd: Command,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Command {
    Spawn { props: Props },
    Update,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Props {
    pub id: Option<String>,
    pub path: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub copy: Vec<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub sender: String,
    pub payload: serde_json::Value,
}
