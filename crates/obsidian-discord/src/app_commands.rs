//! Slash commands: the apps' commands a channel offers, Discord's built-in
//! ones, reading what the composer holds as a command, and the interaction
//! that runs it, as Discord's client does.
//!
//! A server's channels offer the commands of the apps in the server and of
//! those installed on the account; a DM offers its own (a bot's, in a DM
//! with one) and the account's. Discord's client asks for both indexes the
//! first time `/` is typed in a place, as Discord Obsidian does:
//! `GET /guilds/{id}/application-command-index` or
//! `/channels/{id}/application-command-index`, and
//! `/users/@me/application-command-index`.
//!
//! The composer holds a command as text, `/name sub option:value`, and
//! typing after a command's name fills its first required option, as
//! Discord's own composer does before it turns `name:` into a pill.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::ready::permission::ADMINISTRATOR;

/// Chat input commands, the ones typed after `/`.
pub const CHAT_INPUT: u64 = 1;
/// User commands, in the Apps submenu of a person's menu.
pub const USER: u64 = 2;
/// Message commands, in the Apps submenu of a message's menu.
pub const MESSAGE: u64 = 3;

/// What an option holds, by Discord's number.
pub mod kind {
    pub const SUB_COMMAND: u64 = 1;
    pub const SUB_COMMAND_GROUP: u64 = 2;
    pub const STRING: u64 = 3;
    pub const INTEGER: u64 = 4;
    pub const BOOLEAN: u64 = 5;
    pub const USER: u64 = 6;
    pub const CHANNEL: u64 = 7;
    pub const ROLE: u64 = 8;
    pub const MENTIONABLE: u64 = 9;
    pub const NUMBER: u64 = 10;
    pub const ATTACHMENT: u64 = 11;
}

/// The commands a place offers, with the apps they belong to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandIndex {
    pub apps: Vec<CommandApp>,
    /// Chat input commands, in the order Discord lists them.
    pub commands: Vec<AppCommand>,
    /// User and message commands, which the Apps menus offer.
    pub menu: Vec<AppCommand>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandApp {
    pub id: String,
    pub name: String,
    /// Its icon's address on Discord's CDN.
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppCommand {
    pub id: String,
    /// [`CHAT_INPUT`], [`USER`] or [`MESSAGE`].
    pub kind: u64,
    pub application_id: String,
    pub version: String,
    /// The server a server's own command belongs to.
    pub guild_id: Option<String>,
    /// As shown and typed, in the account's language.
    pub name: String,
    /// As its app named it, which the interaction carries.
    pub untranslated: String,
    pub description: String,
    pub options: Vec<CommandOption>,
    /// The permissions a member needs, as the app set them; `None` lets
    /// everyone use it.
    pub default_member_permissions: Option<u64>,
    /// The command as Discord sent it, which the interaction sends back as
    /// Discord's client does; shared, as every suggestion shown holds it.
    pub raw: Arc<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandOption {
    pub kind: u64,
    /// As shown and typed before `:`.
    pub name: String,
    pub untranslated: String,
    pub description: String,
    pub required: bool,
    /// The only values it takes, when it has them.
    pub choices: Vec<Choice>,
    /// A subcommand's or a group's own options.
    pub options: Vec<CommandOption>,
    /// The app suggests values as they're typed.
    pub autocomplete: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub name: String,
    pub value: Value,
}

impl CommandOption {
    /// A subcommand or a group of them, rather than a value.
    pub fn is_branch(&self) -> bool {
        matches!(self.kind, kind::SUB_COMMAND | kind::SUB_COMMAND_GROUP)
    }

    /// What its value is, as Discord's composer says it under the option.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            kind::STRING => "text",
            kind::INTEGER => "whole number",
            kind::NUMBER => "number",
            kind::BOOLEAN => "True or False",
            kind::USER => "user",
            kind::CHANNEL => "channel",
            kind::ROLE => "role",
            kind::MENTIONABLE => "user or role",
            kind::ATTACHMENT => "file",
            _ => "",
        }
    }
}

