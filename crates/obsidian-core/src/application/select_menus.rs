//! Apps' select menus, as Discord's client opens them under a message: a
//! list of the menu's options, or of the server's people, roles or channels,
//! with a search for those. A menu that picks one tells its app as soon as
//! it's picked; one that picks several, when it closes having changed. The
//! choice shows in the menu until the message next changes, as Discord's
//! client shows it until the app answers.

use super::*;
use chat::{CardRow, SelectKind, SelectMenu};

/// The most people, roles or channels a menu lists at once, as a search
/// narrows them.
const LISTED: usize = 50;

/// An app's select menu, open.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenSelect {
    pub room: String,
    pub item: String,
    pub menu: Arc<SelectMenu>,
    /// What it picks so far, by option value or id, in the order picked.
    pub picked: Vec<String>,
    /// What it picked as it opened: a menu of several tells its app only of
    /// a change.
    opened_with: Vec<String>,
    /// What's typed to narrow a menu of people, roles or channels.
    pub query: String,
}

/// One line of an open menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectChoice {
    /// Its option value or id.
    pub value: String,
    pub label: String,
    pub description: Option<String>,
    pub emoji: Option<String>,
    /// A person's picture, by its address.
    pub avatar: Option<String>,
    pub picked: bool,
}

impl OpenSelect {
    /// It picks several, with a box by each line.
    pub fn several(&self) -> bool {
        self.menu.max > 1
    }

    /// A menu of people, roles or channels, which a search narrows.
    pub fn searchable(&self) -> bool {
        self.menu.kind != SelectKind::Text
    }
}

impl Application {
    /// Opens the select menu named `custom_id` under `item`, picking what it
    /// shows as picked, in a room this account may write in, as Discord's
    /// client only lets members who can send use them.
    pub(super) fn open_select(&mut self, item: String, custom_id: String) -> Vec<Effect> {
        let Some(room) = self.state.room_of(&item).cloned() else {
            return vec![];
        };
        if !self.state.can_write_in(&room) {
            return vec![];
        }
        let Some(menu) = self.state.message(&item).and_then(|message| {
            message
                .cards
                .iter()
                .flat_map(|card| &card.rows)
                .find_map(|row| match row {
                    CardRow::Select {
                        disabled: false,
                        menu: Some(menu),
                        ..
                    } if menu.custom_id == custom_id => Some(menu.clone()),
                    _ => None,
                })
        }) else {
            return vec![];
        };
        let picked: Vec<String> = match menu.kind {
            SelectKind::Text => menu
                .options
                .iter()
                .filter(|option| option.default)
                .map(|option| option.value.clone())
                .collect(),
            _ => menu.defaults.clone(),
        };
        self.state.select = Some(OpenSelect {
            room,
            item,
            menu,
            opened_with: picked.clone(),
            picked,
            query: String::new(),
        });
        vec![]
    }

    /// Picks a line of the open menu: a menu of one tells its app and
    /// closes; one of several picks it, or no longer, up to its most.
    pub(super) fn pick_select(&mut self, value: String) -> Vec<Effect> {
        let Some(open) = &mut self.state.select else {
            return vec![];
        };
        if !open.several() {
            open.picked = vec![value];
            let open = self.state.select.take().expect("open");
            return self.choose(open);
        }
        if let Some(at) = open.picked.iter().position(|picked| *picked == value) {
            open.picked.remove(at);
        } else if open.picked.len() < usize::from(open.menu.max) {
            open.picked.push(value);
        }
        vec![]
    }

    /// Closes the open menu; a menu of several tells its app of a change
    /// that picks at least its fewest.
    pub(super) fn close_select(&mut self) -> Vec<Effect> {
        let Some(open) = self.state.select.take() else {
            return vec![];
        };
        if open.several()
            && open.picked != open.opened_with
            && open.picked.len() >= usize::from(open.menu.min)
        {
            return self.choose(open);
        }
        vec![]
    }

    pub(super) fn search_select(&mut self, query: String) -> Vec<Effect> {
        if let Some(open) = &mut self.state.select {
            open.query = query;
        }
        vec![]
    }

    /// Tells the menu's app what's picked, and shows it in the menu until
    /// the message next changes.
    fn choose(&mut self, open: OpenSelect) -> Vec<Effect> {
        let labels: Vec<String> = {
            let choices = self.select_choices_of(&open, false);
            open.picked
                .iter()
                .filter_map(|value| {
                    choices
                        .iter()
                        .find(|choice| choice.value == *value)
                        .map(|choice| choice.label.clone())
                })
                .collect()
        };
        self.show_chosen(&open, &labels);
        vec![Effect::Backend(Command::ChooseSelect {
            room: open.room,
            item: open.item,
            custom_id: open.menu.custom_id.clone(),
            values: open.picked,
        })]
    }

