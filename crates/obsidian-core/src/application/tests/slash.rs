//! Slash commands in the composer.

use super::*;
use obsidian_discord::app_commands::{self as app_commands_kind, CommandIndex};

const ROOM: &str = "discord:10:11";

/// `discord_server` with #general open and ready to write in.
fn writing() -> Application {
    let mut app = discord_server();
    app.dispatch(Action::Navigate(Navigation::Server("discord:10".into())));
    app.dispatch(Action::SelectRoom(ROOM.into()));
    app.apply(Event::Timeline {
        room: ROOM.into(),
        messages: timeline(vec![]),
    });
    app
}

fn index() -> Arc<CommandIndex> {
    Arc::new(CommandIndex::parse(&serde_json::json!({
        "applications": [{"id": "100", "name": "Helper", "icon": null}],
        "application_commands": [
            {"id": "1", "type": 1, "application_id": "100", "version": "7",
             "name": "remind", "description": "Remind me",
             "options": [
                {"type": 3, "name": "what", "description": "Of what", "required": true},
                {"type": 3, "name": "when", "description": "When",
                 "choices": [{"name": "Today", "value": "today"}, {"name": "Tomorrow", "value": "tomorrow"}]},
             ]},
            {"id": "2", "type": 1, "application_id": "100", "version": "8",
             "name": "ping", "description": "Pong"},
        ],
    })))
}

fn rows(app: &Application) -> Vec<String> {
    match &app.state.slash.shown {
        Some(SlashShown::Commands(rows)) => rows.iter().map(|row| row.name.clone()).collect(),
        _ => vec![],
    }
}

#[test]
fn a_slash_asks_for_the_rooms_commands_once_and_lists_them() {
    let mut app = writing();
    let effects = app.dispatch(Action::SetDraft("/".into()));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Backend(Command::Discord(DiscordCommand::Commands { room })) if room == ROOM
    )));
    // Discord's own show while the apps' load.
    assert_eq!(rows(&app)[0], "shrug");
    // Typing on while they load doesn't ask again.
    let effects = app.dispatch(Action::SetDraft("/r".into()));
    assert_eq!(app.state.slash.shown, Some(SlashShown::Loading));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::Backend(Command::Discord(DiscordCommand::Commands { .. }))
    )));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(index()),
    });
    assert_eq!(rows(&app), ["remind"]);
    app.dispatch(Action::SetDraft("/".into()));
    // The apps' commands, then Discord's own: /tts as this is a server.
    assert_eq!(
        rows(&app),
        [
            "remind",
            "ping",
            "shrug",
            "tableflip",
            "unflip",
            "tts",
            "me",
            "spoiler"
        ]
    );
    // Escape closes the list until the draft changes.
    app.dispatch(Action::DismissCommands);
    assert_eq!(app.state.slash.shown, None);
    app.dispatch(Action::SetDraft("/p".into()));
    assert_eq!(rows(&app), ["ping"]);
}

#[test]
fn a_command_written_shows_its_options_and_choices() {
    let mut app = writing();
    app.dispatch(Action::SetDraft("/".into()));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(index()),
    });
    app.dispatch(Action::SetDraft("/remind buy milk when:tom".into()));
    let Some(SlashShown::Options(bar)) = &app.state.slash.shown else {
        panic!("the options show: {:?}", app.state.slash.shown);
    };
    assert_eq!(bar.command, "/remind");
    assert_eq!(bar.app, "Helper");
    let chips: Vec<_> = bar
        .options
        .iter()
        .map(|chip| (chip.name.as_str(), chip.given, chip.current))
        .collect();
    assert_eq!(chips, [("what", true, false), ("when", true, true)]);
    assert_eq!(
        bar.choices,
        [(
            "Tomorrow".to_owned(),
            "/remind buy milk when:Tomorrow ".to_owned()
        )]
    );
}

#[test]
fn enter_runs_the_command_and_clears_the_composer() {
    let mut app = writing();
    app.dispatch(Action::SetDraft("/".into()));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(index()),
    });
    // A required option left out says so, and nothing goes.
    app.dispatch(Action::SetDraft("/remind ".into()));
    assert!(app.dispatch(Action::Send).is_empty());
    assert_eq!(
        app.state.notice.as_deref(),
        Some("Fill in the required option what.")
    );
    app.dispatch(Action::SetDraft("/remind buy milk when:Today".into()));
    let effects = app.dispatch(Action::Send);
    let ran = effects.iter().find_map(|effect| match effect {
        Effect::Backend(Command::Discord(DiscordCommand::RunCommand {
            room,
            application,
            data,
            ..
        })) if room == ROOM => Some((application.clone(), data.clone())),
        _ => None,
    });
    let (application, data) = ran.expect("the command runs");
    assert_eq!(application, "100");
    assert_eq!(data["name"], "remind");
    assert_eq!(
        data["options"],
        serde_json::json!([
            {"type": 3, "name": "what", "value": "buy milk"},
            {"type": 3, "name": "when", "value": "today"},
        ])
    );
    assert_eq!(app.state.draft(), "");
    assert_eq!(app.state.slash.shown, None);
}