impl CommandIndex {
    /// Reads Discord's index of commands, keeping chat input, user and
    /// message commands.
    pub fn parse(body: &Value) -> Self {
        let apps = body["applications"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|app| {
                let id = app["id"].as_str()?.to_owned();
                Some(CommandApp {
                    icon: app["icon"]
                        .as_str()
                        .map(|hash| crate::ready::application_icon(&id, hash)),
                    name: app["name"].as_str().unwrap_or_default().to_owned(),
                    id,
                })
            })
            .collect();
        let (commands, menu) = body["application_commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(command)
            .filter(|command| matches!(command.kind, CHAT_INPUT | USER | MESSAGE))
            .partition(|command| command.kind == CHAT_INPUT);
        Self {
            apps,
            commands,
            menu,
        }
    }

    /// Adds another index's apps and commands, leaving out those it has.
    pub fn merge(&mut self, other: Self) {
        for app in other.apps {
            if !self.apps.iter().any(|known| known.id == app.id) {
                self.apps.push(app);
            }
        }
        for command in other.commands {
            if !self.commands.iter().any(|known| known.id == command.id) {
                self.commands.push(command);
            }
        }
        for command in other.menu {
            if !self.menu.iter().any(|known| known.id == command.id) {
                self.menu.push(command);
            }
        }
    }

    pub fn app(&self, id: &str) -> Option<&CommandApp> {
        self.apps.iter().find(|app| app.id == id)
    }

    /// The commands a member with `permissions` may use, in a server, or
    /// every one in a DM (`None`), as Discord's client filters them by the
    /// permissions their app asks for.
    pub fn usable(&self, permissions: Option<u64>) -> impl Iterator<Item = &AppCommand> {
        self.commands
            .iter()
            .filter(move |command| command.usable(permissions))
    }

    /// The index with only the commands, of every kind, a member with
    /// `permissions` may use; `None` when they may use them all.
    pub fn for_member(&self, permissions: u64) -> Option<Self> {
        let usable = |command: &&AppCommand| command.usable(Some(permissions));
        let all = self
            .commands
            .iter()
            .chain(&self.menu)
            .all(|command| usable(&command));
        (!all).then(|| Self {
            apps: self.apps.clone(),
            commands: self.commands.iter().filter(usable).cloned().collect(),
            menu: self.menu.iter().filter(usable).cloned().collect(),
        })
    }

    /// The user or message commands, by `kind`, with their apps, sorted by
    /// app and then name, as the Apps menu lists them.
    pub fn menu_commands(&self, kind: u64) -> Vec<(&AppCommand, Option<&CommandApp>)> {
        let mut commands: Vec<_> = self
            .menu
            .iter()
            .filter(|command| command.kind == kind)
            .map(|command| (command, self.app(&command.application_id)))
            .collect();
        commands.sort_by_cached_key(|(command, app)| {
            (
                app.map(|app| app.name.to_lowercase()),
                command.name.to_lowercase(),
            )
        });
        commands
    }
}

impl AppCommand {
    fn usable(&self, permissions: Option<u64>) -> bool {
        let (Some(have), Some(need)) = (permissions, self.default_member_permissions) else {
            return true;
        };
        have & ADMINISTRATOR != 0 || (need != 0 && have & need == need)
    }

    /// The interaction's `data` for a user or message command run on
    /// `target`, as Discord's client sends it from the Apps menu.
    pub fn target_data(&self, target: &str) -> Value {
        let mut data = json!({
            "version": self.version,
            "id": self.id,
            "name": self.untranslated,
            "type": self.kind,
            "options": [],
            "application_command": *self.raw,
            "target_id": target,
            "attachments": [],
        });
        if let Some(guild) = &self.guild_id {
            data["guild_id"] = json!(guild);
        }
        data
    }
}

fn command(value: &Value) -> Option<AppCommand> {
    let name = value["name"].as_str()?.to_owned();
    Some(AppCommand {
        id: value["id"].as_str()?.to_owned(),
        kind: value["type"].as_u64().unwrap_or(CHAT_INPUT),
        application_id: value["application_id"].as_str()?.to_owned(),
        version: value["version"].as_str().unwrap_or_default().to_owned(),
        guild_id: value["guild_id"].as_str().map(str::to_owned),
        untranslated: value["name_default"].as_str().unwrap_or(&name).to_owned(),
        description: value["description"].as_str().unwrap_or_default().to_owned(),
        options: options(&value["options"]),
        default_member_permissions: value["default_member_permissions"]
            .as_str()
            .and_then(|bits| bits.parse().ok()),
        raw: Arc::new(value.clone()),
        name,
    })
}

fn options(value: &Value) -> Vec<CommandOption> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let name = option["name"].as_str()?.to_owned();
            Some(CommandOption {
                kind: option["type"].as_u64()?,
                untranslated: option["name_default"].as_str().unwrap_or(&name).to_owned(),
                description: option["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                required: option["required"].as_bool().unwrap_or(false),
                choices: option["choices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|choice| {
                        Some(Choice {
                            name: choice["name"].as_str()?.to_owned(),
                            value: choice["value"].clone(),
                        })
                    })
                    .collect(),
                options: options(&option["options"]),
                autocomplete: option["autocomplete"].as_bool().unwrap_or(false),
                min_value: option["min_value"].as_f64(),
                max_value: option["max_value"].as_f64(),
                min_length: option["min_length"].as_u64(),
                max_length: option["max_length"].as_u64(),
                name,
            })
        })
        .collect()
}

