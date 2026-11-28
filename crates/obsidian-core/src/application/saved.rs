//! Bookmarks and reminders, Discord's messages saved for later, as its
//! client keeps them: the reminders and which messages are bookmarked come
//! once Discord connects, and the bookmarks themselves page in as their
//! list in the inbox opens. A message's menu bookmarks it or sets a
//! reminder from Discord's choices of time; the inbox lists both.

use std::collections::HashSet;

use chrono::{Datelike, Duration as Span, Local, TimeZone, Timelike};

use super::*;
use chat::SavedMessage;

/// The most bookmarks, and reminders, Discord's client lets an account keep.
pub const MAX_BOOKMARKS: usize = 500;
pub const MAX_REMINDERS: usize = 50;

/// What's saved for later.
#[derive(Clone, Debug, Default)]
pub struct Saved {
    /// The reminders, soonest due first.
    pub reminders: Vec<SavedMessage>,
    /// The messages bookmarked, by Discord id.
    pub bookmark_ids: HashSet<String>,
    /// The bookmarks listed so far, the latest saved first.
    pub bookmarks: Vec<SavedMessage>,
    /// Earlier bookmarks follow, or a page of them is on its way.
    pub more: bool,
    pub loading: bool,
    /// Their list has asked for its first page.
    pub listed: bool,
    /// What's saved was asked for since Discord connected.
    asked: bool,
    /// The "Set Custom Reminder" dialog, which Create Reminder's Custom...
    /// opens.
    pub custom: Option<CustomReminder>,
    /// Reminders due up to here, in Unix milliseconds, have had their
    /// notice; 0 until the reminders first came, as Discord's client gives
    /// none for those due before it started.
    due_through: u64,
    /// The reminder whose notice showed last: its room and item, which
    /// opening the room from the notice jumps to.
    noticed: Option<(String, String)>,
}

/// "Set Custom Reminder": a date, from today on, and a time, in local time,
/// as Discord's date and time inputs pick them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomReminder {
    pub item: String,
    pub date: chrono::NaiveDate,
    /// Minutes into the day.
    pub minutes: u32,
    /// The first day of the month its calendar shows.
    pub month: chrono::NaiveDate,
    /// The earliest day it takes, today when it opened.
    pub today: chrono::NaiveDate,
}

impl CustomReminder {
    /// Opens at `now`, as Discord's starts with the current date and time.
    pub fn new<Z: TimeZone>(item: String, now: &chrono::DateTime<Z>) -> Self {
        let today = now.date_naive();
        Self {
            item,
            date: today,
            minutes: now.hour() * 60 + now.minute(),
            month: today.with_day(1).expect("the first"),
            today,
        }
    }

    /// When it's due, in Unix milliseconds, in `zone`; a time a clock
    /// change skips takes the hour after.
    pub fn due_at<Z: TimeZone>(&self, zone: &Z) -> u64 {
        let time = self
            .date
            .and_hms_opt(self.minutes / 60 % 24, self.minutes % 60, 0)
            .expect("a time of day");
        let local = zone.from_local_datetime(&time).earliest().or_else(|| {
            zone.from_local_datetime(&(time + Span::hours(1)))
                .earliest()
        });
        local.map_or(0, |at| u64::try_from(at.timestamp_millis()).unwrap_or(0))
    }

    /// The 42 days its calendar shows, six weeks from the Sunday on or
    /// before the month's first, as Discord's date picker lays them out.
    pub fn days(&self) -> Vec<chrono::NaiveDate> {
        let back = i64::from(self.month.weekday().num_days_from_sunday());
        let first = self.month - Span::days(back);
        (0..42).map(|day| first + Span::days(day)).collect()
    }
}

/// How a message stands: bookmarked, or a reminder due at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SavedOf {
    pub bookmarked: bool,
    pub due_at: Option<u64>,
}

impl Saved {
    pub fn of(&self, message_id: &str) -> SavedOf {
        SavedOf {
            bookmarked: self.bookmark_ids.contains(message_id),
            due_at: self
                .reminders
                .iter()
                .find(|reminder| reminder.message_id == message_id)
                .and_then(|reminder| reminder.due_at),
        }
    }