#[test]
fn built_in_commands_change_the_message() {
    let mut app = writing();
    app.dispatch(Action::SetDraft("/shrug oh well".into()));
    let effects = app.dispatch(Action::Send);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Backend(Command::Send { body, .. }) if body == "oh well ¯\\_(ツ)_/¯"
    )));
}

#[test]
fn a_slash_word_no_app_knows_goes_as_text() {
    let mut app = writing();
    app.dispatch(Action::SetDraft("/".into()));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(index()),
    });
    app.dispatch(Action::SetDraft("/nothing here".into()));
    let effects = app.dispatch(Action::Send);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Backend(Command::Send { body, .. }) if body == "/nothing here"
    )));
}

#[test]
fn a_servers_commands_changing_asks_for_them_again() {
    let mut app = writing();
    app.dispatch(Action::SetDraft("/".into()));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(index()),
    });
    app.dispatch(Action::SetDraft(String::new()));
    app.apply(Event::CommandsChanged {
        server: "discord:10".into(),
    });
    let effects = app.dispatch(Action::SetDraft("/p".into()));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Backend(Command::Discord(DiscordCommand::Commands { .. }))
    )));
}

fn weather() -> Arc<CommandIndex> {
    Arc::new(CommandIndex::parse(&serde_json::json!({
        "applications": [{"id": "300", "name": "Weather", "icon": null}],
        "application_commands": [
            {"id": "5", "type": 1, "application_id": "300", "version": "9",
             "name": "weather", "description": "Weather",
             "options": [
                {"type": 3, "name": "city", "description": "City", "required": true, "autocomplete": true},
                {"type": 5, "name": "metric", "description": "In °C"},
             ]},
        ],
    })))
}

/// The nonce of the ask for suggestions among `effects`, if one went.
fn asked(effects: &[Effect]) -> Option<(String, serde_json::Value)> {
    effects.iter().find_map(|effect| match effect {
        Effect::Backend(Command::Discord(DiscordCommand::Suggest {
            nonce,
            data,
            application,
            ..
        })) if application == "300" => Some((nonce.clone(), data.clone())),
        _ => None,
    })
}

fn bar(app: &Application) -> crate::application::OptionsBar {
    match &app.state.slash.shown {
        Some(SlashShown::Options(bar)) => bar.clone(),
        shown => panic!("no options bar: {shown:?}"),
    }
}

#[test]
fn an_apps_suggestions_show_as_its_option_is_typed() {
    use crate::chat::Suggested;
    use obsidian_discord::app_commands::Choice;
    let mut app = writing();
    app.dispatch(Action::SetDraft("/".into()));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(weather()),
    });
    // Choosing the command asks for the first option's suggestions at once.
    let (nonce, data) =
        asked(&app.dispatch(Action::SetDraft("/weather ".into()))).expect("an ask for suggestions");
    assert_eq!(
        data["options"],
        serde_json::json!([{"type": 3, "name": "city", "value": "", "focused": true}])
    );
    assert_eq!(bar(&app).note.as_deref(), Some("Loading options..."));
    assert_eq!(bar(&app).choices_heading, "OPTIONS");
    app.apply(Event::Suggestions {
        nonce,
        answer: Suggested::Choices(vec![
            Choice {
                name: "Paris, France".into(),
                value: "fr-paris".into(),
            },
            Choice {
                name: "Lyon".into(),
                value: "fr-lyon".into(),
            },
        ]),
    });
    let shown = bar(&app);
    assert_eq!(shown.note, None);
    assert_eq!(
        shown.choices,
        [
            (
                "Paris, France".to_owned(),
                "/weather city:Paris, France ".to_owned()
            ),
            ("Lyon".to_owned(), "/weather city:Lyon ".to_owned()),
        ]
    );
    // Each text asks once; one asked before shows what came for it.
    let (nonce, _) = asked(&app.dispatch(Action::SetDraft("/weather Pa".into()))).unwrap();
    assert!(asked(&app.dispatch(Action::SetDraft("/weather P".into()))).is_some());
    assert!(asked(&app.dispatch(Action::SetDraft("/weather Pa".into()))).is_none());
    assert_eq!(bar(&app).choices_heading, "OPTIONS MATCHING PA");
    app.apply(Event::Suggestions {
        nonce,
        answer: Suggested::Failed,
    });
    assert_eq!(bar(&app).note.as_deref(), Some("Loading options failed"));
    // A dropped ask goes again when its text is typed again.
    let (nonce, _) = asked(&app.dispatch(Action::SetDraft("/weather Lyo".into()))).unwrap();
    app.dispatch(Action::SetDraft("/weather Ly".into()));
    app.apply(Event::Suggestions {
        nonce,
        answer: Suggested::Dropped,
    });
    assert!(asked(&app.dispatch(Action::SetDraft("/weather Lyo".into()))).is_some());
    // The suggestion chosen is done with: nothing more is asked or shown,
    // and it runs with its value.
    let effects = app.dispatch(Action::SetDraft("/weather city:Paris, France ".into()));
    assert!(asked(&effects).is_none());
    assert!(bar(&app).choices.is_empty());
    let effects = app.dispatch(Action::Send);
    let data = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Backend(Command::Discord(DiscordCommand::RunCommand { data, .. })) => {
                Some(data)
            }
            _ => None,
        })
        .expect("the command runs");
    assert_eq!(
        data["options"],
        serde_json::json!([{"type": 3, "name": "city", "value": "fr-paris"}])
    );
}

