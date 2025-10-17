use std::path::{Path, PathBuf};

use anyhow::bail;
use clap::{Command, Parser, Subcommand};
use clap_complete::{aot::Fish, generate_to};
use eos::{ACTOR_DIR, PAUSE_FILE, Props, ROOT, SEND_DIR, SPAWN_DIR};
use nanoid::nanoid;

#[derive(Parser)]
#[command(name = "eos", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

impl Cli {
    fn command() -> Command {
        <Self as clap::CommandFactory>::command()
    }
}

#[derive(Subcommand)]
enum Action {
    /// spawn a script actor
    Script {
        /// the script that gets copied into the actor directory,
        /// which the actor then reads and runs every time it receiving a message
        script: String,
    },
    /// spawn a custom binary as actor
    Spawn {
        /// the path to the binary that gets started as an EOS actor
        path: String,
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
        /// the path to the actor the message should be sent
        path: PathBuf,
        /// a string containing the json representation of a message
        msg: String,
    },
    /// kills an actor
    Kill {
        /// the path to the actor that should be killed
        path: PathBuf,
    },
    /// changes the tick rate of the system
    Tick {
        #[command(subcommand)]
        command: TickCommand,
    },
    #[command(hide = true)]
    Completions { out_dir: PathBuf },
}

#[derive(Subcommand)]
enum TickCommand {
    /// sets the tick rate of the system
    Set {
        /// the tick rate of the system in milliseconds
        #[arg(value_parser = clap::value_parser!(u64).range(500..))]
        milliseconds: u64,
    },
    /// resets the tick rate of the system
    Reset,
}

fn kill(pid: usize) -> anyhow::Result<()> {
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGINT,
    )?)
}

fn notify(pid: usize) -> anyhow::Result<()> {
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGUSR1,
    )?)
}

fn update(pid: usize) -> anyhow::Result<()> {
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGUSR2,
    )?)
}

fn get_root_pid() -> anyhow::Result<usize> {
    let pid_string = std::fs::read_to_string(Path::new(ROOT).join(".pid"))?;
    Ok(pid_string.parse::<usize>()?)
}

fn main() -> anyhow::Result<()> {
    let Cli { command } = Cli::parse();
    let root = Path::new(ROOT);
    let pid_file = root.join(".pid");
    if !pid_file.exists() {
        eprintln!("Actor system is not running!");
    }
    match command {
        Action::Update => update(get_root_pid()?)?,
        Action::Script { script } => {
            let props = Props {
                path: PathBuf::from("/usr/local/bin/script-actor"),
                copy: vec![std::fs::canonicalize(PathBuf::from(
                    shellexpand::full(&script)?.to_string(),
                ))?],
                ..Default::default()
            };
            std::fs::write(
                root.join(SPAWN_DIR).join(nanoid!()),
                serde_json::to_string_pretty(&props)?,
            )?;
            notify(get_root_pid()?)?;
        }
        Action::Spawn { path } => {
            let props = Props {
                path: std::fs::canonicalize(PathBuf::from(shellexpand::full(&path)?.to_string()))?,
                ..Default::default()
            };
            std::fs::write(
                root.join(SPAWN_DIR).join(nanoid!()),
                serde_json::to_string_pretty(&props)?,
            )?;
            notify(get_root_pid()?)?;
        }
        Action::List => {
            if !root.join(".pid").exists() {
                bail!("The actor system isn't running!");
            }
            update(get_root_pid()?)?;
            let mut entries = std::fs::read_dir(root.join(ACTOR_DIR))?;
            while let Some(Ok(dir)) = entries.next() {
                let actor_dir = dir.path();
                if actor_dir.is_file() || !actor_dir.join(".pid").exists() {
                    continue;
                }
                println!("{}", dir.file_name().display());
            }
        }
        Action::Send { path, msg } => {
            if !path.join(".pid").exists() {
                bail!("There is no actor running in the specified directory!");
            }
            let id = path.file_name().expect("Invalid path!").display();
            std::fs::write(
                root.join(SEND_DIR)
                    .join(format!("{id}::{}.json", nanoid!())),
                serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&msg)?)?,
            )?;
        }
        Action::Kill { path } => {
            if !path.join(".pid").exists() {
                bail!("There is no actor running in the specified directory!");
            }
            let actor_pid_string = std::fs::read_to_string(path.join(".pid"))?;
            let actor_pid = actor_pid_string.parse::<usize>()?;
            kill(actor_pid)?;
            update(get_root_pid()?)?;
        }
        Action::Pause { path } => {
            if !path.join(".pid").exists() {
                bail!("There is no actor running in the specified directory!");
            }
            std::fs::File::create(path.join(PAUSE_FILE))?;
        }
        Action::Unpause { path } => {
            if !path.join(".pid").exists() {
                bail!("There is no actor running in the specified directory!");
            }
            std::fs::remove_file(path.join(PAUSE_FILE))?;
        }
        Action::Tick { command } => match command {
            TickCommand::Set { milliseconds } => {
                std::fs::write(root.join(".tick"), milliseconds.to_string())?
            }
            TickCommand::Reset => std::fs::remove_file(root.join(".tick"))?,
        },
        Action::Completions { out_dir } => {
            let mut cmd = Cli::command();
            let bin_name = cmd.get_bin_name().unwrap().to_owned();
            generate_to(Fish, &mut cmd, bin_name, &out_dir)?;
        }
    }
    Ok(())
}
