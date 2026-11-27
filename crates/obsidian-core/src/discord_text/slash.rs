//! Slash commands: the ones a room offers, kept for each server or DM as
//! Discord's client keeps its indexes, and running one.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use obsidian_discord::app_commands::CommandIndex;

use super::{commands::NOT_DISCORD, *};

/// The commands each server or DM offers, by server id or DM channel id,
/// as Discord listed them, before the account's permissions in a channel
/// leave some out.
pub(super) type Indexes = Arc<Mutex<HashMap<String, Arc<CommandIndex>>>>;

/// When the last ask for suggestions went, and how many have been asked.
pub(super) type Pace = Arc<Mutex<(Option<Instant>, u64)>>;

/// How long a modal's submission waits for its app before it fails, as
/// Discord's client waits.
const SUBMIT_WAIT: Duration = Duration::from_secs(10);

/// The least time between two asks for suggestions, as Discord's client
/// waits between them while someone types.
const SUGGEST_EVERY: Duration = Duration::from_millis(500);

pub(super) fn command(
    command: Command,
    discord: Result<&Discord, &str>,
    channels: &mut super::channels::Channels,
    done: &UnboundedSender<Done>,
    events: &UnboundedSender<Event>,
) -> Option<Command> {
    match command {
        Command::Discord(DiscordCommand::Commands { room }) => {
            commands(room, discord, &channels.commands, events);
            None
        }
        Command::Discord(DiscordCommand::SubmitModal {
            room,
            application,
            data,
            nonce,
        }) => {
            if channels.submitting.len() >= super::channels::KEPT_PRESSES {
                channels.submitting.pop_front();
            }
            channels.submitting.push_back(nonce.clone());
            submit(room, application, data, nonce, discord, events);
            None
        }
        Command::Discord(DiscordCommand::Suggest {
            room,
            application,
            data,
            nonce,
        }) => {
            if channels.suggesting.len() >= super::channels::KEPT_PRESSES {
                channels.suggesting.pop_front();
            }
            channels.suggesting.push_back(nonce.clone());
            suggest(
                room,
                application,
                data,
                nonce,
                discord,
                &channels.suggest_pace,
                events,
            );
            None
        }
        Command::Discord(DiscordCommand::RunCommand {
            room,
            application,
            data,
        }) => {
            run(room, application, data, discord, done, events);
            None
        }
        other => Some(other),
    }
}

/// Says which commands `room` offers, asking Discord the first time its
/// server or DM is asked about.
fn commands(
    room: String,
    discord: Result<&Discord, &str>,
    indexes: &Indexes,
    events: &UnboundedSender<Event>,
) {
    let (Ok(discord), Some((guild, channel))) = (discord, discord::parse_room(&room)) else {
        let why = discord.err().unwrap_or(NOT_DISCORD).to_owned();
        let _ = events.send(Event::Commands {
            room,
            index: Err(why),
        });
        return;
    };
    let guild = (guild != obsidian_discord::ready::PRIVATE).then(|| guild.to_owned());
    let (channel, place) = (
        channel.to_owned(),
        guild.clone().unwrap_or_else(|| channel.to_owned()),
    );
    let known = indexes
        .lock()
        .ok()
        .and_then(|indexes| indexes.get(&place).cloned());
    let (discord, events, indexes) = (discord.clone(), events.clone(), indexes.clone());
    tokio::spawn(async move {
        let index = match known {
            Some(index) => Ok(index),
            None => discord
                .command_index(guild.as_deref(), &channel)
                .await
                .map(Arc::new)
                .inspect(|index| {
                    if let Ok(mut indexes) = indexes.lock() {
                        indexes.insert(place, index.clone());
                    }
                })
                .map_err(|error| error.to_string()),
        };
        let index = index.map(|index| usable(&discord, guild.as_deref(), &channel, index));
        let _ = events.send(Event::Commands { room, index });
    });
}

/// The commands of `index` this account may use in `channel`, as Discord's
/// client filters them by the permissions their apps ask for.
fn usable(
    discord: &Discord,
    guild: Option<&str>,
    channel: &str,
    index: Arc<CommandIndex>,
) -> Arc<CommandIndex> {
    let Some(guild) = guild else {
        return index;
    };
    let snapshot = discord.snapshots().borrow().clone();
    let me = snapshot.user_id.clone().unwrap_or_default();
    let permissions = snapshot
        .servers
        .iter()
        .find(|server| server.id == guild)
        .and_then(|server| {
            let channel = server.server.channel(channel)?;
            Some(server.channel_permissions(channel, &me, &server.marks.roles))
        });
    // A thread's channel isn't listed; its commands are its server's.
    let Some(permissions) = permissions else {
        return index;
    };
    index.for_member(permissions).map_or(index, Arc::new)
}

