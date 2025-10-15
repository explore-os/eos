use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const MAILBOX_DIR: &str = "spool";
pub const MAILBOX_HEAD: &str = "current";
pub const ACTOR_DIR: &str = "running";
pub const SPAWN_DIR: &str = "spawn";
pub const SEND_DIR: &str = "send";

#[derive(Debug, Serialize, Deserialize)]
pub struct Props {
    pub path: std::path::PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
}

pub struct Message<T>
where
    T: Serialize + DeserializeOwned,
{
    pub sender: String,
    pub payload: T,
}
