//! Bookmarks and reminders in the window: a Discord message's menu, which
//! bookmarks it or sets it a reminder from Discord's times, and the inbox's
//! Bookmarks and Reminders tabs.

use super::*;
use chrono::Datelike;

/// What a reminder row's id holds: a time it's due, in Unix milliseconds.
const DUE: &str = "due:";
/// The reminder row that marks one complete.
const COMPLETE: &str = "complete";
/// The reminder row that opens "Set Custom Reminder".
const CUSTOM: &str = "custom";
/// How far apart the custom reminder's times are, as Discord's time input.
const TIME_STEP: u32 = 15;

impl Adapter {
    /// A message's menu is opening: how it's saved, and its reminder's
    /// submenu.
    pub(super) fn show_saved_menu(&mut self, window: &ObsidianWindow, item: &str) {
        let state = &self.app.state;
        let room = state.room_of(item);
        let savable = state.discord_online
            && !state.preview
            && room.is_some_and(|room| Network::of(room) == Network::Discord);
        let Some(id) = item.strip_prefix("d:").filter(|_| savable) else {
            window.set_menu_saved(0);
            window.set_menu_reminder_rows(ModelRc::default());
            return;
        };
        let of = state.saved.of(id);
        window.set_menu_saved(match (of.due_at, of.bookmarked) {
            (Some(_), _) => 3,
            (None, true) => 2,
            (None, false) => 1,
        });
        let mut rows = Vec::new();
        if let Some(due_at) = of.due_at {
            let now = obsidian_core::clock::now_ms();
            rows.push(MenuRow {
                text: obsidian_core::application::reminder_due(due_at, now).into(),
                disabled: true,
                plain: true,
                ..Default::default()
            });
            rows.push(MenuRow {
                id: COMPLETE.into(),
                text: "Mark Complete".into(),
                icon: self.icons.check.clone(),
                ..Default::default()
            });
        }
        for (index, (label, due_at)) in obsidian_core::application::reminder_times_now()
            .into_iter()
            .enumerate()
        {
            rows.push(MenuRow {
                id: format!("{DUE}{due_at}").into(),
                text: label.into(),
                plain: true,
                separated: index == 0 && of.due_at.is_some(),
                ..Default::default()
            });
        }
        rows.push(MenuRow {
            id: CUSTOM.into(),
            text: "Custom...".into(),
            plain: true,
            ..Default::default()
        });
        window.set_menu_reminder_rows(ModelRc::new(VecModel::from(rows)));
    }

    /// "Set Custom Reminder" while it's open: its date and time, and the
    /// calendar's month.
    pub(super) fn show_custom_reminder(&mut self, window: &ObsidianWindow) {
        let Some(custom) = &self.app.state.saved.custom else {
            if window.get_custom_reminder_open() {
                window.set_custom_reminder_open(false);
                window.set_custom_reminder_days(ModelRc::default());
                window.set_custom_reminder_times(ModelRc::default());
            }
            return;
        };
        let twelve_hour = self.layout.twelve_hour_clock;
        window.set_custom_reminder_date(
            obsidian_core::clock::numeric_date(custom.date, twelve_hour).into(),
        );
        window.set_custom_reminder_time(
            obsidian_core::clock::time_of_day(custom.minutes, twelve_hour).into(),
        );
        window.set_custom_reminder_month(custom.month.format("%B %Y").to_string().into());
        window.set_custom_reminder_earlier(custom.month > custom.today);
        let days: Vec<ReminderDay> = custom
            .days()
            .into_iter()
            .map(|day| ReminderDay {
                day: day.day() as i32,
                in_month: day.month() == custom.month.month(),
                past: day < custom.today,
                today: day == custom.today,
                picked: day == custom.date,
            })
            .collect();
        window.set_custom_reminder_days(ModelRc::new(VecModel::from(days)));
        window.set_custom_reminder_time_near((custom.minutes / TIME_STEP) as i32);
        window.set_custom_reminder_time_index(if custom.minutes % TIME_STEP == 0 {
            (custom.minutes / TIME_STEP) as i32
        } else {
            -1
        });
        if !window.get_custom_reminder_open() || window.get_custom_reminder_times().row_count() == 0
        {
            let times: Vec<SharedString> = (0..24 * 60 / TIME_STEP)
                .map(|step| obsidian_core::clock::time_of_day(step * TIME_STEP, twelve_hour).into())
                .collect();
            window.set_custom_reminder_times(ModelRc::new(VecModel::from(times)));
        }
        window.set_custom_reminder_open(true);
    }