#[test]
fn a_true_or_false_option_offers_both() {
    let mut app = writing();
    app.dispatch(Action::SetDraft("/".into()));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(weather()),
    });
    app.dispatch(Action::SetDraft("/weather city:Paris metric:t".into()));
    assert_eq!(
        bar(&app).choices,
        [(
            "True".to_owned(),
            "/weather city:Paris metric:True ".to_owned()
        )]
    );
}

fn feedback_modal() -> Arc<obsidian_discord::modal::Modal> {
    obsidian_discord::modal::Modal::parse(&serde_json::json!({
        "id": "900", "nonce": "77", "channel_id": "11", "custom_id": "feedback",
        "title": "Feedback",
        "application": {"id": "100", "name": "Helper"},
        "components": [
            {"type": 18, "label": "Your idea", "component":
                {"type": 4, "custom_id": "idea", "style": 2, "required": true, "min_length": 5}},
            {"type": 18, "label": "Area", "component": {"type": 3, "custom_id": "area",
                "options": [{"label": "Voice", "value": "voice"}, {"label": "Chat", "value": "chat"}]}},
        ],
    }))
    .unwrap()
}

#[test]
fn an_apps_modal_is_filled_in_checked_and_sent() {
    use obsidian_discord::modal::Answer;
    let mut app = writing();
    app.apply(Event::Modal {
        room: ROOM.into(),
        modal: feedback_modal(),
    });
    assert_eq!(app.state.modal.as_ref().unwrap().modal.title, "Feedback");
    // What isn't filled in as it may be says so, and nothing goes.
    app.dispatch(Action::ModalText(0, "Hi".into()));
    assert!(app.dispatch(Action::SubmitModal).is_empty());
    let form = app.state.modal.as_ref().unwrap();
    assert_eq!(
        form.problems,
        [
            Some("Must be between 5 and 4000 in length.".to_owned()),
            Some("This field is required.".to_owned()),
        ]
    );
    app.dispatch(Action::ModalText(0, "Push to talk on mouse buttons".into()));
    app.dispatch(Action::ModalMenu(Some(1)));
    app.dispatch(Action::ModalPick(1, 1));
    let form = app.state.modal.as_ref().unwrap();
    assert_eq!(form.answers[1], Answer::Chosen(vec!["chat".into()]));
    assert_eq!(form.open, None, "picking closes a menu of one");
    let effects = app.dispatch(Action::SubmitModal);
    let (nonce, data) = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Backend(Command::Discord(DiscordCommand::SubmitModal {
                room,
                application,
                data,
                nonce,
                ..
            })) if room == ROOM && application == "100" => Some((nonce.clone(), data.clone())),
            _ => None,
        })
        .expect("the modal is submitted");
    assert_eq!(data["custom_id"], "feedback");
    assert_eq!(
        data["components"][1]["component"]["values"],
        serde_json::json!(["chat"])
    );
    // The app not taking it says so, and it can go again.
    app.apply(Event::ModalAnswered {
        nonce,
        taken: false,
    });
    let form = app.state.modal.as_ref().unwrap();
    assert_eq!(
        form.error.as_deref(),
        Some("Something went wrong. Try again.")
    );
    let nonce = app
        .dispatch(Action::SubmitModal)
        .iter()
        .find_map(|effect| match effect {
            Effect::Backend(Command::Discord(DiscordCommand::SubmitModal { nonce, .. })) => {
                Some(nonce.clone())
            }
            _ => None,
        })
        .unwrap();
    // A late failure of the first doesn't touch the second.
    app.apply(Event::ModalAnswered {
        nonce: "1".into(),
        taken: false,
    });
    assert!(app.state.modal.as_ref().unwrap().sending.is_some());
    app.apply(Event::ModalAnswered { nonce, taken: true });
    assert!(app.state.modal.is_none());
}

