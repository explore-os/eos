use std::path::{Path, PathBuf};

use anyhow::bail;
use async_nats::{Client, connect};
use bytes::Bytes;
#[cfg(feature = "_setup")]
use clap::Command;
use clap::{Parser, Subcommand};
use eos::{
    Dirs, EOS_CTL, Message, PAUSE_FILE, PID_FILE, Props, ROOT, Request, Response, SEND_DIR,
    SPAWN_DIR, STATE_FILE, TICK_FILE,
};
use futures_util::StreamExt;
use nanoid::nanoid;
use tokio::fs;

#[cfg(feature = "_setup")]
use clap_complete::{aot::Fish, generate_to};

#[cfg(feature = "_setup")]
#[derive(Parser)]
struct SetupCli {
    out_dir: PathBuf,
}
#[derive(Parser)]
struct Cli {
    #[arg(short, long, default_value = "nats://msgbus:4222")]
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
    /// spawn an actor
    Spawn {
        /// the requested id for the actor
        #[arg(short, long)]
        id: Option<String>,
        #[command(subcommand)]
        command: SpawnCommand,
    },
    /// list all the running actors
    List,
    /// notifies the supervisor to perform some checks and clean up dead actors
    Update,
    /// pauses an actor
    Pause {
        /// the directory for the actor to pause
        path: PathBuf,
    },
    /// unpauses an actor
    Unpause {
        /// the directory for the actor to unpause
        path: PathBuf,
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
    /// helper to edit actor files
    Edit {
        /// the path to the actor which state you want to edit
        path: PathBuf,
    },
    /// kills an actor
    Kill {
        /// the path to the actor that should be killed
        paths: Vec<PathBuf>,
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
    Plot { value: String },
}

#[derive(Subcommand)]
enum SpawnCommand {
    Script { script: PathBuf },
    Bin { path: PathBuf },
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

async fn get_pid(path: impl AsRef<Path>) -> anyhow::Result<i32> {
    let pid_file = path.as_ref().join(PID_FILE);
    if !fs::try_exists(&pid_file).await? {
        bail!("The actor system isn't running!");
    }
    Ok(fs::read_to_string(pid_file).await?.parse()?)
}

fn kill(pid: i32) -> anyhow::Result<()> {
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGINT,
    )?)
}

fn notify(pid: i32) -> anyhow::Result<()> {
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGUSR1,
    )?)
}

async fn update_nats(client: &Client) -> anyhow::Result<()> {
    let session = nanoid!();

    let mut sub = client.subscribe(format!("eos.response.{session}")).await?;

    client
        .publish(
            EOS_CTL,
            Bytes::from(serde_json::to_vec(&Request {
                session_id: session.clone(),
                cmd: eos::Command::Update,
            })?),
        )
        .await?;

    while let Some(msg) = sub.next().await {
        if let Ok(response) = serde_json::from_slice::<Response>(&msg.payload) {
            match response {
                Response::Done => {}
                Response::Failed { err } => {
                    eprintln!("Failed to spawn actor: {err}")
                }
                _ => (),
            }
            break;
        }
    }
    Ok(())
}

async fn update_unix() -> anyhow::Result<()> {
    let Dirs { root_dir, .. } = Dirs::get();
    let pid_file = root_dir.join(PID_FILE);
    let pid = fs::read_to_string(pid_file).await?;
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid.parse()?),
        nix::sys::signal::Signal::SIGUSR2,
    )?)
}

async fn update(nats: Option<Client>) -> anyhow::Result<()> {
    if let Some(nats) = nats {
        Ok(update_nats(&nats).await?)
    } else {
        Ok(update_unix().await?)
    }
}

async fn spawn_nats(client: &Client, props: Props) -> anyhow::Result<()> {
    let session = nanoid!();

    let mut sub = client.subscribe(format!("eos.response.{session}")).await?;

    client
        .publish(
            EOS_CTL,
            Bytes::from(serde_json::to_vec(&Request {
                session_id: session.clone(),
                cmd: eos::Command::Spawn { props },
            })?),
        )
        .await?;

    while let Some(msg) = sub.next().await {
        if let Ok(response) = serde_json::from_slice::<Response>(&msg.payload) {
            match response {
                Response::Spawned { id } => {
                    println!("Actor spawned with id: {id}");
                }
                Response::Failed { err } => {
                    eprintln!("Failed to spawn actor: {err}")
                }
                _ => (),
            }
            break;
        }
    }

    Ok(())
}

async fn spawn_unix(props: Props) -> anyhow::Result<()> {
    let Dirs { root_dir, .. } = Dirs::get();
    std::fs::write(
        root_dir.join(SPAWN_DIR).join(nanoid!()),
        serde_json::to_string_pretty(&props)?,
    )?;
    notify(get_pid(root_dir).await?)?;
    Ok(())
}

