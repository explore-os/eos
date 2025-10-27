use std::path::PathBuf;
use std::sync::RwLock;
use std::{net::UdpSocket, path::Path};

use lazy_static::lazy_static;
use redb::{CacheStats, Database, ReadableDatabase, TableDefinition};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::system::System;

lazy_static! {
    pub static ref SYSTEM: RwLock<System> = RwLock::new(System::new());
}

#[cfg(feature = "docker")]
pub const NATS_URL: &str = "nats://msgbus:4222";

#[cfg(not(feature = "docker"))]
pub const NATS_URL: &str = "nats://localhost:4222";

pub mod dirs {
    pub const LOGS: &str = "logs";
    pub const STORAGE: &str = "storage";
}
pub const EOS_CTL: &str = "eos.ctl";
pub const DEFAULT_TICK: u64 = 2000;

#[cfg(feature = "docker")]
const ROOT: &str = "/explore";

#[cfg(not(feature = "docker"))]
const ROOT: &str = ".";

pub fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

const TELEPLOT_ADDR: &str = "127.0.0.1:47269";
const TABLE: TableDefinition<&str, String> = TableDefinition::new("DATA");

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
    Send(Message),
    Pause { id: Option<String> },
    Unpause { id: Option<String> },
    Tick,
    SetTick { tick: u64 },
    ResetTick,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Props {
    pub script: PathBuf,
    pub id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub from: Option<String>,
    pub to: String,
    pub payload: Value,
}

pub fn teleplot(value: &str) -> anyhow::Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.send_to(value.as_bytes(), TELEPLOT_ADDR)?;
    Ok(())
}

#[derive(Clone)]
pub struct Db {
    storage_dir: PathBuf,
    name: String,
}

impl Db {
    pub fn new(storage_dir: impl AsRef<Path>, name: &str) -> Self {
        Self {
            storage_dir: storage_dir.as_ref().to_path_buf(),
            name: name.to_string(),
        }
    }

    fn db(&self) -> anyhow::Result<Database> {
        Ok(Database::create(self.storage_dir.join(&self.name))?)
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
