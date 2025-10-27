use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::bail;
use async_nats::{Client, connect};
use bytes::Bytes;
#[cfg(feature = "_setup")]
use clap::Command;
use clap::{Parser, Subcommand};
use common::{EOS_CTL, Message, Props, ROOT, Request, Response, STORAGE_DIR};
use env_logger::Env;
use futures_util::StreamExt;
use nanoid::nanoid;

#[cfg(feature = "_setup")]
use clap_complete::{aot::Fish, generate_to};
use rs9p::srv::srv_async_unix;
use tokio::{spawn, sync::RwLock};

use crate::{
    common::{DEFAULT_TICK, teleplot},
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
    #[cfg_attr(
        feature = "remote",
        arg(short, long, default_value = "nats://msgbus:4222")
    )]
    #[cfg_attr(
        not(feature = "remote"),
        arg(short, long, default_value = "nats://localhost:4222")
    )]
    nats: String,
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
    Serve,
    /// spawn an actor
    Spawn {
        /// the requested id for the actor
        #[arg(short, long)]
        id: Option<String>,
        script: PathBuf,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    #[cfg(feature = "_setup")]
    {
        let SetupCli { out_dir } = SetupCli::parse();
        let mut cmd = Cli::command();
        generate_to(Fish, &mut cmd, "eos", &out_dir)?;
        std::process::exit(0);
    }

    let Cli { nats, command } = Cli::parse();
    let root = Path::new(ROOT);
    let client = connect(&nats).await?;
    let storage = root.join(STORAGE_DIR);
    match command {
        Action::Db { name, command } => {
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
            spawn_actor(client, Props { id, script }).await?;
        }
        Action::List => {
            list(client).await?;
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
                kind: common::MessageKind::Notification,
                from: sender,
                to: id,
                payload: serde_json::from_str(&msg)?,
            };
            send(client, common::Command::Send(msg)).await?;
        }
        Action::Pause { path } => {
            send(
                client,
                common::Command::Pause {
                    id: path.map(|p| p.file_name().unwrap().display().to_string()),
                },
            )
            .await?;
        }
        Action::Unpause { path } => {
            send(
                client,
                common::Command::Unpause {
                    id: path.map(|p| p.file_name().unwrap().display().to_string()),
                },
            )
            .await?;
        }
        Action::Tick { command } => match command {
            TickCommand::Reset => {
                send(client, common::Command::ResetTick).await?;
            }
            TickCommand::Set { milliseconds } => {
                send(client, common::Command::SetTick { tick: milliseconds }).await?;
            }
        },
        Action::Plot { value } => {
            common::teleplot(&value)?;
        }
        Action::Serve => {
            let config = Arc::new(RwLock::new(Config { tick: DEFAULT_TICK }));
            let sys = Arc::new(RwLock::new(System::new(&nats)));

            {
                let config = config.clone();
                let sys = sys.clone();
                spawn(async move {
                    if let Ok(client) = async_nats::connect(&nats).await {
                        let mut subscriber = client.subscribe(EOS_CTL).await.unwrap();
                        spawn(async move {
                            while let Some(message) = subscriber.next().await {
                                match serde_json::from_slice::<common::Request>(&message.payload) {
                                    Ok(Request { session_id, cmd }) => {
                                        let response = match cmd {
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
                                                    actor.mailbox.push_back(msg);
                                                }
                                                Response::Done
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
                        sys.write().await.tick().await.unwrap();
                    }
                });
            }

            srv_async_unix(FsOverlay::new(sys), "/tmp/eos-operator:0").await?;
        }
    }
    Ok(())
}
