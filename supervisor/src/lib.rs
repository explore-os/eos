use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const MAILBOX_DIR: &str = "spool";
pub const MAILBOX_HEAD: &str = "current";
pub const ACTOR_DIR: &str = "running";
pub const SPAWN_DIR: &str = "spawn";
pub const SEND_DIR: &str = "send";
pub const PAUSE_FILE: &str = "paused";

#[derive(Debug, Serialize, Deserialize)]
pub struct Props {
    pub path: std::path::PathBuf,
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