    fn forget(&mut self, message_id: &str) {
        self.bookmark_ids.remove(message_id);
        self.bookmarks
            .retain(|saved| saved.message_id != message_id);
        self.reminders
            .retain(|saved| saved.message_id != message_id);
    }

    /// A message saved, here or elsewhere: a reminder takes a bookmark's
    /// place, as Discord keeps one or the other.
    fn keep(&mut self, saved: SavedMessage) {
        self.forget(&saved.message_id);
        if saved.due_at.is_some() {
            let at = self
                .reminders
                .partition_point(|reminder| reminder.due_at <= saved.due_at);
            self.reminders.insert(at, saved);
        } else {
            self.bookmark_ids.insert(saved.message_id.clone());
            if self.listed {
                self.bookmarks.insert(0, saved);
            }
        }
    }
}

impl Application {
    /// Asks what's saved once Discord connects, as its client does.
    pub(super) fn ask_saved(&mut self) -> Vec<Effect> {
        if !self.state.discord_online {
            self.state.saved.asked = false;
            return vec![];
        }
        if self.state.saved.asked || self.state.preview {
            return vec![];
        }
        self.state.saved.asked = true;
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::SavedMessages,
        ))]
    }

    pub(super) fn saved_listed(
        &mut self,
        result: Result<(Vec<SavedMessage>, Vec<String>), String>,
    ) {
        let Ok((mut reminders, ids)) = result else {
            return;
        };
        reminders.sort_by_key(|reminder| reminder.due_at);
        let saved = &mut self.state.saved;
        if saved.due_through == 0 {
            saved.due_through = crate::clock::now_ms();
        }
        saved.reminders = reminders;
        saved.bookmark_ids = ids.into_iter().collect();
        // A list shown before stays, less what's no longer bookmarked.
        let ids = &saved.bookmark_ids;
        saved
            .bookmarks
            .retain(|bookmark| ids.contains(&bookmark.message_id));
    }

    /// The bookmarks' list asks for its first page, or the next.
    pub(super) fn load_bookmarks(&mut self) -> Vec<Effect> {
        let saved = &mut self.state.saved;
        if saved.loading || (saved.listed && !saved.more) || !self.state.discord_online {
            return vec![];
        }
        let before = saved
            .listed
            .then(|| saved.bookmarks.last().map(|last| last.saved_at))
            .flatten();
        saved.listed = true;
        saved.loading = true;
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::Bookmarks { before },
        ))]
    }

    pub(super) fn bookmarks_came(
        &mut self,
        before: Option<u64>,
        result: Result<(Vec<SavedMessage>, bool), String>,
    ) {
        let saved = &mut self.state.saved;
        saved.loading = false;
        let Ok((page, more)) = result else {
            saved.more = false;
            return;
        };
        if before.is_none() {
            saved.bookmarks.clear();
        }
        // As Discord's client: only those still bookmarked, once each.
        for bookmark in page {
            if saved.bookmark_ids.contains(&bookmark.message_id)
                && !saved
                    .bookmarks
                    .iter()
                    .any(|shown| shown.message_id == bookmark.message_id)
            {
                saved.bookmarks.push(bookmark);
            }
        }
        saved.more = more;
    }

    pub(super) fn message_saved(&mut self, saved: SavedMessage) {
        self.state.saved.keep(saved);
    }

    pub(super) fn message_unsaved(&mut self, message: &str) {
        self.state.saved.forget(message);
    }

    /// Says what became of a bookmark or a reminder, as Discord's toasts do.
    pub(super) fn save_done(&mut self, reminder: bool, removed: bool, result: Result<(), String>) {
        self.state.notice = Some(match (result, reminder, removed) {
            (Err(error), _, _) if !error.is_empty() => error,
            (Err(_), _, _) => "Oops, something went wrong...".into(),
            (Ok(()), true, false) => "Reminder added.".into(),
            (Ok(()), false, false) => "Bookmark added. For your eyes only.".into(),
            (Ok(()), true, true) => "Reminder removed.".into(),
            (Ok(()), false, true) => "Bookmark removed.".into(),
        });
    }

    /// Bookmarks an item, or with `due_at` sets it a reminder, within
    /// Discord's limits.
    pub(super) fn save_message(&mut self, item: String, due_at: Option<u64>) -> Vec<Effect> {
        let Some(room) = self.saved_room(&item) else {
            return vec![];
        };
        let Some(id) = item.strip_prefix("d:") else {
            return vec![];
        };
        let saved = &self.state.saved;
        let already = saved.of(id);
        let full = match due_at {
            Some(_) => already.due_at.is_none() && saved.reminders.len() >= MAX_REMINDERS,
            None => !already.bookmarked && saved.bookmark_ids.len() >= MAX_BOOKMARKS,
        };
        if full {
            self.state.notice = Some(match due_at {
                Some(_) => format!(
                    "You\u{2019}ve reached the limit for reminders ({MAX_REMINDERS} total). To add more, please delete a few reminders."
                ),
                None => format!(
                    "You\u{2019}ve reached the limit for bookmarks ({MAX_BOOKMARKS} total). To add more, please delete a few messages."
                ),
            });
            return vec![];
        }
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::SaveMessage { room, item, due_at },
        ))]
    }

    /// Removes an item's bookmark, or its reminder, which marks it complete.
    pub(super) fn unsave_message(&mut self, item: String) -> Vec<Effect> {
        let Some(room) = self.saved_room(&item) else {
            return vec![];
        };
        let Some(id) = item.strip_prefix("d:") else {
            return vec![];
        };
        let of = self.state.saved.of(id);
        if !of.bookmarked && of.due_at.is_none() {
            return vec![];
        }
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::UnsaveMessage {
                room,
                item,
                reminder: of.due_at.is_some(),
            },
        ))]
    }

    /// The Discord room of an item: one listed as saved, or on screen.
    /// Create Reminder's Custom... opens "Set Custom Reminder" for an item
    /// a reminder can be set on.
    pub(super) fn open_custom_reminder(&mut self, item: String) -> Vec<Effect> {
        if self.saved_room(&item).is_none() {
            return vec![];
        }
        self.state.saved.custom = Some(CustomReminder::new(item, &Local::now()));
        vec![]
    }

    /// A day picked on its calendar, from today on.
    pub(super) fn custom_reminder_date(&mut self, date: chrono::NaiveDate) -> Vec<Effect> {
        if let Some(custom) = &mut self.state.saved.custom
            && date >= custom.today
        {
            custom.date = date;
            custom.month = date.with_day(1).expect("the first");
        }
        vec![]
    }

    /// Its calendar a month back, or with 1 on, never before this month.
    pub(super) fn custom_reminder_month(&mut self, step: i32) -> Vec<Effect> {
        if let Some(custom) = &mut self.state.saved.custom {
            let month = if step < 0 {
                custom.month.checked_sub_months(chrono::Months::new(1))
            } else {
                custom.month.checked_add_months(chrono::Months::new(1))
            };
            if let Some(month) = month
                && month >= custom.today.with_day(1).expect("the first")
            {
                custom.month = month;
            }
        }
        vec![]
    }

    /// A time picked, in minutes into the day.
    pub(super) fn custom_reminder_time(&mut self, minutes: u32) -> Vec<Effect> {
        if let Some(custom) = &mut self.state.saved.custom
            && minutes < 24 * 60
        {
            custom.minutes = minutes;
        }
        vec![]
    }

    /// Save sets the reminder at the date and time chosen.
    pub(super) fn save_custom_reminder(&mut self) -> Vec<Effect> {
        let Some(custom) = self.state.saved.custom.take() else {
            return vec![];
        };
        let due_at = custom.due_at(&Local);
        self.save_message(custom.item, Some(due_at))
    }

    /// When the next reminder comes due, in Unix milliseconds, for a timer
    /// to notice it: as Discord's client, only one due within a week of
    /// `now` and after the reminders first came.
    pub fn next_reminder_due(&self, now: u64) -> Option<u64> {
        let saved = &self.state.saved;
        if saved.due_through == 0 {
            return None;
        }
        const WEEK: u64 = 7 * 24 * 3_600_000;
        saved
            .reminders
            .iter()
            .filter_map(|reminder| reminder.due_at)
            .find(|due_at| *due_at > saved.due_through)
            .filter(|due_at| *due_at <= now + WEEK)
    }

    /// Reminders came due by `now`: each has a notice, "⏰ Reminder Due"
    /// over what its message says, as Discord's client gives, unless
    /// notifications are held off by Do Not Disturb.
    pub(super) fn reminders_due(&mut self, now: u64) -> Vec<Effect> {
        let saved = &mut self.state.saved;
        if saved.due_through == 0 || now <= saved.due_through {
            return vec![];
        }
        let from = std::mem::replace(&mut saved.due_through, now);
        if self.state.do_not_disturb() {
            return vec![];
        }
        let due: Vec<SavedMessage> = self
            .state
            .saved
            .reminders
            .iter()
            .filter(|reminder| {
                reminder
                    .due_at
                    .is_some_and(|due_at| due_at > from && due_at <= now)
            })
            .cloned()
            .collect();
        let mut effects = vec![];
        for reminder in due {
            // As Discord's, none for a message gone or out of reach.
            let Some(message) = reminder.message.as_deref() else {
                continue;
            };
            if !self.state.rooms.iter().any(|room| room.id == reminder.room) {
                continue;
            }
            let picture = message
                .avatar
                .as_ref()
                .and_then(|avatar| self.state.pictures.get(avatar).cloned());
            effects.push(Effect::Notify(chat::Alert {
                room: reminder.room.clone(),
                title: "\u{23F0} Reminder Due".into(),
                body: chat::notification_text(&message.body),
                picture,
                call: false,
                mention: false,
                reminder: true,
            }));
            self.state.saved.noticed = Some((reminder.room.clone(), reminder.item()));
        }
        effects
    }

    /// Opening a room from a notification; the last reminder's notice
    /// shows its message, as Discord's jumps to it.
    pub(super) fn open_noticed(&mut self, room: String) -> Vec<Effect> {
        match self.state.saved.noticed.take() {
            Some((noticed, item)) if noticed == room => {
                self.dispatch(Action::JumpInRoom { room, item })
            }
            noticed => {
                self.state.saved.noticed = noticed;
                self.open(room, true)
            }
        }
    }

    fn saved_room(&self, item: &str) -> Option<String> {
        let id = item.strip_prefix("d:")?;
        let saved = &self.state.saved;
        saved
            .reminders
            .iter()
            .chain(&saved.bookmarks)
            .find(|saved| saved.message_id == id)
            .map(|saved| saved.room.clone())
            .or_else(|| self.state.room_of(item).cloned())
            .filter(|room| Network::of(room) == Network::Discord)
    }
}