    /// The message's menu shows what's chosen, and picks it when it opens
    /// again, until its next version comes.
    fn show_chosen(&mut self, open: &OpenSelect, labels: &[String]) {
        let mut menu = (*open.menu).clone();
        match menu.kind {
            SelectKind::Text => {
                for option in &mut menu.options {
                    option.default = open.picked.contains(&option.value);
                }
            }
            _ => menu.defaults = open.picked.clone(),
        }
        let menu = Arc::new(menu);
        let text = if labels.is_empty() {
            None
        } else {
            Some(labels.join(", "))
        };
        let shown = |messages: &mut Arc<Vec<Arc<ChatMessage>>>| {
            let Some(at) = messages
                .iter()
                .position(|message| message.item == open.item)
            else {
                return;
            };
            let mut message = (*messages[at]).clone();
            for row in message.cards.iter_mut().flat_map(|card| &mut card.rows) {
                if let CardRow::Select {
                    text: shown,
                    menu: Some(current),
                    ..
                } = row
                    && current.custom_id == menu.custom_id
                {
                    if let Some(text) = &text {
                        shown.clone_from(text);
                    }
                    *current = menu.clone();
                }
            }
            Arc::make_mut(messages)[at] = Arc::new(message);
        };
        shown(&mut self.state.messages);
        if let Some(aside) = &mut self.state.aside {
            shown(&mut aside.messages);
        }
    }

    /// The open menu's lines: its options, or the people, roles or channels
    /// its search finds.
    pub fn select_choices(&self) -> Vec<SelectChoice> {
        match &self.state.select {
            Some(open) => self.select_choices_of(open, true),
            None => vec![],
        }
    }

    fn select_choices_of(&self, open: &OpenSelect, searched: bool) -> Vec<SelectChoice> {
        let query = if searched {
            open.query.trim().to_lowercase()
        } else {
            String::new()
        };
        let found = |name: &str| query.is_empty() || name.to_lowercase().contains(&query);
        let picked = |value: &str| open.picked.iter().any(|picked| picked == value);
        let line = |value: &str, label: &str| SelectChoice {
            value: value.to_owned(),
            label: label.to_owned(),
            description: None,
            emoji: None,
            avatar: None,
            picked: picked(value),
        };
        let people = || {
            self.state
                .members
                .as_ref()
                .and_then(|members| members.as_ref().ok())
                .into_iter()
                .flatten()
                .filter(|member| {
                    found(&member.name) || member.username.as_deref().is_some_and(found)
                })
                .map(|member| SelectChoice {
                    description: member.username.clone(),
                    avatar: member.avatar.clone(),
                    ..line(&member.id, &member.name)
                })
        };
        let roles = || {
            self.server_roles(&open.room)
                .into_iter()
                .filter(|role| found(&role.name))
                .map(|role| line(&role.id, &format!("@{}", role.name)))
        };
        let choices: Vec<SelectChoice> = match open.menu.kind {
            SelectKind::Text => open
                .menu
                .options
                .iter()
                .map(|option| SelectChoice {
                    description: option.description.clone(),
                    emoji: option.emoji.clone(),
                    ..line(&option.value, &option.label)
                })
                .collect(),
            SelectKind::User => people().collect(),
            SelectKind::Role => roles().collect(),
            SelectKind::Mentionable => people().chain(roles()).collect(),
            SelectKind::Channel => {
                let server = discord::parse_room(&open.room).map(|(guild, _)| guild);
                self.state
                    .rooms
                    .iter()
                    .filter(|room| {
                        discord::parse_room(&room.id).map(|(guild, _)| guild) == server
                            && server.is_some()
                            && found(&room.name)
                            && channel_fits(room, &open.menu.channel_types)
                    })
                    .filter_map(|room| {
                        let (_, channel) = discord::parse_room(&room.id)?;
                        Some(SelectChoice {
                            description: (!room.category.is_empty()).then(|| room.category.clone()),
                            ..line(channel, &room.name)
                        })
                    })
                    .collect()
            }
        };
        if open.menu.kind == SelectKind::Text {
            choices
        } else {
            choices.into_iter().take(LISTED).collect()
        }
    }
}

/// Whether a channel is of a kind a channel select offers: a text or
/// announcement channel (0, 5), a voice channel (2), a thread (11, 12) or a
/// forum (15).
fn channel_fits(room: &ChatRoom, types: &[u64]) -> bool {
    if types.is_empty() {
        return true;
    }
    let kinds: &[u64] = if room.voice {
        &[2, 13]
    } else if room.forum {
        &[15, 16]
    } else if room.parent.is_some() {
        &[10, 11, 12]
    } else {
        &[0, 5]
    };
    kinds.iter().any(|kind| types.contains(kind))
}
