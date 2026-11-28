//! Slash commands in the composer, as Discord's composer has them: `/` lists
//! the commands the room offers, a command chosen shows its options above
//! the composer, and Enter runs it. Discord's built-in commands (`/shrug`,
//! `/me`...) change the message instead and send it.

use obsidian_discord::app_commands::{self, BuiltIn, Choice, CommandIndex, CommandOption};

use crate::chat::Suggested;

use std::path::PathBuf;

use super::*;

/// How many commands the list holds; the window shows some at a time.
pub const COMMAND_SUGGESTIONS: usize = 50;

/// The section Discord's list files its own commands under.
const BUILT_IN: &str = "Built-In";

#[derive(Default)]
pub struct Slash {
    /// The commands each Discord room offers, once they came.
    indexes: HashMap<String, Arc<CommandIndex>>,
    /// Rooms whose commands are on their way.
    loading: HashSet<String>,
    /// Rooms whose commands Discord didn't list; their `/` words are text.
    failed: HashSet<String>,
    /// The command chosen from the list in each room, by id, among those
    /// sharing its name.
    picked: HashMap<String, String>,
    /// The draft whose list Escape closed, which stays closed until it
    /// changes.
    dismissed: Option<String>,
    /// What [`Self::shown`] was made from, so an event that changes neither
    /// makes it again.
    made_from: Option<(String, String, usize)>,
    /// An app's suggestions for the option being typed.
    suggestions: Option<Suggestions>,
    /// The suggestions last shown for each option, by room and option name,
    /// whose names send their values, as Discord's client sends them.
    last_suggested: HashMap<(String, String), Vec<Choice>>,
    /// What the selected room's composer shows above it.
    pub shown: Option<Shown>,
    /// The apps' commands this account used, by Discord's client's key for
    /// each, as its clients counted them, with this session's uses added;
    /// Frequently Used Commands ranks them.
    used: HashMap<String, chat::PlaceUses>,
    /// This session's uses, added again to the counts when they come.
    used_here: Vec<(String, u64)>,
    /// The files picked for file options, by room and option name.
    files: HashMap<(String, String), PathBuf>,
}

/// The most commands Discord's Apps menu lists under Frequently Used
/// Commands, and as matches of its search.
pub const MENU_COMMANDS_SHOWN: usize = 10;

/// An app's suggestions for one option of one command in one room, with
/// the other options as typed, by the text typed for it.
struct Suggestions {
    /// The room, the command's id, the option and the other options' values.
    key: (String, String, String, String),
    answers: HashMap<String, Answer>,
    /// Asks awaiting their answer, by nonce, with the text they're for.
    asked: HashMap<String, String>,
}

enum Answer {
    Waiting,
    Choices(Vec<Choice>),
    Failed,
}

