//! Discord's Streamer Mode: while it's on, what a stream shouldn't show
//! stays hidden. As Discord's client keeps it (`StreamerModeStore`), it's
//! off until turned on, or until OBS or XSplit runs with "Automatically
//! enable" on, and once on it hides personal information and invite links,
//! and holds back sound effects and notifications, each unless turned off.
//! Hiding the window from screen capture is off unless turned on, and only
//! offered where the system can, on Windows.

use super::*;

/// Streamer Mode's settings, in `layout.json`, with Discord's defaults.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct StreamerMode {
    pub enabled: bool,
    /// On while OBS or XSplit runs, off once they stop.
    pub auto_toggle: bool,
    pub hide_personal_information: bool,
    pub hide_instant_invites: bool,
    pub disable_sounds: bool,
    pub disable_notifications: bool,
    pub enable_content_protection: bool,
}

impl Default for StreamerMode {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_toggle: true,
            hide_personal_information: true,
            hide_instant_invites: true,
            disable_sounds: true,
            disable_notifications: true,
            enable_content_protection: false,
        }
    }
}

/// What Streamer Mode hides now: nothing while it's off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Hiding {
    pub personal: bool,
    pub invites: bool,
    pub sounds: bool,
    pub notifications: bool,
    pub window: bool,
}

impl StreamerMode {
    pub fn hiding(&self) -> Hiding {
        if !self.enabled {
            return Hiding::default();
        }
        Hiding {
            personal: self.hide_personal_information,
            invites: self.hide_instant_invites,
            sounds: self.disable_sounds,
            notifications: self.disable_notifications,
            window: self.enable_content_protection && CAN_PROTECT,
        }
    }
}

thread_local! {
    /// What Streamer Mode hides, for the code that draws what it hides.
    static HIDING: Cell<Hiding> = const {
        Cell::new(Hiding {
            personal: false,
            invites: false,
            sounds: false,
            notifications: false,
            window: false,
        })
    };
}

/// What Streamer Mode hides now.
pub(super) fn hiding() -> Hiding {
    HIDING.get()
}

/// Whether the window can be kept out of screen captures here.
pub(super) const CAN_PROTECT: bool = cfg!(windows);

/// What an invite link's field says while Streamer Mode hides it; Copy
/// still copies it, as in Discord.
pub(super) const HIDDEN_LINK: &str = "Hidden; Use copy button";

/// The settings' switches, by the number the window gives each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Switch {
    Enabled,
    AutoToggle,
    HidePersonal,
    HideInvites,
    DisableSounds,
    DisableNotifications,
    ContentProtection,
}

impl Switch {
    pub fn from_index(index: i32) -> Option<Self> {
        Some(match index {
            0 => Self::Enabled,
            1 => Self::AutoToggle,
            2 => Self::HidePersonal,
            3 => Self::HideInvites,
            4 => Self::DisableSounds,
            5 => Self::DisableNotifications,
            6 => Self::ContentProtection,
            _ => return None,
        })
    }
}

impl Adapter {
    /// Flips one of Streamer Mode's switches, and shows what it changes.
    pub(super) fn set_streamer_mode(&mut self, window: &ObsidianWindow, switch: Switch, on: bool) {
        let mode = &mut self.layout.streamer_mode;
        let setting = match switch {
            Switch::Enabled => &mut mode.enabled,
            Switch::AutoToggle => &mut mode.auto_toggle,
            Switch::HidePersonal => &mut mode.hide_personal_information,
            Switch::HideInvites => &mut mode.hide_instant_invites,
            Switch::DisableSounds => &mut mode.disable_sounds,
            Switch::DisableNotifications => &mut mode.disable_notifications,
            Switch::ContentProtection => &mut mode.enable_content_protection,
        };
        if *setting == on {
            return;
        }
        *setting = on;
        if switch == Switch::Enabled {
            self.streamer_notice_dismissed = false;
            tracing::info!(on, "streamer mode");
        }
        self.layout.write(&self.root);
        self.show_streamer_mode(window);
        self.refresh(window);
    }

    /// OBS or XSplit started or stopped: with "Automatically enable" on,
    /// Streamer Mode follows, as Discord's RUNNING_STREAMER_TOOLS_CHANGE.
    pub(super) fn streamer_tools_running(&mut self, window: &ObsidianWindow, running: bool) {
        if self.layout.streamer_mode.auto_toggle {
            self.set_streamer_mode(window, Switch::Enabled, running);
        }
    }

    /// The settings' switches, the notice over the window, and what's
    /// hidden; the window kept out of screen captures, where it can be.
    pub(super) fn show_streamer_mode(&self, window: &ObsidianWindow) {
        let mode = &self.layout.streamer_mode;
        let hiding = mode.hiding();
        let before = HIDING.replace(hiding);
        window.set_streamer_enabled(mode.enabled);
        window.set_streamer_auto(mode.auto_toggle);
        window.set_streamer_hide_personal(mode.hide_personal_information);
        window.set_streamer_hide_invites(mode.hide_instant_invites);
        window.set_streamer_no_sounds(mode.disable_sounds);
        window.set_streamer_no_notifications(mode.disable_notifications);
        window.set_streamer_capture(mode.enable_content_protection);
        window.set_streamer_can_protect(CAN_PROTECT);
        window.set_streamer_notice(mode.enabled && !self.streamer_notice_dismissed);
        if before.window != hiding.window {
            protect(window, hiding.window);
        }
    }

    /// Follows OBS and XSplit, which the game detection's scan notices.
    #[cfg(feature = "discord")]
    pub(super) fn watch_streamer_tools(&self) {
        let mut running =
            obsidian_core::obsidian_discord::game_detection::streamer_tools::subscribe();
        let weak = self.weak.clone();
        let watched = slint::spawn_local(async move {
            while running.changed().await.is_ok() {
                let now = *running.borrow_and_update();
                let Some(adapter) = weak.upgrade() else {
                    return;
                };
                let window = adapter.borrow().window.upgrade();
                if let Some(window) = window {
                    adapter.borrow_mut().streamer_tools_running(&window, now);
                }
            }
        });
        if let Err(error) = watched {
            tracing::debug!(%error, "could not follow streaming tools");
        }
    }
}

/// Keeps the window out of screen captures, or lets them see it again, as
/// Discord's "Hide Discord Window from Screen Capture" does through
/// Electron's content protection: on Windows, its display affinity.
#[cfg(windows)]
fn protect(window: &ObsidianWindow, on: bool) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    };
    window.window().with_winit_window(|winit| {
        let Ok(handle) = winit.window_handle() else {
            return;
        };
        if let RawWindowHandle::Win32(handle) = handle.as_raw() {
            let affinity = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
            // SAFETY: sets how the window shows to capture; a handle that
            // went away only fails.
            let done = unsafe { SetWindowDisplayAffinity(handle.hwnd.get() as _, affinity) };
            if done == 0 {
                tracing::debug!("could not change the window's capture affinity");
            }
        }
    });
}

#[cfg(not(windows))]
fn protect(_window: &ObsidianWindow, _on: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_as_discords_defaults_say_once_on() {
        let mut mode = StreamerMode::default();
        assert_eq!(mode.hiding(), Hiding::default(), "off until turned on");
        mode.enabled = true;
        let hiding = mode.hiding();
        assert!(hiding.personal && hiding.invites && hiding.sounds && hiding.notifications);
        assert!(!hiding.window, "screen capture sees it unless asked");
        mode.disable_sounds = false;
        assert!(!mode.hiding().sounds);
    }
}
