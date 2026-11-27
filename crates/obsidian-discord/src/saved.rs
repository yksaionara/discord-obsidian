//! Bookmarks and reminders, Discord's saved messages: a message saved for
//! this account alone, with a time it's due when it's a reminder. See
//! <https://docs.discord.food/resources/user#saved-messages>.
//!
//! As Discord's client does, the reminders and the ids of the bookmarked
//! messages come at once when it connects (`GET
//! /users/@me/saved-messages?bookmark_ids=true`), and the bookmarks
//! themselves 25 at a time, newest saved first, when their list opens.

use serde_json::{Value, json};

use crate::message::{self, Message, iso_ms};

/// The most bookmarks, and the most reminders, Discord's client lets an
/// account keep; past them Discord answers `TOO_MANY_SAVED_MESSAGES`.
pub const MAX_BOOKMARKS: usize = 500;
pub const MAX_REMINDERS: usize = 50;
/// How many bookmarks a page of them brings, as Discord's client asks.
pub const BOOKMARKS_PAGE: usize = 25;
/// What Discord's client names as where a message was saved from, its
/// message menu.
pub const FROM_MENU: &str = "context_menu";

/// A saved message: a bookmark, or a reminder once it has a time it's due.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub channel_id: String,
    pub message_id: String,
    pub guild_id: Option<String>,
    /// When it was saved, in Unix milliseconds; bookmarks page by it.
    pub saved_at_ms: u64,
    /// When a reminder is due, in Unix milliseconds.
    pub due_at_ms: Option<u64>,
    /// The message, when Discord sends it and this account can still read it.
    pub message: Option<Box<Message>>,
}

impl Saved {
    /// A saved message object: `save_data`, and the `message`.
    pub fn parse(value: &Value) -> Option<Self> {
        let data = &value["save_data"];
        Some(Self {
            channel_id: data["channel_id"].as_str()?.to_owned(),
            message_id: data["message_id"].as_str()?.to_owned(),
            guild_id: data["guild_id"].as_str().map(str::to_owned),
            saved_at_ms: data["saved_at"].as_str().and_then(iso_ms).unwrap_or(0),
            due_at_ms: data["due_at"].as_str().and_then(iso_ms),
            message: value
                .get("message")
                .filter(|message| message.is_object())
                .and_then(message::parse)
                .map(Box::new),
        })
    }

    pub fn reminder(&self) -> bool {
        self.due_at_ms.is_some()
    }
}

/// What Discord says is saved, as it connects: the reminders, and which
/// messages are bookmarked, by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedList {
    pub reminders: Vec<Saved>,
    pub bookmark_ids: Vec<String>,
}

impl SavedList {
    pub fn parse(value: &Value) -> Self {
        Self {
            reminders: value["reminders"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Saved::parse)
                .collect(),
            bookmark_ids: value["bookmark_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect(),
        }
    }
}

/// A page of bookmarks, and whether earlier ones follow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bookmarks {
    pub bookmarks: Vec<Saved>,
    pub more: bool,
}

impl Bookmarks {
    pub fn parse(value: &Value) -> Self {
        Self {
            bookmarks: value["results"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Saved::parse)
                .collect(),
            more: value["has_more"].as_bool().unwrap_or(false),
        }
    }
}

/// The body that saves a message: due at a time for a reminder, or not for
/// a bookmark, as Discord's client sends it.
pub fn save_body(due_at_ms: Option<u64>) -> Value {
    json!({
        "due_at": due_at_ms.map(iso),
        "source": FROM_MENU,
    })
}

/// Unix milliseconds as Discord writes a time, in UTC.
pub fn iso(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(i64::try_from(ms).unwrap_or(i64::MAX))
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3f+00:00")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_reminders_and_bookmarks_as_discord_sends_them() {
        let list = SavedList::parse(&json!({
            "reminders": [{
                "save_data": {"channel_id": "11", "message_id": "5", "guild_id": "10",
                    "saved_at": "2026-11-23T10:00:00.000000+00:00",
                    "due_at": "2026-11-23T11:00:00.000000+00:00",
                    "author_summary": "", "channel_summary": "", "message_summary": "", "notes": ""},
                "message": {"id": "5", "channel_id": "11", "type": 0, "content": "lunch?",
                    "author": {"id": "2", "username": "bob"}, "attachments": [], "embeds": []},
            }],
            "bookmark_ids": ["7", "8"],
        }));
        let reminder = &list.reminders[0];
        assert!(reminder.reminder());
        assert_eq!(
            reminder.due_at_ms.unwrap() - reminder.saved_at_ms,
            3_600_000
        );
        assert_eq!(reminder.message.as_ref().unwrap().content, "lunch?");
        assert_eq!(list.bookmark_ids, ["7", "8"]);

        let page = Bookmarks::parse(&json!({
            "results": [{"save_data": {"channel_id": "11", "message_id": "7",
                "saved_at": "2026-11-23T09:00:00.000000+00:00", "due_at": null}, "message": null}],
            "has_more": true,
        }));
        assert!(page.more && !page.bookmarks[0].reminder());
        assert!(page.bookmarks[0].message.is_none(), "one it can't read");
    }

    #[test]
    fn saves_with_the_time_a_reminder_is_due() {
        let due = iso_ms("2026-11-23T11:00:00.000000+00:00").unwrap();
        assert_eq!(iso(due), "2026-11-23T11:00:00.000+00:00");
        assert_eq!(
            save_body(Some(due)),
            json!({"due_at": "2026-11-23T11:00:00.000+00:00", "source": "context_menu"})
        );
        assert_eq!(save_body(None)["due_at"], Value::Null);
    }
}
