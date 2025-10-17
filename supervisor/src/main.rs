use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::bail;
use env_logger::Env;
use eos::{ACTOR_DIR, MAILBOX_DIR, MAILBOX_HEAD, PAUSE_FILE, Props, ROOT, SEND_DIR, SPAWN_DIR};
use faccess::PathExt;
use nanoid::nanoid;
use tokio::signal::unix::{SignalKind, signal};
use tokio::{fs, process::Command, spawn};

async fn spawn_actor(
    root: impl AsRef<Path>,
    send_dir: impl AsRef<Path>,
    props: Props,
) -> anyhow::Result<String> {
    if !props.path.is_file() {
        bail!("{} is not a file", props.path.display());
    }

    if !props.path.executable() {
        bail!("{} is not an executable file", props.path.display());
    }
    let id = nanoid!();
    let actor_dir = root.as_ref().join(&id);
    fs::create_dir_all(&actor_dir).await?;
    for f in &props.copy {
        fs::copy(f, actor_dir.join(f.file_name().unwrap())).await?;
    }
    let message_dir = actor_dir.join(MAILBOX_DIR);
    fs::create_dir_all(&message_dir).await?;
    let message_file = message_dir.join(MAILBOX_HEAD);
    let state_file = actor_dir.join("state.json");
    let process = Command::new(&props.path)
        .arg(&id)
        .arg(state_file)
        .arg(message_file)
        .arg(send_dir.as_ref())
        .args(
            props
                .copy
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>(),
        )
        .args(props.args)
        .spawn()?;

    fs::write(
        actor_dir.join(".pid"),
        process.id().unwrap_or_default().to_string(),
    )
    .await?;

    Ok(id)
}

async fn check_props(
    props_dir: impl AsRef<Path>,
    actor_dir: impl AsRef<Path>,
    send_dir: impl AsRef<Path>,
) -> anyhow::Result<()> {
    log::info!("Checking spawn dir: {}", props_dir.as_ref().display());
    let mut entries = fs::read_dir(&props_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().is_dir() {
            continue;
        }
        let props_file = entry.path();
        log::info!("Found props file: {}", props_file.display());
        let content = fs::read_to_string(&props_file).await?;
        log::info!("Spawning from file: {}", props_file.display());
        let props: Props = serde_json::from_str::<Props>(&content)?;
        spawn_actor(&actor_dir, &send_dir, props).await?;
        fs::remove_file(props_file).await?;
    }
    Ok(())
}

async fn move_next_message(message_dir: impl AsRef<Path>, tick_speed: u64) -> anyhow::Result<bool> {
    let mut update = false;
    let mut entries = fs::read_dir(&message_dir).await?;
    let mut messages = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().is_dir() {
            continue;
        }
        messages.push((entry.path(), entry.metadata().await?.modified()?));
    }
    messages.sort_by(|(_, a), (_, b)| {
        if a < b {
            Ordering::Less
        } else if a > b {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    });
    let now = std::time::SystemTime::now();
    for (msg_path, t) in messages {
        if now.duration_since(t)?.as_secs() < tick_speed {
            continue;
        }
        fs::rename(
            &msg_path,
            msg_path
                .parent()
                .expect("It would be quite weird if there was no parent here")
                .join(MAILBOX_HEAD),
        )
        .await?;
        update = true;
        break;
    }
    Ok(update)
}

async fn check_actors(actor_dir: impl AsRef<Path>, tick_speed: u64) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(&actor_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().is_file() {
            continue;
        }
        let actor_dir = entry.path();
        if fs::try_exists(actor_dir.join(PAUSE_FILE)).await? {
            continue;
        }
        let message_dir = actor_dir.join(MAILBOX_DIR);
        if fs::try_exists(message_dir.join(MAILBOX_HEAD)).await? {
            continue;
        }
        if move_next_message(&message_dir, tick_speed).await? {
            tokio::time::sleep(Duration::from_secs(tick_speed)).await;
            let pid = fs::read_to_string(actor_dir.join(".pid"))
                .await?
                .parse::<usize>()?;
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGUSR1,
            )?;
        }
    }
    Ok(())
}

async fn kill_actors(actor_dir: impl AsRef<Path>) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(&actor_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().is_file() {
            continue;
        }
        let pid_string = fs::read_to_string(entry.path().join(".pid")).await?;
        let pid = pid_string.parse::<usize>()?;

        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        )?;

        for _ in 0..30 {
            if !fs::try_exists(Path::new("/proc").join(format!("{pid}"))).await? {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        fs::remove_dir_all(entry.path()).await?;
    }
    Ok(())
}