/// How many texts' suggestions are kept for one option at most.
const KEPT_ANSWERS: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    /// The commands whose names start with what follows `/`, or a note
    /// while they load.
    Commands(Vec<CommandRow>),
    Loading,
    /// The command written, with its options.
    Options(OptionsBar),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandRow {
    /// The command's id, or a built-in's name.
    pub id: String,
    pub name: String,
    pub description: String,
    /// Its app's name, or "Built-In".
    pub app: String,
    pub icon: Option<String>,
    /// Its options' names, required ones first, shown after its name.
    pub options: Vec<String>,
    /// What choosing it writes in the composer.
    pub insert: String,
    /// It starts a new section: the first of its app.
    pub heading: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionsBar {
    /// The command with its subcommands, as `/music play`.
    pub command: String,
    pub app: String,
    pub icon: Option<String>,
    pub options: Vec<OptionChip>,
    /// The option being typed: its name, what it takes and its description.
    pub current: Option<(String, String)>,
    /// That option's choices matching what's typed, or its app's
    /// suggestions, with what choosing each writes in the composer.
    pub choices: Vec<(String, String)>,
    /// Over the choices: "OPTIONS", or "OPTIONS MATCHING" what's typed.
    pub choices_heading: String,
    /// Instead of choices: the app's suggestions are on their way, or
    /// didn't come.
    pub note: Option<String>,
    /// The option being typed takes a file, which a file picker chooses.
    pub takes_file: bool,
}

/// What an Apps menu lists: nothing yet while the room's commands load,
/// then its user or message commands, which may be none.
#[derive(Clone, Debug, PartialEq)]
pub enum AppsMenu {
    Loading,
    Commands(Vec<MenuCommand>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuCommand {
    pub id: String,
    pub name: String,
    pub app: String,
    pub icon: Option<String>,
}

/// An Apps menu as Discord's client lays it out: with nothing searched,
/// Frequently Used Commands, then each app with its commands; searched, the
/// commands whose name or app match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppsLayout {
    pub frequent: Vec<MenuCommand>,
    /// Each app's name and icon, and its commands, in the menu's order.
    pub apps: Vec<(String, Option<String>, Vec<MenuCommand>)>,
    /// What matched the search, while there is one.
    pub found: Option<Vec<MenuCommand>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionChip {
    pub name: String,
    pub required: bool,
    /// A value is typed for it.
    pub given: bool,
    /// It's the one being typed.
    pub current: bool,
}

impl Slash {
    /// Forgets the commands of a server's rooms, which Discord's list of
    /// them changed; the next `/` asks again.
    pub(super) fn forget_server(&mut self, server: &str) {
        let of_server = |room: &String| {
            discord::parse_room(room).is_some_and(|(guild, _)| discord::server_id(guild) == server)
        };
        self.indexes.retain(|room, _| !of_server(room));
        self.failed.retain(|room| !of_server(room));
        self.made_from = None;
    }

    /// An app answered an ask for suggestions, or the ask failed or was
    /// dropped, which a later look at the same text asks again.
    pub(super) fn suggested(&mut self, nonce: &str, answer: Suggested) {
        let Some(suggestions) = &mut self.suggestions else {
            return;
        };
        let Some(query) = suggestions.asked.remove(nonce) else {
            return;
        };
        match answer {
            Suggested::Choices(choices) => {
                suggestions.answers.insert(query, Answer::Choices(choices));
            }
            Suggested::Failed => {
                suggestions.answers.insert(query, Answer::Failed);
            }
            Suggested::Dropped => {
                suggestions.answers.remove(&query);
            }
        }
        self.made_from = None;
    }
}

impl Application {
    /// The commands Discord listed for a room arrived.
    pub(super) fn commands_listed(
        &mut self,
        room: String,
        index: Result<Arc<CommandIndex>, String>,
    ) {
        let slash = &mut self.state.slash;
        slash.loading.remove(&room);
        match index {
            Ok(index) => {
                slash.indexes.insert(room, index);
            }
            Err(error) => {
                tracing::debug!(%error, "Discord didn't list the commands");
                slash.failed.insert(room);
            }
        }
        slash.made_from = None;
    }

    /// Remembers the command chosen from the list, which runs when its name
    /// is shared.
    pub(super) fn pick_command(&mut self, id: String) -> Vec<Effect> {
        if let Some(room) = self.state.selected.clone() {
            self.state.slash.picked.insert(room, id);
            self.state.slash.made_from = None;
        }
        vec![]
    }

    /// Closes the list until the draft changes.
    pub(super) fn dismiss_commands(&mut self) -> Vec<Effect> {
        self.state.slash.dismissed = Some(self.state.draft().to_owned());
        self.state.slash.shown = None;
        vec![]
    }

    /// Makes what the composer shows above it for the selected room's
    /// draft, asking for the room's commands the first time `/` starts one.
    pub(super) fn update_slash(&mut self) -> Vec<Effect> {
        let state = &self.state;
        let room = state.selected.clone().unwrap_or_default();
        let draft = state.draft();
        let commanding = !room.is_empty()
            && !state.preview
            && Network::of(&room) == Network::Discord
            && draft.starts_with('/')
            && !state.editing()
            && state.can_write_in(&room)
            && state.slash.dismissed.as_deref() != Some(draft);
        if !commanding {
            let closed = state.slash.dismissed.as_deref() == Some(draft);
            let slash = &mut self.state.slash;
            if !closed {
                slash.dismissed = None;
            }
            slash.shown = None;
            slash.made_from = None;
            return vec![];
        }
        let index = state.slash.indexes.get(&room).cloned();
        let key = (
            room.clone(),
            draft.to_owned(),
            index
                .as_ref()
                .map_or(0, |index| Arc::as_ptr(index) as usize),
        );
        if state.slash.made_from.as_ref() == Some(&key) {
            return vec![];
        }
        let mut effects = Vec::new();
        let slash = &state.slash;
        if index.is_none() && !slash.loading.contains(&room) && !slash.failed.contains(&room) {
            effects.push(Effect::Backend(Command::Discord(
                DiscordCommand::Commands { room: room.clone() },
            )));
        }
        let in_server =
            discord::parse_room(&room).is_some_and(|(guild, _)| guild != discord::DM_GUILD);
        let picked = slash.picked.get(&room).map(String::as_str);
        let mut shown = shown(draft, index.as_deref(), in_server, picked);
        let asking = if let (Some(Shown::Options(_)), Some(index)) = (&shown, &index) {
            let guild = discord::parse_room(&room)
                .map(|(guild, _)| guild)
                .filter(|guild| *guild != discord::DM_GUILD);
            app_commands::find(&index.commands, draft, picked).and_then(|command| {
                let asking = app_commands::read(command, draft).suggestion_request(guild)?;
                Some((command.id.clone(), command.application_id.clone(), asking))
            })
        } else {
            None
        };
        let draft = draft.to_owned();
        if let Some((command, application, asking)) = asking
            && let Some(Shown::Options(bar)) = &mut shown
        {
            effects.extend(self.suggest(&room, &draft, command, application, asking, bar));
        }
        let slash = &mut self.state.slash;
        if !effects.iter().all(|effect| {
            matches!(
                effect,
                Effect::Backend(Command::Discord(DiscordCommand::Suggest { .. }))
            )
        }) {
            slash.loading.insert(room.clone());
        }
        slash.shown = match shown {
            None if index.is_none() && slash.loading.contains(&room) => Some(Shown::Loading),
            shown => shown,
        };
        slash.made_from = Some(key);
        effects
    }

    /// Shows the app's suggestions for the option being typed in `bar`,
    /// asking for them the first time this text is typed.
    fn suggest(
        &mut self,
        room: &str,
        draft: &str,
        command: String,
        application: String,
        asking: app_commands::Suggesting,
        bar: &mut OptionsBar,
    ) -> Vec<Effect> {
        let slash = &mut self.state.slash;
        let key = (
            room.to_owned(),
            command,
            asking.option.clone(),
            asking.context.clone(),
        );
        let suggestions = match &mut slash.suggestions {
            Some(suggestions) if suggestions.key == key => suggestions,
            other => other.insert(Suggestions {
                key,
                answers: HashMap::new(),
                asked: HashMap::new(),
            }),
        };
        bar.choices_heading = choices_heading(&asking.query);
        // A value chosen, or typed and ended with a space, is done with, as
        // Discord's composer moves on from an option once one is chosen.
        if !asking.query.is_empty() && draft.ends_with(char::is_whitespace) {
            return vec![];
        }
        let mut effects = Vec::new();
        match suggestions.answers.get(&asking.query) {
            None => {
                if suggestions.answers.len() >= KEPT_ANSWERS {
                    suggestions.answers.clear();
                }
                let nonce = obsidian_discord::message::nonce();
                suggestions
                    .answers
                    .insert(asking.query.clone(), Answer::Waiting);
                suggestions
                    .asked
                    .insert(nonce.clone(), asking.query.clone());
                bar.note = Some("Loading options...".into());
                effects.push(Effect::Backend(Command::Discord(DiscordCommand::Suggest {
                    room: room.to_owned(),
                    application,
                    data: asking.data,
                    nonce,
                })));
            }
            Some(Answer::Waiting) => bar.note = Some("Loading options...".into()),
            Some(Answer::Failed) => bar.note = Some("Loading options failed".into()),
            Some(Answer::Choices(choices)) => {
                let names = choices.iter().map(|choice| choice.name.as_str());
                bar.choices = inserts(draft, &asking.option, &asking.query, names, false);
                slash
                    .last_suggested
                    .insert((room.to_owned(), asking.option), choices.clone());
            }
        }
        effects
    }

    /// Sends what the composer holds as the command it names, if it names
    /// one. `wired` is the draft with mentions as Discord writes them.
    /// `None` sends it as a message.
    pub(super) fn send_command(&mut self, room: &str, wired: &str) -> Option<Vec<Effect>> {
        if Network::of(room) != Network::Discord || !wired.starts_with('/') {
            return None;
        }
        let slash = &self.state.slash;
        let Some(index) = slash.indexes.get(room).cloned() else {
            // A command's name typed before the list came waits for it.
            if slash.loading.contains(room) {
                self.state.notice =
                    Some("Discord is still listing this channel's commands.".into());
                return Some(vec![]);
            }
            return None;
        };
        let picked = slash.picked.get(room).map(String::as_str);
        let command = app_commands::find(&index.commands, wired, picked)?;
        let invocation = app_commands::read(command, wired);
        let guild = discord::parse_room(room)
            .map(|(guild, _)| guild)
            .filter(|guild| *guild != discord::DM_GUILD);
        let suggested = |option: &str| -> Option<&[Choice]> {
            slash
                .last_suggested
                .get(&(room.to_owned(), option.to_owned()))
                .map(Vec::as_slice)
        };
        let data = match invocation.data_with(guild, &suggested) {
            Ok(data) => data,
            Err(error) => {
                self.state.notice = Some(error);
                return Some(vec![]);
            }
        };
        let mut files = Vec::new();
        for option in invocation.file_options() {
            match slash.files.get(&(room.to_owned(), option.to_owned())) {
                Some(path) => files.push(path.clone()),
                None => {
                    self.state.notice = Some(format!("Choose a file for {option}."));
                    return Some(vec![]);
                }
            }
        }
        let application = command.application_id.clone();
        let (command_id, command_guild) = (command.id.clone(), command.guild_id.clone());
        self.command_used(room, &command_id, command_guild.as_deref());
        self.state.notice = None;
        self.state.drafts.remove(room);
        self.state.slash.picked.remove(room);
        self.state
            .slash
            .last_suggested
            .retain(|(known, _), _| known != room);
        self.state.slash.files.retain(|(known, _), _| known != room);
        self.state.slash.suggestions = None;
        self.state.slash.made_from = None;
        Some(vec![
            Effect::Backend(Command::SaveDraft {
                room: room.to_owned(),
                body: String::new(),
            }),
            Effect::Backend(Command::Discord(DiscordCommand::RunCommand {
                room: room.to_owned(),
                application,
                data,
                files,
            })),
        ])
    }
}

/// The Apps submenu of a message's or a person's menu, as Discord's has it:
/// the user or message commands of the apps in the room, which run on the
/// person or the message, as Discord's client runs them, with their app's
/// icon.
impl Application {
    /// The room whose Apps a person's menu lists: the open Discord channel
    /// or DM, for a Discord user.
    pub fn person_apps_room(&self, user: &str) -> Option<&str> {
        let room = self.state.selected.as_deref()?;
        let discord_user = !user.is_empty() && user.bytes().all(|byte| byte.is_ascii_digit());
        (discord_user && !self.state.preview && Network::of(room) == Network::Discord)
            .then_some(room)
    }

    /// The room and the message id whose Apps a message's menu lists: a
    /// sent Discord message's.
    pub fn message_apps_target(&self, item: &str) -> Option<(String, String)> {
        let room = self.state.room_of(item)?;
        let id = discord::item_order(item)?;
        (!self.state.preview && Network::of(room) == Network::Discord)
            .then(|| (room.clone(), id.to_string()))
    }

    /// What the Apps menu of `kind` (user or message commands) lists in
    /// `room`.
    pub fn apps_menu(&self, room: &str, kind: u64) -> AppsMenu {
        let slash = &self.state.slash;
        let Some(index) = slash.indexes.get(room) else {
            return if slash.failed.contains(room) {
                AppsMenu::Commands(vec![])
            } else {
                AppsMenu::Loading
            };
        };
        AppsMenu::Commands(
            index
                .menu_commands(kind)
                .into_iter()
                .map(|(command, app)| MenuCommand {
                    id: command.id.clone(),
                    name: command.name.clone(),
                    app: app.map(|app| app.name.clone()).unwrap_or_default(),
                    icon: app.and_then(|app| app.icon.clone()),
                })
                .collect(),
        )
    }

    /// The Apps menu of `kind` in `room` laid out as Discord's is, for a
    /// search; none while its commands load.
    pub fn apps_layout(&self, room: &str, kind: u64, query: &str) -> Option<AppsLayout> {
        let AppsMenu::Commands(commands) = self.apps_menu(room, kind) else {
            return None;
        };
        let query = query.trim().to_lowercase();
        if !query.is_empty() {
            let mut found: Vec<MenuCommand> = commands
                .into_iter()
                .filter(|command| {
                    command.name.to_lowercase().contains(&query)
                        || command.app.to_lowercase().contains(&query)
                })
                .collect();
            // Names starting with what's typed first, as a search ranks them.
            found.sort_by_key(|command| !command.name.to_lowercase().starts_with(&query));
            found.truncate(MENU_COMMANDS_SHOWN);
            return Some(AppsLayout {
                found: Some(found),
                ..AppsLayout::default()
            });
        }
        let guild = discord::parse_room(room)
            .map(|(guild, _)| guild)
            .filter(|guild| *guild != discord::DM_GUILD);
        let index = self.state.slash.indexes.get(room);
        let now = crate::clock::now_ms();
        let mut frequent: Vec<(u32, MenuCommand)> = commands
            .iter()
            .filter_map(|command| {
                let known = index?.menu.iter().find(|known| known.id == command.id)?;
                let key = command_key(&known.id, known.guild_id.as_deref(), guild);
                let uses = self.state.slash.used.get(&key)?;
                let score = super::place_search::frecency(uses, now);
                (score > 0).then(|| (score, command.clone()))
            })
            .collect();
        frequent.sort_by(|a, b| b.0.cmp(&a.0));
        frequent.truncate(MENU_COMMANDS_SHOWN);
        let mut apps: Vec<(String, Option<String>, Vec<MenuCommand>)> = Vec::new();
        for command in commands {
            match apps.last_mut() {
                Some((app, _, listed)) if *app == command.app => listed.push(command),
                _ => apps.push((command.app.clone(), command.icon.clone(), vec![command])),
            }
        }
        Some(AppsLayout {
            frequent: frequent.into_iter().map(|(_, command)| command).collect(),
            apps,
            found: None,
        })
    }

    /// The apps' commands used, as Discord's clients counted them, with this
    /// session's added again.
    pub(super) fn commands_used(&mut self, used: Vec<(String, chat::PlaceUses)>) {
        let slash = &mut self.state.slash;
        slash.used = used.into_iter().collect();
        for (key, at) in &slash.used_here {
            super::place_search::track(&mut slash.used, key, *at);
        }
    }

    /// Counts a command run in `room`, for Frequently Used.
    fn command_used(&mut self, room: &str, id: &str, guild_id: Option<&str>) {
        let guild = discord::parse_room(room)
            .map(|(guild, _)| guild)
            .filter(|guild| *guild != discord::DM_GUILD);
        let key = command_key(id, guild_id, guild);
        let now = crate::clock::now_ms();
        let slash = &mut self.state.slash;
        slash.used_here.push((key.clone(), now));
        super::place_search::track(&mut slash.used, &key, now);
    }

    /// A file picked for the option being typed in the selected room's
    /// command: it goes with the command, and the composer names it, as
    /// Discord's shows the file in the option. Returns the composer's text.
    pub fn attach_command_file(&mut self, path: PathBuf) -> Option<String> {
        let room = self.state.selected.clone()?;
        let draft = self.state.drafts.get(&room).cloned().unwrap_or_default();
        let index = self.state.slash.indexes.get(&room).cloned()?;
        let picked = self.state.slash.picked.get(&room).map(String::as_str);
        let command = app_commands::find(&index.commands, &draft, picked)?;
        let invocation = app_commands::read(command, &draft);
        let option = invocation
            .current()
            .filter(|option| option.kind == app_commands::kind::ATTACHMENT)?;
        let name = path
            .file_name()?
            .to_string_lossy()
            .replace(char::is_whitespace, "_");
        let typed = invocation
            .given
            .last()
            .filter(|(known, _)| known.name == option.name)
            .map_or("", |(_, value)| value.as_str());
        let (_, text) = inserts(
            &draft,
            &option.name,
            typed,
            std::iter::once(name.as_str()),
            false,
        )
        .into_iter()
        .next()?;
        self.state
            .slash
            .files
            .insert((room.clone(), option.name.clone()), path);
        Some(text)
    }

    /// Asks for a room's commands, as an Apps menu opening does, unless
    /// they came or are on their way.
    pub(super) fn load_commands(&mut self, room: String) -> Vec<Effect> {
        // Frequently Used ranks by the counts the places' come with.
        let mut effects = self.ask_places_used();
        effects.extend(self.load_room_commands(room));
        effects
    }

    fn load_room_commands(&mut self, room: String) -> Vec<Effect> {
        let slash = &mut self.state.slash;
        if Network::of(&room) != Network::Discord
            || slash.indexes.contains_key(&room)
            || slash.failed.contains(&room)
            || !slash.loading.insert(room.clone())
        {
            return vec![];
        }
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::Commands { room },
        ))]
    }

    /// Runs a user or message command from an Apps menu on its target.
    pub(super) fn run_menu_command(
        &mut self,
        room: &str,
        command: &str,
        target: &str,
    ) -> Vec<Effect> {
        let Some(command) = self
            .state
            .slash
            .indexes
            .get(room)
            .and_then(|index| index.menu.iter().find(|known| known.id == command))
        else {
            return vec![];
        };
        let (application, data) = (command.application_id.clone(), command.target_data(target));
        let (id, guild_id) = (command.id.clone(), command.guild_id.clone());
        self.command_used(room, &id, guild_id.as_deref());
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::RunCommand {
                room: room.to_owned(),
                application,
                data,
                files: Vec::new(),
            },
        ))]
    }
}

