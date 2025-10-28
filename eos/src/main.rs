use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::bail;

use axum::{Json, Router, extract::State, routing::post};
#[cfg(feature = "_setup")]
use clap::Command;
use clap::{Parser, Subcommand};
use common::{Message, Props, Response};

use serde::Serialize;
use stringlit::s;

#[cfg(feature = "_setup")]
use clap_complete::{aot::Fish, generate_to};
use rs9p::srv::srv_async_unix;
use tokio::sync::RwLock;

use crate::{
    common::{
        DEFAULT_TICK, EOS_SOCKET, KILL_FILE, RPC_PORT,
        dirs::{LOGS, MOUNT, STORAGE},
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
enum SockType {
    Mount,
    Rpc,
}

#[derive(Subcommand)]
enum Action {
    Root,
    Sock,
    Mount,
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
    pub fn is_serve(&self) -> bool {
        matches!(self, Action::Serve { .. })
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

#[derive(Clone)]
struct AppState {
    config: Arc<RwLock<Config>>,
    sys: Arc<RwLock<System>>,
}

async fn spawn(State(state): State<Arc<AppState>>, Json(props): Json<Props>) -> Json<Response> {
    Json(match state.sys.write().await.spawn_actor(props).await {
        Ok(id) => {
            _ = teleplot("system.actor.spawned:1");
            Response::Spawned { id }
        }
        Err(err) => Response::Failed {
            err: err.to_string(),
        },
    })
}

async fn list(State(state): State<Arc<AppState>>) -> Json<Response> {
    let actors = state
        .sys
        .read()
        .await
        .actors
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    Json(Response::Actors { actors })
}

async fn send(State(state): State<Arc<AppState>>, Json(msg): Json<Message>) -> Json<Response> {
    let mut sys = state.sys.write().await;
    if let Some(actor) = sys.actors.get_mut(&msg.to) {
        actor.mailbox.push_back(msg);
    }
    Json(Response::Done)
}

async fn pause(
    State(state): State<Arc<AppState>>,
    Json(id): Json<Option<String>>,
) -> Json<Response> {
    let mut sys = state.sys.write().await;
    if let Some(id) = id
        && let Some(actor) = sys.actors.get_mut(&id)
    {
        actor.paused = true;
    } else {
        sys.paused = true;
    }
    Json(Response::Done)
}

async fn unpause(
    State(state): State<Arc<AppState>>,
    Json(id): Json<Option<String>>,
) -> Json<Response> {
    let mut sys = state.sys.write().await;
    if let Some(id) = id
        && let Some(actor) = sys.actors.get_mut(&id)
    {
        actor.paused = false;
    } else {
        sys.paused = false;
    }
    Json(Response::Done)
}

async fn tick(State(state): State<Arc<AppState>>) -> Json<Response> {
    let mut sys = state.sys.write().await;
    Json(match sys.tick().await {
        Ok(()) => Response::Done,
        Err(err) => Response::Failed {
            err: err.to_string(),
        },
    })
}

async fn set_tick(State(state): State<Arc<AppState>>, Json(tick): Json<u64>) -> Json<Response> {
    let mut config = state.config.write().await;
    config.tick = tick;
    Json(Response::Done)
}

async fn reset_tick(State(state): State<Arc<AppState>>) -> Json<Response> {
    let mut config = state.config.write().await;
    config.tick = DEFAULT_TICK;
    Json(Response::Done)
}

// async fn rename(
//     State(state): State<Arc<AppState>>,
//     old: String,
//     new: String,
// ) -> Response {
//     let mut sys = state.sys.write().await;
//     if sys.actors.contains_key(&new) {
//         Response::Failed {
//             err: s!("Actor with the same id already exists"),
//         }
//     } else {
//         if let Some(actor) = sys.actors.remove(&old) {
//             sys.actors.insert(new, actor);
//         }
//         Response::Done
//     }
// }

async fn kill(State(state): State<Arc<AppState>>, Json(ids): Json<Vec<String>>) -> Json<Response> {
    let mut sys = state.sys.write().await;
    for id in ids {
        if let Err(err) = sys.kill_actor(&id).await {
            tracing::error!("Failed to kill actor {}: {}", id, err);
        }
    }
    Json(Response::Done)
}

async fn shutdown() -> Json<Response> {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(2000)).await;
        if let Err(e) =
            nix::sys::signal::kill(nix::unistd::getpid(), nix::sys::signal::Signal::SIGTERM)
        {
            tracing::error!("Failed to send SIGTERM: {}", e);
        }
        std::process::exit(0);
    });
    Json(Response::Done)
}

async fn rpc0(endpoint: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let response: Response = serde_json::from_str(
        &client
            .post(format!("http://localhost:{RPC_PORT}/{endpoint}"))
            .send()
            .await?
            .text()
            .await?,
    )?;

    match response {
        Response::Done => {
            tracing::info!("Command executed successfully");
        }
        Response::Actors { actors } => {
            tracing::info!("Actors: {:?}", actors);
        }
        Response::Spawned { id } => {
            tracing::info!("Actor spawned with id: {id}");
        }
        Response::Failed { err } => {
            tracing::error!("Failed: {err}")
        }
    }
    Ok(())
}

async fn rpc<T: Serialize>(endpoint: &str, data: &T) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let response: Response = serde_json::from_str(
        &client
            .post(format!("http://localhost:{RPC_PORT}/{endpoint}"))
            .json(data)
            .send()
            .await?
            .text()
            .await?,
    )?;

    match response {
        Response::Done => {
            tracing::info!("Command executed successfully");
        }
        Response::Actors { actors } => {
            tracing::info!("Actors: {:?}", actors);
        }
        Response::Spawned { id } => {
            tracing::info!("Actor spawned with id: {id}");
        }
        Response::Failed { err } => {
            tracing::error!("Failed: {err}")
        }
    }
    Ok(())
}