/// Discord's own commands, which change the message rather than ask an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltIn {
    Shrug,
    TableFlip,
    Unflip,
    Tts,
    Me,
    Spoiler,
}

impl BuiltIn {
    /// In the order Discord's client lists them.
    pub const ALL: [BuiltIn; 6] = [
        BuiltIn::Shrug,
        BuiltIn::TableFlip,
        BuiltIn::Unflip,
        BuiltIn::Tts,
        BuiltIn::Me,
        BuiltIn::Spoiler,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BuiltIn::Shrug => "shrug",
            BuiltIn::TableFlip => "tableflip",
            BuiltIn::Unflip => "unflip",
            BuiltIn::Tts => "tts",
            BuiltIn::Me => "me",
            BuiltIn::Spoiler => "spoiler",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            BuiltIn::Shrug => "Appends ¯\\_(ツ)_/¯ to your message.",
            BuiltIn::TableFlip => "Appends (╯°□°)╯︵ ┻━┻ to your message.",
            BuiltIn::Unflip => "Appends ┬─┬ ノ( ゜-゜ノ) to your message.",
            BuiltIn::Tts => {
                "Use text-to-speech to read the message to all members currently viewing the channel."
            }
            BuiltIn::Me => "Displays text with emphasis.",
            BuiltIn::Spoiler => "Marks your message as a spoiler.",
        }
    }

    /// Its message must have text, as `/me`'s must; `/shrug`'s needn't.
    pub fn needs_text(self) -> bool {
        matches!(self, BuiltIn::Tts | BuiltIn::Me | BuiltIn::Spoiler)
    }

    /// Only in servers, as Discord's client offers `/tts`; whether the
    /// account may send TTS there is the caller's to say.
    pub fn servers_only(self) -> bool {
        self == BuiltIn::Tts
    }

    /// What it sends for `text`, as Discord's client writes it.
    pub fn content(self, text: &str) -> String {
        match self {
            BuiltIn::Shrug => format!("{text} ¯\\_(ツ)_/¯").trim().to_owned(),
            BuiltIn::TableFlip => format!("{text} (╯°□°)╯︵ ┻━┻").trim().to_owned(),
            BuiltIn::Unflip => format!("{text} ┬─┬ノ( º _ ºノ)").trim().to_owned(),
            BuiltIn::Tts => text.to_owned(),
            BuiltIn::Me => format!("_{text}_"),
            BuiltIn::Spoiler => format!("||{text}||").trim().to_owned(),
        }
    }

    /// The built-in command `name` names.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|builtin| builtin.name() == name)
    }
}

/// The command name being typed: the composer holds `/` and a word, and the
/// caret hasn't left it.
pub fn query(text: &str) -> Option<&str> {
    let name = text.strip_prefix('/')?;
    (!name.contains(char::is_whitespace)).then_some(name)
}

/// What the composer holds, read as a command of the index: the command,
/// the subcommands chosen and the options given.
#[derive(Debug, PartialEq)]
pub struct Invocation<'a> {
    pub command: &'a AppCommand,
    /// The group and subcommand chosen, outermost first.
    pub path: Vec<&'a CommandOption>,
    /// Each option given, with its text as typed.
    pub given: Vec<(&'a CommandOption, String)>,
    /// The options of the subcommand chosen, or of the command, which the
    /// composer offers next; also when no subcommand is chosen yet, those
    /// to choose from.
    pub offered: &'a [CommandOption],
}

/// Finds the command `text` starts with among `commands`, preferring the one
/// with id `picked` when several share its name, as the one chosen from the
/// list. `None` when `text` names none.
pub fn find<'a>(
    commands: impl IntoIterator<Item = &'a AppCommand>,
    text: &str,
    picked: Option<&str>,
) -> Option<&'a AppCommand> {
    let rest = text.strip_prefix('/')?;
    let name = rest.split(char::is_whitespace).next()?;
    let mut first = None;
    for command in commands {
        if command.name == name {
            if picked == Some(command.id.as_str()) {
                return Some(command);
            }
            first = first.or(Some(command));
        }
    }
    first
}

