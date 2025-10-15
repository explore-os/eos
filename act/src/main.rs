use std::path::PathBuf;

use clap::{Parser, Subcommand};
use nanoid::nanoid;
use supervisor::{ACTOR_DIR, Props, SEND_DIR, SPAWN_DIR};

#[derive(Parser)]
struct Cli {
    #[arg(default_value = "/var/actors")]
    root: PathBuf,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Spawn { path: PathBuf, args: Vec<String> },
    List,
    Cleanup,
    Send { path: PathBuf, msg: String },
    Kill { path: PathBuf },
    Shutdown,
}

fn kill(pid: usize) -> anyhow::Result<()> {
    Ok(nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::sys::signal::Signal::SIGKILL,
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

fn main() -> anyhow::Result<()> {
    let Cli { root, command } = Cli::parse();
    let pid_file = root.join(".pid");
    let pid_string = std::fs::read_to_string(pid_file)?;
    let pid = pid_string.parse::<usize>()?;
    match command {
        Action::Cleanup => cleanup(pid)?,
        Action::Spawn { path, args } => {
            let props = Props { path, args };
            std::fs::write(
                root.join(SPAWN_DIR).join(nanoid!()),
                serde_json::to_string_pretty(&props)?,
            )?;
            notify(pid)?;
        }
        Action::List => {
            cleanup(pid)?;
            let mut entries = std::fs::read_dir(root.join(ACTOR_DIR))?;
            while let Some(Ok(dir)) = entries.next() {
                if dir.path().is_file() {
                    continue;
                }
                println!("{}", dir.file_name().display());
            }
        }
        Action::Send { path, msg } => {
            let id = path.file_name().expect("Invalid path!").display();
            std::fs::write(
                root.join(SEND_DIR)
                    .join(format!("{id}::{}.json", nanoid!())),
                serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&msg)?)?,
            )?;
        }
        Action::Kill { path } => {
            let actor_pid_string = std::fs::read_to_string(path.join(".pid"))?;
            let actor_pid = actor_pid_string.parse::<usize>()?;
            kill(actor_pid)?;
            cleanup(pid)?;
        }
        Action::Shutdown => {
            kill(pid)?;
        }
    }
    Ok(())
}
