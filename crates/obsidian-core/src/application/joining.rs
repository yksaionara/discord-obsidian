//! What a Discord server asks of someone who just joined it, as Discord's
//! client asks: its onboarding questions, one at a time, then its
//! membership form, such as agreeing to its rules. The invite's channel
//! opens once both are done, or at once when the server asks neither.

use obsidian_discord::joining::{FieldKind, FormAnswer, Onboarding, Verification};

use super::*;

/// What the window shows for a server just joined.
#[derive(Clone, Debug, PartialEq)]
pub struct Joining {
    /// The server, by its server id, and the invite's channel, by its room
    /// id, which opens once it's done.
    pub server: String,
    pub room: String,
    /// The server's name, as its invite showed it.
    pub name: String,
    pub invite: Option<String>,
    /// The server has a membership form, as joining said.
    pub form: bool,
    pub step: JoinStep,
    /// Why the last step didn't go.
    pub error: Option<String>,
    pub sending: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JoinStep {
    /// Its onboarding questions are on their way.
    Asking,
    /// One of its questions, by place, with the options chosen so far, by
    /// id, across all of them.
    Question {
        onboarding: Arc<Onboarding>,
        at: usize,
        chosen: Vec<String>,
    },
    /// Its membership form is on its way.
    AskingForm,
    Form {
        form: Arc<Verification>,
        answers: Vec<FormAnswer>,
    },
    /// The form went to the server's moderators, who review it.
    Pending,
}

impl Application {
    /// A server was joined: what it asks comes first, then the invite's
    /// channel opens.
    pub(super) fn start_joining(
        &mut self,
        joined: chat::JoinedServer,
        name: String,
        invite: Option<String>,
    ) -> Vec<Effect> {
        let server = joined.server.clone();
        self.state.joining = Some(Joining {
            server: server.clone(),
            room: joined.room,
            name,
            invite,
            form: joined.form,
            step: JoinStep::Asking,
            error: None,
            sending: false,
        });
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::Onboarding { server },
        ))]
    }

    /// The onboarding questions came: the first shows, or with none to
    /// ask, the form or the channel follows.
    pub(super) fn onboarding_came(
        &mut self,
        server: &str,
        result: Result<Arc<Onboarding>, String>,
    ) -> Vec<Effect> {
        let Some(joining) = self.joining_for(server) else {
            return vec![];
        };
        match result {
            Ok(onboarding) if onboarding.asks() => {
                joining.step = JoinStep::Question {
                    onboarding,
                    at: 0,
                    chosen: Vec::new(),
                };
                vec![]
            }
            // A server without onboarding, or one Discord wouldn't say, asks
            // nothing more of it.
            _ => self.after_onboarding(),
        }
    }

    /// An option of the question showing picked: the only one of a question
    /// of one, or one more or fewer.
    pub(super) fn pick_onboarding(&mut self, option: String) -> Vec<Effect> {
        let Some(Joining {
            step:
                JoinStep::Question {
                    onboarding,
                    at,
                    chosen,
                },
            sending: false,
            error,
            ..
        }) = &mut self.state.joining
        else {
            return vec![];
        };
        let Some(prompt) = onboarding.prompts.get(*at) else {
            return vec![];
        };
        if !prompt.options.iter().any(|known| known.id == option) {
            return vec![];
        }
        if let Some(place) = chosen.iter().position(|known| *known == option) {
            chosen.remove(place);
        } else {
            if prompt.single {
                chosen.retain(|known| !prompt.options.iter().any(|other| other.id == *known));
            }
            chosen.push(option);
        }
        *error = None;
        vec![]
    }

    /// Next, Skip or Finish: a required question wants an answer first;
    /// after the last, the answers go.
    pub(super) fn next_onboarding(&mut self, back: bool) -> Vec<Effect> {
        let Some(joining) = &mut self.state.joining else {
            return vec![];
        };
        let JoinStep::Question {
            onboarding,
            at,
            chosen,
        } = &mut joining.step
        else {
            return vec![];
        };
        if back {
            *at = at.saturating_sub(1);
            joining.error = None;
            return vec![];
        }
        let Some(prompt) = onboarding.prompts.get(*at) else {
            return vec![];
        };
        let answered = prompt
            .options
            .iter()
            .any(|option| chosen.contains(&option.id));
        if prompt.required && !answered {
            joining.error = Some("This question is required.".into());
            return vec![];
        }
        joining.error = None;
        if *at + 1 < onboarding.prompts.len() {
            *at += 1;
            return vec![];
        }
        joining.sending = true;
        let body = onboarding.responses(chosen, crate::clock::now_ms());
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::AnswerOnboarding {
                server: joining.server.clone(),
                body,
            },
        ))]
    }

    pub(super) fn onboarding_answered(
        &mut self,
        server: &str,
        result: Result<(), String>,
    ) -> Vec<Effect> {
        let Some(joining) = self.joining_for(server) else {
            return vec![];
        };
        joining.sending = false;
        if let Err(error) = result {
            joining.error = Some(format!("Couldn't save your answers: {error}"));
            return vec![];
        }
        self.after_onboarding()
    }

    /// Onboarding done: the form, if the server has one, else the channel.
    fn after_onboarding(&mut self) -> Vec<Effect> {
        let Some(joining) = &mut self.state.joining else {
            return vec![];
        };
        if !joining.form {
            return self.finish_joining();
        }
        joining.step = JoinStep::AskingForm;
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::MembershipForm {
                server: joining.server.clone(),
                invite: joining.invite.clone(),
            },
        ))]
    }

    pub(super) fn membership_form_came(
        &mut self,
        server: &str,
        result: Result<Arc<Verification>, String>,
    ) -> Vec<Effect> {
        let Some(joining) = self.joining_for(server) else {
            return vec![];
        };
        match result {
            Ok(form) if !form.is_empty() => {
                joining.step = JoinStep::Form {
                    answers: form.answers(),
                    form,
                };
                vec![]
            }
            Ok(_) => self.finish_joining(),
            Err(error) => {
                joining.error = Some(format!("Couldn't load the server's form: {error}"));
                vec![]
            }
        }
    }

    /// A field of the form answered: the rules agreed to or not, a text, or
    /// a choice by place.
    pub(super) fn answer_membership(&mut self, field: usize, answer: FormAnswer) -> Vec<Effect> {
        if let Some(Joining {
            step: JoinStep::Form { form, answers },
            sending: false,
            error,
            ..
        }) = &mut self.state.joining
            && let (Some(known), Some(slot)) = (form.fields.get(field), answers.get_mut(field))
        {
            let fits = matches!(
                (&known.kind, &answer),
                (FieldKind::Terms(_), FormAnswer::Agreed(_))
                    | (FieldKind::Text { .. }, FormAnswer::Text(_))
                    | (FieldKind::Choice(_), FormAnswer::Chosen(_))
            );
            if fits {
                *slot = answer;
                *error = None;
            }
        }
        vec![]
    }

    /// Submit: the form goes once every field holds what it may.
    pub(super) fn submit_membership(&mut self) -> Vec<Effect> {
        let Some(joining) = &mut self.state.joining else {
            return vec![];
        };
        let JoinStep::Form { form, answers } = &joining.step else {
            return vec![];
        };
        if joining.sending {
            return vec![];
        }
        if !form.complete(answers) {
            joining.error = Some(if form.only_terms() {
                "You must finish reading the rules to join.".into()
            } else {
                "Answer the required questions first.".into()
            });
            return vec![];
        }
        joining.sending = true;
        joining.error = None;
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::RequestMembership {
                server: joining.server.clone(),
                body: form.request(answers),
            },
        ))]
    }

    pub(super) fn membership_requested(
        &mut self,
        server: &str,
        result: Result<bool, String>,
    ) -> Vec<Effect> {
        let Some(joining) = self.joining_for(server) else {
            return vec![];
        };
        joining.sending = false;
        match result {
            Ok(true) => self.finish_joining(),
            Ok(false) => {
                joining.step = JoinStep::Pending;
                vec![]
            }
            Err(error) => {
                joining.error = Some(format!("Couldn't send the form: {error}"));
                vec![]
            }
        }
    }

    /// Closes it: done, or left for later, as Discord's client lets one
    /// close it; the server is joined either way, and its channel opens.
    pub(super) fn finish_joining(&mut self) -> Vec<Effect> {
        let Some(joining) = self.state.joining.take() else {
            return vec![];
        };
        self.open_joined(&joining.server, joining.room)
    }

    fn joining_for(&mut self, server: &str) -> Option<&mut Joining> {
        self.state
            .joining
            .as_mut()
            .filter(|joining| joining.server == server)
    }
}