/// Reads `text` as `command` with its subcommands and options.
pub fn read<'a>(command: &'a AppCommand, text: &str) -> Invocation<'a> {
    let rest = text
        .strip_prefix('/')
        .and_then(|rest| rest.strip_prefix(command.name.as_str()))
        .unwrap_or_default();
    let mut path = Vec::new();
    let mut offered = command.options.as_slice();
    let mut rest = rest.trim_start();
    // Groups and subcommands come as words.
    while offered.iter().any(CommandOption::is_branch) {
        let word = rest.split(char::is_whitespace).next().unwrap_or_default();
        let Some(branch) = offered
            .iter()
            .find(|option| option.is_branch() && option.name == word)
        else {
            break;
        };
        // A word still being typed may be a longer subcommand's start.
        let after = &rest[word.len()..];
        if after.is_empty()
            && offered
                .iter()
                .any(|option| option.name.starts_with(word) && option.name != word)
        {
            break;
        }
        path.push(branch);
        offered = &branch.options;
        rest = after.trim_start();
    }
    let given = if offered.iter().any(CommandOption::is_branch) {
        Vec::new()
    } else {
        values(offered, rest)
    };
    Invocation {
        command,
        path,
        given,
        offered,
    }
}

/// The options typed in `text` as `name:value`, and before the first of them,
/// the value of the first required option, or the first option.
fn values<'a>(options: &'a [CommandOption], text: &str) -> Vec<(&'a CommandOption, String)> {
    // Where each `name:` starts, at a word's start.
    let mut marks: Vec<(usize, &CommandOption)> = Vec::new();
    for (at, _) in text.char_indices() {
        if at > 0 && !text[..at].ends_with(char::is_whitespace) {
            continue;
        }
        if let Some(option) = options.iter().find(|option| {
            text[at..]
                .strip_prefix(option.name.as_str())
                .is_some_and(|after| after.starts_with(':'))
        }) {
            marks.push((at, option));
        }
    }
    let mut given: Vec<(&CommandOption, String)> = Vec::new();
    let free = text[..marks.first().map_or(text.len(), |(at, _)| *at)].trim();
    for (index, (at, option)) in marks.iter().enumerate() {
        let start = at + option.name.len() + 1;
        let end = marks.get(index + 1).map_or(text.len(), |(next, _)| *next);
        let value = text[start..end].trim().to_owned();
        given.retain(|(known, _)| known.name != option.name);
        given.push((option, value));
    }
    if !free.is_empty() {
        let first = options
            .iter()
            .find(|option| {
                option.required && !given.iter().any(|(known, _)| known.name == option.name)
            })
            .or_else(|| {
                options
                    .iter()
                    .find(|option| !given.iter().any(|(known, _)| known.name == option.name))
            });
        if let Some(option) = first {
            given.insert(0, (option, free.to_owned()));
        }
    }
    given
}

