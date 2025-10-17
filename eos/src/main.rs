use std::path::{Path, PathBuf};

use anyhow::bail;
use clap::{Parser, Subcommand};
use nanoid::nanoid;
use supervisor::{ACTOR_DIR, PAUSE_FILE, Props, SEND_DIR, SPAWN_DIR};

#[derive(Parser)]
struct Cli {
    #[arg(default_value = "/eos")]
    root: PathBuf,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand, PartialEq)]
enum Action {
    Script {
        script: String,
        // #[arg(short, long)]
        // copy: Option<Vec<PathBuf>>,
        // #[arg(short, long)]
        // args: Option<Vec<String>>,
    },
    Spawn {
        path: String,
        // #[arg(short, long)]
        // copy: Option<Vec<PathBuf>>,
        // #[arg(short, long)]
        // args: Option<Vec<String>>,
    },
    List,
    Refresh,
    Pause {
        path: PathBuf,
    },
    Unpause {
        path: PathBuf,
    },
    Send {
        path: PathBuf,
        msg: String,
    },
    Kill {
        path: PathBuf,
    },
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

fn cleanup(pid: usize) -> anyhow::Result<()> {
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGUSR2,
    )?)
}

fn get_root_pid(root: impl AsRef<Path>) -> anyhow::Result<usize> {
    let pid_string = std::fs::read_to_string(root.as_ref().join(".pid"))?;
    Ok(pid_string.parse::<usize>()?)
}

fn main() -> anyhow::Result<()> {
    let Cli { root, command } = Cli::parse();
    let pid_file = root.join(".pid");
    if !pid_file.exists() && Action::Start != command {
        eprintln!("Actor system is not running!");
    }
    match command {
        Action::Refresh => cleanup(get_root_pid(root)?)?,
        Action::Script { script } => {
            let mut script_copy = vec![std::fs::canonicalize(PathBuf::from(
                shellexpand::full(&script)?.to_string(),
            ))?];
            // script_copy.extend(copy.unwrap_or_default().into_iter());
            let props = Props {
                path: PathBuf::from("/usr/local/bin/script-actor"),
                args: Vec::new(),
                copy: script_copy,
            };
            std::fs::write(
                root.join(SPAWN_DIR).join(nanoid!()),
                serde_json::to_string_pretty(&props)?,
            )?;
            notify(get_root_pid(root)?)?;
        }
        Action::Spawn { path } => {
            let props = Props {
                path: std::fs::canonicalize(PathBuf::from(shellexpand::full(&path)?.to_string()))?,
                args: args.unwrap_or_default(),
                copy: copy.unwrap_or_default(),
            };
            std::fs::write(
                root.join(SPAWN_DIR).join(nanoid!()),
                serde_json::to_string_pretty(&props)?,
            )?;
            notify(get_root_pid(root)?)?;
        }
        Action::List => {
            if !root.join(".pid").exists() {
                bail!("The actor system isn't running!");
            }
            cleanup(get_root_pid(&root)?)?;
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
            cleanup(get_root_pid(root)?)?;
        }
        Action::Shutdown => {
            kill(get_root_pid(root)?)?;
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
    }
    Ok(())
}
