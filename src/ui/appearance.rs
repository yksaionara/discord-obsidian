//! Appearance: choosing a theme, and making and editing the person's own.

use super::*;

impl Adapter {
    /// Shows the window in a theme, built in or the person's own, and keeps
    /// it chosen. False for an id Discord Obsidian doesn't know, which changes nothing.
    pub(super) fn choose_theme(&mut self, window: &ObsidianWindow, id: &str) -> bool {
        let Some((id, chosen)) =
            theme::lookup(id, &self.layout.custom_themes).map(|(id, theme)| (id.to_owned(), theme))
        else {
            return false;
        };
        window.set_theme_id(id.as_str().into());
        self.set_theme(window, &chosen);
        self.layout.theme = Some(id);
        // Choosing a theme stops following the system, as in Discord.
        self.layout.theme_sync = false;
        window.set_theme_sync(false);
        self.layout.write(&self.root);
        true
    }

    /// Discord's "Sync with computer": shows Dark or Light as the system
    /// shows its apps, and keeps following it.
    pub(super) fn sync_theme(&mut self, window: &ObsidianWindow) {
        self.layout.theme_sync = true;
        window.set_theme_sync(true);
        self.show_synced(window, theme::synced(theme::system_dark(window)));
        self.layout.write(&self.root);
    }

    /// Follows the system into dark or light mode while synced, unless the
    /// theme editor is open.
    pub(super) fn follow_system_theme(&mut self, window: &ObsidianWindow) {
        if !self.layout.theme_sync || self.theme_editor.is_some() {
            return;
        }
        let id = theme::synced(theme::system_dark(window));
        if window.get_theme_id() != id {
            self.show_synced(window, id);
            self.layout.write(&self.root);
        }
    }

    fn show_synced(&mut self, window: &ObsidianWindow, id: &'static str) {
        if let Some((id, synced)) = theme::lookup(id, &[]) {
            window.set_theme_id(id.into());
            self.set_theme(window, &synced);
            self.layout.theme = Some(id.to_owned());
        }
    }

    /// Shows the window in another theme's colours, building again what
    /// carries the old mention colour in its markup: message rows, the
    /// topic, pins and search results.
    pub(super) fn set_theme(&mut self, window: &ObsidianWindow, theme: &theme::Theme) {
        if theme.apply(window) {
            self.restyle_mentions();
            self.refresh(window);
        }
    }

    /// Marks for building again, at the next refresh, what carries the
    /// mention colour in its markup.
    pub(super) fn restyle_mentions(&mut self) {
        self.restyle = true;
        self.topic_shown = None;
        self.models.pins.clear();
        self.models.search_results.clear();
    }

    /// Opens the editor on a copy of the theme shown, which the person can
    /// swap for any other.
    pub(super) fn new_theme(&mut self, window: &ObsidianWindow) {
        let current = window.get_theme_id();
        let starts: Vec<_> = theme::starts(&self.layout.custom_themes).collect();
        let index = starts
            .iter()
            .position(|start| start.id == current.as_str())
            .unwrap_or(0);
        let names: Vec<SharedString> = starts.iter().map(|start| start.name.into()).collect();
        let (base, start) = (starts[index].base, starts[index].theme);
        window.set_theme_name(copy_name(starts[index].name).into());
        window.set_theme_starts(ModelRc::new(VecModel::from(names)));
        window.set_theme_start(index as i32);
        self.open_theme_editor(window, theme::Editor::new(None, base, start));
    }

    /// Opens the editor on one of the person's own themes.
    pub(super) fn edit_theme(&mut self, window: &ObsidianWindow, id: &str) {
        let Some(custom) = self
            .layout
            .custom_themes
            .iter()
            .find(|custom| custom.id == id)
        else {
            return;
        };
        window.set_theme_name(custom.name.as_str().into());
        let editor = theme::Editor::new(Some(custom.id.clone()), custom.base, custom.theme);
        self.open_theme_editor(window, editor);
    }

    fn open_theme_editor(&mut self, window: &ObsidianWindow, editor: theme::Editor) {
        window.set_theme_background(editor.theme.chat);
        window.set_theme_accent(editor.theme.accent);
        window.set_theme_editing_saved(editor.editing.is_some());
        window.set_theme_dark(editor.theme.dark);
        window.set_theme_colours(editor.rows());
        window.set_theme_can_save(true);
        window.set_theme_editor(true);
        self.set_theme(window, &editor.theme);
        self.theme_editor = Some(editor);
    }

    /// The new theme starts again from another, and takes its name too
    /// unless the person named it.
    pub(super) fn theme_start_chosen(&mut self, window: &ObsidianWindow, index: i32) {
        let custom = &self.layout.custom_themes;
        let start = |index: i32| {
            let index = usize::try_from(index).ok()?;
            theme::starts(custom)
                .nth(index)
                .map(|start| (start.base, start.theme, copy_name(start.name)))
        };
        let (Some(editor), Some((base, theme, name))) = (self.theme_editor.as_mut(), start(index))
        else {
            return;
        };
        let named = start(window.get_theme_start())
            .is_none_or(|(_, _, before)| before != window.get_theme_name().as_str());
        if !named {
            window.set_theme_name(name.into());
        }
        editor.restart(base, theme);
        window.set_theme_background(theme.chat);
        window.set_theme_accent(theme.accent);
        window.set_theme_start(index);
        window.set_theme_dark(theme.dark);
        window.set_theme_can_save(true);
        self.set_theme(window, &theme);
    }