impl Invocation<'_> {
    /// The option being typed now: the last one given, or before any, the
    /// first required one.
    pub fn current(&self) -> Option<&CommandOption> {
        self.given.last().map(|(option, _)| *option).or_else(|| {
            self.offered
                .iter()
                .find(|option| option.required && !option.is_branch())
        })
    }

    /// Required options not given yet.
    pub fn missing(&self) -> impl Iterator<Item = &CommandOption> {
        self.offered.iter().filter(|option| {
            option.required
                && !option.is_branch()
                && !self
                    .given
                    .iter()
                    .any(|(known, value)| known.name == option.name && !value.is_empty())
        })
    }

    /// The interaction's `options`, or why the command can't run as typed,
    /// in the words of Discord's composer. `guild` is the server, which
    /// `@everyone` means for a mentionable option.
    pub fn wire_options(&self, guild: Option<&str>) -> Result<Value, String> {
        self.wire_options_with(guild, &|_| None)
    }

    /// As [`Self::wire_options`], where `suggested` gives the choices an
    /// app last suggested for an option, by its name: text that names one
    /// sends its value, as Discord's client sends it.
    pub fn wire_options_with<'s>(
        &self,
        guild: Option<&str>,
        suggested: &dyn Fn(&str) -> Option<&'s [Choice]>,
    ) -> Result<Value, String> {
        if self.offered.iter().any(CommandOption::is_branch) {
            let names: Vec<&str> = self
                .offered
                .iter()
                .filter(|option| option.is_branch())
                .map(|option| option.name.as_str())
                .collect();
            return Err(format!("Choose a subcommand: {}.", names.join(", ")));
        }
        if let Some(option) = self.missing().next() {
            return Err(format!("Fill in the required option {}.", option.name));
        }
        let mut options = Vec::new();
        for (option, text) in &self.given {
            if text.is_empty() {
                continue;
            }
            options.push(json!({
                "type": option.kind,
                "name": option.untranslated,
                "value": value(option, text, guild, suggested(&option.name))?,
            }));
        }
        Ok(self.nest(options))
    }

    /// `options` inside the subcommand and group chosen.
    fn nest(&self, mut options: Vec<Value>) -> Value {
        for branch in self.path.iter().rev() {
            options = vec![json!({
                "type": branch.kind,
                "name": branch.untranslated,
                "options": options,
            })];
        }
        Value::Array(options)
    }

    /// The interaction's `data`, as Discord's client sends it.
    pub fn data(&self, guild: Option<&str>) -> Result<Value, String> {
        self.data_with(guild, &|_| None)
    }

    /// As [`Self::data`], with the choices apps suggested; see
    /// [`Self::wire_options_with`].
    pub fn data_with<'s>(
        &self,
        guild: Option<&str>,
        suggested: &dyn Fn(&str) -> Option<&'s [Choice]>,
    ) -> Result<Value, String> {
        let options = self.wire_options_with(guild, suggested)?;
        let mut data = json!({
            "version": self.command.version,
            "id": self.command.id,
            "name": self.command.untranslated,
            "type": CHAT_INPUT,
            "options": options,
            "application_command": *self.command.raw,
            "attachments": [],
        });
        if let Some(guild) = &self.command.guild_id {
            data["guild_id"] = json!(guild);
        }
        Ok(data)
    }

    /// What to ask the command's app for suggestions with, when the option
    /// being typed takes them: as Discord's client asks, its text so far as
    /// the focused option's value, with the other options that hold a
    /// valid value. `None` when the option being typed takes none.
    pub fn suggestion_request(&self, guild: Option<&str>) -> Option<Suggesting> {
        let focused = self.current().filter(|option| option.autocomplete)?;
        let query = self
            .given
            .iter()
            .rev()
            .find(|(option, _)| option.name == focused.name)
            .map_or("", |(_, text)| text.as_str())
            .to_owned();
        let mut options = Vec::new();
        let mut context = String::new();
        for (option, text) in &self.given {
            if option.name == focused.name || text.is_empty() {
                continue;
            }
            if let Ok(value) = value(option, text, guild, None) {
                context.push_str(&format!("{}={value}\0", option.name));
                options.push(json!({
                    "type": option.kind,
                    "name": option.untranslated,
                    "value": value,
                }));
            }
        }
        // Where it's typed among the options, as Discord's composer keeps
        // them in the order they're given.
        let at = self
            .given
            .iter()
            .filter(|(option, text)| option.name != focused.name && !text.is_empty())
            .take_while(|(option, _)| {
                self.given
                    .iter()
                    .position(|(known, _)| known.name == option.name)
                    < self
                        .given
                        .iter()
                        .position(|(known, _)| known.name == focused.name)
            })
            .count();
        options.insert(
            at.min(options.len()),
            json!({
                "type": focused.kind,
                "name": focused.untranslated,
                "value": query,
                "focused": true,
            }),
        );
        let mut data = json!({
            "version": self.command.version,
            "id": self.command.id,
            "name": self.command.untranslated,
            "type": CHAT_INPUT,
            "options": self.nest(options),
            "application_command": *self.command.raw,
        });
        if let Some(guild) = &self.command.guild_id {
            data["guild_id"] = json!(guild);
        }
        Some(Suggesting {
            option: focused.name.clone(),
            query,
            context,
            data,
        })
    }
}

/// An ask for an app's suggestions: for which option, with what typed, the
/// other options' values it depends on, and the interaction's `data`.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggesting {
    pub option: String,
    pub query: String,
    /// The other options' values, as `name=value\0` each: suggestions for
    /// other values are other suggestions, as Discord's client keys them.
    pub context: String,
    pub data: Value,
}

/// The choices of an APPLICATION_COMMAND_AUTOCOMPLETE_RESPONSE, by their
/// name in the account's language.
pub fn suggested(choices: &Value) -> Vec<Choice> {
    choices
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|choice| {
            let name = choice["name_localized"]
                .as_str()
                .or_else(|| choice["name"].as_str())?;
            Some(Choice {
                name: name.to_owned(),
                value: choice["value"].clone(),
            })
        })
        .collect()
}