#[test]
fn an_apps_menu_lists_the_menu_commands_and_runs_one_on_its_target() {
    let mut app = writing();
    let menu_index = Arc::new(CommandIndex::parse(&serde_json::json!({
        "applications": [
            {"id": "100", "name": "Helper", "icon": null},
            {"id": "200", "name": "Archive", "icon": "abc"},
        ],
        "application_commands": [
            {"id": "1", "type": 1, "application_id": "100", "version": "7",
             "name": "ping", "description": "Pong"},
            {"id": "5", "type": 3, "application_id": "100", "version": "9",
             "name": "Translate"},
            {"id": "6", "type": 3, "application_id": "200", "version": "10",
             "name": "Save"},
            {"id": "7", "type": 2, "application_id": "100", "version": "11",
             "name": "Wave"},
        ],
    })));
    assert_eq!(
        app.apps_menu(ROOM, app_commands_kind::MESSAGE),
        AppsMenu::Loading
    );
    let effects = app.dispatch(Action::LoadCommands(ROOM.into()));
    // With the counts Frequently Used Commands ranks by, as the places'.
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::Backend(Command::Discord(DiscordCommand::PlacesUsed)),
            Effect::Backend(Command::Discord(DiscordCommand::Commands { room })),
        ] if room == ROOM
    ));
    // Opening it again while they load doesn't ask again.
    assert!(app.dispatch(Action::LoadCommands(ROOM.into())).is_empty());
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(menu_index),
    });
    let AppsMenu::Commands(commands) = app.apps_menu(ROOM, app_commands_kind::MESSAGE) else {
        panic!("the commands came");
    };
    // By app, then by name.
    let names: Vec<_> = commands
        .iter()
        .map(|command| (command.app.as_str(), command.name.as_str()))
        .collect();
    assert_eq!(names, [("Archive", "Save"), ("Helper", "Translate")]);
    assert!(commands[0].icon.is_some());
    // Laid out as Discord's: each app with its commands, and a search over
    // commands and apps alike.
    let layout = app
        .apps_layout(ROOM, app_commands_kind::MESSAGE, "")
        .unwrap();
    assert!(layout.frequent.is_empty() && layout.found.is_none());
    let apps: Vec<_> = layout
        .apps
        .iter()
        .map(|(app, _, commands)| (app.as_str(), commands.len()))
        .collect();
    assert_eq!(apps, [("Archive", 1), ("Helper", 1)]);
    let found = app
        .apps_layout(ROOM, app_commands_kind::MESSAGE, "help")
        .unwrap()
        .found
        .unwrap();
    assert_eq!(found[0].name, "Translate", "found by its app's name");
    // Discord's counts, by command id, rank Frequently Used.
    app.apply(Event::CommandsUsed(vec![(
        "6".into(),
        chat::PlaceUses {
            total: 3,
            recent: vec![crate::clock::now_ms()],
        },
    )]));
    let layout = app
        .apps_layout(ROOM, app_commands_kind::MESSAGE, "")
        .unwrap();
    assert_eq!(layout.frequent.len(), 1);
    assert_eq!(layout.frequent[0].name, "Save");
    let AppsMenu::Commands(user) = app.apps_menu(ROOM, app_commands_kind::USER) else {
        panic!("the commands came");
    };
    assert_eq!(user.len(), 1);
    assert_eq!(
        app.message_apps_target("d:123"),
        Some((ROOM.to_owned(), "123".to_owned()))
    );
    let effects = app.dispatch(Action::RunMenuCommand {
        room: ROOM.into(),
        command: "5".into(),
        target: "123".into(),
    });
    let [
        Effect::Backend(Command::Discord(DiscordCommand::RunCommand {
            room,
            application,
            data,
            ..
        })),
    ] = effects.as_slice()
    else {
        panic!("the command runs");
    };
    assert_eq!(room, ROOM);
    assert_eq!(application, "100");
    assert_eq!(data["type"], 3);
    assert_eq!(data["target_id"], "123");
    // Running it counts for Frequently Used, as Discord's client counts it.
    let layout = app
        .apps_layout(ROOM, app_commands_kind::MESSAGE, "")
        .unwrap();
    assert!(
        layout
            .frequent
            .iter()
            .any(|command| command.name == "Translate")
    );
    // A person's menu in the channel has Apps; a Matrix person's doesn't.
    assert!(app.person_choices("42").contains(&MenuChoice::Apps));
    assert!(
        !app.person_choices("@bob:example.org")
            .contains(&MenuChoice::Apps)
    );
}

