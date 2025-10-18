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

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
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
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Props {
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