/// An option's value from its text, checked as Discord's composer checks it.
fn value(
    option: &CommandOption,
    text: &str,
    guild: Option<&str>,
    suggested: Option<&[Choice]>,
) -> Result<Value, String> {
    let invalid = || format!("{} isn't a valid {}.", option.name, option.kind_name());
    // A suggestion the app made, by its name, sends its value.
    if option.autocomplete
        && let Some(choice) = suggested
            .into_iter()
            .flatten()
            .find(|choice| choice.name == text)
    {
        let number = |value: &Value| value.as_f64().or_else(|| value.as_str()?.parse().ok());
        match option.kind {
            kind::INTEGER => {
                if let Some(number) = number(&choice.value) {
                    return Ok(json!(number as i64));
                }
            }
            kind::NUMBER => {
                if let Some(number) = number(&choice.value) {
                    return Ok(json!(number));
                }
            }
            _ => {
                if let Some(value) = choice.value.as_str() {
                    return Ok(json!(value));
                }
            }
        }
    }
    if !option.choices.is_empty() {
        return option
            .choices
            .iter()
            .find(|choice| {
                choice.name.eq_ignore_ascii_case(text)
                    || choice.value.as_str() == Some(text)
                    || choice
                        .value
                        .as_f64()
                        .is_some_and(|number| text.parse() == Ok(number))
            })
            .map(|choice| choice.value.clone())
            .ok_or_else(|| {
                let names: Vec<&str> = option
                    .choices
                    .iter()
                    .map(|choice| choice.name.as_str())
                    .collect();
                format!("Choose {} from: {}.", option.name, names.join(", "))
            });
    }
    let in_range = |number: f64| {
        option.min_value.is_none_or(|min| number >= min)
            && option.max_value.is_none_or(|max| number <= max)
    };
    let range = || match (option.min_value, option.max_value) {
        (Some(min), Some(max)) => format!("{} must be between {min} and {max}.", option.name),
        (Some(min), None) => format!("{} must be at least {min}.", option.name),
        (None, Some(max)) => format!("{} must be at most {max}.", option.name),
        (None, None) => invalid(),
    };
    match option.kind {
        kind::STRING => {
            let length = text.chars().count() as u64;
            if option.min_length.is_some_and(|min| length < min)
                || option.max_length.is_some_and(|max| length > max)
            {
                return Err(format!(
                    "{} must be {}–{} characters long.",
                    option.name,
                    option.min_length.unwrap_or(0),
                    option.max_length.unwrap_or(6000)
                ));
            }
            Ok(json!(text))
        }
        kind::INTEGER => {
            let number: i64 = text.parse().map_err(|_| invalid())?;
            if !in_range(number as f64) {
                return Err(range());
            }
            Ok(json!(number))
        }
        kind::NUMBER => {
            let number: f64 = text.parse().map_err(|_| invalid())?;
            if !number.is_finite() || !in_range(number) {
                return Err(range());
            }
            Ok(json!(number))
        }
        kind::BOOLEAN => match text.to_lowercase().as_str() {
            "true" => Ok(json!(true)),
            "false" => Ok(json!(false)),
            _ => Err(invalid()),
        },
        kind::USER => mentioned(text, &["<@!", "<@"])
            .map(|id| json!(id))
            .ok_or_else(invalid),
        kind::CHANNEL => mentioned(text, &["<#"])
            .map(|id| json!(id))
            .ok_or_else(invalid),
        kind::ROLE => mentioned(text, &["<@&"])
            .map(|id| json!(id))
            .ok_or_else(invalid),
        kind::MENTIONABLE => {
            if text == "@everyone"
                && let Some(guild) = guild
            {
                return Ok(json!(guild));
            }
            mentioned(text, &["<@&", "<@!", "<@"])
                .map(|id| json!(id))
                .ok_or_else(invalid)
        }
        kind::ATTACHMENT => Err(format!(
            "{} takes a file, which Discord Obsidian can't attach to commands yet.",
            option.name
        )),
        _ => Err(invalid()),
    }
}