async fn check_alive(actor_dir: impl AsRef<Path>) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(&actor_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().is_file() {
            continue;
        }
        let pid_string = fs::read_to_string(entry.path().join(".pid")).await?;
        let pid = pid_string.parse::<usize>()?;
        if !fs::try_exists(Path::new("/proc").join(format!("{pid}"))).await? {
            fs::remove_dir_all(entry.path()).await?;
        }
    }
    Ok(())
}

async fn check_queue(
    actor_dir: impl AsRef<Path>,
    send_dir: impl AsRef<Path>,
    tick_speed: u64,
) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(&send_dir).await?;
    let mut messages = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().is_dir() {
            continue;
        }
        messages.push((entry.path(), entry.metadata().await?.modified()?));
    }
    let actor_dir = actor_dir.as_ref();
    let now = std::time::SystemTime::now();
    for (msg_path, t) in messages {
        if now.duration_since(t)?.as_secs() < tick_speed {
            continue;
        }
        if let Some((target, id)) = msg_path
            .file_name()
            .expect("ARE YOU KIDDING ME, OF COURSE A FILE MUST HAVE A FILE NAME. FFS!")
            .to_string_lossy()
            .split_once("::")
        {
            let target_dir = actor_dir.join(target).join(MAILBOX_DIR);
            fs::rename(&msg_path, &target_dir.join(id)).await?;
        }
    }
    Ok(())
}

async fn tick() -> anyhow::Result<u64> {
    let tick_path = Path::new(ROOT).join(".tick");
    if fs::try_exists(&tick_path).await? {
        Ok(fs::read_to_string(tick_path).await?.trim().parse()?)
    } else {
        fs::write(tick_path, "2").await?;
        Ok(2)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let root = Path::new(ROOT);
    if root.join(".pid").exists() {
        eprintln!(
            "The pid file for the supervisor already exists, terminating. If the supervisor is not running, feel free to delete the file and try again. ({})",
            root.join(".pid").display()
        );
        return Ok(());
    }

    let pid = std::process::id();
    log::info!("Running as PID: {pid}",);
    fs::write(root.join(".pid"), format!("{pid}")).await?;

    log::info!("supervisor started");

    let mut spawn_signal = signal(SignalKind::user_defined1())?;

    let props_dir = root.join(SPAWN_DIR);
    std::fs::create_dir_all(&props_dir)?;

    let actor_dir = root.join(ACTOR_DIR);
    std::fs::create_dir_all(&actor_dir)?;

    let send_dir = root.join(SEND_DIR);
    std::fs::create_dir_all(&send_dir)?;

    {
        let root_dir = root.clone();
        let send_dir = send_dir.clone();
        let props_dir = props_dir.clone();
        let actor_dir = actor_dir.clone();
        spawn(async move {
            loop {
                spawn_signal.recv().await;
                if fs::try_exists(root_dir.join(PAUSE_FILE))
                    .await
                    .expect("WHY YOU NO READ FILE?")
                {
                    continue;
                }
                if let Err(e) = check_props(&props_dir, &actor_dir, &send_dir).await {
                    log::error!("{e}");
                }
            }
        });
    }

    let mut cleanup_signal = signal(SignalKind::user_defined2())?;
    {
        let root_dir = root.clone();
        let actor_dir = actor_dir.clone();
        spawn(async move {
            loop {
                cleanup_signal.recv().await;
                if fs::try_exists(root_dir.join(PAUSE_FILE))
                    .await
                    .expect("WHY YOU NO READ FILE?")
                {
                    continue;
                }
                if let Err(e) = check_alive(&actor_dir).await {
                    log::error!("{e}");
                }
            }
        });
    }

    {
        let root_dir = root.clone();
        let actor_dir = actor_dir.clone();
        spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("Muahahaha, this should never happen!");
            log::info!("Actor system is shutting down");
            _ = fs::remove_file(root_dir.join(".pid")).await;
            _ = fs::remove_file(root_dir.join(".tick")).await;
            _ = kill_actors(&actor_dir).await;
            std::process::exit(0);
        });
    }

    {
        let root_dir = root.clone();
        let actor_dir = actor_dir.clone();
        spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if fs::try_exists(root_dir.join(PAUSE_FILE))
                    .await
                    .expect("WHY YOU NO READ FILE?")
                {
                    continue;
                }
                if let Err(e) = check_alive(&actor_dir).await {
                    log::error!("{e}");
                }
            }
        });
    }

    loop {
        let tick = tick().await?;
        tokio::time::sleep(Duration::from_secs(tick)).await;
        if fs::try_exists(root.join(PAUSE_FILE)).await? {
            continue;
        }
        if let Err(e) = check_actors(&actor_dir, tick).await {
            log::error!("{e}");
        }
        if let Err(e) = check_queue(&actor_dir, &send_dir, tick).await {
            log::error!("{e}");
        }
    }
}