/// Asks an app for suggestions, at most every [`SUGGEST_EVERY`]: one asked
/// sooner waits, and is dropped if another is asked meanwhile.
fn suggest(
    room: String,
    application: String,
    data: serde_json::Value,
    nonce: String,
    discord: Result<&Discord, &str>,
    pace: &Pace,
    events: &UnboundedSender<Event>,
) {
    use crate::chat::Suggested;
    let (Ok(discord), Some((guild, channel))) = (discord, discord::parse_room(&room)) else {
        let _ = events.send(Event::Suggestions {
            nonce,
            answer: Suggested::Failed,
        });
        return;
    };
    let guild = (guild != obsidian_discord::ready::PRIVATE).then(|| guild.to_owned());
    let asked = pace.lock().map_or(0, |mut pace| {
        pace.1 += 1;
        pace.1
    });
    let (discord, events, pace, channel) = (
        discord.clone(),
        events.clone(),
        pace.clone(),
        channel.to_owned(),
    );
    tokio::spawn(async move {
        let wait = pace
            .lock()
            .ok()
            .and_then(|pace| pace.0)
            .map_or(Duration::ZERO, |last| {
                SUGGEST_EVERY.saturating_sub(last.elapsed())
            });
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
            if pace.lock().is_ok_and(|pace| pace.1 != asked) {
                let _ = events.send(Event::Suggestions {
                    nonce,
                    answer: Suggested::Dropped,
                });
                return;
            }
        }
        if let Ok(mut pace) = pace.lock() {
            pace.0 = Some(Instant::now());
        }
        if let Err(error) = discord
            .suggest(guild.as_deref(), &channel, &application, data, &nonce)
            .await
        {
            tracing::debug!(%error, "could not ask an app for suggestions");
            let _ = events.send(Event::Suggestions {
                nonce,
                answer: Suggested::Failed,
            });
        }
    });
}

/// Submits a modal, saying it failed when Discord refuses it or its app
/// doesn't answer within [`SUBMIT_WAIT`].
fn submit(
    room: String,
    application: String,
    data: serde_json::Value,
    nonce: String,
    discord: Result<&Discord, &str>,
    events: &UnboundedSender<Event>,
) {
    let failed = |nonce: String| Event::ModalAnswered {
        nonce,
        taken: false,
    };
    let (Ok(discord), Some((guild, channel))) = (discord, discord::parse_room(&room)) else {
        let _ = events.send(failed(nonce));
        return;
    };
    let guild = (guild != obsidian_discord::ready::PRIVATE).then(|| guild.to_owned());
    let (discord, events, channel) = (discord.clone(), events.clone(), channel.to_owned());
    tokio::spawn(async move {
        if let Err(error) = discord
            .submit_modal(guild.as_deref(), &channel, &application, data, &nonce)
            .await
        {
            tracing::debug!(%error, "could not submit an app's modal");
            let _ = events.send(failed(nonce));
            return;
        }
        // An answer that came meanwhile has closed it, and this is let go.
        tokio::time::sleep(SUBMIT_WAIT).await;
        let _ = events.send(failed(nonce));
    });
}

fn run(
    room: String,
    application: String,
    data: serde_json::Value,
    discord: Result<&Discord, &str>,
    done: &UnboundedSender<Done>,
    events: &UnboundedSender<Event>,
) {
    let (Ok(discord), Some((guild, channel))) = (discord, discord::parse_room(&room)) else {
        let why = discord.err().unwrap_or(NOT_DISCORD).to_owned();
        let _ = events.send(Event::RoomError { room, message: why });
        return;
    };
    let guild = (guild != obsidian_discord::ready::PRIVATE).then(|| guild.to_owned());
    let (discord, done, channel) = (discord.clone(), done.clone(), channel.to_owned());
    tokio::spawn(async move {
        let ran = discord
            .run_command(guild.as_deref(), &channel, &application, data)
            .await;
        let _ = done.send(match ran {
            Ok(nonce) => Done::Pressed { room, nonce },
            Err(error) => Done::Failed {
                room,
                message: format!("This interaction failed: {error}"),
            },
        });
    });
}
