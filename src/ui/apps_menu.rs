//! The Apps submenu of a message's and a person's menu, as Discord's has it:
//! the apps' message or user commands in the room, which run on the message
//! or the person. It opens with the menu, says so while the room's commands
//! load, and fills in when they come.

use obsidian_core::application::{AppsMenu, MESSAGE_COMMANDS, USER_COMMANDS};

use super::*;

/// What a command's row reports when it's chosen, before the command's id.
pub(super) const APP_COMMAND: &str = "app-command:";

/// An Apps submenu showing: what its commands run on, and its rows, which
/// fill in once the room's commands come.
pub(super) struct OpenApps {
    room: String,
    kind: u64,
    target: String,
    rows: ModelRc<MenuRow>,
    loading: bool,
}

/// The rows of the Apps submenu of `kind` in `room`, and whether they wait
/// for the room's commands.
pub(super) fn rows(adapter: &Adapter, room: &str, kind: u64) -> (Vec<MenuRow>, bool) {
    let note = |text: &str| MenuRow {
        text: text.into(),
        disabled: true,
        plain: true,
        ..Default::default()
    };
    match adapter.app.apps_menu(room, kind) {
        AppsMenu::Loading => (vec![note("Loading...")], true),
        AppsMenu::Commands(commands) if commands.is_empty() => {
            (vec![note("No Commands Available")], false)
        }
        AppsMenu::Commands(commands) => (
            commands
                .into_iter()
                .map(|command| MenuRow {
                    id: format!("{APP_COMMAND}{}", command.id).into(),
                    text: command.name.into(),
                    initial: initial(&command.app),
                    picture: command.icon.unwrap_or_default().into(),
                    trailing: adapter.icons.send.clone(),
                    ..Default::default()
                })
                .collect(),
            false,
        ),
    }
}

/// The rows of a person's Apps, for their menu: the open Discord channel's
/// user commands.
pub(super) fn person_rows(adapter: &Adapter, user: &str) -> Vec<MenuRow> {
    adapter
        .app
        .person_apps_room(user)
        .map(|room| rows(adapter, room, USER_COMMANDS).0)
        .unwrap_or_default()
}

impl Adapter {
    /// A message's menu is opening: its Apps show, in a Discord channel.
    pub(super) fn open_message_apps(&mut self, window: &ObsidianWindow, item: &str) {
        let Some((room, target)) = self.app.message_apps_target(item) else {
            self.apps = None;
            window.set_menu_apps_shown(false);
            return;
        };
        let (rows, loading) = rows(self, &room, MESSAGE_COMMANDS);
        let rows = ModelRc::new(VecModel::from(rows));
        window.set_menu_apps(rows.clone());
        window.set_menu_apps_shown(true);
        self.watch_apps(window, room, MESSAGE_COMMANDS, target, rows, loading);
    }

    /// A person's menu opened with Apps, whose rows are the submenu at
    /// `index`.
    pub(super) fn open_person_apps(&mut self, window: &ObsidianWindow, user: &str, index: usize) {
        let room = self.app.person_apps_room(user).map(str::to_owned);
        let rows = window.get_menu_submenus().row_data(index);
        let (Some(room), Some(rows)) = (room, rows) else {
            self.apps = None;
            return;
        };
        let loading = matches!(self.app.apps_menu(&room, USER_COMMANDS), AppsMenu::Loading);
        self.watch_apps(window, room, USER_COMMANDS, user.to_owned(), rows, loading);
    }

    fn watch_apps(
        &mut self,
        window: &ObsidianWindow,
        room: String,
        kind: u64,
        target: String,
        rows: ModelRc<MenuRow>,
        loading: bool,
    ) {
        self.apps = Some(OpenApps {
            room: room.clone(),
            kind,
            target,
            rows,
            loading,
        });
        if loading {
            self.action(window, Action::LoadCommands(room));
        }
    }

    /// Fills in the Apps showing once the room's commands came.
    pub(super) fn refresh_apps(&mut self) {
        let Some(open) = self.apps.as_ref().filter(|open| open.loading) else {
            return;
        };
        let (rows, loading) = rows(self, &open.room, open.kind);
        if loading {
            return;
        }
        if let Some(model) = open.rows.as_any().downcast_ref::<VecModel<MenuRow>>() {
            model.set_vec(rows);
        }
        if let Some(open) = &mut self.apps {
            open.loading = false;
        }
    }

    /// Runs the command whose row was chosen on what its menu is about.
    pub(super) fn run_app_command(&mut self, window: &ObsidianWindow, id: &str) {
        let Some(command) = id.strip_prefix(APP_COMMAND) else {
            return;
        };
        let Some(open) = self.apps.take() else {
            return;
        };
        self.action(
            window,
            Action::RunMenuCommand {
                room: open.room,
                command: command.to_owned(),
                target: open.target,
            },
        );
    }
}