/// Discord's choices of when a reminder is due, from `now`: in 30 minutes,
/// an hour and four hours, at 9 the next morning (or this one, before 9),
/// and Monday at 9; each with its label, and Unix milliseconds.
pub fn reminder_times<Z: TimeZone>(now: chrono::DateTime<Z>) -> Vec<(&'static str, u64)> {
    let ms = |time: chrono::DateTime<Z>| u64::try_from(time.timestamp_millis()).unwrap_or(0);
    let nine = |day: chrono::NaiveDate| {
        now.timezone()
            .from_local_datetime(&day.and_hms_opt(9, 0, 0).expect("9:00"))
            .earliest()
            .unwrap_or_else(|| now.clone())
    };
    let today = now.date_naive();
    let morning = if now.hour() >= 9 {
        ("Tomorrow at 9:00am", nine(today + Span::days(1)))
    } else {
        ("Today at 9:00 AM", nine(today))
    };
    // As Discord's client: the coming Monday, or this one before 9.
    let weekday = now.weekday().num_days_from_sunday();
    let to_monday = match weekday {
        0 => 1,
        1 if now.hour() < 9 => 0,
        day => 8 - i64::from(day),
    };
    vec![
        ("in 30m", ms(now.clone() + Span::minutes(30))),
        ("in 1h", ms(now.clone() + Span::hours(1))),
        ("in 4h", ms(now.clone() + Span::hours(4))),
        (morning.0, ms(morning.1)),
        ("Monday at 9:00am", ms(nine(today + Span::days(to_monday)))),
    ]
}

