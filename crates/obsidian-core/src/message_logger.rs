//! The Message Logger add-on: messages deleted while shown stay, in red, and
//! edited ones keep what they said before. It keeps only what's already
//! loaded, in memory, and nothing of it reaches disk, so a restart shows
//! messages as Discord has them again. Its switches are Settings > Add-ons'.

use std::sync::{
    RwLock,
    atomic::{AtomicBool, Ordering},
};

/// The add-on's switches, as Settings > Add-ons sets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switches {
    /// The add-on is on at all.
    pub on: bool,
    /// Deleted messages stay, in red.
    pub deletions: bool,
    /// Edited messages keep what they said before.
    pub edits: bool,
    /// Bots' messages go as they always did.
    pub ignore_bots: bool,
    /// A ping deleted soon after says so, wherever it was.
    pub ghost_pings: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            on: true,
            deletions: true,
            edits: true,
            ignore_bots: false,
            ghost_pings: true,
        }
    }
}

// Off until the window sets them, so nothing changes without it.
static ON: AtomicBool = AtomicBool::new(false);
static DELETIONS: AtomicBool = AtomicBool::new(false);
static EDITS: AtomicBool = AtomicBool::new(false);
static IGNORE_BOTS: AtomicBool = AtomicBool::new(false);
static GHOST_PINGS: AtomicBool = AtomicBool::new(false);
/// This account's user id, whose own deleted messages go as they always did.
static ME: RwLock<String> = RwLock::new(String::new());

/// The most earlier texts one message keeps.
pub const MAX_EARLIER: usize = 10;

pub fn set(switches: Switches) {
    ON.store(switches.on, Ordering::Relaxed);
    DELETIONS.store(switches.deletions, Ordering::Relaxed);
    EDITS.store(switches.edits, Ordering::Relaxed);
    IGNORE_BOTS.store(switches.ignore_bots, Ordering::Relaxed);
    GHOST_PINGS.store(switches.ghost_pings, Ordering::Relaxed);
}

/// Remembers this account's user id.
pub fn note_me(me: &str) {
    if ME.read().is_ok_and(|known| *known == me) {
        return;
    }
    if let Ok(mut known) = ME.write() {
        me.clone_into(&mut known);
    }
}

// Tests set the switches for their own thread, so those running beside
// them keep the add-on off.
#[cfg(test)]
thread_local! {
    static TESTING: std::cell::Cell<Option<(Switches, &'static str)>> = const { std::cell::Cell::new(None) };
}

/// The switches for this thread's test, and who "this account" is.
#[cfg(test)]
pub fn test_with(switches: Option<Switches>, me: &'static str) {
    TESTING.set(switches.map(|switches| (switches, me)));
}

fn switches() -> Switches {
    #[cfg(test)]
    if let Some((switches, _)) = TESTING.get() {
        return switches;
    }
    #[cfg(test)]
    return Switches {
        on: false,
        ..Switches::default()
    };
    #[allow(unreachable_code)]
    Switches {
        on: ON.load(Ordering::Relaxed),
        deletions: DELETIONS.load(Ordering::Relaxed),
        edits: EDITS.load(Ordering::Relaxed),
        ignore_bots: IGNORE_BOTS.load(Ordering::Relaxed),
        ghost_pings: GHOST_PINGS.load(Ordering::Relaxed),
    }
}

fn is_me(author: &str) -> bool {
    #[cfg(test)]
    if let Some((_, me)) = TESTING.get() {
        return me == author;
    }
    ME.read().is_ok_and(|me| !me.is_empty() && *me == author)
}

/// Whether a deleted message by `author`, a bot or not, stays.
pub fn keeps_deleted(author: &str, bot: bool) -> bool {
    let switches = switches();
    switches.on && switches.deletions && !(bot && switches.ignore_bots) && !is_me(author)
}

/// Whether an edit by a bot or not keeps the text it replaced.
pub fn keeps_edits(bot: bool) -> bool {
    let switches = switches();
    switches.on && switches.edits && !(bot && switches.ignore_bots)
}

pub fn ghost_pings() -> bool {
    let switches = switches();
    switches.on && switches.ghost_pings
}

/// Now, in Unix milliseconds.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// How long after it was sent a message went, as its label says it:
/// "1m later", "3h later"; nothing within a minute.
pub fn later(sent: u64, deleted: u64) -> String {
    let seconds = deleted.saturating_sub(sent) / 1000;
    match seconds {
        ..60 => String::new(),
        60..3600 => format!("{}m later", seconds / 60),
        3600..86_400 => format!("{}h later", seconds / 3600),
        _ => format!("{}d later", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_long_after_it_went() {
        assert_eq!(later(0, 30_000), "");
        assert_eq!(later(0, 120_000), "2m later");
        assert_eq!(later(0, 7_200_000), "2h later");
        assert_eq!(later(0, 3 * 86_400_000), "3d later");
    }
}
