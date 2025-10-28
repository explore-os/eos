use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::bail;

#[cfg(feature = "_setup")]
use clap::Command;
use clap::{Parser, Subcommand};
use common::{EosServiceClient, Message, Props, Response};
use futures::future;
use futures_util::StreamExt;

use stringlit::s;
use tarpc::{client, context, tokio_serde::formats::Bincode};

#[cfg(feature = "_setup")]
use clap_complete::{aot::Fish, generate_to};
use rs9p::srv::srv_async_unix;
use tokio::sync::RwLock;

use crate::{
    common::{
        DEFAULT_TICK, EOS_SOCKET, RPC_SOCKET,
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
enum SockType {
    Mount,
    Rpc,
}

#[derive(Subcommand)]
enum Action {
    Root,
    Sock {
        #[command(subcommand)]
        sock_type: SockType,
    },
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

async fn handle_response(response: Response) {
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
            eprintln!("Failed: {err}")
        }
    }
}

async fn spawn_actor(client: &EosServiceClient, props: Props) -> anyhow::Result<()> {
    let response = client.spawn(context::current(), props).await?;
    handle_response(response).await;
    Ok(())
}

async fn list(client: &EosServiceClient) -> anyhow::Result<()> {
    let response = client.list(context::current()).await?;
    handle_response(response).await;
    Ok(())
}

struct Config {
    tick: u64,
}

async fn client() -> anyhow::Result<EosServiceClient> {
    let mut transport = tarpc::serde_transport::unix::connect(RPC_SOCKET, Bincode::default);
    transport.config_mut().max_frame_length(usize::MAX);
    Ok(EosServiceClient::new(client::Config::default(), transport.await?).spawn())
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
        .level(log::LevelFilter::Info)
        .level_for("tarpc", log::LevelFilter::Error)
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
        .level(log::LevelFilter::Info)
        .level_for("rs9p", log::LevelFilter::Warn)
        // .level_for("tarpc", log::LevelFilter::Error)
        .chain(std::io::stdout())
        .chain(fern::DateBased::new("/explore/logs/eos.log.", "%Y-%m-%d"))
        .apply()?;
    Ok(())
}

async fn send_shutdown(client: &EosServiceClient) -> anyhow::Result<()> {
    let response = client.shutdown(context::current()).await?;
    handle_response(response).await;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

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
            let script = tokio::fs::read_to_string(PathBuf::from(
                shellexpand::full(&script.display().to_string())?.to_string(),
            ))
            .await?;
            let client = client().await?;
            spawn_actor(&client, Props { id, script }).await?;
        }
        Action::List => {
            let client = client().await?;
            list(&client).await?;
        }
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
            let client = client().await?;
            let response = client.send(context::current(), msg).await?;
            handle_response(response).await;
        }
        Action::Kill { paths } => {
            let ids: Result<Vec<_>, _> = paths
                .iter()
                .map(|p| {
                    p.file_name()
                        .ok_or_else(|| anyhow::anyhow!("Invalid path: no file name found"))
                        .map(|name| name.display().to_string())
                })
                .collect();

            let client = client().await?;
            let response = client.kill(context::current(), ids?).await?;
            handle_response(response).await;
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

            let client = client().await?;
            let response = client.pause(context::current(), id).await?;
            handle_response(response).await;
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

            let client = client().await?;
            let response = client.unpause(context::current(), id).await?;
            handle_response(response).await;
        }
        Action::Tick { command } => {
            let client = client().await?;
            match command {
                TickCommand::Now => {
                    let response = client.tick(context::current()).await?;
                    handle_response(response).await;
                }
                TickCommand::Reset => {
                    let response = client.reset_tick(context::current()).await?;
                    handle_response(response).await;
                }
                TickCommand::Set { milliseconds } => {
                    let response = client.set_tick(context::current(), milliseconds).await?;
                    handle_response(response).await;
                }
            }
        }
        Action::Plot { value } => {
            common::teleplot(&value)?;
        }
        Action::Sock {
            sock_type: SockType::Mount,
        } => {
            print!("{EOS_SOCKET}");
        }
        Action::Sock {
            sock_type: SockType::Rpc,
        } => {
            print!("{RPC_SOCKET}");
        }
        Action::Shutdown => {
            let client = client().await?;
            send_shutdown(&client).await?;
        }
        Action::Serve => {
            let config = Arc::new(RwLock::new(Config { tick: DEFAULT_TICK }));
            let sys = Arc::new(RwLock::new(System::new()));

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
                                .arg("/explore/system")
                                .spawn()
                            {
                                Ok(_) => tracing::info!("Successfully mounted 9p filesystem"),
                                Err(e) => tracing::error!("Failed to mount 9p filesystem: {}", e),
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

            // Start RPC server
            {
                use common::EosService;
                use tarpc::server::{self, Channel};

                #[derive(Clone)]
                struct EosServer {
                    config: Arc<RwLock<Config>>,
                    sys: Arc<RwLock<System>>,
                }

                impl EosService for EosServer {
                    async fn spawn(self, _: context::Context, props: Props) -> Response {
                        match self.sys.write().await.spawn_actor(props).await {
                            Ok(id) => {
                                _ = teleplot("system.actor.spawned:1");
                                Response::Spawned { id }
                            }
                            Err(err) => Response::Failed {
                                err: err.to_string(),
                            },
                        }
                    }

                    async fn list(self, _: context::Context) -> Response {
                        let actors = self
                            .sys
                            .read()
                            .await
                            .actors
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>();
                        Response::Actors { actors }
                    }

                    async fn send(self, _: context::Context, msg: Message) -> Response {
                        let mut sys = self.sys.write().await;
                        if let Some(actor) = sys.actors.get_mut(&msg.to) {
                            actor.mailbox.push_back(dbg!(msg));
                        }
                        Response::Done
                    }

                    async fn pause(self, _: context::Context, id: Option<String>) -> Response {
                        let mut sys = self.sys.write().await;
                        if let Some(id) = id
                            && let Some(actor) = sys.actors.get_mut(&id)
                        {
                            actor.paused = true;
                        } else {
                            sys.paused = true;
                        }
                        Response::Done
                    }

                    async fn unpause(self, _: context::Context, id: Option<String>) -> Response {
                        let mut sys = self.sys.write().await;
                        if let Some(id) = id
                            && let Some(actor) = sys.actors.get_mut(&id)
                        {
                            actor.paused = false;
                        } else {
                            sys.paused = false;
                        }
                        Response::Done
                    }

                    async fn tick(self, _: context::Context) -> Response {
                        let mut sys = self.sys.write().await;
                        match sys.tick().await {
                            Ok(()) => Response::Done,
                            Err(err) => Response::Failed {
                                err: err.to_string(),
                            },
                        }
                    }

                    async fn set_tick(self, _: context::Context, tick: u64) -> Response {
                        let mut config = self.config.write().await;
                        config.tick = tick;
                        Response::Done
                    }

                    async fn reset_tick(self, _: context::Context) -> Response {
                        let mut config = self.config.write().await;
                        config.tick = DEFAULT_TICK;
                        Response::Done
                    }

                    async fn rename(
                        self,
                        _: context::Context,
                        old: String,
                        new: String,
                    ) -> Response {
                        let mut sys = self.sys.write().await;
                        if sys.actors.contains_key(&new) {
                            Response::Failed {
                                err: s!("Actor with the same id already exists"),
                            }
                        } else {
                            if let Some(actor) = sys.actors.remove(&old) {
                                sys.actors.insert(new, actor);
                            }
                            Response::Done
                        }
                    }

                    async fn kill(self, _: context::Context, ids: Vec<String>) -> Response {
                        let mut sys = self.sys.write().await;
                        for id in ids {
                            if let Err(err) = sys.kill_actor(&id).await {
                                tracing::error!("Failed to kill actor {}: {}", id, err);
                            }
                        }
                        Response::Done
                    }

                    async fn shutdown(self, _: context::Context) -> Response {
                        if let Err(e) = nix::sys::signal::kill(
                            nix::unistd::getpid(),
                            nix::sys::signal::Signal::SIGTERM,
                        ) {
                            tracing::error!("Failed to send SIGTERM: {}", e);
                        }
                        std::process::exit(0);
                    }
                }

                async fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
                    tokio::spawn(fut);
                }

                // Remove existing socket if it exists
                let _ = tokio::fs::remove_file(RPC_SOCKET).await;

                tracing::info!("RPC server listening on {}", RPC_SOCKET);

                let mut listener = match tarpc::serde_transport::unix::listen(
                    RPC_SOCKET,
                    Bincode::default,
                )
                .await
                {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("Failed to create RPC listener: {}", e);
                        return Ok(());
                    }
                };
                listener.config_mut().max_frame_length(usize::MAX);
                listener
                    // Ignore accept errors.
                    .filter_map(|r| future::ready(r.ok()))
                    .map(server::BaseChannel::with_defaults)
                    .map(|channel| {
                        let server = EosServer {
                            config: config.clone(),
                            sys: sys.clone(),
                        };
                        channel.execute(server.serve()).for_each(spawn)
                    })
                    .for_each(|_| async {})
                    .await;
            }
        }
    }
    Ok(())
}