/// Discord's client's key for a command's uses: its id, and the server's
/// with it for a command of one server's own run there.
fn command_key(id: &str, command_guild: Option<&str>, guild: Option<&str>) -> String {
    match (command_guild, guild) {
        (Some(_), Some(guild)) => format!("{id}:{guild}"),
        _ => id.to_owned(),
    }
}

/// A built-in command the draft starts with, and the text after its name;
/// `/tts` only in a server.
pub(super) fn builtin(draft: &str, in_server: bool) -> Option<(BuiltIn, &str)> {
    let rest = draft.strip_prefix('/')?;
    let name = rest.split(char::is_whitespace).next()?;
    let builtin = BuiltIn::named(name).filter(|builtin| in_server || !builtin.servers_only())?;
    let text = rest[name.len()..].trim_start();
    // Discord's composer takes `message:` as the option's name too.
    Some((
        builtin,
        text.strip_prefix("message:").unwrap_or(text).trim(),
    ))
}

/// What the composer shows above it for `draft`: the commands matching the
/// name being typed, or the options of the command written.
fn shown(
    draft: &str,
    index: Option<&CommandIndex>,
    in_server: bool,
    picked: Option<&str>,
) -> Option<Shown> {
    if let Some(query) = app_commands::query(draft) {
        let rows = rows(query, index, in_server);
        return (!rows.is_empty()).then_some(Shown::Commands(rows));
    }
    if let Some((builtin, _)) = builtin(draft, in_server) {
        return Some(Shown::Options(OptionsBar {
            command: format!("/{}", builtin.name()),
            app: BUILT_IN.into(),
            icon: None,
            options: vec![OptionChip {
                name: "message".into(),
                required: builtin.needs_text(),
                given: !draft[1 + builtin.name().len()..].trim().is_empty(),
                current: true,
            }],
            current: Some(("message".into(), "Your message".into())),
            choices: Vec::new(),
            choices_heading: String::new(),
            note: None,
            takes_file: false,
        }));
    }
    let index = index?;
    let command = app_commands::find(&index.commands, draft, picked)?;
    let invocation = app_commands::read(command, draft);
    let mut name = format!("/{}", command.name);
    for branch in &invocation.path {
        name.push(' ');
        name.push_str(&branch.name);
    }
    let current = invocation.current();
    let options = invocation
        .offered
        .iter()
        .map(|option| OptionChip {
            name: option.name.clone(),
            required: option.required,
            given: invocation
                .given
                .iter()
                .any(|(known, value)| known.name == option.name && !value.is_empty()),
            current: current.is_some_and(|current| current.name == option.name),
        })
        .collect();
    let typed = current
        .and_then(|option| {
            invocation
                .given
                .last()
                .filter(|(known, _)| known.name == option.name)
        })
        .map_or("", |(_, value)| value.as_str());
    // A True or False option offers both, as Discord's composer does.
    let choices = match current {
        Some(option) if !option.choices.is_empty() => {
            let names = option.choices.iter().map(|choice| choice.name.as_str());
            inserts(draft, &option.name, typed, names, true)
        }
        Some(option) if option.kind == app_commands::kind::BOOLEAN => inserts(
            draft,
            &option.name,
            typed,
            ["True", "False"].into_iter(),
            true,
        ),
        _ => Vec::new(),
    };
    let app = index.app(&command.application_id);
    Some(Shown::Options(OptionsBar {
        command: name,
        app: app.map(|app| app.name.clone()).unwrap_or_default(),
        icon: app.and_then(|app| app.icon.clone()),
        options,
        current: current.map(|option| {
            let takes = option.kind_name();
            let description = if takes.is_empty() {
                option.description.clone()
            } else {
                format!("{} · {takes}", option.description)
            };
            (option.name.clone(), description)
        }),
        choices_heading: choices_heading(typed),
        choices,
        note: None,
        takes_file: current.is_some_and(|option| option.kind == app_commands::kind::ATTACHMENT),
    }))
}