/// The id in a mention written `<prefix…id>`, or a bare id.
fn mentioned<'a>(text: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let id = prefixes
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix)?.strip_suffix('>'))
        .unwrap_or(text);
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> CommandIndex {
        CommandIndex::parse(&json!({
            "applications": [
                {"id": "100", "name": "Helper", "icon": "abc", "flags": "0"},
                {"id": "200", "name": "Music"},
            ],
            "application_commands": [
                {"id": "1", "type": 1, "application_id": "100", "version": "11",
                 "name": "ping", "description": "Pong!", "default_member_permissions": null},
                {"id": "2", "type": 1, "application_id": "100", "version": "12",
                 "name": "remind", "description": "Remind me",
                 "options": [
                    {"type": 3, "name": "what", "description": "Of what", "required": true},
                    {"type": 4, "name": "minutes", "description": "In", "min_value": 1, "max_value": 60},
                    {"type": 5, "name": "dm", "description": "By DM"},
                 ]},
                {"id": "3", "type": 1, "application_id": "200", "version": "13",
                 "name": "music", "description": "Music",
                 "options": [
                    {"type": 1, "name": "play", "description": "Plays", "options": [
                        {"type": 3, "name": "song", "description": "Song", "required": true},
                    ]},
                    {"type": 2, "name": "queue", "description": "Queue", "options": [
                        {"type": 1, "name": "clear", "description": "Clears"},
                    ]},
                 ]},
                {"id": "4", "type": 1, "application_id": "100", "version": "14",
                 "name": "ban", "description": "Bans", "default_member_permissions": "4",
                 "options": [
                    {"type": 6, "name": "user", "description": "Who", "required": true},
                    {"type": 3, "name": "reason", "description": "Why",
                     "choices": [{"name": "Spam", "value": "spam"}, {"name": "Rude", "value": "rude"}]},
                 ]},
                {"id": "5", "type": 2, "application_id": "100", "version": "15", "name": "Profile"},
            ],
            "version": "99",
        }))
    }

    #[test]
    fn reads_the_index_keeping_chat_input_commands() {
        let index = index();
        assert_eq!(index.apps.len(), 2);
        assert_eq!(
            index.apps[0].icon.as_deref(),
            Some(crate::ready::application_icon("100", "abc").as_str())
        );
        let names: Vec<&str> = index
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect();
        assert_eq!(names, ["ping", "remind", "music", "ban"]);
        assert_eq!(index.commands[3].default_member_permissions, Some(4));
        let menu: Vec<_> = index
            .menu_commands(USER)
            .into_iter()
            .map(|(command, app)| (command.name.as_str(), app.map(|app| app.name.as_str())))
            .collect();
        assert_eq!(menu, [("Profile", Some("Helper"))]);
        assert!(index.menu_commands(MESSAGE).is_empty());
    }

    #[test]
    fn runs_a_menu_command_on_its_target() {
        let index = index();
        let data = index.menu[0].target_data("42");
        assert_eq!(data["type"], 2);
        assert_eq!(data["id"], "5");
        assert_eq!(data["version"], "15");
        assert_eq!(data["name"], "Profile");
        assert_eq!(data["target_id"], "42");
        assert_eq!(data["options"], json!([]));
        assert_eq!(data["application_command"]["name"], "Profile");
        assert!(data.get("guild_id").is_none());
        // A member who may not ban doesn't get the command that bans.
        assert!(
            index
                .for_member(0)
                .is_some_and(|only| !only.commands.iter().any(|command| command.name == "ban"))
        );
        assert!(index.for_member(ADMINISTRATOR).is_none());
    }

    #[test]
    fn hides_commands_whose_permissions_the_member_lacks() {
        let index = index();
        let usable = |permissions| {
            index
                .usable(permissions)
                .map(|command| command.name.clone())
                .collect::<Vec<_>>()
        };
        assert!(!usable(Some(0)).contains(&"ban".to_owned()));
        assert!(usable(Some(4)).contains(&"ban".to_owned()));
        assert!(usable(Some(ADMINISTRATOR)).contains(&"ban".to_owned()));
        assert!(
            usable(None).contains(&"ban".to_owned()),
            "DMs list every one"
        );
    }

    #[test]
    fn the_first_text_fills_the_first_required_option() {
        let index = index();
        let remind = find(&index.commands, "/remind buy milk minutes:5", None).unwrap();
        let invocation = read(remind, "/remind buy milk minutes:5");
        let given: Vec<(&str, &str)> = invocation
            .given
            .iter()
            .map(|(option, text)| (option.name.as_str(), text.as_str()))
            .collect();
        assert_eq!(given, [("what", "buy milk"), ("minutes", "5")]);
        assert_eq!(
            invocation.wire_options(None).unwrap(),
            json!([
                {"type": 3, "name": "what", "value": "buy milk"},
                {"type": 4, "name": "minutes", "value": 5},
            ])
        );
    }

    #[test]
    fn checks_values_as_discord_does() {
        let index = index();
        let remind = &index.commands[1];
        let error = |text| read(remind, text).wire_options(None).unwrap_err();
        assert_eq!(error("/remind "), "Fill in the required option what.");
        assert_eq!(
            error("/remind x minutes:90"),
            "minutes must be between 1 and 60."
        );
        assert_eq!(
            error("/remind x minutes:soon"),
            "minutes isn't a valid whole number."
        );
        assert_eq!(
            error("/remind x dm:maybe"),
            "dm isn't a valid True or False."
        );
        assert!(read(remind, "/remind x dm:True").wire_options(None).is_ok());
        let ban = &index.commands[3];
        assert_eq!(
            read(ban, "/ban <@42> reason:Spam")
                .wire_options(None)
                .unwrap(),
            json!([
                {"type": 6, "name": "user", "value": "42"},
                {"type": 3, "name": "reason", "value": "spam"},
            ])
        );
        assert_eq!(
            read(ban, "/ban <@42> reason:bored")
                .wire_options(None)
                .unwrap_err(),
            "Choose reason from: Spam, Rude."
        );
    }

    #[test]
    fn subcommands_nest_their_options() {
        let index = index();
        let music = &index.commands[2];
        let invocation = read(music, "/music play never gonna");
        assert_eq!(invocation.path.len(), 1);
        assert_eq!(
            invocation.data(None).unwrap()["options"],
            json!([{"type": 1, "name": "play", "options": [
                {"type": 3, "name": "song", "value": "never gonna"},
            ]}])
        );
        let invocation = read(music, "/music queue clear");
        assert_eq!(
            invocation.wire_options(None).unwrap(),
            json!([{"type": 2, "name": "queue", "options": [
                {"type": 1, "name": "clear", "options": []},
            ]}])
        );
        assert_eq!(
            read(music, "/music ").wire_options(None).unwrap_err(),
            "Choose a subcommand: play, queue."
        );
    }

    #[test]
    fn the_interaction_sends_the_command_back_as_discord_gave_it() {
        let index = index();
        let ping = &index.commands[0];
        let data = read(ping, "/ping").data(None).unwrap();
        assert_eq!(data["id"], "1");
        assert_eq!(data["version"], "11");
        assert_eq!(data["name"], "ping");
        assert_eq!(data["type"], 1);
        assert_eq!(data["application_command"], *ping.raw);
        assert_eq!(data["attachments"], json!([]));
        assert!(data.get("guild_id").is_none());
    }

    #[test]
    fn finds_the_name_being_typed_and_the_one_picked() {
        assert_eq!(query("/re"), Some("re"));
        assert_eq!(query("/remind x"), None);
        assert_eq!(query("hi /re"), None);
        let mut index = index();
        let mut other = index.commands[0].clone();
        other.id = "9".into();
        index.commands.push(other);
        assert_eq!(find(&index.commands, "/ping", None).unwrap().id, "1");
        assert_eq!(find(&index.commands, "/ping", Some("9")).unwrap().id, "9");
        assert_eq!(find(&index.commands, "/nothing", None), None);
    }

    #[test]
    fn asks_for_suggestions_with_whats_typed_and_sends_the_one_chosen() {
        let index = CommandIndex::parse(&json!({
            "applications": [{"id": "100", "name": "Weather"}],
            "application_commands": [{
                "id": "1", "type": 1, "application_id": "100", "version": "3",
                "name": "weather", "description": "Weather",
                "options": [
                    {"type": 3, "name": "city", "description": "City", "required": true, "autocomplete": true},
                    {"type": 4, "name": "days", "description": "Days", "autocomplete": true},
                ],
            }],
        }));
        let weather = &index.commands[0];
        let asking = read(weather, "/weather ").suggestion_request(None).unwrap();
        assert_eq!(asking.option, "city");
        assert_eq!(asking.query, "");
        let asking = read(weather, "/weather Par")
            .suggestion_request(None)
            .unwrap();
        assert_eq!(
            asking.data["options"],
            json!([{"type": 3, "name": "city", "value": "Par", "focused": true}])
        );
        assert!(asking.data.get("attachments").is_none());
        // The other options go along, and set what the suggestions are for.
        let asking = read(weather, "/weather Paris days:2")
            .suggestion_request(None)
            .unwrap();
        assert_eq!(asking.option, "days");
        assert_eq!(asking.context, "city=\"Paris\"\0");
        assert_eq!(
            asking.data["options"],
            json!([
                {"type": 3, "name": "city", "value": "Paris"},
                {"type": 4, "name": "days", "value": "2", "focused": true},
            ])
        );
        // A suggestion chosen sends its value; other text goes as typed.
        let choices = suggested(&json!([
            {"name": "Paris", "name_localized": "Paris, France", "value": "fr-paris"},
            {"name": "Two days", "value": "2"},
        ]));
        let suggested = |option: &str| -> Option<&[Choice]> {
            (option == "city" || option == "days").then_some(choices.as_slice())
        };
        let options = |text| {
            read(weather, text)
                .wire_options_with(None, &suggested)
                .unwrap()
        };
        assert_eq!(
            options("/weather Paris, France days:Two days"),
            json!([
                {"type": 3, "name": "city", "value": "fr-paris"},
                {"type": 4, "name": "days", "value": 2},
            ])
        );
        assert_eq!(
            options("/weather Lyon"),
            json!([{"type": 3, "name": "city", "value": "Lyon"}])
        );
        // A command without suggestions asks for none.
        assert_eq!(
            read(&index.commands[0], "/weather Paris days:2")
                .suggestion_request(None)
                .unwrap()
                .option,
            "days"
        );
    }

    #[test]
    fn built_ins_write_what_discords_do() {
        assert_eq!(BuiltIn::Shrug.content("hi"), "hi ¯\\_(ツ)_/¯");
        assert_eq!(BuiltIn::Shrug.content(""), "¯\\_(ツ)_/¯");
        assert_eq!(BuiltIn::Me.content("waves"), "_waves_");
        assert_eq!(BuiltIn::Spoiler.content("end"), "||end||");
        assert_eq!(BuiltIn::named("tableflip"), Some(BuiltIn::TableFlip));
    }
}
