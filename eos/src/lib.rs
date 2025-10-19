use std::net::UdpSocket;
use std::path::PathBuf;

use redb::{CacheStats, Database, ReadableDatabase, TableDefinition};
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
pub const STATE_FILE: &str = "state.json";
pub const EOS_CTL: &str = "eos.ctl";

const TELEPLOT_ADDR: &str = "teleplot:47269";
const TABLE: TableDefinition<&str, String> = TableDefinition::new("DATA");

pub struct Dirs {
    pub root_dir: PathBuf,
    pub spawn_dir: PathBuf,
    pub actor_dir: PathBuf,
    pub send_dir: PathBuf,
    pub storage_dir: PathBuf,
}

impl Dirs {
    pub fn init() -> anyhow::Result<Self> {
        let Self {
            root_dir,
            actor_dir,
            spawn_dir,
            send_dir,
            storage_dir,
        } = Self::get();

        std::fs::create_dir_all(&actor_dir)?;
        std::fs::create_dir_all(&spawn_dir)?;
        std::fs::create_dir_all(&send_dir)?;
        Ok(Self {
            root_dir,
            actor_dir,
            spawn_dir,
            send_dir,
            storage_dir,
        })
    }
    pub fn get() -> Self {
        let root = PathBuf::from(ROOT);
        let actor_dir = root.join(ACTOR_DIR);
        let storage_dir = actor_dir.join("storage");
        Self {
            root_dir: root.clone(),
            spawn_dir: root.join(SPAWN_DIR),
            actor_dir,
            send_dir: root.join(SEND_DIR),
            storage_dir,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Done,
    Failed { err: String },
    Spawned { id: String },
    Actors { actors: Vec<String> },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub session_id: String,
    pub cmd: Command,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Command {
    Spawn { props: Props },
    List,
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
    pub sender: Option<String>,
    pub payload: serde_json::Value,
}

pub fn teleplot(value: &str) -> anyhow::Result<()> {
    let sock = UdpSocket::bind("127.0.0.1:0")?;
    sock.send_to(value.as_bytes(), TELEPLOT_ADDR)?;
    Ok(())
}

#[derive(Clone)]
pub struct Db {
    id: String,
}

impl Db {
    pub fn new(id: &str) -> anyhow::Result<Self> {
        Ok(Self { id: id.to_string() })
    }

    fn db(&self) -> anyhow::Result<Database> {
        let Dirs { storage_dir, .. } = Dirs::get();
        Ok(Database::create(storage_dir.join(&self.id))?)
    }

    pub fn stats(&self) -> anyhow::Result<CacheStats> {
        Ok(self.db()?.cache_stats())
    }

    pub fn compact(&self) -> anyhow::Result<bool> {
        Ok(self.db()?.compact()?)
    }

    pub fn store<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        value: T,
    ) -> anyhow::Result<()> {
        let write_txn = self.db()?.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            table.insert(key, serde_json::to_string(&value)?)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> anyhow::Result<()> {
        let write_txn = self.db()?.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            table.remove(key)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn load<T: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>> {
        let read_txn = self.db()?.begin_read()?;
        let table = read_txn.open_table(TABLE)?;
        let result = if let Some(value) = table.get(key)? {
            Some(serde_json::from_str(&value.value())?)
        } else {
            None
        };
        Ok(result)
    }

    pub fn exists(&self, key: &str) -> anyhow::Result<bool> {
        let read_txn = self.db()?.begin_read()?;
        let table = read_txn.open_table(TABLE)?;
        Ok(table.get(key)?.is_some())
    }
}
