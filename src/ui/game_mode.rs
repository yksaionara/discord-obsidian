//! Game Mode: while a game runs, Discord Obsidian steps out of its way. Only
//! calls, mentions and DMs sound or notify, animated pictures and the
//! window's transitions hold still, and memory the window doesn't need goes
//! back to the system, so the game has it. It ends when the game does.

use super::*;

impl Adapter {
    /// Turns Game Mode on or off when the game Discord sees, or the setting,
    /// changes. Cheap when nothing did.
    pub(super) fn watch_game_mode(&mut self, window: &ObsidianWindow) {
        let gaming =
            self.layout.game_mode.unwrap_or(true) && self.app.state.discord_playing.is_some();
        if gaming == GAMING.get() {
            return;
        }
        GAMING.set(gaming);
        window.set_gaming(gaming);
        window
            .global::<obsidian_ui::Theme>()
            .set_reduce_motion(self.layout.reduce_motion || gaming);
        show_motion(window);
        if gaming {
            tracing::info!(
                game = self
                    .app
                    .state
                    .discord_playing
                    .as_ref()
                    .map(|playing| playing.name.as_str()),
                "game mode on"
            );
            shell::release_memory();
        } else {
            tracing::info!("game mode off");
        }
    }

    /// Whether a conversation is a DM or group DM.
    pub(super) fn direct(&self, room: &str) -> bool {
        self.app
            .state
            .rooms
            .iter()
            .any(|chat| chat.id == room && chat.direct)
    }
}
