//! An app's modal over the window: its rows from the core's form, and its
//! callbacks to the core's actions.

use obsidian_core::application::{ModalAnswer as Answer, ModalField as Field, ModalWrap as Wrap};

use super::*;

impl Adapter {
    /// The form an app opened, or none.
    pub(super) fn show_modal(&self, window: &ObsidianWindow) {
        let form = self.app.state.modal.as_ref();
        window.set_modal_open(form.is_some());
        let Some(form) = form else {
            if self.models.modal_rows.row_count() > 0 {
                self.models.modal_rows.set_vec(Vec::new());
            }
            return;
        };
        let modal = &form.modal;
        window.set_modal_title(modal.title.as_str().into());
        window.set_modal_app(modal.app_name.as_str().into());
        window.set_modal_app_initial(initial(&modal.app_name));
        window.set_modal_app_icon(modal.app_icon.clone().unwrap_or_default().into());
        window.set_modal_error(form.error.clone().unwrap_or_default().into());
        window.set_modal_sending(form.sending.is_some());
        let rows = modal
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let answer = form.answers.get(index).unwrap_or(&Answer::None);
                let (label, description) = match &item.wrap {
                    Wrap::Label { label, description } => (label.as_str(), description.as_str()),
                    Wrap::Row | Wrap::Bare => ("", ""),
                };
                let chosen = |value: &str| matches!(answer, Answer::Chosen(chosen) if chosen.iter().any(|known| known == value));
                let mut row = ModalRow {
                    label: label.into(),
                    description: description.into(),
                    problem: form
                        .problems
                        .get(index)
                        .cloned()
                        .flatten()
                        .unwrap_or_default()
                        .into(),
                    ..Default::default()
                };
                match &item.field {
                    Field::Text {
                        label: own,
                        paragraph,
                        placeholder,
                        required,
                        ..
                    } => {
                        row.kind = i32::from(*paragraph);
                        if row.label.is_empty() {
                            row.label = own.as_str().into();
                        }
                        row.required = *required;
                        row.placeholder = placeholder.as_str().into();
                        row.value = match answer {
                            Answer::Text(text) => text.as_str().into(),
                            _ => SharedString::default(),
                        };
                    }
                    Field::Choice {
                        kind,
                        options,
                        placeholder,
                        required,
                        min,
                        ..
                    } => {
                        row.kind = match *kind {
                            21 => 3,
                            22 => 4,
                            _ => 2,
                        };
                        row.required = *required && *min > 0;
                        let picked: Vec<&str> = options
                            .iter()
                            .filter(|option| chosen(&option.value))
                            .map(|option| option.label.as_str())
                            .collect();
                        row.value = picked.join(", ").into();
                        row.summary = if picked.is_empty() {
                            if placeholder.is_empty() {
                                "Make a selection".into()
                            } else {
                                placeholder.as_str().into()
                            }
                        } else {
                            row.value.clone()
                        };
                        row.open = form.open == Some(index);
                        row.options = ModelRc::from(Rc::new(VecModel::from(
                            options
                                .iter()
                                .map(|option| ModalChoice {
                                    label: option.label.as_str().into(),
                                    description: option.description.as_str().into(),
                                    chosen: chosen(&option.value),
                                })
                                .collect::<Vec<_>>(),
                        )));
                    }
                    Field::Checkbox { .. } => {
                        row.kind = 5;
                        row.checked = matches!(answer, Answer::Checked(true));
                    }
                    Field::Display { content } => {
                        row.kind = 6;
                        row.content = super::messages::styled(content);
                    }
                    Field::Unsupported { required, .. } => {
                        row.kind = 7;
                        row.required = *required;
                    }
                }
                row
            })
            .collect();
        update_model(&self.models.modal_rows, rows);
    }
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
    let index = |index: i32| usize::try_from(index).unwrap_or(usize::MAX);
    act!(on_modal_text(at, text) => Action::ModalText(index(at), text.into()));
    act!(on_modal_pick(at, option) => Action::ModalPick(index(at), index(option)));
    act!(on_modal_menu(at) => Action::ModalMenu(Some(index(at))));
    act!(on_submit_modal() => Action::SubmitModal);
    act!(on_close_modal() => Action::CloseModal);
}
