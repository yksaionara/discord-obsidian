//! Apps' modals: the form an app opens in answer to a slash command or a
//! button, filled in and submitted as in Discord's client.

use obsidian_discord::modal::{Answer, Field, Modal};

use super::*;

/// What Discord's form says when the app didn't take it.
const FAILED: &str = "Something went wrong. Try again.";

/// A modal open over the window.
#[derive(Clone, Debug)]
pub struct ModalForm {
    /// The room whose interaction opened it.
    pub room: String,
    pub modal: Arc<Modal>,
    /// What each field holds.
    pub answers: Vec<Answer>,
    /// Why each field can't go, after a submission tried.
    pub problems: Vec<Option<String>>,
    /// Why the submission failed.
    pub error: Option<String>,
    /// The nonce of the submission on its way.
    pub sending: Option<String>,
    /// The select menu open, by its field.
    pub open: Option<usize>,
}

impl Application {
    /// An app opened a form, which replaces any open one, as in Discord.
    pub(super) fn modal_opened(&mut self, room: String, modal: Arc<Modal>) {
        self.state.modal = Some(ModalForm {
            room,
            answers: modal.answers(),
            problems: vec![None; modal.items.len()],
            modal,
            error: None,
            sending: None,
            open: None,
        });
    }

    pub(super) fn modal_text(&mut self, index: usize, text: String) -> Vec<Effect> {
        if let Some(form) = &mut self.state.modal
            && let Some(answer @ Answer::Text(_)) = form.answers.get_mut(index)
        {
            *answer = Answer::Text(text);
            form.problems[index] = None;
        }
        vec![]
    }

    /// Picks a field's option: a select menu or radio group's one, or one
    /// more or fewer of a group's; option 0 of a checkbox ticks it.
    pub(super) fn modal_pick(&mut self, index: usize, option: usize) -> Vec<Effect> {
        let Some(form) = &mut self.state.modal else {
            return vec![];
        };
        if form.sending.is_some() {
            return vec![];
        }
        match (
            form.modal.items.get(index).map(|item| &item.field),
            form.answers.get_mut(index),
        ) {
            (Some(Field::Choice { options, max, .. }), Some(Answer::Chosen(chosen))) => {
                let Some(value) = options.get(option).map(|option| option.value.clone()) else {
                    return vec![];
                };
                if *max <= 1 {
                    *chosen = vec![value];
                    form.open = None;
                } else if let Some(at) = chosen.iter().position(|known| *known == value) {
                    chosen.remove(at);
                } else if chosen.len() < *max {
                    // In the options' order, as Discord's menu lists them.
                    chosen.push(value);
                    chosen.sort_by_key(|known| {
                        options.iter().position(|option| option.value == *known)
                    });
                }
                form.problems[index] = None;
            }
            (Some(Field::Checkbox { .. }), Some(Answer::Checked(checked))) => *checked = !*checked,
            _ => {}
        }
        vec![]
    }

    /// Opens a select menu's options, or closes them.
    pub(super) fn modal_menu(&mut self, index: Option<usize>) -> Vec<Effect> {
        if let Some(form) = &mut self.state.modal {
            form.open = if form.open == index { None } else { index };
        }
        vec![]
    }

    /// Sends the form, once every field holds what it may.
    pub(super) fn submit_modal(&mut self) -> Vec<Effect> {
        let Some(form) = &mut self.state.modal else {
            return vec![];
        };
        if form.sending.is_some() {
            return vec![];
        }
        form.problems = form.modal.problems(&form.answers);
        form.open = None;
        if form.problems.iter().any(Option::is_some) {
            return vec![];
        }
        let nonce = obsidian_discord::message::nonce();
        form.error = None;
        form.sending = Some(nonce.clone());
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::SubmitModal {
                room: form.room.clone(),
                application: form.modal.app_id.clone(),
                data: form.modal.data(&form.answers),
                nonce,
            },
        ))]
    }

    pub(super) fn close_modal(&mut self) -> Vec<Effect> {
        self.state.modal = None;
        vec![]
    }

    /// What became of a submission: taken, it closes; otherwise it says so
    /// and can be sent again.
    pub(super) fn modal_answered(&mut self, nonce: &str, taken: bool) {
        let Some(form) = &mut self.state.modal else {
            return;
        };
        if form.sending.as_deref() != Some(nonce) {
            return;
        }
        if taken {
            self.state.modal = None;
        } else {
            form.sending = None;
            form.error = Some(FAILED.into());
        }
    }
}
