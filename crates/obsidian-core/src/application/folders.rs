//! A server folder's Folder Settings, as Discord's client offers them from a
//! folder's menu: its name and its colour, which go into the account's
//! server list on Discord, everywhere it's signed in.

use super::*;

/// The longest folder name Discord's Folder Name field takes.
pub const FOLDER_NAME_LIMIT: usize = 32;
/// Discord's colour for a folder without one, its BRAND_500, which it saves
/// as no colour at all.
pub const DEFAULT_FOLDER_COLOR: u32 = 0x5865f2;

/// Folder Settings while it's open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderSettings {
    /// The folder, as [`chat::ServerFolder::id`].
    pub id: String,
    pub name: String,
    /// Its colour as RGB; none for Discord's default.
    pub color: Option<u32>,
}

impl Application {
    /// Folder Settings for a Discord folder on the rail, with its name and
    /// colour.
    pub(super) fn open_folder_settings(&mut self, id: String) -> Vec<Effect> {
        let Some(folder) = self
            .state
            .servers
            .iter()
            .filter(|server| server.discord)
            .filter_map(|server| server.folder.as_ref())
            .find(|folder| folder.id == id)
        else {
            return vec![];
        };
        self.state.folder_settings = Some(FolderSettings {
            id,
            name: folder.name.clone().unwrap_or_default(),
            color: folder.color,
        });
        vec![]
    }

    /// Its name, as typed, up to Discord's 32 characters.
    pub(super) fn write_folder_name(&mut self, name: String) -> Vec<Effect> {
        if let Some(settings) = &mut self.state.folder_settings {
            settings.name = name.chars().take(FOLDER_NAME_LIMIT).collect();
        }
        vec![]
    }

    /// Its colour; Discord's default is kept as none, as its client does.
    pub(super) fn pick_folder_color(&mut self, color: Option<u32>) -> Vec<Effect> {
        if let Some(settings) = &mut self.state.folder_settings {
            settings.color = color
                .map(|color| color & 0xff_ffff)
                .filter(|color| *color != DEFAULT_FOLDER_COLOR);
        }
        vec![]
    }

    /// Done: the folder takes its name, none when it's blank, and its colour.
    pub(super) fn save_folder_settings(&mut self) -> Vec<Effect> {
        let Some(settings) = self.state.folder_settings.take() else {
            return vec![];
        };
        let name = Some(settings.name.trim().to_owned()).filter(|name| !name.is_empty());
        match server_order::with_folder(&self.state.servers, &settings.id, name, settings.color) {
            Some(entries) => self.place_servers(entries),
            None => vec![],
        }
    }
}
