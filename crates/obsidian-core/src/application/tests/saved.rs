//! Bookmarks and reminders.

use super::*;
use chat::SavedMessage;

const ROOM: &str = "discord:10:11";

fn writing() -> Application {
    let mut app = discord_server();
    app.state.discord_online = true;
    app.dispatch(Action::Navigate(Navigation::Server("discord:10".into())));
    app.dispatch(Action::SelectRoom(ROOM.into()));
    app.apply(Event::Timeline {
        room: ROOM.into(),
        messages: timeline(vec![message("d:5", false), message("d:6", false)]),
    });
    app
}

fn saved(id: &str, saved_at: u64, due_at: Option<u64>) -> SavedMessage {
    SavedMessage {
        room: ROOM.into(),
        message_id: id.into(),
        saved_at,
        due_at,
        message: None,
    }
}

fn asked(effects: &[Effect]) -> Option<DiscordCommand> {
    effects.iter().find_map(|effect| match effect {
        Effect::Backend(Command::Discord(
            command @ (DiscordCommand::SaveMessage { .. }
            | DiscordCommand::UnsaveMessage { .. }
            | DiscordCommand::Bookmarks { .. }),
        )) => Some(command.clone()),
        _ => None,
    })
}

#[test]
fn a_message_is_bookmarked_or_given_a_reminder_in_its_room() {
    let mut app = writing();
    let effects = app.dispatch(Action::Bookmark("d:5".into()));
    assert!(matches!(
        asked(&effects),
        Some(DiscordCommand::SaveMessage { room, item, due_at: None }) if room == ROOM && item == "d:5"
    ));
    let effects = app.dispatch(Action::Remind {
        item: "d:6".into(),
        due_at: 9_000,
    });
    assert!(matches!(
        asked(&effects),
        Some(DiscordCommand::SaveMessage {
            due_at: Some(9_000),
            ..
        })
    ));
    app.apply(Event::SaveDone {
        room: ROOM.into(),
        reminder: false,
        removed: false,
        result: Ok(()),
    });
    assert_eq!(
        app.state.notice.as_deref(),
        Some("Bookmark added. For your eyes only.")
    );
    // Nothing saved has nothing to remove.
    assert!(asked(&app.dispatch(Action::Unsave("d:5".into()))).is_none());
}

#[test]
fn what_discord_says_is_saved_shows_in_each_menu_and_changes_live() {
    let mut app = writing();
    app.apply(Event::SavedMessages(Ok((
        vec![saved("6", 100, Some(5_000))],
        vec!["5".into()],
    ))));
    assert!(app.state.saved.of("5").bookmarked);
    assert_eq!(app.state.saved.of("6").due_at, Some(5_000));
    // Removing a reminder marks it complete.
    assert!(matches!(
        asked(&app.dispatch(Action::Unsave("d:6".into()))),
        Some(DiscordCommand::UnsaveMessage { reminder: true, .. })
    ));
    // A reminder set on another device takes the bookmark's place.
    app.apply(Event::MessageSaved(saved("5", 200, Some(4_000))));
    let of = app.state.saved.of("5");
    assert!(!of.bookmarked && of.due_at == Some(4_000));
    let due: Vec<_> = app
        .state
        .saved
        .reminders
        .iter()
        .map(|reminder| reminder.message_id.as_str())
        .collect();
    assert_eq!(due, ["5", "6"], "soonest due first");
    app.apply(Event::MessageUnsaved {
        message: "6".into(),
    });
    assert_eq!(app.state.saved.of("6"), SavedOf::default());
}

#[test]
fn bookmarks_page_in_as_discords_client_asks() {
    let mut app = writing();
    app.apply(Event::SavedMessages(Ok((
        vec![],
        vec!["5".into(), "6".into(), "7".into()],
    ))));
    assert!(matches!(
        asked(&app.dispatch(Action::LoadBookmarks)),
        Some(DiscordCommand::Bookmarks { before: None })
    ));
    // One on its way asks no more.
    assert!(asked(&app.dispatch(Action::LoadBookmarks)).is_none());
    app.apply(Event::Bookmarks {
        before: None,
        result: Ok((vec![saved("7", 300, None), saved("6", 200, None)], true)),
    });
    assert!(matches!(
        asked(&app.dispatch(Action::LoadBookmarks)),
        Some(DiscordCommand::Bookmarks { before: Some(200) })
    ));
    // Removed meanwhile: it doesn't show.
    app.apply(Event::MessageUnsaved {
        message: "5".into(),
    });
    app.apply(Event::Bookmarks {
        before: Some(200),
        result: Ok((vec![saved("5", 100, None)], false)),
    });
    let listed: Vec<_> = app
        .state
        .saved
        .bookmarks
        .iter()
        .map(|bookmark| bookmark.message_id.as_str())
        .collect();
    assert_eq!(listed, ["7", "6"]);
    assert!(
        asked(&app.dispatch(Action::LoadBookmarks)).is_none(),
        "the last page"
    );
    // A new one goes first.
    app.apply(Event::MessageSaved(saved("8", 400, None)));
    assert_eq!(app.state.saved.bookmarks[0].message_id, "8");
}

#[test]
fn discords_limits_are_kept() {
    let mut app = writing();
    let ids: Vec<String> = (1000..1000 + MAX_BOOKMARKS)
        .map(|id| id.to_string())
        .collect();
    app.apply(Event::SavedMessages(Ok((vec![], ids))));
    assert!(asked(&app.dispatch(Action::Bookmark("d:5".into()))).is_none());
    assert!(app.state.notice.as_deref().is_some_and(|notice| {
        notice.starts_with("You\u{2019}ve reached the limit for bookmarks (500 total)")
    }));
}
