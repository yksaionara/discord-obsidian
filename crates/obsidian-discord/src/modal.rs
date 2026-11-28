//! Apps' modals: the forms an app opens in answer to a slash command or a
//! button (INTERACTION_MODAL_CREATE), checked and sent back as Discord's
//! client does (`POST /interactions`, type 5).

use std::sync::Arc;

use serde_json::{Value, json};

/// The component types a modal holds, by Discord's number.
mod kind {
    pub const ACTION_ROW: u64 = 1;
    pub const STRING_SELECT: u64 = 3;
    pub const TEXT_INPUT: u64 = 4;
    pub const USER_SELECT: u64 = 5;
    pub const ROLE_SELECT: u64 = 6;
    pub const MENTIONABLE_SELECT: u64 = 7;
    pub const CHANNEL_SELECT: u64 = 8;
    pub const TEXT_DISPLAY: u64 = 10;
    pub const LABEL: u64 = 18;
    pub const RADIO_GROUP: u64 = 21;
    pub const CHECKBOX_GROUP: u64 = 22;
    pub const CHECKBOX: u64 = 23;
    pub const FILE_UPLOAD: u64 = 19;
}

/// Whether a field's kind picks the server's people, roles or channels,
/// which its options don't list.
pub fn picks_entities(kind: u64) -> bool {
    matches!(
        kind,
        kind::USER_SELECT | kind::ROLE_SELECT | kind::MENTIONABLE_SELECT | kind::CHANNEL_SELECT
    )
}

/// The kinds of a user, role, mentionable and channel menu, by Discord's
/// number, for the code that lists what they pick.
pub use kind::{CHANNEL_SELECT, MENTIONABLE_SELECT, ROLE_SELECT, USER_SELECT};

/// A text input's most characters, when its app sets none, as Discord's
/// client allows.
const TEXT_MAX: usize = 4000;

/// A form an app opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Modal {
    /// The interaction that opened it, which the submission names.
    pub id: String,
    /// The nonce of the interaction this session sent that opened it.
    pub nonce: Option<String>,
    pub channel_id: String,
    pub custom_id: String,
    pub title: String,
    pub app_id: String,
    pub app_name: String,
    /// The app's icon's address on Discord's CDN.
    pub app_icon: Option<String>,
    /// What it shows and asks, top to bottom.
    pub items: Vec<Item>,
}

/// One of a modal's top-level components.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub wrap: Wrap,
    pub field: Field,
}

