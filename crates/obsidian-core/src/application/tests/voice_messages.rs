//! Voice messages: playing them, recording one in place of the composer,
//! and where they may be forwarded.

use super::*;
use crate::voice_message::{Recorded, Recording};

const ROOM: &str = "discord:10:11";

/// `discord_server` with #general open and ready to write in.
fn writing() -> Application {
    showing(vec![])
}

fn showing(messages: Vec<ChatMessage>) -> Application {
    let mut app = discord_server();
    app.dispatch(Action::Navigate(Navigation::Server("discord:10".into())));
    app.dispatch(Action::SelectRoom(ROOM.into()));
    app.apply(Event::Timeline {
        room: ROOM.into(),
        messages: timeline(messages),
    });
    app
}

/// Bob's voice message, four seconds long.
fn voice_message(item: &str) -> ChatMessage {
    ChatMessage {
        body: String::new(),
        can_forward: true,
        attachments: vec![chat::Attachment {
            name: "voice-message.ogg".into(),
            size: Some(16_000),
            image: false,
            dimensions: None,
            preview: None,
            preview_failed: false,
            placement: chat::Placement::Message,
            spoiler: false,
            video: false,
            gif: None,
            url: None,
            voice: Some(chat::VoiceClip {
                duration_ms: 4_000,
                waveform: Arc::from([0u8, 255]),
            }),
        }],
        ..message(item, false)
    }
}

fn recording(app: &Application) -> Arc<Recording> {
    app.state
        .voice_recording
        .as_ref()
        .expect("recording")
        .recording
        .clone()
}

#[test]
fn recording_starts_in_the_open_channel_from_the_chosen_microphone() {
    let mut app = writing();
    app.state.voice.microphone = Some("mic-2".into());
    let effects = app.dispatch(Action::RecordVoiceMessage);
    let shown = recording(&app);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Backend(Command::RecordVoiceMessage { room, microphone, recording })
            if room == ROOM && microphone.as_deref() == Some("mic-2")
                && Arc::ptr_eq(recording, &shown)
    )));
    assert_eq!(app.state.voice_recording.as_ref().unwrap().room, ROOM);

    let effects = app.dispatch(Action::StopVoiceRecording);
    assert!(matches!(
        effects.as_slice(),
        [Effect::Backend(Command::StopVoiceRecording)]
    ));
    // It waits, stopped, to be sent.
    assert!(app.state.voice_recording.is_some());

    let effects = app.dispatch(Action::SendVoiceRecording);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Backend(Command::SendVoiceRecording)))
    );
    assert!(app.state.voice_recording.is_none(), "the composer is back");
}

#[test]
fn a_recording_goes_with_its_channel_and_can_be_thrown_away() {
    let mut app = writing();
    app.dispatch(Action::RecordVoiceMessage);
    app.dispatch(Action::SelectRoom("discord:10:13".into()));
    assert!(app.state.voice_recording.is_none());

    app.dispatch(Action::SelectRoom(ROOM.into()));
    app.dispatch(Action::RecordVoiceMessage);
    let effects = app.dispatch(Action::DiscardVoiceRecording);
    assert!(matches!(
        effects.as_slice(),
        [Effect::Backend(Command::DiscardVoiceRecording)]
    ));
    assert!(app.state.voice_recording.is_none());
}

#[test]
fn a_microphone_that_wont_open_says_so_as_discord_does() {
    let mut app = writing();
    app.dispatch(Action::RecordVoiceMessage);
    let first = recording(&app);
    app.dispatch(Action::RecordVoiceMessage);
    // The first one's failure is old news.
    app.apply(Event::VoiceRecordingFailed(first));
    assert!(app.state.voice_recording.is_some());
    assert_eq!(app.state.notice, None);

    app.apply(Event::VoiceRecordingFailed(recording(&app)));
    assert!(app.state.voice_recording.is_none());
    assert_eq!(
        app.state.notice.as_deref(),
        Some("Failed to start recording")
    );
}

#[test]
fn a_stopped_recording_plays_before_it_goes_and_its_place_is_forgotten() {
    let mut app = writing();
    app.dispatch(Action::RecordVoiceMessage);
    let item = chat::VOICE_RECORDING_ITEM.to_owned();
    // Still recording: nothing to play.
    assert!(
        app.dispatch(Action::ToggleVoiceMessage {
            item: item.clone(),
            index: 0
        })
        .is_empty()
    );
    recording(&app).finish_with(Recorded {
        path: "voice.ogg".into(),
        duration_ms: 4_000,
        waveform: Arc::from([10u8, 200]),
    });
    let effects = app.dispatch(Action::ToggleVoiceMessage {
        item: item.clone(),
        index: 0,
    });
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Backend(Command::PlayVoiceRecording { .. })))
    );
    assert_eq!(app.state.voice_message.as_ref().unwrap().item, item);

    // Half way through, waiting there paused.
    app.dispatch(Action::ToggleVoiceMessage {
        item: item.clone(),
        index: 0,
    });
    app.dispatch(Action::SeekVoiceMessage {
        item: item.clone(),
        index: 0,
        fraction: 0.5,
    });
    assert_eq!(
        app.state
            .voice_message
            .as_ref()
            .unwrap()
            .clock
            .position_ms(),
        2_000
    );
    app.dispatch(Action::SendVoiceRecording);
    assert!(app.state.voice_message.is_none());
    assert!(!app.state.voice_positions.contains_key(&item));
}

