//! Bookmarks and reminders, on the Discord side: what's saved as Discord
//! Obsidian connects, a page of bookmarks as their list asks, and saving or
//! removing one, each told back as an [`Event`].

use obsidian_discord::{
    saved::Saved,
    service::{Discord, Live},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    chat::{Command, DiscordCommand, Event, SavedMessage},
    discord,
};

use super::render::found_in;

/// Carries out the commands for saved messages; others go back.
pub(super) fn command(
    command: Command,
    discord: Result<&Discord, &str>,
    events: &UnboundedSender<Event>,
) -> Option<Command> {
    let Command::Discord(asked) = command else {
        return Some(command);
    };
    let Ok(discord) = discord else {
        return match asked {
            DiscordCommand::SavedMessages => {
                let _ = events.send(Event::SavedMessages(Err("Discord isn't connected.".into())));
                None
            }
            DiscordCommand::Bookmarks { .. }
            | DiscordCommand::SaveMessage { .. }
            | DiscordCommand::UnsaveMessage { .. } => None,
            other => Some(Command::Discord(other)),
        };
    };
    let (discord, events) = (discord.clone(), events.clone());
    match asked {
        DiscordCommand::SavedMessages => {
            tokio::spawn(async move {
                let result = match discord.saved_messages().await {
                    Ok(list) => Ok((
                        list.reminders
                            .iter()
                            .map(|saved| saved_message(&discord, saved))
                            .collect(),
                        list.bookmark_ids,
                    )),
                    Err(error) => {
                        tracing::warn!(
                            error = format!("{error:#}"),
                            "could not list saved messages"
                        );
                        Err(error.to_string())
                    }
                };
                let _ = events.send(Event::SavedMessages(result));
            });
        }
        DiscordCommand::Bookmarks { before } => {
            tokio::spawn(async move {
                let result = match discord.bookmarks(before).await {
                    Ok(page) => Ok((
                        page.bookmarks
                            .iter()
                            .map(|saved| saved_message(&discord, saved))
                            .collect(),
                        page.more,
                    )),
                    Err(error) => {
                        tracing::warn!(error = format!("{error:#}"), "could not list bookmarks");
                        Err(error.to_string())
                    }
                };
                let _ = events.send(Event::Bookmarks { before, result });
            });
        }
        DiscordCommand::SaveMessage { room, item, due_at } => {
            let place = place(&room, &item)?;
            tokio::spawn(async move {
                let (channel, message) = place;
                let result = discord.save_message(&channel, &message, due_at).await;
                let saved = result
                    .as_ref()
                    .ok()
                    .map(|saved| saved_message(&discord, saved));
                let _ = events.send(Event::SaveDone {
                    room,
                    reminder: due_at.is_some(),
                    removed: false,
                    result: result.map(|_| ()).map_err(|error| error.to_string()),
                });
                // The gateway tells of it too; this shows it at once.
                if let Some(saved) = saved {
                    let _ = events.send(Event::MessageSaved(saved));
                }
            });
        }
        DiscordCommand::UnsaveMessage {
            room,
            item,
            reminder,
        } => {
            let place = place(&room, &item)?;
            tokio::spawn(async move {
                let (channel, message) = place;
                let result = discord.unsave_message(&channel, &message).await;
                if result.is_ok() {
                    let _ = events.send(Event::MessageUnsaved {
                        message: message.clone(),
                    });
                }
                let _ = events.send(Event::SaveDone {
                    room,
                    reminder,
                    removed: true,
                    result: result.map_err(|error| error.to_string()),
                });
            });
        }
        other => return Some(Command::Discord(other)),
    }
    None
}

/// What the gateway says of saved messages, from another device or this one.
pub(super) fn live(event: &Live, discord: &Discord, events: &UnboundedSender<Event>) -> bool {
    match event {
        Live::Saved(saved) => {
            let _ = events.send(Event::MessageSaved(saved_message(discord, saved)));
            true
        }
        Live::Unsaved { message, .. } => {
            let _ = events.send(Event::MessageUnsaved {
                message: message.clone(),
            });
            true
        }
        _ => false,
    }
}

/// The channel and message ids of a room's item.
fn place(room: &str, item: &str) -> Option<(String, String)> {
    let (_, channel) = discord::parse_room(room)?;
    let message = item.strip_prefix("d:")?;
    Some((channel.to_owned(), message.to_owned()))
}

/// A saved message as its list shows it, the message rendered as a search's
/// are, when it came.
fn saved_message(discord: &Discord, saved: &Saved) -> SavedMessage {
    let guild = saved
        .guild_id
        .as_deref()
        .unwrap_or(obsidian_discord::ready::PRIVATE);
    let room = discord::room_id(guild, &saved.channel_id);
    let message = saved.message.as_deref().map(|message| {
        found_in(discord, &room, std::slice::from_ref(message))
            .pop()
            .unwrap_or_default()
    });
    SavedMessage {
        room,
        message_id: saved.message_id.clone(),
        saved_at: saved.saved_at_ms,
        due_at: saved.due_at_ms,
        message: message.map(std::sync::Arc::new),
    }
}