/// How a field sits in the modal, which its submission repeats.
#[derive(Clone, Debug, PartialEq)]
pub enum Wrap {
    /// In an action row, the older form, a text input labelled by itself.
    Row,
    /// Under a label, with a description below it.
    Label { label: String, description: String },
    /// On its own: a text display.
    Bare,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Text {
        custom_id: String,
        /// A row's text input labels itself.
        label: String,
        /// Several lines rather than one: style 2.
        paragraph: bool,
        placeholder: String,
        value: String,
        required: bool,
        min_length: usize,
        max_length: usize,
    },
    /// A select menu, or a group of radio buttons or checkboxes: one of
    /// its options, or between `min` and `max`. A user, role, mentionable
    /// or channel menu lists no options: it picks the server's, by id.
    Choice {
        kind: u64,
        custom_id: String,
        options: Vec<ChoiceOption>,
        placeholder: String,
        required: bool,
        min: usize,
        max: usize,
        /// What a user, role, mentionable or channel menu picks as it
        /// opens, by id.
        defaults: Vec<String>,
        /// The kinds of channel a channel menu offers; empty for any.
        channel_types: Vec<u64>,
    },
    /// Files to upload, between `min` and `max`.
    Files {
        custom_id: String,
        required: bool,
        min: usize,
        max: usize,
    },
    Checkbox {
        custom_id: String,
        value: bool,
    },
    /// Markdown it shows.
    Display {
        content: String,
    },
    /// A component Discord Obsidian can't fill in yet, such as a file
    /// upload or a user picker.
    Unsupported {
        kind: u64,
        custom_id: String,
        required: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceOption {
    pub label: String,
    pub value: String,
    pub description: String,
    /// Chosen when the modal opens.
    pub default: bool,
}

/// What a field holds now.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Text(String),
    /// The values of the options chosen.
    Chosen(Vec<String>),
    Checked(bool),
    /// The files picked, in order.
    Files(Vec<std::path::PathBuf>),
    None,
}

impl Modal {
    /// Reads INTERACTION_MODAL_CREATE.
    pub fn parse(data: &Value) -> Option<Arc<Self>> {
        let application = &data["application"];
        let app_id = application["id"].as_str().unwrap_or_default().to_owned();
        Some(Arc::new(Self {
            id: data["id"].as_str()?.to_owned(),
            nonce: data["nonce"].as_str().map(str::to_owned),
            channel_id: data["channel_id"].as_str()?.to_owned(),
            custom_id: data["custom_id"].as_str()?.to_owned(),
            title: data["title"].as_str().unwrap_or_default().to_owned(),
            app_name: application["name"].as_str().unwrap_or_default().to_owned(),
            app_icon: application["icon"]
                .as_str()
                .map(|hash| crate::ready::application_icon(&app_id, hash)),
            app_id,
            items: data["components"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(item)
                .collect(),
        }))
    }

    /// What each field holds as the modal opens: a text input's value, the
    /// options chosen by default, a checkbox's default.
    pub fn answers(&self) -> Vec<Answer> {
        self.items
            .iter()
            .map(|item| match &item.field {
                Field::Text { value, .. } => Answer::Text(value.clone()),
                Field::Choice {
                    kind,
                    options,
                    defaults,
                    ..
                } if picks_entities(*kind) => Answer::Chosen(defaults.clone()),
                Field::Choice { options, .. } => Answer::Chosen(
                    options
                        .iter()
                        .filter(|option| option.default)
                        .map(|option| option.value.clone())
                        .collect(),
                ),
                Field::Files { .. } => Answer::Files(Vec::new()),
                Field::Checkbox { value, .. } => Answer::Checked(*value),
                Field::Display { .. } | Field::Unsupported { .. } => Answer::None,
            })
            .collect()
    }

    /// Why each field can't go as it holds `answers`, in the words of
    /// Discord's form, or `None` where it can.
    pub fn problems(&self, answers: &[Answer]) -> Vec<Option<String>> {
        const REQUIRED: &str = "This field is required.";
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let answer = answers.get(index).unwrap_or(&Answer::None);
                match (&item.field, answer) {
                    (
                        Field::Text {
                            required,
                            min_length,
                            max_length,
                            ..
                        },
                        answer,
                    ) => {
                        let length = match answer {
                            Answer::Text(text) => text.chars().count(),
                            _ => 0,
                        };
                        if length == 0 {
                            required.then(|| REQUIRED.to_owned())
                        } else if length < *min_length || length > *max_length {
                            Some(format!(
                                "Must be between {min_length} and {max_length} in length."
                            ))
                        } else {
                            None
                        }
                    }
                    (
                        Field::Choice {
                            required, min, max, ..
                        },
                        answer,
                    ) => {
                        let count = match answer {
                            Answer::Chosen(chosen) => chosen.len(),
                            _ => 0,
                        };
                        if count == 0 {
                            (*required && *min > 0).then(|| REQUIRED.to_owned())
                        } else if count < *min {
                            Some(format!("Select at least {min}"))
                        } else if count > *max {
                            Some(format!("Select at most {max}"))
                        } else {
                            None
                        }
                    }
                    (
                        Field::Files {
                            required, min, max, ..
                        },
                        answer,
                    ) => {
                        let count = match answer {
                            Answer::Files(files) => files.len(),
                            _ => 0,
                        };
                        if count == 0 {
                            (*required && *min > 0).then(|| REQUIRED.to_owned())
                        } else if count < *min {
                            Some(format!("Select at least {min}"))
                        } else if count > *max {
                            Some(format!("Select at most {max}"))
                        } else {
                            None
                        }
                    }
                    (Field::Unsupported { required: true, .. }, _) => Some(
                        "Discord Obsidian can't fill in this field yet. Use Discord in a browser for this form."
                            .to_owned(),
                    ),
                    _ => None,
                }
            })
            .collect()
    }