#[test]
fn a_file_option_takes_a_picked_file_which_goes_with_the_command() {
    let mut app = writing();
    app.dispatch(Action::SetDraft("/".into()));
    app.apply(Event::Commands {
        room: ROOM.into(),
        index: Ok(Arc::new(CommandIndex::parse(&serde_json::json!({
            "applications": [{"id": "100", "name": "Helper", "icon": null}],
            "application_commands": [
                {"id": "3", "type": 1, "application_id": "100", "version": "9",
                 "name": "upscale", "description": "Upscale a picture",
                 "options": [
                    {"type": 11, "name": "image", "description": "The picture", "required": true},
                    {"type": 4, "name": "times", "description": "How much"},
                 ]},
            ],
        })))),
    });
    app.dispatch(Action::SetDraft("/upscale image:".into()));
    let Some(SlashShown::Options(bar)) = &app.state.slash.shown else {
        panic!("the options show");
    };
    assert!(bar.takes_file, "Choose File shows for a file option");
    // Typed without a file picked, it says so.
    app.dispatch(Action::SetDraft("/upscale image:cat.png".into()));
    assert!(app.dispatch(Action::Send).is_empty());
    assert_eq!(
        app.state.notice.as_deref(),
        Some("Choose a file for image.")
    );
    app.dispatch(Action::SetDraft("/upscale image:".into()));
    let text = app
        .attach_command_file("/home/me/my cat.png".into())
        .expect("the composer names the file");
    assert_eq!(text, "/upscale image:my_cat.png ");
    app.dispatch(Action::SetDraft(format!("{text}times:2")));
    let effects = app.dispatch(Action::Send);
    let Some((data, files)) = effects.iter().find_map(|effect| match effect {
        Effect::Backend(Command::Discord(DiscordCommand::RunCommand { data, files, .. })) => {
            Some((data.clone(), files.clone()))
        }
        _ => None,
    }) else {
        panic!("the command runs: {:?}", app.state.notice);
    };
    assert_eq!(files, [std::path::PathBuf::from("/home/me/my cat.png")]);
    // The file's value is its place among the files, as Discord's client sends it.
    assert_eq!(data["options"][0]["type"], 11);
    assert_eq!(data["options"][0]["value"], 0);
    assert_eq!(data["options"][1]["value"], 2);
}

#[test]
fn a_modals_channel_menu_lists_the_servers_channels_and_its_files_go_with_it() {
    use obsidian_discord::modal::Answer;
    let mut app = writing();
    app.apply(Event::Modal {
        room: ROOM.into(),
        modal: obsidian_discord::modal::Modal::parse(&serde_json::json!({
            "id": "901", "nonce": "78", "channel_id": "11", "custom_id": "report",
            "application": {"id": "100", "name": "Helper"},
            "components": [
                {"type": 18, "label": "Where", "component": {"type": 8, "custom_id": "where"}},
                {"type": 18, "label": "Screenshot", "component": {"type": 19, "custom_id": "shot"}},
            ],
        }))
        .unwrap(),
    });
    let listed = app.modal_entities(0);
    assert!(
        listed.iter().any(|choice| choice.label == "general"),
        "{listed:?}"
    );
    let at = listed
        .iter()
        .position(|choice| choice.label == "general")
        .unwrap();
    app.dispatch(Action::ModalPick(0, at));
    app.dispatch(Action::ModalFiles(1, vec!["/tmp/shot.png".into()]));
    let form = app.state.modal.as_ref().unwrap();
    assert_eq!(form.answers[0], Answer::Chosen(vec!["11".into()]));
    let (data, files) = app
        .dispatch(Action::SubmitModal)
        .iter()
        .find_map(|effect| match effect {
            Effect::Backend(Command::Discord(DiscordCommand::SubmitModal {
                data, files, ..
            })) => Some((data.clone(), files.clone())),
            _ => None,
        })
        .expect("it goes");
    assert_eq!(
        data["components"][0]["component"]["values"],
        serde_json::json!(["11"])
    );
    assert_eq!(
        data["components"][1]["component"]["values"],
        serde_json::json!([0])
    );
    assert_eq!(files, [std::path::PathBuf::from("/tmp/shot.png")]);
}
