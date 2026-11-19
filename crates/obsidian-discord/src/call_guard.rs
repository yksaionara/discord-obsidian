//! Takes this account out of its call when Discord Obsidian is ended without
//! quitting, as Task Manager's End task does. Quitting ends the gateway
//! session, which leaves the call at once; a killed process can't, and
//! Discord would keep the session, and this account in its call, for minutes.
//!
//! While a call lasts, a guard process, `obsidian --call-guard`, waits on a
//! pipe from the app. Leaving the call says `bye` and the guard exits. If the
//! pipe closes without it, the app was killed: the guard connects to
//! Discord, leaves the call and ends its own session. It holds no more than
//! the pipe while it waits.
//!
//! On Windows the guard is started through a launcher that exits at once,
//! so the guard isn't the app's child: Task Manager ends an app's children
//! with it.

use std::{
    io::{BufRead, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::OnceLock,
    time::Duration,
};

use anyhow::{Context, Result};
use serde_json::json;

use crate::gateway::{Gateway, Qos, Reconnects};

/// The argument that starts the guard.
pub const ARGUMENT: &str = "--call-guard";
/// The guard proper, started by the launcher.
const RUN: &str = "--call-guard-run";

/// The program that runs the guard, set once by the app; without it, as in
/// tests, calls have no guard.
static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// Lets calls start a guard, by running `program` with [`ARGUMENT`].
pub fn enable(program: PathBuf) {
    let _ = PROGRAM.set(program);
}

/// How long the guard tries to reach Discord.
const LEAVE_WITHIN: Duration = Duration::from_secs(20);

/// A call's guard, from the app's side. Dropped, it tells the guard the
/// call ended.
pub struct Guard {
    stdin: Option<ChildStdin>,
    launcher: Child,
}

impl Guard {
    /// Starts a guard for a call in `guild`, or the private one for a DM.
    pub fn start(token: &str, guild: &str) -> Option<Self> {
        let program = PROGRAM.get()?;
        let mut command = Command::new(program);
        command
            .arg(ARGUMENT)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut launcher = command
            .spawn()
            .inspect_err(|error| tracing::warn!(%error, "could not start the call's guard"))
            .ok()?;
        let mut stdin = launcher.stdin.take()?;
        writeln!(stdin, "token {token}\nguild {guild}").ok()?;
        stdin.flush().ok()?;
        Some(Self {
            stdin: Some(stdin),
            launcher,
        })
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "bye");
            let _ = stdin.flush();
        }
        // The launcher exits as soon as the guard is up.
        let _ = self.launcher.try_wait();
    }
}

/// The guard process: `obsidian --call-guard`, as the launcher, or the guard
/// it starts. Returns once the call ended or was left for the app.
pub fn run(argument: &str) -> Result<()> {
    if argument == ARGUMENT && cfg!(windows) {
        let program = std::env::current_exe().context("no program to start the guard")?;
        let mut command = Command::new(program);
        command
            .arg(RUN)
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
        }
        command.spawn().context("could not start the guard")?;
        return Ok(());
    }
    let mut token = None;
    let mut guild = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        match line.split_once(' ') {
            Some(("token", value)) => token = Some(value.to_owned()),
            Some(("guild", value)) => guild = Some(value.to_owned()),
            _ if line == "bye" => return Ok(()),
            _ => {}
        }
    }
    // The app went without saying bye: it was ended.
    let (Some(token), Some(guild)) = (token, guild) else {
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("no runtime to leave the call")?;
    runtime.block_on(async {
        tokio::time::timeout(LEAVE_WITHIN, leave(&token, &guild))
            .await
            .context("Discord took too long to answer")?
    })
}

/// Connects as this account, leaves its call in `guild`, and ends the
/// session again, invisibly.
async fn leave(token: &str, guild: &str) -> Result<()> {
    if let Some(number) = crate::cache::build_number() {
        crate::client::recall_build_number(number);
    }
    let presence = json!({"status": "invisible", "since": 0, "activities": [], "afk": false});
    let (mut gateway, ready) =
        Gateway::connect(token, presence, &mut Reconnects::default(), Qos::default()).await?;
    drop(ready);
    let server = (guild != crate::ready::PRIVATE).then_some(guild);
    gateway
        .update_voice_state(server, None, false, false, false)
        .await?;
    // The voice state goes out before the close, in order.
    gateway.close().await;
    Ok(())
}