/// The heading over an option's choices, as Discord's list says it.
fn choices_heading(typed: &str) -> String {
    if typed.is_empty() {
        "OPTIONS".into()
    } else {
        format!("OPTIONS MATCHING {}", typed.to_uppercase())
    }
}

/// The choices named `names` of the option `option`, those that start with
/// `typed` when `filter`, each with the draft it makes: the typed value
/// replaced by the choice's name.
fn inserts<'a>(
    draft: &str,
    option: &str,
    typed: &str,
    names: impl Iterator<Item = &'a str>,
    filter: bool,
) -> Vec<(String, String)> {
    let lower = typed.to_lowercase();
    // The draft up to the value being typed, which ends it.
    let head = draft
        .trim_end()
        .strip_suffix(typed)
        .unwrap_or(draft)
        .trim_end();
    let head = if head.ends_with(&format!("{option}:")) || head.ends_with(':') {
        head.to_owned()
    } else if typed.is_empty() {
        format!("{} {option}:", draft.trim_end())
    } else {
        format!("{head} ")
    };
    names
        .filter(|name| !filter || (name.to_lowercase().starts_with(&lower) && *name != typed))
        .take(COMMAND_SUGGESTIONS)
        .map(|name| (name.to_owned(), format!("{head}{name} ")))
        .collect()
}