    /// The submission's `data`, as Discord's client sends it: each
    /// component in its place, with what it holds.
    /// The files the form's file fields hold, in the order `data` numbers
    /// them; they upload before it goes, into its `attachments`.
    pub fn files(&self, answers: &[Answer]) -> Vec<std::path::PathBuf> {
        answers
            .iter()
            .flat_map(|answer| match answer {
                Answer::Files(files) => files.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    pub fn data(&self, answers: &[Answer]) -> Value {
        // A file field's values are its files' places among the form's, as
        // Discord's client numbers them in `attachments`.
        let mut files = 0;
        let components: Vec<Value> = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let answer = answers.get(index).unwrap_or(&Answer::None);
                let field = field_value(&item.field, answer, &mut files);
                match &item.wrap {
                    Wrap::Row => json!({"type": kind::ACTION_ROW, "components": [field]}),
                    Wrap::Label { .. } => json!({"type": kind::LABEL, "component": field}),
                    Wrap::Bare => field,
                }
            })
            .collect();
        json!({
            "id": self.id,
            "custom_id": self.custom_id,
            "components": components,
        })
    }
}

fn field_value(field: &Field, answer: &Answer, files: &mut usize) -> Value {
    match (field, answer) {
        (Field::Files { custom_id, .. }, answer) => {
            let count = match answer {
                Answer::Files(picked) => picked.len(),
                _ => 0,
            };
            let values: Vec<usize> = (*files..*files + count).collect();
            *files += count;
            json!({
                "type": kind::FILE_UPLOAD,
                "custom_id": custom_id,
                "values": if count == 0 { Value::Null } else { json!(values) },
            })
        }
        (Field::Text { custom_id, .. }, answer) => json!({
            "type": kind::TEXT_INPUT,
            "custom_id": custom_id,
            "value": match answer {
                Answer::Text(text) => json!(text),
                _ => Value::Null,
            },
        }),
        (
            Field::Choice {
                kind: choice,
                custom_id,
                ..
            },
            answer,
        ) => {
            let chosen = match answer {
                Answer::Chosen(chosen) => chosen.as_slice(),
                _ => &[],
            };
            // A radio group holds one value; the others a list.
            if *choice == kind::RADIO_GROUP {
                json!({"type": choice, "custom_id": custom_id, "value": chosen.first()})
            } else {
                json!({"type": choice, "custom_id": custom_id, "values": chosen})
            }
        }
        (Field::Checkbox { custom_id, .. }, answer) => json!({
            "type": kind::CHECKBOX,
            "custom_id": custom_id,
            "value": matches!(answer, Answer::Checked(true)),
        }),
        (Field::Display { .. }, _) => json!({"type": kind::TEXT_DISPLAY}),
        (
            Field::Unsupported {
                kind: other,
                custom_id,
                ..
            },
            _,
        ) => json!({"type": other, "custom_id": custom_id, "values": null}),
    }
}

/// A top-level component: a row with a text input, a label with a field,
/// or a text display.
fn item(value: &Value) -> Option<Item> {
    match value["type"].as_u64()? {
        kind::ACTION_ROW => Some(Item {
            wrap: Wrap::Row,
            field: field(&value["components"][0])?,
        }),
        kind::LABEL => Some(Item {
            wrap: Wrap::Label {
                label: value["label"].as_str().unwrap_or_default().to_owned(),
                description: value["description"].as_str().unwrap_or_default().to_owned(),
            },
            field: field(&value["component"])?,
        }),
        kind::TEXT_DISPLAY => Some(Item {
            wrap: Wrap::Bare,
            field: Field::Display {
                content: value["content"].as_str().unwrap_or_default().to_owned(),
            },
        }),
        _ => None,
    }
}

fn field(value: &Value) -> Option<Field> {
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
    let custom_id = text("custom_id");
    let required = value["required"].as_bool();
    let count = |key: &str| value[key].as_u64().map(|count| count as usize);
    Some(match value["type"].as_u64()? {
        kind::TEXT_INPUT => Field::Text {
            label: text("label"),
            paragraph: value["style"].as_u64() == Some(2),
            placeholder: text("placeholder"),
            value: text("value"),
            required: required.unwrap_or(false),
            min_length: count("min_length").unwrap_or(0),
            max_length: count("max_length").unwrap_or(TEXT_MAX),
            custom_id,
        },
        choice @ (kind::STRING_SELECT | kind::RADIO_GROUP | kind::CHECKBOX_GROUP) => {
            let options: Vec<ChoiceOption> = value["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    Some(ChoiceOption {
                        label: option["label"].as_str()?.to_owned(),
                        value: option["value"].as_str()?.to_owned(),
                        description: option["description"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        default: option["default"].as_bool().unwrap_or(false),
                    })
                })
                .collect();
            let single = choice == kind::RADIO_GROUP;
            let required = required.unwrap_or(choice == kind::STRING_SELECT);
            Field::Choice {
                kind: choice,
                placeholder: text("placeholder"),
                min: if single {
                    usize::from(required)
                } else {
                    count("min_values").unwrap_or(usize::from(choice == kind::STRING_SELECT))
                },
                max: if single {
                    1
                } else {
                    count("max_values").unwrap_or(if choice == kind::STRING_SELECT {
                        1
                    } else {
                        options.len()
                    })
                },
                options,
                required,
                defaults: Vec::new(),
                channel_types: Vec::new(),
                custom_id,
            }
        }
        choice @ (kind::USER_SELECT
        | kind::ROLE_SELECT
        | kind::MENTIONABLE_SELECT
        | kind::CHANNEL_SELECT) => Field::Choice {
            kind: choice,
            placeholder: text("placeholder"),
            min: count("min_values").unwrap_or(1),
            max: count("max_values").unwrap_or(1),
            options: Vec::new(),
            required: required.unwrap_or(true),
            defaults: value["default_values"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|default| default["id"].as_str().map(str::to_owned))
                .collect(),
            channel_types: value["channel_types"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect(),
            custom_id,
        },
        kind::FILE_UPLOAD => Field::Files {
            required: required.unwrap_or(true),
            min: count("min_values").unwrap_or(1),
            max: count("max_values").unwrap_or(1),
            custom_id,
        },
        kind::CHECKBOX => Field::Checkbox {
            value: value["default"].as_bool().unwrap_or(false),
            custom_id,
        },
        other => Field::Unsupported {
            kind: other,
            required: required.unwrap_or(false),
            custom_id,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modal() -> Arc<Modal> {
        Modal::parse(&json!({
            "id": "900", "nonce": "77", "channel_id": "5", "custom_id": "report",
            "title": "Report a bug",
            "application": {"id": "100", "name": "Helper", "icon": "abc"},
            "components": [
                {"type": 1, "components": [{"type": 4, "custom_id": "title", "label": "Title",
                  "style": 1, "min_length": 3, "max_length": 40, "required": true, "value": "Crash"}]},
                {"type": 18, "label": "What happened?", "description": "Steps, please",
                 "component": {"type": 4, "custom_id": "steps", "style": 2, "required": false}},
                {"type": 10, "content": "Thanks for **helping**."},
                {"type": 18, "label": "Area", "component": {"type": 3, "custom_id": "area",
                  "placeholder": "Pick one", "options": [
                    {"label": "Voice", "value": "voice"},
                    {"label": "Chat", "value": "chat", "default": true}]}},
                {"type": 18, "label": "Severity", "component": {"type": 21, "custom_id": "severity", "required": true,
                  "options": [{"label": "Low", "value": "low"}, {"label": "High", "value": "high"}]}},
                {"type": 18, "label": "Contact me", "component": {"type": 23, "custom_id": "contact"}},
            ],
        }))
        .unwrap()
    }

    #[test]
    fn reads_a_modal_and_what_its_fields_hold() {
        let modal = modal();
        assert_eq!(modal.title, "Report a bug");
        assert_eq!(modal.app_name, "Helper");
        assert_eq!(modal.items.len(), 6);
        assert!(matches!(
            &modal.items[1].field,
            Field::Text {
                paragraph: true,
                required: false,
                ..
            }
        ));
        assert_eq!(
            modal.answers(),
            [
                Answer::Text("Crash".into()),
                Answer::Text(String::new()),
                Answer::None,
                Answer::Chosen(vec!["chat".into()]),
                Answer::Chosen(vec![]),
                Answer::Checked(false),
            ]
        );
    }

    #[test]
    fn checks_fields_as_discords_form_does() {
        let modal = modal();
        let mut answers = modal.answers();
        answers[0] = Answer::Text("Hi".into());
        let problems = modal.problems(&answers);
        assert_eq!(
            problems[0].as_deref(),
            Some("Must be between 3 and 40 in length.")
        );
        assert_eq!(problems[1], None, "optional and empty");
        assert_eq!(problems[4].as_deref(), Some("This field is required."));
        answers[0] = Answer::Text("Crash on join".into());
        answers[4] = Answer::Chosen(vec!["high".into()]);
        assert!(modal.problems(&answers).iter().all(Option::is_none));
    }

    #[test]
    fn submits_each_component_in_its_place() {
        let modal = modal();
        let mut answers = modal.answers();
        answers[1] = Answer::Text("Join, then crash".into());
        answers[4] = Answer::Chosen(vec!["high".into()]);
        answers[5] = Answer::Checked(true);
        assert_eq!(
            modal.data(&answers),
            json!({
                "id": "900",
                "custom_id": "report",
                "components": [
                    {"type": 1, "components": [{"type": 4, "custom_id": "title", "value": "Crash"}]},
                    {"type": 18, "component": {"type": 4, "custom_id": "steps", "value": "Join, then crash"}},
                    {"type": 10},
                    {"type": 18, "component": {"type": 3, "custom_id": "area", "values": ["chat"]}},
                    {"type": 18, "component": {"type": 21, "custom_id": "severity", "value": "high"}},
                    {"type": 18, "component": {"type": 23, "custom_id": "contact", "value": true}},
                ],
            })
        );
    }

    #[test]
    fn picks_people_roles_channels_and_files_as_discords_client_sends_them() {
        let modal = Modal::parse(&json!({
            "id": "901", "channel_id": "5", "custom_id": "assign",
            "application": {"id": "100", "name": "Helper"},
            "components": [
                {"type": 18, "label": "Who", "component": {"type": 5, "custom_id": "who",
                  "default_values": [{"id": "42", "type": "user"}]}},
                {"type": 18, "label": "Where", "component": {"type": 8, "custom_id": "where",
                  "channel_types": [0, 5], "required": false}},
                {"type": 18, "label": "Screenshots", "component": {"type": 19, "custom_id": "shots",
                  "min_values": 1, "max_values": 3}},
            ],
        }))
        .unwrap();
        let mut answers = modal.answers();
        assert_eq!(answers[0], Answer::Chosen(vec!["42".into()]), "its default");
        assert!(matches!(
            &modal.items[1].field,
            Field::Choice { kind: CHANNEL_SELECT, channel_types, .. } if *channel_types == [0, 5]
        ));
        let problems = modal.problems(&answers);
        assert_eq!(problems[2].as_deref(), Some("This field is required."));
        answers[2] = Answer::Files(vec!["a.png".into(), "b.png".into()]);
        assert!(modal.problems(&answers).iter().all(Option::is_none));
        assert_eq!(modal.files(&answers).len(), 2);
        let data = modal.data(&answers);
        assert_eq!(
            data["components"][0]["component"],
            json!({"type": 5, "custom_id": "who", "values": ["42"]})
        );
        // Files go by their places among the form's.
        assert_eq!(
            data["components"][2]["component"],
            json!({"type": 19, "custom_id": "shots", "values": [0, 1]})
        );
    }
}