    /// The calendar's day at `index`, of the 42 it shows.
    pub(super) fn custom_reminder_day(&self, index: i32) -> Option<Action> {
        let custom = self.app.state.saved.custom.as_ref()?;
        let day = *custom.days().get(usize::try_from(index).ok()?)?;
        Some(Action::CustomReminderDate(day))
    }

    /// Arms the timer for the next reminder coming due, as Discord's client
    /// keeps one for the soonest within a week; it notices them when it
    /// fires and arms itself again.
    pub(super) fn arm_reminders(&mut self) {
        let now = obsidian_core::clock::now_ms();
        let next = self.app.next_reminder_due(now);
        if next == self.reminder_armed {
            return;
        }
        self.reminder_armed = next;
        let Some(due_at) = next else {
            self.reminder_timer.stop();
            return;
        };
        let weak = self.weak.clone();
        self.reminder_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(due_at.saturating_sub(now).max(1)),
            move || {
                let Some(adapter) = weak.upgrade() else {
                    return;
                };
                let window = adapter.borrow().window.upgrade();
                let mut adapter = adapter.borrow_mut();
                adapter.reminder_armed = None;
                match window {
                    Some(window) => adapter.action(&window, Action::RemindersDue),
                    None => {
                        let effects = adapter.app.dispatch(Action::RemindersDue);
                        adapter.execute(effects);
                        adapter.arm_reminders();
                    }
                }
            },
        );
    }

    /// The inbox's Bookmarks or Reminders: each saved message with who
    /// wrote it, where, what it says, and when it was sent or is due.
    pub(super) fn saved_rows(&self, reminders: bool) -> Vec<InboxRow> {
        let state = &self.app.state;
        let now = obsidian_core::clock::now_ms();
        let twelve_hour = self.layout.twelve_hour_clock;
        let listed = if reminders {
            &state.saved.reminders
        } else {
            &state.saved.bookmarks
        };
        listed
            .iter()
            .map(|saved| {
                let room = state.rooms.iter().find(|room| room.id == saved.room);
                let place = room.map_or_else(String::new, |room| {
                    if room.direct {
                        room.name.clone()
                    } else {
                        format!("#{}", room.name)
                    }
                });
                let message = saved.message.as_deref();
                let author = message.map_or("", |message| message.author.as_str());
                InboxRow {
                    id: saved.room.as_str().into(),
                    item: saved.item().into(),
                    name: author.into(),
                    place: place.into(),
                    initial: initial(author),
                    tint: tint(author),
                    picture_url: message
                        .and_then(|message| message.avatar.as_deref())
                        .unwrap_or_default()
                        .into(),
                    body: match message {
                        Some(message) => {
                            obsidian_core::chat::notification_text(&message.body).into()
                        }
                        // As Discord's, for one gone or out of reach.
                        None if reminders => {
                            "A message you set a reminder for could not be found.".into()
                        }
                        None => "A message you bookmarked could not be found.".into(),
                    },
                    when: match saved.due_at {
                        Some(due_at) => obsidian_core::application::due_label(due_at, now).into(),
                        None => message
                            .filter(|message| message.timestamp > 0)
                            .map(|message| {
                                obsidian_core::clock::sent(
                                    message.timestamp,
                                    now / 1000,
                                    twelve_hour,
                                    false,
                                )
                            })
                            .unwrap_or_default()
                            .into(),
                    },
                    overdue: saved.due_at.is_some_and(|due_at| due_at < now),
                    mentions: 0,
                }
            })
            .collect()
    }
}

/// What a row of a message's reminder submenu does to `item`.
pub(super) fn reminder_action(item: &str, id: &str) -> Option<Action> {
    if id == COMPLETE {
        return Some(Action::Unsave(item.to_owned()));
    }
    if id == CUSTOM {
        return Some(Action::OpenCustomReminder(item.to_owned()));
    }
    let due_at = id.strip_prefix(DUE)?.parse().ok()?;
    Some(Action::Remind {
        item: item.to_owned(),
        due_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reminder_row_sets_its_time_or_completes_it() {
        assert!(matches!(
            reminder_action("d:5", "due:1800"),
            Some(Action::Remind { item, due_at: 1800 }) if item == "d:5"
        ));
        assert!(matches!(
            reminder_action("d:5", COMPLETE),
            Some(Action::Unsave(item)) if item == "d:5"
        ));
        assert!(matches!(
            reminder_action("d:5", CUSTOM),
            Some(Action::OpenCustomReminder(item)) if item == "d:5"
        ));
        assert!(reminder_action("d:5", "").is_none());
    }
}
