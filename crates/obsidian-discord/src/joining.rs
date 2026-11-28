//! What a server asks of someone joining it: its onboarding questions
//! (`GET /guilds/{id}/onboarding`, answered with `POST
//! /guilds/{id}/onboarding-responses`) and its membership form, such as its
//! rules (`GET /guilds/{id}/member-verification`, answered with `PUT
//! /guilds/{id}/requests/@me`); see
//! <https://docs.discord.food/resources/guild#onboarding-object> and
//! <https://docs.discord.food/resources/guild#member-verification-object>.

use serde_json::{Map, Value, json};

/// The longest answers the form takes, by field, as Discord limits them.
pub const SHORT_ANSWER: usize = 150;
pub const LONG_ANSWER: usize = 1000;

/// A server's membership form.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Verification {
    /// When it last changed, which the answer names.
    pub version: Option<String>,
    pub description: String,
    pub fields: Vec<FormField>,
    /// Each field as Discord sent it, which the answer repeats with its
    /// response, as it must hold them all.
    raw: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormField {
    pub kind: FieldKind,
    pub label: String,
    pub description: String,
    pub placeholder: String,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// The server's rules, which the member agrees to.
    Terms(Vec<String>),
    /// A short answer, or a paragraph.
    Text { paragraph: bool },
    /// One of these.
    Choice(Vec<String>),
    /// A kind Discord Obsidian doesn't know.
    Other,
}

/// What a field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormAnswer {
    Agreed(bool),
    Text(String),
    Chosen(Option<usize>),
    None,
}

