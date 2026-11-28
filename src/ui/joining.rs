//! What a Discord server just joined asks of its new member, over the
//! window: its onboarding questions and its membership form; see the core's
//! `application/joining.rs`.

use obsidian_core::application::{JoinStep, Joining};
use obsidian_core::obsidian_discord::joining::{FieldKind, FormAnswer};

use super::*;

impl Adapter {
    /// The joining dialog, while a server asks something of its new member.
    /// It's made again only when more than what's typed changed, so a text
    /// field being typed in keeps its place and the keys.
    pub(super) fn show_joining(&mut self, window: &ObsidianWindow) {
        let Some(joining) = &self.app.state.joining else {
            self.joining_shown = None;
            if window.get_joining_open() {
                window.set_joining_open(false);
            }
            return;
        };
        let key = shape(joining);
        if self.joining_shown.as_ref() != Some(&key) {
            window.set_joining_prompt(prompt(joining));
            self.joining_shown = Some(key);
        }
        window.set_joining_open(true);
    }
}

/// What the dialog shows but what's typed in its text fields.
fn shape(joining: &Joining) -> String {
    let mut shown = joining.clone();
    if let JoinStep::Form { answers, .. } = &mut shown.step {
        for answer in answers {
            if let FormAnswer::Text(text) = answer {
                text.clear();
            }
        }
    }
    format!("{shown:?}")
}

fn prompt(joining: &Joining) -> JoiningPrompt {
    let mut prompt = JoiningPrompt {
        name: joining.name.as_str().into(),
        error: joining.error.as_deref().unwrap_or_default().into(),
        sending: joining.sending,
        ..Default::default()
    };
    match &joining.step {
        JoinStep::Asking | JoinStep::AskingForm => prompt.kind = 0,
        JoinStep::Question {
            onboarding,
            at,
            chosen,
        } => {
            prompt.kind = 1;
            let Some(question) = onboarding.prompts.get(*at) else {
                return prompt;
            };
            prompt.title = question.title.as_str().into();
            prompt.hint = if question.single {
                "Choose one:"
            } else {
                "Choose all that apply."
            }
            .into();
            let answered = question
                .options
                .iter()
                .any(|option| chosen.contains(&option.id));
            let last = *at + 1 == onboarding.prompts.len();
            prompt.next = if last {
                "Finish"
            } else if answered || question.required {
                "Next"
            } else {
                "Skip"
            }
            .into();
            prompt.can_back = *at > 0;
            if onboarding.prompts.len() > 1 {
                prompt.progress = format!("{} of {}", at + 1, onboarding.prompts.len()).into();
            }
            let options: Vec<JoinOption> = question
                .options
                .iter()
                .map(|option| {
                    let (emoji, emoji_url) = match &option.emoji {
                        Some((Some(id), _)) => (
                            String::new(),
                            obsidian_core::obsidian_discord::ready::custom_emoji(id, false, 48),
                        ),
                        Some((None, name)) => (name.clone(), String::new()),
                        None => (String::new(), String::new()),
                    };
                    JoinOption {
                        id: option.id.as_str().into(),
                        title: option.title.as_str().into(),
                        description: option.description.as_str().into(),
                        emoji: emoji.into(),
                        emoji_url: emoji_url.into(),
                        chosen: chosen.contains(&option.id),
                    }
                })
                .collect();
            prompt.options = ModelRc::new(VecModel::from(options));
        }
        JoinStep::Form { form, answers } => {
            prompt.kind = 2;
            prompt.description = form.description.as_str().into();
            let fields: Vec<JoinField> = form
                .fields
                .iter()
                .zip(answers)
                .map(|(field, answer)| {
                    let mut shown = JoinField {
                        label: field.label.as_str().into(),
                        description: field.description.as_str().into(),
                        required: field.required,
                        placeholder: field.placeholder.as_str().into(),
                        chosen: -1,
                        ..Default::default()
                    };
                    let strings = |list: &[String]| {
                        ModelRc::new(VecModel::from(
                            list.iter()
                                .map(|item| SharedString::from(item.as_str()))
                                .collect::<Vec<_>>(),
                        ))
                    };
                    match &field.kind {
                        FieldKind::Terms(rules) => {
                            shown.kind = 0;
                            shown.rules = strings(rules);
                        }
                        FieldKind::Text { paragraph } => {
                            shown.kind = if *paragraph { 2 } else { 1 }
                        }
                        FieldKind::Choice(choices) => {
                            shown.kind = 3;
                            shown.choices = strings(choices);
                        }
                        FieldKind::Other => shown.kind = 4,
                    }
                    match answer {
                        FormAnswer::Agreed(agreed) => shown.agreed = *agreed,
                        FormAnswer::Text(text) => shown.value = text.as_str().into(),
                        FormAnswer::Chosen(Some(at)) => shown.chosen = *at as i32,
                        FormAnswer::Chosen(None) | FormAnswer::None => {}
                    }
                    shown
                })
                .collect();
            prompt.fields = ModelRc::new(VecModel::from(fields));
        }
        JoinStep::Pending => prompt.kind = 3,
    }
    prompt
}

pub(super) fn bind(window: &ObsidianWindow, adapter: &Rc<RefCell<Adapter>>) {
    macro_rules! act {
        ($callback:ident($($arg:ident),*) => $action:expr) => {{
            let state = adapter.clone();
            let weak = window.as_weak();
            window.$callback(move |$($arg),*| {
                if let Some(window) = weak.upgrade() {
                    state.borrow_mut().action(&window, $action);
                }
            });
        }};
    }
    let field = |at: i32| usize::try_from(at).unwrap_or(usize::MAX);
    act!(on_joining_pick(id) => Action::PickOnboarding(id.into()));
    act!(on_joining_next(back) => Action::NextOnboarding(back));
    act!(on_joining_agree(at, agreed) => Action::AnswerMembership(field(at), FormAnswer::Agreed(agreed)));
    act!(on_joining_text(at, text) => Action::AnswerMembership(field(at), FormAnswer::Text(text.into())));
    act!(on_joining_choose(at, choice) => Action::AnswerMembership(field(at), FormAnswer::Chosen(usize::try_from(choice).ok())));
    act!(on_joining_submit() => Action::SubmitMembership);
    act!(on_joining_close() => Action::FinishJoining);
}
