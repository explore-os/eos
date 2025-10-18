use std::path::PathBuf;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
pub struct Envelope<T>
where
    T: Serialize + DeserializeOwned,
{
    pub session_id: String,

    #[serde(bound = "T: DeserializeOwned")]
    pub payload: T,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Failed { err: String },
    Spawned { id: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
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