impl Verification {
    pub fn parse(value: &Value) -> Self {
        let raw: Vec<Value> = value["form_fields"].as_array().cloned().unwrap_or_default();
        let text = |field: &Value, key: &str| field[key].as_str().unwrap_or_default().to_owned();
        let strings = |field: &Value, key: &str| -> Vec<String> {
            field[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let fields = raw
            .iter()
            .map(|field| FormField {
                kind: match field["field_type"].as_str() {
                    Some("TERMS") => FieldKind::Terms(strings(field, "values")),
                    Some("TEXT_INPUT") => FieldKind::Text { paragraph: false },
                    Some("PARAGRAPH") => FieldKind::Text { paragraph: true },
                    Some("MULTIPLE_CHOICE") => FieldKind::Choice(strings(field, "choices")),
                    _ => FieldKind::Other,
                },
                label: text(field, "label"),
                description: text(field, "description"),
                placeholder: text(field, "placeholder"),
                required: field["required"].as_bool().unwrap_or(false),
            })
            .collect();
        Self {
            version: value["version"].as_str().map(str::to_owned),
            description: text(value, "description"),
            fields,
            raw,
        }
    }

    /// Whether it asks anything of a new member.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Whether only its rules are asked: Discord takes the answer at once,
    /// where other questions wait for the server's moderators.
    pub fn only_terms(&self) -> bool {
        self.fields
            .iter()
            .all(|field| matches!(field.kind, FieldKind::Terms(_)))
    }

    /// What each field holds as the form opens.
    pub fn answers(&self) -> Vec<FormAnswer> {
        self.fields
            .iter()
            .map(|field| match field.kind {
                FieldKind::Terms(_) => FormAnswer::Agreed(false),
                FieldKind::Text { .. } => FormAnswer::Text(String::new()),
                FieldKind::Choice(_) => FormAnswer::Chosen(None),
                FieldKind::Other => FormAnswer::None,
            })
            .collect()
    }

    /// Whether the answers may go: each required field answered, the rules
    /// agreed to, and texts within Discord's lengths.
    pub fn complete(&self, answers: &[FormAnswer]) -> bool {
        self.fields
            .iter()
            .zip(answers)
            .all(|(field, answer)| match answer {
                FormAnswer::Agreed(agreed) => *agreed || !field.required,
                FormAnswer::Text(text) => {
                    let longest = match field.kind {
                        FieldKind::Text { paragraph: true } => LONG_ANSWER,
                        _ => SHORT_ANSWER,
                    };
                    let length = text.trim().chars().count();
                    (length > 0 || !field.required) && length <= longest
                }
                FormAnswer::Chosen(chosen) => chosen.is_some() || !field.required,
                FormAnswer::None => !field.required,
            })
    }

    /// The answer, as `PUT /guilds/{id}/requests/@me` takes it: every field
    /// as Discord sent it, with its `response`.
    pub fn request(&self, answers: &[FormAnswer]) -> Value {
        let fields: Vec<Value> = self
            .raw
            .iter()
            .zip(answers)
            .map(|(field, answer)| {
                let mut field = field.clone();
                let response = match answer {
                    FormAnswer::Agreed(agreed) => json!(agreed),
                    FormAnswer::Text(text) => json!(text.trim()),
                    FormAnswer::Chosen(Some(index)) => json!(index),
                    FormAnswer::Chosen(None) | FormAnswer::None => Value::Null,
                };
                if let Some(object) = field.as_object_mut() {
                    object.insert("response".into(), response);
                }
                field
            })
            .collect();
        json!({"version": self.version, "form_fields": fields})
    }
}

/// Whether Discord took a membership form at once, or holds it for the
/// server's moderators to approve, from the join request it answers with.
pub fn approved(request: &Value) -> bool {
    request["application_status"].as_str() != Some("SUBMITTED")
}

/// A server's onboarding: the questions it asks those who join.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Onboarding {
    pub enabled: bool,
    /// The questions asked when joining, in order.
    pub prompts: Vec<Prompt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub id: String,
    pub title: String,
    pub single: bool,
    pub required: bool,
    pub options: Vec<PromptOption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptOption {
    pub id: String,
    pub title: String,
    pub description: String,
    /// A Unicode emoji, or a custom one's id, and its name.
    pub emoji: Option<(Option<String>, String)>,
}

impl Onboarding {
    /// Reads `GET /guilds/{id}/onboarding`: only the questions asked while
    /// joining, `in_onboarding`, as Discord's client asks them.
    pub fn parse(value: &Value) -> Self {
        let text = |value: &Value, key: &str| value[key].as_str().unwrap_or_default().to_owned();
        let prompts = value["prompts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|prompt| prompt["in_onboarding"].as_bool().unwrap_or(true))
            .filter_map(|prompt| {
                Some(Prompt {
                    id: prompt["id"].as_str()?.to_owned(),
                    title: text(prompt, "title"),
                    single: prompt["single_select"].as_bool().unwrap_or(false),
                    required: prompt["required"].as_bool().unwrap_or(false),
                    options: prompt["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|option| {
                            let emoji = &option["emoji"];
                            Some(PromptOption {
                                id: option["id"].as_str()?.to_owned(),
                                title: text(option, "title"),
                                description: text(option, "description"),
                                emoji: emoji["name"].as_str().filter(|name| !name.is_empty()).map(
                                    |name| {
                                        (emoji["id"].as_str().map(str::to_owned), name.to_owned())
                                    },
                                ),
                            })
                        })
                        .collect(),
                })
            })
            .filter(|prompt: &Prompt| !prompt.options.is_empty())
            .collect();
        Self {
            enabled: value["enabled"].as_bool().unwrap_or(false),
            prompts,
        }
    }

    /// Whether it asks anything of someone joining.
    pub fn asks(&self) -> bool {
        self.enabled && !self.prompts.is_empty()
    }

    /// The answers, as `POST /guilds/{id}/onboarding-responses` takes them:
    /// the options chosen, and when each question and option was seen.
    pub fn responses(&self, chosen: &[String], seen_ms: u64) -> Value {
        let mut prompts = Map::new();
        let mut options = Map::new();
        for prompt in &self.prompts {
            prompts.insert(prompt.id.clone(), json!(seen_ms));
            for option in &prompt.options {
                options.insert(option.id.clone(), json!(seen_ms));
            }
        }
        json!({
            "onboarding_responses": chosen,
            "onboarding_prompts_seen": prompts,
            "onboarding_responses_seen": options,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_is_answered_with_every_field_and_its_response() {
        let form = Verification::parse(&json!({
            "version": "2022-12-21T16:08:17.822000+00:00",
            "description": "Alien",
            "form_fields": [
                {"field_type": "TERMS", "label": "Read and agree to the server rules",
                 "required": true, "values": ["No spam", "Be kind"]},
                {"field_type": "TEXT_INPUT", "label": "Why?", "required": true},
                {"field_type": "MULTIPLE_CHOICE", "label": "Pick", "required": false,
                 "choices": ["Alien", "E.T."]},
            ],
        }));
        assert!(!form.only_terms());
        let mut answers = form.answers();
        assert!(!form.complete(&answers), "the rules aren't agreed to yet");
        answers[0] = FormAnswer::Agreed(true);
        answers[1] = FormAnswer::Text("x".repeat(151));
        assert!(
            !form.complete(&answers),
            "a short answer takes 150 characters"
        );
        answers[1] = FormAnswer::Text(" Curious ".into());
        answers[2] = FormAnswer::Chosen(Some(1));
        assert!(form.complete(&answers));
        let request = form.request(&answers);
        assert_eq!(request["version"], "2022-12-21T16:08:17.822000+00:00");
        assert_eq!(request["form_fields"][0]["response"], true);
        assert_eq!(request["form_fields"][0]["values"][1], "Be kind");
        assert_eq!(request["form_fields"][1]["response"], "Curious");
        assert_eq!(request["form_fields"][2]["response"], 1);
        assert!(approved(&json!({"application_status": "APPROVED"})));
        assert!(!approved(&json!({"application_status": "SUBMITTED"})));
    }

    #[test]
    fn onboarding_asks_its_joining_questions_and_says_what_was_seen() {
        let onboarding = Onboarding::parse(&json!({
            "enabled": true,
            "prompts": [
                {"id": "1", "title": "What brings you here?", "single_select": false,
                 "required": true, "in_onboarding": true,
                 "options": [
                    {"id": "11", "title": "Chat", "description": "",
                     "emoji": {"id": null, "name": "💬"}},
                    {"id": "12", "title": "Games", "description": "Play along",
                     "emoji": {"id": "99", "name": "controller"}},
                 ]},
                {"id": "2", "title": "Later", "in_onboarding": false,
                 "options": [{"id": "21", "title": "Only in Channels & Roles"}]},
            ],
        }));
        assert!(onboarding.asks());
        assert_eq!(
            onboarding.prompts.len(),
            1,
            "only those asked while joining"
        );
        let prompt = &onboarding.prompts[0];
        assert!(prompt.required && !prompt.single);
        assert_eq!(prompt.options[0].emoji, Some((None, "💬".into())));
        assert_eq!(
            prompt.options[1].emoji,
            Some((Some("99".into()), "controller".into()))
        );
        let responses = onboarding.responses(&["12".into()], 1_700_000_000_000);
        assert_eq!(responses["onboarding_responses"], json!(["12"]));
        assert_eq!(
            responses["onboarding_prompts_seen"]["1"],
            1_700_000_000_000u64
        );
        assert_eq!(
            responses["onboarding_responses_seen"]["11"],
            1_700_000_000_000u64
        );
        assert!(!Onboarding::parse(&json!({"enabled": false, "prompts": []})).asks());
    }
}