/// The commands whose names start with `query`: the apps', each app's
/// together, then Discord's own.
fn rows(query: &str, index: Option<&CommandIndex>, in_server: bool) -> Vec<CommandRow> {
    let query = query.to_lowercase();
    let mut rows: Vec<CommandRow> = Vec::new();
    if let Some(index) = index {
        let mut matching: Vec<_> = index
            .commands
            .iter()
            .filter(|command| command.name.to_lowercase().starts_with(&query))
            .collect();
        // Each app's commands together, in the order Discord lists the apps.
        matching.sort_by_key(|command| {
            index
                .apps
                .iter()
                .position(|app| app.id == command.application_id)
                .unwrap_or(usize::MAX)
        });
        for command in matching {
            let app = index.app(&command.application_id);
            let options = names(&command.options);
            rows.push(CommandRow {
                heading: rows
                    .last()
                    .is_none_or(|last| last.app != app.map_or("", |app| app.name.as_str())),
                id: command.id.clone(),
                insert: format!("/{} ", command.name),
                name: command.name.clone(),
                description: command.description.clone(),
                app: app.map(|app| app.name.clone()).unwrap_or_default(),
                icon: app.and_then(|app| app.icon.clone()),
                options,
            });
        }
    }
    for builtin in BuiltIn::ALL {
        if !builtin.name().starts_with(&query) || (builtin.servers_only() && !in_server) {
            continue;
        }
        rows.push(CommandRow {
            heading: rows.last().is_none_or(|last| last.app != BUILT_IN),
            id: builtin.name().into(),
            name: builtin.name().into(),
            description: builtin.description().into(),
            app: BUILT_IN.into(),
            icon: None,
            options: vec!["message".into()],
            insert: format!("/{} ", builtin.name()),
        });
    }
    rows.truncate(COMMAND_SUGGESTIONS);
    rows
}

/// The options a command's row names after it: subcommands, or required
/// options and then the rest.
fn names(options: &[CommandOption]) -> Vec<String> {
    let mut sorted: Vec<&CommandOption> = options.iter().collect();
    sorted.sort_by_key(|option| !option.required);
    sorted
        .into_iter()
        .map(|option| option.name.clone())
        .collect()
}
