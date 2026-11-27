//! Apps' select menus under their messages.

use super::*;
use chat::{Card, CardRow, SelectKind, SelectMenu, SelectOption};

const ROOM: &str = "discord:10:11";

fn option(label: &str, value: &str, default: bool) -> SelectOption {
    SelectOption {
        label: label.into(),
        value: value.into(),
        description: None,
        emoji: None,
        default,
    }
}

fn menu(kind: SelectKind, min: u8, max: u8) -> SelectMenu {
    SelectMenu {
        kind,
        custom_id: "pick".into(),
        options: vec![
            option("Red", "r", false),
            option("Green", "g", true),
            option("Blue", "b", false),
        ],
        defaults: vec![],
        channel_types: vec![],
        min,
        max,
    }
}

/// #general open, with an app's message whose only row is `menu`.
fn showing(menu: SelectMenu) -> Application {
    let mut app = discord_server();
    app.dispatch(Action::Navigate(Navigation::Server("discord:10".into())));
    app.dispatch(Action::SelectRoom(ROOM.into()));
    let message = ChatMessage {
        cards: vec![Card {
            framed: false,
            color: None,
            rows: vec![CardRow::Select {
                text: "Green".into(),
                disabled: false,
                menu: Some(Arc::new(menu)),
            }],
        }],
        ..message("d:5", false)
    };
    app.apply(Event::Timeline {
        room: ROOM.into(),
        messages: timeline(vec![message]),
    });
    app
}

fn open(app: &mut Application) {
    app.dispatch(Action::OpenSelect {
        item: "d:5".into(),
        custom_id: "pick".into(),
    });
}

fn chosen(effects: &[Effect]) -> Option<Vec<String>> {
    effects.iter().find_map(|effect| match effect {
        Effect::Backend(Command::ChooseSelect {
            room,
            item,
            custom_id,
            values,
        }) if room == ROOM && item == "d:5" && custom_id == "pick" => Some(values.clone()),
        _ => None,
    })
}

fn shown_text(app: &Application) -> String {
    match &app.state.message("d:5").unwrap().cards[0].rows[0] {
        CardRow::Select { text, .. } => text.clone(),
        _ => panic!("a menu"),
    }
}

#[test]
fn a_menu_of_one_tells_its_app_as_soon_as_its_picked() {
    let mut app = showing(menu(SelectKind::Text, 1, 1));
    open(&mut app);
    let lines = app.select_choices();
    let picked: Vec<_> = lines
        .iter()
        .map(|line| (line.label.as_str(), line.picked))
        .collect();
    assert_eq!(picked, [("Red", false), ("Green", true), ("Blue", false)]);
    let effects = app.dispatch(Action::PickSelect("b".into()));
    assert_eq!(chosen(&effects), Some(vec!["b".to_owned()]));
    assert!(app.state.select.is_none(), "it closes");
    // It shows the choice, and picks it when it opens again.
    assert_eq!(shown_text(&app), "Blue");
    open(&mut app);
    assert_eq!(app.state.select.as_ref().unwrap().picked, ["b"]);
}

#[test]
fn a_menu_of_several_tells_its_app_once_it_closes_changed() {
    let mut app = showing(menu(SelectKind::Text, 1, 2));
    open(&mut app);
    // Closed as it opened: nothing to tell.
    assert_eq!(chosen(&app.dispatch(Action::CloseSelect)), None);
    open(&mut app);
    assert!(app.dispatch(Action::PickSelect("r".into())).is_empty());
    // No more than two.
    app.dispatch(Action::PickSelect("b".into()));
    assert_eq!(app.state.select.as_ref().unwrap().picked, ["g", "r"]);
    let effects = app.dispatch(Action::CloseSelect);
    assert_eq!(chosen(&effects), Some(vec!["g".to_owned(), "r".to_owned()]));
    assert_eq!(shown_text(&app), "Green, Red");

    // Fewer than its fewest isn't sent.
    open(&mut app);
    app.dispatch(Action::PickSelect("g".into()));
    app.dispatch(Action::PickSelect("r".into()));
    assert_eq!(chosen(&app.dispatch(Action::CloseSelect)), None);
}

#[test]
fn a_menu_of_roles_lists_the_servers_and_searches_them() {
    let mut app = showing(menu(SelectKind::Role, 1, 1));
    open(&mut app);
    let open = app.state.select.as_ref().unwrap();
    assert!(open.searchable() && !open.several());
    let roles: Vec<_> = app
        .select_choices()
        .into_iter()
        .map(|line| line.label)
        .collect();
    assert_eq!(roles, ["@Bot", "@Mods", "@Helpers"]);
    app.dispatch(Action::SearchSelect("MOD".into()));
    let effects = app.dispatch(Action::PickSelect(app.select_choices()[0].value.clone()));
    assert_eq!(chosen(&effects), Some(vec!["20".to_owned()]));
    assert_eq!(shown_text(&app), "@Mods");
}

#[test]
fn a_menu_of_channels_offers_the_servers_of_the_kinds_asked() {
    let mut picker = menu(SelectKind::Channel, 1, 1);
    picker.channel_types = vec![2];
    let mut app = showing(picker);
    open(&mut app);
    let channels: Vec<_> = app
        .select_choices()
        .into_iter()
        .map(|line| (line.value, line.label))
        .collect();
    assert_eq!(channels, [("14".to_owned(), "Lounge".to_owned())]);
}

#[test]
fn a_disabled_menu_and_another_channels_dont_open() {
    let mut app = showing(menu(SelectKind::Text, 1, 1));
    app.dispatch(Action::OpenSelect {
        item: "d:5".into(),
        custom_id: "other".into(),
    });
    assert!(app.state.select.is_none());
    open(&mut app);
    app.dispatch(Action::SelectRoom("discord:10:13".into()));
    assert!(app.state.select.is_none(), "it closes with its channel");
}