    /// A colour's field changed: the window shows it at once if it's a
    /// colour.
    pub(super) fn theme_colour_edited(&mut self, window: &ObsidianWindow, index: i32, text: &str) {
        let Some(editor) = self.theme_editor.as_mut() else {
            return;
        };
        let changed = usize::try_from(index).is_ok_and(|index| editor.edit(index, text));
        window.set_theme_can_save(editor.valid());
        if changed {
            let theme = editor.theme;
            self.set_theme(window, &theme);
        }
    }

    /// A colour came from the picker: the window shows it at once.
    pub(super) fn theme_colour_picked(&mut self, window: &ObsidianWindow, index: i32, colour: slint::Color) {
        let Some(editor) = self.theme_editor.as_mut() else {
            return;
        };
        if usize::try_from(index).is_ok_and(|index| editor.pick(index, colour)) {
            let theme = editor.theme;
            window.set_theme_background(theme.chat);
            window.set_theme_accent(theme.accent);
            window.set_theme_can_save(editor.valid());
            self.set_theme(window, &theme);
        }
    }

    /// The background or accent changed among the quick colours: every
    /// colour is made again from the two.
    pub(super) fn theme_quick_picked(&mut self, window: &ObsidianWindow, background: slint::Color, accent: slint::Color) {
        let Some(editor) = self.theme_editor.as_mut() else {
            return;
        };
        editor.generate(background, accent);
        let theme = editor.theme;
        window.set_theme_background(background);
        window.set_theme_accent(accent);
        window.set_theme_dark(theme.dark);
        window.set_theme_can_save(true);
        self.set_theme(window, &theme);
    }

    pub(super) fn theme_dark_toggled(&mut self, window: &ObsidianWindow, dark: bool) {
        let Some(editor) = self.theme_editor.as_mut() else {
            return;
        };
        editor.theme.dark = dark;
        let theme = editor.theme;
        self.set_theme(window, &theme);
    }

    /// Keeps the theme as edited, among the person's own, and chooses it.
    pub(super) fn save_theme(&mut self, window: &ObsidianWindow) {
        let Some(editor) = self.theme_editor.take_if(|editor| editor.valid()) else {
            return;
        };
        let name = match window.get_theme_name().trim() {
            "" => "My theme".to_owned(),
            name => name.to_owned(),
        };
        let custom = &mut self.layout.custom_themes;
        let saved = editor
            .editing
            .as_ref()
            .and_then(|id| custom.iter_mut().find(|custom| custom.id == *id));
        let id = match saved {
            Some(saved) => {
                saved.name = name;
                saved.theme = editor.theme;
                saved.id.clone()
            }
            None => {
                let id = theme::new_id(custom);
                custom.push(theme::Custom {
                    id: id.clone(),
                    name,
                    base: editor.base,
                    theme: editor.theme,
                });
                id
            }
        };
        close_theme_editor(window);
        theme::show_cards(window, &self.layout.custom_themes);
        self.choose_theme(window, &id);
    }

    /// Closes the editor and shows the theme from before it opened.
    pub(super) fn cancel_theme(&mut self, window: &ObsidianWindow) {
        if self.theme_editor.take().is_some() {
            close_theme_editor(window);
            // The theme Appearance shows as chosen, which may be the default
            // presentation's rather than one the layout names.
            let shown = window.get_theme_id();
            let (_, before) = theme::lookup(&shown, &self.layout.custom_themes).unwrap_or_else(|| {
                theme::chosen(self.layout.theme.as_deref(), &self.layout.custom_themes)
            });
            self.set_theme(window, &before);
        }
    }

    /// Deletes the theme being edited. If it was the chosen one, Yap shows.
    pub(super) fn delete_theme(&mut self, window: &ObsidianWindow) {
        let Some(editor) = self.theme_editor.take() else {
            return;
        };
        close_theme_editor(window);
        if let Some(id) = editor.editing {
            self.layout.custom_themes.retain(|custom| custom.id != id);
        }
        theme::show_cards(window, &self.layout.custom_themes);
        let (id, _) = theme::chosen(self.layout.theme.as_deref(), &self.layout.custom_themes);
        let id = id.to_owned();
        self.choose_theme(window, &id);
    }
}

/// What a new theme copied from another is called until the person names it.
fn copy_name(name: &str) -> String {
    format!("{name} copy")
}

/// Drops the editor's models from the window, so they're held only while
/// it's open.
fn close_theme_editor(window: &ObsidianWindow) {
    window.set_theme_editor(false);
    window.set_theme_colours(ModelRc::default());
    window.set_theme_starts(ModelRc::default());
}