struct Config {
    tick: u64,
}

struct OptionDropper<T>(Option<T>);

impl<T> Drop for OptionDropper<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            drop(value);
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::spawn(async {
        tokio::signal::ctrl_c().await.unwrap();
        std::process::exit(0);
    });

    #[cfg(feature = "_setup")]
    {
        let SetupCli { out_dir } = SetupCli::parse();
        let mut cmd = Cli::command();
        generate_to(Fish, &mut cmd, "eos", &out_dir)?;
        std::process::exit(0);
    }

    let Cli { command } = Cli::parse();

    let _log_guard = if command.is_serve() {
        let logs = root().join(LOGS);
        if !std::fs::exists(&logs)? {
            std::fs::create_dir_all(logs)?;
        }
        let file_appender = tracing_appender::rolling::hourly(LOGS, "eos.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt().with_writer(non_blocking).init();
        OptionDropper(Some(guard))
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stdout)
            .init();
        OptionDropper(None)
    };

    match command {
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
            let script = tokio::fs::read_to_string(PathBuf::from(
                shellexpand::full(&script.display().to_string())?.to_string(),
            ))
            .await?;
            rpc("spawn", &Props { id, script }).await?;
        }
        Action::List => rpc0("list").await?,
        Action::Send { path, msg, sender } => {
            let id = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid path: no file name found"))?
                .display()
                .to_string();
            let sender =
                sender.and_then(|sender| sender.file_name().map(|d| d.display().to_string()));
            let msg = Message {
                from: sender,
                to: id,
                payload: serde_json::from_str(&msg)?,
            };
            rpc("send", &msg).await?;
        }
        Action::Kill { paths } => {
            let ids: Result<Vec<String>, _> = paths
                .iter()
                .map(|p| {
                    p.file_name()
                        .ok_or_else(|| anyhow::anyhow!("Invalid path: no file name found"))
                        .map(|name| name.display().to_string())
                })
                .collect();

            rpc("kill", &ids?).await?;
        }
        Action::Pause { path } => {
            let id = match path {
                Some(p) => Some(
                    p.file_name()
                        .ok_or_else(|| anyhow::anyhow!("Invalid path: no file name found"))?
                        .display()
                        .to_string(),
                ),
                None => None,
            };

            rpc("pause", &id).await?;
        }
        Action::Unpause { path } => {
            let id = match path {
                Some(p) => Some(
                    p.file_name()
                        .ok_or_else(|| anyhow::anyhow!("Invalid path: no file name found"))?
                        .display()
                        .to_string(),
                ),
                None => None,
            };

            rpc("unpause", &id).await?;
        }
        Action::Tick { command } => match command {
            TickCommand::Now => {
                rpc0("tick/now").await?;
            }
            TickCommand::Reset => {
                rpc0("tick/reset").await?;
            }
            TickCommand::Set { milliseconds } => {
                rpc("tick/set", &milliseconds).await?;
            }
        },
        Action::Plot { value } => {
            common::teleplot(&value)?;
        }
        Action::Sock => {
            print!("{EOS_SOCKET}");
        }
        Action::Mount => {
            print!("{}", root().join(MOUNT).display());
        }
        Action::Shutdown => {
            rpc0("shutdown").await?;
        }
        Action::Serve => {
            tokio::spawn(async {
                tokio::signal::ctrl_c().await.unwrap();
                std::process::exit(0);
            });

            let config = Arc::new(RwLock::new(Config { tick: DEFAULT_TICK }));
            let sys = Arc::new(RwLock::new(System::new()));

            {
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        if tokio::fs::try_exists(KILL_FILE).await.unwrap() {
                            tokio::fs::remove_file(KILL_FILE).await.unwrap();
                            std::process::exit(0);
                        }
                    }
                });
            }

            {
                let sys = sys.clone();
                let config = config.clone();
                tokio::spawn(async move {
                    loop {
                        let tick = config.read().await.tick;
                        tokio::time::sleep(Duration::from_millis(tick)).await;
                        if let Err(e) = sys.write().await.tick().await {
                            tracing::error!("Failed to tick: {e}");
                        }
                    }
                });
            }

            {
                // Remove existing socket if it exists
                let _ = tokio::fs::remove_file(EOS_SOCKET).await;
                let sys = sys.clone();
                tokio::spawn(async move {
                    srv_async_unix(FsOverlay::new(sys), EOS_SOCKET)
                        .await
                        .unwrap();
                    std::process::exit(0);
                });
            }

            tokio::spawn(async {
                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    match tokio::fs::try_exists(EOS_SOCKET).await {
                        Ok(true) => {
                            let mnt = root().join(MOUNT);

                            for _ in 0..5 {
                                if let Ok(mut c) = tokio::process::Command::new("sudo")
                                    .arg("umount")
                                    .arg(&mnt)
                                    .spawn()
                                {
                                    if let Ok(_) = c.wait().await {
                                        tracing::info!(
                                            "Successfully unmounted previous 9p filesystem"
                                        );
                                        break;
                                    }
                                }
                            }

                            if !tokio::fs::try_exists(&mnt).await.unwrap() {
                                tokio::fs::create_dir_all(&mnt)
                                    .await
                                    .expect("Could not create mnt dir!");
                            }
                            match tokio::process::Command::new("sudo")
                                .arg("mount")
                                .arg("-t")
                                .arg("9p")
                                .arg("-o")
                                .arg(format!(
                                    "version=9p2000.L,trans=unix,uname={}",
                                    std::env::var("USER").unwrap_or_else(|_| s!("vscode"))
                                ))
                                .arg(EOS_SOCKET)
                                .arg(mnt)
                                .spawn()
                            {
                                Ok(_) => tracing::info!("Successfully mounted 9p filesystem"),
                                Err(e) => {
                                    tracing::error!("Failed to mount 9p filesystem: {}", e)
                                }
                            }
                            break;
                        }
                        Ok(false) => continue,
                        Err(e) => {
                            tracing::error!("Failed to check if {} exists: {}", EOS_SOCKET, e);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            });

            {
                let state = Arc::new(AppState {
                    config: config.clone(),
                    sys: sys.clone(),
                });

                let app = Router::new()
                    .route("/spawn", post(spawn))
                    .route("/send", post(send))
                    .route("/pause", post(pause))
                    .route("/unpause", post(unpause))
                    .route("/tick/now", post(tick))
                    .route("/tick/reset", post(reset_tick))
                    .route("/tick/set", post(set_tick))
                    .route("/list", post(list))
                    .route("/kill", post(kill))
                    .route("/shutdown", post(shutdown))
                    .with_state(state);

                tracing::info!("RPC server listening on port {}", RPC_PORT);
                let listener = tokio::net::TcpListener::bind(format!("localhost:{}", RPC_PORT))
                    .await
                    .unwrap();
                axum::serve(listener, app).await.unwrap();
            }
        }
    }
    Ok(())
}
