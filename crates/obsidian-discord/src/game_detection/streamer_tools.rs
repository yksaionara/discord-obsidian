//! Whether OBS or XSplit runs on this computer, as Discord's desktop client
//! watches for them to turn Streamer Mode on and off by itself. The game
//! detection's scan of the processes notices them, so no second scan runs.
use std::sync::OnceLock;
use tokio::sync::watch;

/// The executables Discord's Streamer Mode setting names: OBS's and
/// XSplit's, by file name without its extension, in lower case.
const TOOLS: [&str; 5] = ["obs", "obs32", "obs64", "xsplit.core", "xsplitbroadcaster"];

fn channel() -> &'static watch::Sender<bool> {
    static CHANNEL: OnceLock<watch::Sender<bool>> = OnceLock::new();
    CHANNEL.get_or_init(|| watch::channel(false).0)
}

/// Follows whether a streaming tool runs.
pub fn subscribe() -> watch::Receiver<bool> {
    channel().subscribe()
}

pub(crate) fn set(running: bool) {
    channel().send_if_modified(|old| std::mem::replace(old, running) != running);
}

/// Whether a process's name is one of the streaming tools.
pub(crate) fn is_tool(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    TOOLS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_obs_and_xsplit_by_name() {
        for name in ["obs", "obs64.exe", "OBS", "XSplit.Core.exe"] {
            assert!(is_tool(name.as_ref()), "{name}");
        }
        for name in ["obsidian", "lobster.exe", "xsplit"] {
            assert!(!is_tool(name.as_ref()), "{name}");
        }
    }
}