/// The choices of [`reminder_times`] from now, here.
pub fn reminder_times_now() -> Vec<(&'static str, u64)> {
    reminder_times(Local::now())
}

/// When a reminder is due, as Discord's client says it: "Reminder due in 3
/// hours", or "Reminder due 3 hours ago".
pub fn reminder_due(due_at: u64, now: u64) -> String {
    if due_at >= now {
        format!("Reminder due in {}", humanize(due_at - now))
    } else {
        format!("Reminder due {} ago", humanize(now - due_at))
    }
}

/// When a reminder is due, as its line in the inbox says it: "Due in 3
/// hours", or "Due 3 hours ago".
pub fn due_label(due_at: u64, now: u64) -> String {
    if due_at >= now {
        format!("Due in {}", humanize(due_at - now))
    } else {
        format!("Due {} ago", humanize(now - due_at))
    }
}

/// A span as moment.js, which Discord's client uses, says it.
fn humanize(ms: u64) -> String {
    let seconds = ms / 1000;
    let round = |value: u64, unit: u64| (value + unit / 2) / unit;
    match seconds {
        0..45 => "a few seconds".into(),
        45..90 => "a minute".into(),
        90..2_700 => format!("{} minutes", round(seconds, 60)),
        2_700..5_400 => "an hour".into(),
        5_400..79_200 => format!("{} hours", round(seconds, 3_600)),
        79_200..129_600 => "a day".into(),
        129_600..2_246_400 => format!("{} days", round(seconds, 86_400)),
        2_246_400..3_888_000 => "a month".into(),
        _ => format!("{} months", round(seconds, 2_592_000).max(2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_discords_times_for_a_reminder() {
        let zone = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
        let at = |text: &str| {
            chrono::DateTime::parse_from_rfc3339(text)
                .unwrap()
                .with_timezone(&zone)
        };
        let ms = |text: &str| at(text).timestamp_millis() as u64;
        // A Wednesday afternoon.
        let times = reminder_times(at("2026-11-25T15:10:00+03:00"));
        let labels: Vec<_> = times.iter().map(|(label, _)| *label).collect();
        assert_eq!(
            labels,
            [
                "in 30m",
                "in 1h",
                "in 4h",
                "Tomorrow at 9:00am",
                "Monday at 9:00am"
            ]
        );
        assert_eq!(times[0].1, ms("2026-11-25T15:40:00+03:00"));
        assert_eq!(times[3].1, ms("2026-11-26T09:00:00+03:00"));
        assert_eq!(times[4].1, ms("2026-11-30T09:00:00+03:00"));
        // Before 9 on a Monday: this morning, and this Monday.
        let early = reminder_times(at("2026-11-30T07:00:00+03:00"));
        assert_eq!(
            early[3],
            ("Today at 9:00 AM", ms("2026-11-30T09:00:00+03:00"))
        );
        assert_eq!(early[4].1, ms("2026-11-30T09:00:00+03:00"));
        // A Sunday: tomorrow is Monday.
        let sunday = reminder_times(at("2026-11-29T20:00:00+03:00"));
        assert_eq!(sunday[4].1, ms("2026-11-30T09:00:00+03:00"));
    }

    #[test]
    fn says_when_a_reminder_is_due_as_discord_does() {
        let hour = 3_600_000;
        assert_eq!(reminder_due(10 * hour, 7 * hour), "Reminder due in 3 hours");
        assert_eq!(
            reminder_due(7 * hour, 10 * hour),
            "Reminder due 3 hours ago"
        );
        assert_eq!(
            reminder_due(hour, hour - 20_000),
            "Reminder due in a few seconds"
        );
        assert_eq!(reminder_due(30 * hour, 0), "Reminder due in a day");
        assert_eq!(reminder_due(20 * 60_000, 0), "Reminder due in 20 minutes");
        assert_eq!(due_label(0, 2 * hour), "Due 2 hours ago");
    }
}