#[test]
fn the_preview_doesnt_record() {
    let mut app = writing();
    app.state.preview = true;
    assert!(app.dispatch(Action::RecordVoiceMessage).is_empty());
    assert!(app.state.voice_recording.is_none());
}

#[test]
fn a_voice_message_plays_from_where_it_stopped() {
    let mut app = showing(vec![voice_message("d:5")]);
    app.state.voice.speaker = Some("speakers-1".into());
    let effects = app.dispatch(Action::ToggleVoiceMessage {
        item: "d:5".into(),
        index: 0,
    });
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Backend(Command::PlayVoiceMessage { room, item, index: 0, speakers, .. })
            if room == ROOM && item == "d:5" && speakers.as_deref() == Some("speakers-1")
    )));
    let clock = app.state.voice_message.as_ref().unwrap().clock.clone();
    assert!(clock.playing());
    // Pressed again, it pauses where it is.
    assert!(
        app.dispatch(Action::ToggleVoiceMessage {
            item: "d:5".into(),
            index: 0,
        })
        .is_empty()
    );
    assert!(!clock.playing());
    // Moved to three quarters, then stopped with its channel: it plays on
    // from there.
    app.dispatch(Action::SeekVoiceMessage {
        item: "d:5".into(),
        index: 0,
        fraction: 0.75,
    });
    app.apply(Event::VoiceMessage(None));
    assert!(app.state.voice_message.is_none());
    app.dispatch(Action::ToggleVoiceMessage {
        item: "d:5".into(),
        index: 0,
    });
    assert_eq!(
        app.state
            .voice_message
            .as_ref()
            .unwrap()
            .clock
            .position_ms(),
        3_000
    );
}

#[test]
fn speed_and_mute_carry_to_the_next_voice_message() {
    let mut app = showing(vec![voice_message("d:5"), voice_message("d:6")]);
    app.dispatch(Action::ToggleVoiceMessage {
        item: "d:5".into(),
        index: 0,
    });
    app.dispatch(Action::VoiceMessageRate);
    app.dispatch(Action::MuteVoiceMessages);
    let clock = app.state.voice_message.as_ref().unwrap().clock.clone();
    assert_eq!(clock.rate(), 1.5);
    app.dispatch(Action::ToggleVoiceMessage {
        item: "d:6".into(),
        index: 0,
    });
    let next = &app.state.voice_message.as_ref().unwrap().clock;
    assert_eq!(app.state.voice_message.as_ref().unwrap().item, "d:6");
    assert_eq!(next.rate(), 1.5);
}

#[test]
fn the_volume_slider_sets_how_loud_voice_messages_play() {
    let mut app = showing(vec![voice_message("d:5"), voice_message("d:6")]);
    app.dispatch(Action::ToggleVoiceMessage {
        item: "d:5".into(),
        index: 0,
    });
    let clock = app.state.voice_message.as_ref().unwrap().clock.clone();
    assert_eq!(clock.gain(), 1.0, "full until the slider moves");
    app.dispatch(Action::VoiceMessageVolume(0.5));
    let half = 0.5f32.powf(2.8);
    assert_eq!(
        clock.gain(),
        half,
        "Discord's curve, travel to the power 2.8"
    );
    app.dispatch(Action::VoiceMessageVolume(0.0));
    assert!(app.state.voice_muted, "the bottom of the slider mutes");
    assert_eq!(clock.gain(), 0.0);
    app.dispatch(Action::MuteVoiceMessages);
    assert_eq!(clock.gain(), half, "unmuting brings back the level it had");
    app.dispatch(Action::ToggleVoiceMessage {
        item: "d:6".into(),
        index: 0,
    });
    let next = &app.state.voice_message.as_ref().unwrap().clock;
    assert_eq!(next.gain(), half, "the level stays for the next one");
}

#[test]
fn a_voice_message_forwards_only_where_voice_messages_may_go() {
    let mut app = showing(vec![voice_message("d:5")]);
    app.dispatch(Action::StartForward("d:5".into()));
    // @everyone may write in #memes, but not send voice messages there.
    app.dispatch(Action::ChooseForward("discord:10:13".into()));
    let effects = app.dispatch(Action::Forward(String::new()));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Backend(Command::Forward { .. })))
    );
    let forwarding = app.state.forwarding.as_ref().expect("the dialog stays");
    assert_eq!(
        forwarding.error.as_deref(),
        Some("Voice messages are disabled in this channel.")
    );
}