async fn spawn(nats: Option<Client>, props: Props) -> anyhow::Result<()> {
    if let Some(nats) = nats {
        Ok(spawn_nats(&nats, props).await?)
    } else {
        Ok(spawn_unix(props).await?)
    }
}

async fn list_nats(client: &Client) -> anyhow::Result<()> {
    let session = nanoid!();

    let mut sub = client.subscribe(format!("eos.response.{session}")).await?;

    client
        .publish(
            EOS_CTL,
            Bytes::from(serde_json::to_vec(&Request {
                session_id: session.clone(),
                cmd: eos::Command::List,
            })?),
        )
        .await?;

    while let Some(msg) = sub.next().await {
        if let Ok(response) = serde_json::from_slice::<Response>(&msg.payload) {
            match response {
                Response::Actors { actors } => {
                    for actor in actors {
                        println!("- {actor}");
                    }
                }
                Response::Failed { err } => {
                    eprintln!("Failed to get actor list: {err}")
                }
                _ => (),
            }
            break;
        }
    }

    Ok(())
}
async fn list_unix() -> anyhow::Result<()> {
    let Dirs {
        root_dir,
        actor_dir,
        ..
    } = Dirs::get();
    if !root_dir.join(PID_FILE).exists() {
        bail!("The actor system isn't running!");
    }
    update_unix().await?;
    let mut entries = std::fs::read_dir(actor_dir)?;
    while let Some(Ok(dir)) = entries.next() {
        let actor_dir = dir.path();
        if actor_dir.is_file() || !actor_dir.join(PID_FILE).exists() {
            continue;
        }
        println!("{}", dir.file_name().display());
    }
    Ok(())
}

async fn list(nats: Option<Client>) -> anyhow::Result<()> {
    if let Some(nats) = nats {
        Ok(list_nats(&nats).await?)
    } else {
        Ok(list_unix().await?)
    }
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

    let Cli { nats, command } = Cli::parse();
    let root = Path::new(ROOT);
    let pid_file = root.join(PID_FILE);
    if !pid_file.exists() {
        eprintln!("Actor system is not running!");
    }
    let nats = connect(nats).await.ok();
    match command {
        Action::Db { name, command } => {
            let db = eos::Db::new(&name);
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
        Action::Update => update(nats).await?,
        Action::Spawn { id, command } => {
            let props = match command {
                SpawnCommand::Script { script } => Props {
                    id,
                    path: PathBuf::from("/usr/local/bin/script-actor"),
                    copy: vec![std::fs::canonicalize(PathBuf::from(
                        shellexpand::full(&script.display().to_string())?.to_string(),
                    ))?],
                    ..Default::default()
                },
                SpawnCommand::Bin { path } => Props {
                    id,
                    path,
                    ..Default::default()
                },
            };

            spawn(nats, props).await?;
        }
        Action::List => {
            list(nats).await?;
        }
        Action::Send { path, msg, sender } => {
            let id = path.file_name().expect("Invalid path!").display();
            let sender = if let Some(sender) = sender {
                if !sender.join(PID_FILE).exists() {
                    bail!("The specified sender is not running");
                }
                Some(
                    sender
                        .file_name()
                        .expect("Invalid path!")
                        .display()
                        .to_string(),
                )
            } else {
                None
            };
            std::fs::write(
                root.join(SEND_DIR)
                    .join(format!("{id}::{}.json", nanoid!())),
                serde_json::to_string_pretty(&Message {
                    sender,
                    payload: serde_json::from_str::<serde_json::Value>(&msg)?,
                })?,
            )?;
        }
        Action::Kill { paths } => {
            for path in paths {
                if !path.join(PID_FILE).exists() {
                    bail!("There is no actor running in the specified directory!");
                }
                let actor_pid = std::fs::read_to_string(path.join(PID_FILE))?;
                kill(actor_pid.parse()?)?;
            }
            update(nats).await?;
        }
        Action::Pause { path } => {
            if !path.join(PID_FILE).exists() {
                bail!("There is no actor running in the specified directory!");
            }
            std::fs::File::create(path.join(PAUSE_FILE))?;
        }
        Action::Unpause { path } => {
            if !path.join(PID_FILE).exists() {
                bail!("There is no actor running in the specified directory!");
            }
            std::fs::remove_file(path.join(PAUSE_FILE))?;
        }
        Action::Tick { command } => match command {
            TickCommand::Set { milliseconds } => {
                std::fs::write(root.join(TICK_FILE), milliseconds.to_string())?
            }
            TickCommand::Reset => std::fs::remove_file(root.join(TICK_FILE))?,
        },
        Action::Edit { path } => {
            if !path.join(PID_FILE).exists() {
                bail!("There is no actor running in the specified directory!");
            }
            _ = std::process::Command::new("code")
                .arg("-r")
                .arg(path.join(STATE_FILE))
                .spawn()?;
        }
        Action::Plot { value } => {
            eos::teleplot(&value)?;
        }
    }
    Ok(())
}
