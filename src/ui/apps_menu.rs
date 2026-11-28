//! The Apps submenu of a message's and a person's menu, as Discord's has it:
//! the apps' message or user commands in the room, which run on the message
//! or the person. It opens with the menu, says so while the room's commands
//! load, and fills in when they come. As Discord's, a search field heads it,
//! then Frequently Used Commands and each app's commands under its name; a
//! search lists the commands whose name or app match.

use obsidian_core::application::{AppsMenu, MESSAGE_COMMANDS, MenuCommand, USER_COMMANDS};

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
    /// What its search field holds.
    query: String,
}

/// The id of the search field's row, and of a group's label; see MenuRows.
const SEARCH: &str = "menu-search";
const HEADING: &str = "menu-heading";

/// The rows of the Apps submenu of `kind` in `room`, for what its search
/// holds, and whether they wait for the room's commands. The search field
/// heads them, but while they load or there are none to search.
pub(super) fn rows(adapter: &Adapter, room: &str, kind: u64, query: &str) -> (Vec<MenuRow>, bool) {
    let note = |text: &str| MenuRow {
        text: text.into(),
        disabled: true,
        plain: true,
        ..Default::default()
    };
    let heading = |text: &str| MenuRow {
        id: HEADING.into(),
        text: text.into(),
        disabled: true,
        plain: true,
        ..Default::default()
    };
    let command = |command: MenuCommand, icon: bool| MenuRow {
        id: format!("{APP_COMMAND}{}", command.id).into(),
        text: command.name.into(),
        initial: if icon {
            initial(&command.app)
        } else {
            SharedString::default()
        },
        picture: if icon {
            command.icon.unwrap_or_default().into()
        } else {
            SharedString::default()
        },
        plain: !icon,
        trailing: adapter.icons.send.clone(),
        ..Default::default()
    };
    let searching = !query.trim().is_empty();
    match adapter.app.apps_menu(room, kind) {
        AppsMenu::Loading => (vec![note("Loading...")], true),
        AppsMenu::Commands(commands) if commands.is_empty() && !searching => {
            (vec![note("No Commands Available")], false)
        }
        AppsMenu::Commands(_) => {
            let layout = adapter
                .app
                .apps_layout(room, kind, query)
                .unwrap_or_default();
            let mut rows = vec![search_row()];
            rows.extend(listed(layout, &note, &heading, &command));
            (rows, false)
        }
    }
}

/// The search field heading the menu, as Discord's: "Search commands".
fn search_row() -> MenuRow {
    MenuRow {
        id: SEARCH.into(),
        text: "Search commands".into(),
        disabled: true,
        plain: true,
        ..Default::default()
    }
}

/// The rows under the search field.
fn listed(
    layout: obsidian_core::application::AppsLayout,
    note: &dyn Fn(&str) -> MenuRow,
    heading: &dyn Fn(&str) -> MenuRow,
    command: &dyn Fn(MenuCommand, bool) -> MenuRow,
) -> Vec<MenuRow> {
    if let Some(found) = layout.found {
        if found.is_empty() {
            return vec![note("No Commands Available")];
        }
        return found
            .into_iter()
            .map(|found| command(found, true))
            .collect();
    }
    let mut rows = Vec::new();
    if !layout.frequent.is_empty() {
        rows.push(heading("Frequently Used Commands"));
        rows.extend(
            layout
                .frequent
                .into_iter()
                .map(|found| command(found, true)),
        );
    }
    rows.push(heading("Apps"));
    for (app, icon, commands) in layout.apps {
        rows.push(MenuRow {
            text: app.as_str().into(),
            initial: initial(&app),
            picture: icon.unwrap_or_default().into(),
            disabled: true,
            ..Default::default()
        });
        rows.extend(commands.into_iter().map(|found| command(found, false)));
    }
    rows
}

/// The rows of a person's Apps, for their menu: the open Discord channel's
/// user commands.
pub(super) fn person_rows(adapter: &Adapter, user: &str) -> Vec<MenuRow> {
    adapter
        .app
        .person_apps_room(user)
        .map(|room| rows(adapter, room, USER_COMMANDS, "").0)
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
        let (rows, loading) = rows(self, &room, MESSAGE_COMMANDS, "");
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
            query: String::new(),
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
        let (rows, loading) = rows(self, &open.room, open.kind, &open.query);
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

    /// The Apps menu's search changed: the rows under its field follow,
    /// and the field itself stays, keeping what's typed.
    pub(super) fn search_apps(&mut self, query: &str) {
        let Some(open) = self.apps.as_mut() else {
            return;
        };
        open.query = query.to_owned();
        let (room, kind, query) = (open.room.clone(), open.kind, open.query.clone());
        let (rows, loading) = rows(self, &room, kind, &query);
        let Some(open) = self.apps.as_ref() else {
            return;
        };
        let Some(model) = open.rows.as_any().downcast_ref::<VecModel<MenuRow>>() else {
            return;
        };
        if loading || model.row_data(0).is_none_or(|row| row.id != SEARCH) {
            model.set_vec(rows);
            return;
        }
        while model.row_count() > 1 {
            model.remove(model.row_count() - 1);
        }
        model.extend(rows.into_iter().skip(1));
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
