use std::{
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::bail;
use async_nats::{Client, connect};
use bytes::Bytes;
#[cfg(feature = "_setup")]
use clap::Command;
use clap::{Parser, Subcommand};
use common::{EOS_CTL, Message, Props, Request, Response};
use futures_util::StreamExt;
use nanoid::nanoid;
use stringlit::s;

#[cfg(feature = "_setup")]
use clap_complete::{aot::Fish, generate_to};
use rs9p::srv::srv_async_unix;
use tokio::{spawn, sync::RwLock};

use crate::{
    common::{
        DEFAULT_TICK, EOS_SOCKET, NATS_URL,
        dirs::{LOGS, STORAGE},
        root, teleplot,
    },
    file_overlay::FsOverlay,
    system::System,
};

mod common;
mod file_overlay;
mod system;

#[cfg(feature = "_setup")]
#[derive(Parser)]
struct SetupCli {
    out_dir: PathBuf,
}
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[cfg(feature = "_setup")]
impl Cli {
    fn command() -> Command {
        <Self as clap::CommandFactory>::command()
    }
}

#[derive(Subcommand)]
enum Action {
    Root,
    Sock,
    Shutdown,
    Serve,
    /// spawn an actor
    Spawn {
        /// the requested id for the actor
        #[arg(short, long)]
        id: Option<String>,
        script: PathBuf,
    },
    /// Kill an actor
    Kill {
        /// the directories for the actors to kill
        paths: Vec<PathBuf>,
    },
    /// list all the running actors
    List,
    /// pauses an actor
    Pause {
        /// the directory for the actor to pause
        path: Option<PathBuf>,
    },
    /// unpauses an actor
    Unpause {
        /// the directory for the actor to unpause
        path: Option<PathBuf>,
    },
    /// puts message in the send queue and notifies the supervisor that a message is available
    Send {
        /// the id of the sender
        #[arg(short, long)]
        sender: Option<PathBuf>,
        /// the path to the actor the message should be sent
        path: PathBuf,
        /// a string containing the json representation of a message
        msg: String,
    },
    /// changes the tick rate of the system
    Tick {
        #[command(subcommand)]
        command: TickCommand,
    },
    /// handles "db" access
    Db {
        /// the db name
        name: String,
        /// the db command
        #[command(subcommand)]
        command: DbCommand,
    },
    /// send data to a teleplot instance
    Plot {
        value: String,
    },
}

impl Action {
    fn init(self) -> Result<Self, fern::InitError> {
        if let Action::Serve = &self {
            let logs = root().join(LOGS);
            if !std::fs::exists(&logs)? {
                std::fs::create_dir_all(logs)?;
            }
            file_logger()?;
        } else {
            cli_logger()?;
        }
        Ok(self)
    }
}

#[derive(Subcommand)]
enum DbCommand {
    /// store a value in an actors kv-store
    Store {
        /// the key to store
        key: String,
        /// the value to store
        value: String,
    },
    /// delete a value in an actors kv-store
    Delete {
        /// the key to delete
        key: String,
    },
    /// load a value from an actors kv-store
    Load {
        /// the key to load
        key: String,
    },
    /// checks if a value in an actors kv-store exists
    Exists {
        /// the key to check
        key: String,
    },
    /// compact the db for an actor
    Compact,
    /// print cache stats for the db of an actor
    Stats,
}

#[derive(Subcommand)]
enum TickCommand {
    /// sets the tick rate of the system
    Set {
        /// the tick rate of the system in milliseconds (must be 100 or higher)
        #[arg(value_parser = clap::value_parser!(u64).range(100..))]
        milliseconds: u64,
    },
    /// resets the tick rate of the system
    Reset,
    /// Ticks once
    Now,
}

async fn send(client: Client, cmd: common::Command) -> anyhow::Result<()> {
    let session = nanoid!();

    let mut sub = client.subscribe(format!("eos.response.{session}")).await?;

    client
        .publish(
            EOS_CTL,
            Bytes::from(serde_json::to_vec(&Request {
                session_id: session.clone(),
                cmd,
            })?),
        )
        .await?;

    while let Some(msg) = sub.next().await {
        if let Ok(response) = serde_json::from_slice::<Response>(&msg.payload) {
            match response {
                Response::Done => {
                    println!("Command executed successfully");
                }
                Response::Actors { actors } => {
                    println!("Actors: {:?}", actors);
                }
                Response::Spawned { id } => {
                    println!("Actor spawned with id: {id}");
                }
                Response::Failed { err } => {
                    eprintln!("Failed to spawn actor: {err}")
                }
            }
            break;
        }
    }

    Ok(())
}

async fn spawn_actor(client: Client, props: Props) -> anyhow::Result<()> {
    send(client, common::Command::Spawn { props }).await
}
async fn list(client: Client) -> anyhow::Result<()> {
    send(client, common::Command::List).await
}

struct Config {
    tick: u64,
}

async fn respond(client: &Client, session_id: String, response: Response) -> anyhow::Result<()> {
    if let Err(e) = client
        .publish(
            format!("eos.response.{session_id}"),
            Bytes::from(serde_json::to_vec(&response).unwrap()),
        )
        .await
    {
        log::error!("{e}");
    }
    Ok(())
}

async fn client() -> Client {
    connect(NATS_URL).await.unwrap()
}

fn cli_logger() -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_seconds(SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}

fn file_logger() -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_seconds(SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .level_for("rs9p", log::LevelFilter::Warn)
        .chain(std::io::stdout())
        .chain(fern::DateBased::new("/explore/logs/eos.log.", "%Y-%m-%d"))
        .apply()?;
    Ok(())
}

async fn send_shutdown(client: Client) -> anyhow::Result<()> {
    Ok(client
        .publish(
            EOS_CTL,
            Bytes::from(serde_json::to_vec(&Request {
                session_id: String::new(),
                cmd: common::Command::Shutdown,
            })?),
        )
        .await?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "_setup")]
    {
        let SetupCli { out_dir } = SetupCli::parse();
        let mut cmd = Cli::command();
        generate_to(Fish, &mut cmd, "eos", &out_dir)?;
        std::process::exit(0);
    }

    let Cli { command } = Cli::parse();
    match command.init()? {
        Action::Root => {
            println!("{}", root().display())
        }
        Action::Db { name, command } => {
            let storage = root().join(STORAGE);
            let db = common::Db::new(&storage, &name);
            match command {
                DbCommand::Store { key, value } => {
                    db.store(&key, serde_json::to_value(value)?)?;
                }
                DbCommand::Delete { key } => {
                    db.delete(&key)?;
                }
                DbCommand::Load { key } => {
                    let value = db.load::<serde_json::Value>(&key)?;
                    match value {
                        Some(value) => println!("{}", serde_json::to_string_pretty(&value)?),
                        None => bail!("Key not found"),
                    }
                }
                DbCommand::Exists { key } => {
                    println!("{}", serde_json::to_string_pretty(&db.exists(&key)?)?);
                }
                DbCommand::Compact => {
                    if db.compact()? {
                        println!("Database compacted");
                    } else {
                        println!("No need to compact database");
                    }
                }
                DbCommand::Stats => {
                    println!("{:#?}", db.stats()?);
                }
            }
        }
        Action::Spawn { id, script } => {
            let script = script.to_string_lossy().to_string();
            let script = PathBuf::from_str(&shellexpand::full(&script)?)?;
            spawn_actor(client().await, Props { id, script }).await?;
        }
        Action::List => {
            list(client().await).await?;
        }
        Action::Send { path, msg, sender } => {
            let id = path
                .file_name()
                .expect("Invalid path!")
                .display()
                .to_string();
            let sender = sender.map(|sender| {
                sender
                    .file_name()
                    .expect("Invalid path!")
                    .display()
                    .to_string()
            });
            let msg = Message {
                from: sender,
                to: id,
                payload: serde_json::from_str(&msg)?,
            };
            send(client().await, common::Command::Send(msg)).await?;
        }
        Action::Kill { paths } => {
            send(
                client().await,
                common::Command::Kill {
                    ids: paths
                        .iter()
                        .map(|p| p.file_name().unwrap().display().to_string())
                        .collect(),
                },
            )
            .await?;
        }
        Action::Pause { path } => {
            send(
                client().await,
                common::Command::Pause {
                    id: path.map(|p| p.file_name().unwrap().display().to_string()),
                },
            )
            .await?;
        }
        Action::Unpause { path } => {
            send(
                client().await,
                common::Command::Unpause {
                    id: path.map(|p| p.file_name().unwrap().display().to_string()),
                },
            )
            .await?;
        }
        Action::Tick { command } => match command {
            TickCommand::Now => {
                send(client().await, common::Command::Tick).await?;
            }
            TickCommand::Reset => {
                send(client().await, common::Command::ResetTick).await?;
            }
            TickCommand::Set { milliseconds } => {
                send(
                    client().await,
                    common::Command::SetTick { tick: milliseconds },
                )
                .await?;
            }
        },
        Action::Plot { value } => {
            common::teleplot(&value)?;
        }
        Action::Sock => {
            print!("{EOS_SOCKET}");
        }
        Action::Shutdown => {
            send_shutdown(client().await).await?;
        }
        Action::Serve => {
            send_shutdown(client().await).await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let config = Arc::new(RwLock::new(Config { tick: DEFAULT_TICK }));
            let sys = Arc::new(RwLock::new(System::new()));

            {
                let config = config.clone();
                let sys = sys.clone();
                spawn(async move {
                    if let Ok(client) = async_nats::connect(NATS_URL).await {
                        let mut subscriber = client.subscribe(EOS_CTL).await.unwrap();
                        spawn(async move {
                            while let Some(message) = subscriber.next().await {
                                match serde_json::from_slice::<common::Request>(&message.payload) {
                                    Ok(Request { session_id, cmd }) => {
                                        let response = match cmd {
                                            common::Command::Shutdown => {
                                                respond(&client, session_id, Response::Done)
                                                    .await
                                                    .unwrap();

                                                nix::sys::signal::kill(
                                                    nix::unistd::getpid(),
                                                    nix::sys::signal::Signal::SIGTERM,
                                                )
                                                .unwrap();
                                                std::process::exit(0);
                                            }
                                            common::Command::Rename { old, new } => {
                                                let mut sys = sys.write().await;
                                                if sys.actors.contains_key(&new) {
                                                    Response::Failed {
                                                        err: s!(
                                                            "Actor with the same id already exists"
                                                        ),
                                                    }
                                                } else {
                                                    if let Some(actor) = sys.actors.remove(&old) {
                                                        sys.actors.insert(new, actor);
                                                    }
                                                    Response::Done
                                                }
                                            }
                                            common::Command::Kill { ids } => {
                                                let mut sys = sys.write().await;
                                                for id in ids {
                                                    sys.actors.remove(&id);
                                                }
                                                Response::Done
                                            }
                                            common::Command::Pause { id } => {
                                                let mut sys = sys.write().await;
                                                if let Some(id) = id
                                                    && let Some(actor) = sys.actors.get_mut(&id)
                                                {
                                                    actor.paused = true;
                                                } else {
                                                    sys.paused = true;
                                                }
                                                Response::Done
                                            }
                                            common::Command::Unpause { id } => {
                                                let mut sys = sys.write().await;
                                                if let Some(id) = id
                                                    && let Some(actor) = sys.actors.get_mut(&id)
                                                {
                                                    actor.paused = false;
                                                } else {
                                                    sys.paused = false;
                                                }
                                                Response::Done
                                            }
                                            common::Command::Send(msg) => {
                                                let mut sys = sys.write().await;
                                                if let Some(actor) = sys.actors.get_mut(&msg.to) {
                                                    actor.mailbox.push_back(dbg!(msg));
                                                }
                                                Response::Done
                                            }
                                            common::Command::Tick => {
                                                let mut sys = sys.write().await;
                                                match sys.tick().await {
                                                    Ok(()) => Response::Done,
                                                    Err(err) => Response::Failed {
                                                        err: err.to_string(),
                                                    },
                                                }
                                            }
                                            common::Command::SetTick { tick } => {
                                                let mut config = config.write().await;
                                                config.tick = tick;
                                                Response::Done
                                            }
                                            common::Command::ResetTick => {
                                                let mut config = config.write().await;
                                                config.tick = DEFAULT_TICK;
                                                Response::Done
                                            }
                                            common::Command::Spawn { props } => {
                                                match sys.write().await.spawn_actor(props).await {
                                                    Ok(id) => {
                                                        _ = teleplot("system.actor.spawned:1");
                                                        Response::Spawned { id }
                                                    }
                                                    Err(err) => Response::Failed {
                                                        err: err.to_string(),
                                                    },
                                                }
                                            }
                                            common::Command::List => {
                                                let actors = sys
                                                    .read()
                                                    .await
                                                    .actors
                                                    .keys()
                                                    .cloned()
                                                    .collect::<Vec<_>>();
                                                Response::Actors { actors }
                                            }
                                        };
                                        respond(&client, session_id, response).await.unwrap();
                                    }
                                    Err(e) => log::error!("Invalid message format: {e}"),
                                };
                            }
                        });
                    }
                });
            }

            {
                let sys = sys.clone();
                spawn(async move {
                    loop {
                        let tick = config.read().await.tick;
                        tokio::time::sleep(Duration::from_millis(tick)).await;
                        if let Err(e) = sys.write().await.tick().await {
                            log::error!("Failed to tick: {e}");
                        }
                    }
                });
            }

            spawn(async {
                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if tokio::fs::try_exists(EOS_SOCKET).await.unwrap() {
                        _ = tokio::process::Command::new("sudo")
                            .arg("mount")
                            .arg("-t")
                            .arg("9p")
                            .arg("-o")
                            .arg(format!(
                                "version=9p2000.L,trans=unix,uname={}",
                                std::env::var("USER").unwrap_or_else(|_| s!("vscode"))
                            ))
                            .arg(EOS_SOCKET)
                            .arg("/explore/system")
                            .spawn();
                        break;
                    }
                }
            });

            srv_async_unix(FsOverlay::new(sys), EOS_SOCKET).await?;
        }
    }
    Ok(())
}
