/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Themes in the app (PROJECT.md, D64): loading the imported themes at start, keeping
//! `app_t::theme` in step with the setting or the page's preview, applying, importing and
//! removing a theme, and the `/theme` command.
//!
//! The harness checks a theme file's shape, ids and `#rrggbb` values. Whether each colour name
//! is a field of `theme_t`, whether its `extends` resolves, and whether its id is a built-in id
//! are checked here, before anything is saved (D69).

use harness::settings::DefaultTheme;

use crate::app::{ToastKind, app_t};
use crate::settings_page::PageRow;
use crate::settings_spec::Section;
use crate::theme::theme_t;
use crate::theme_set::{ThemeErrorKind, theme_error_t, theme_set_t};

/// A path as the user typed, pasted or dropped it: trimmed, without the quotes or the
/// `file://` prefix a terminal adds to a dragged file, with `\ ` escapes undone and a leading
/// `~` expanded from `home`.
pub fn clean_path(raw: &str, home: Option<&str>) -> String {
    let mut path = raw.trim().trim_matches(['\r', '\n']).trim().to_string();
    for quote in ['\'', '"'] {
        if path.len() >= 2 && path.starts_with(quote) && path.ends_with(quote) {
            path = path[1..path.len() - 1].to_string();
            break;
        }
    }
    if let Some(rest) = path.strip_prefix("file://") {
        path = percent_decode(rest.strip_prefix("localhost").unwrap_or(rest));
    }
    if path.contains('\\') {
        let mut out = String::with_capacity(path.len());
        let mut chars = path.chars();
        while let Some(c) = chars.next() {
            match (c, chars.clone().next()) {
                ('\\', Some(next)) if !next.is_alphanumeric() => {
                    out.push(next);
                    chars.next();
                }
                _ => out.push(c),
            }
        }
        path = out;
    }
    if let Some(home) = home.filter(|h| !h.is_empty()) {
        if path == "~" {
            return home.to_string();
        }
        if let Some(rest) = path.strip_prefix("~/") {
            return format!("{}/{rest}", home.trim_end_matches('/'));
        }
    }
    path
}

/// `%xx` escapes to bytes; anything that is not a valid escape stays as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl app_t {
    /// Read the imported themes and settle the theme at start. Returns what the user should
    /// hear once: skipped theme files, and a theme in the settings that does not resolve (it is
    /// replaced by `ncagent-dark` for this run; the settings are not rewritten).
    pub(crate) fn load_themes(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        match self.runtime.themes() {
            Ok(loaded) => {
                if !loaded.skipped.is_empty() {
                    notes.push(format!(
                        "{} theme file(s) ignored: {}",
                        loaded.skipped.len(),
                        loaded.skipped.join("; ")
                    ));
                }
                self.themes = theme_set_t::new(loaded.themes);
            }
            Err(err) => notes.push(format!("could not read the imported themes: {err}")),
        }
        let id = self.settings.theme.clone();
        match self.themes.resolve(&id) {
            Ok(theme) => self.theme = theme,
            Err(err) => {
                self.theme = theme_t::dark();
                notes.push(format!("{err}; using {DefaultTheme}"));
            }
        }
        self.theme_id = id;
        notes
    }

    /// The theme the page previews: the one under the pointer, else the one under the cursor
    /// in the theme list. `None` when the page does not show.
    pub fn preview_theme(&self) -> Option<String> {
        let page = self.shown_page()?;
        if let Some((x, y)) = self.hover
            && let Some((_, id)) = page
                .preview_areas
                .iter()
                .find(|(r, _)| r.contains(ratatui::layout::Position::new(x, y)))
        {
            return Some(id.clone());
        }
        if self.focus != crate::input::Focus::Settings
            || page.pane != crate::settings_page::Pane::Rows
            || page.prompt.is_some()
        {
            return None;
        }
        match self.page_cursor_row() {
            Some(PageRow::Theme(id)) => Some(id),
            _ => None,
        }
    }

    /// Make `theme` the preview, or else the theme of the settings. A theme that does not
    /// resolve leaves `ncagent-dark` in use.
    pub(crate) fn sync_theme(&mut self) {
        let preview = self
            .preview_theme()
            .filter(|id| self.themes.resolve(id).is_ok());
        let want = preview.unwrap_or_else(|| self.settings.theme.clone());
        if want == self.theme_id {
            return;
        }
        self.theme = self
            .themes
            .resolve(&want)
            .unwrap_or_else(|_| theme_t::dark());
        self.theme_id = want;
    }

    /// Use the theme with this id from now on, and save it. A theme that does not resolve is
    /// refused with its error.
    pub fn apply_theme(&mut self, id: &str) -> bool {
        if let Err(err) = self.themes.resolve(id) {
            self.say(err.to_string(), ToastKind::Error);
            return false;
        }
        let ok = self.change_setting(|s| s.theme = id.to_string());
        if ok {
            self.theme_id.clear();
            self.sync_theme();
            self.say(
                format!("theme: {}", self.themes.name_of(id)),
                ToastKind::Info,
            );
        }
        ok
    }

    /// Import the theme file at `raw` (as typed): read and check it, save a copy, and apply
    /// it. Each failure is a toast that quotes the error. True when it was imported.
    pub fn import_theme(&mut self, raw: &str) -> bool {
        let home = std::env::var("HOME").ok();
        let path = clean_path(raw, home.as_deref());
        if path.is_empty() {
            self.say("type the path of a theme .json file", ToastKind::Warning);
            return false;
        }
        let custom = match self.runtime.read_theme_file(&path) {
            Ok(custom) => custom,
            Err(err) => {
                self.say(format!("could not import: {err}"), ToastKind::Error);
                return false;
            }
        };
        if theme_set_t::is_builtin(&custom.id) {
            let err = theme_error_t {
                theme: custom.id.clone(),
                kind: ThemeErrorKind::BuiltinId {
                    id: custom.id.clone(),
                },
            };
            self.say(format!("could not import: {err}"), ToastKind::Error);
            return false;
        }
        if let Err(err) = theme_t::from_custom(&self.themes, &custom) {
            self.say(format!("could not import: {err}"), ToastKind::Error);
            return false;
        }
        let replaced = self.themes.custom.contains_key(&custom.id);
        let id = custom.id.clone();
        let name = custom.name.clone();
        match self.runtime.save_theme(custom) {
            Ok(loaded) => self.themes = theme_set_t::new(loaded.themes),
            Err(err) => {
                self.say(format!("could not save the theme: {err}"), ToastKind::Error);
                return false;
            }
        }
        self.theme_id.clear();
        if self.apply_theme(&id) {
            let verb = if replaced { "replaced" } else { "imported" };
            self.say(
                format!("{verb} “{name}” ({id}) and applied it"),
                ToastKind::Info,
            );
        }
        true
    }

    /// Remove an imported theme. When it was in use, `ncagent-dark` takes its place.
    pub fn remove_custom_theme(&mut self, id: &str) {
        let name = self.themes.name_of(id);
        match self.runtime.remove_theme(id) {
            Ok(loaded) => self.themes = theme_set_t::new(loaded.themes),
            Err(err) => {
                self.say(
                    format!("could not remove the theme: {err}"),
                    ToastKind::Error,
                );
                return;
            }
        }
        self.theme_id.clear();
        let mut text = format!("removed the theme “{name}”");
        if self.settings.theme == id && self.change_setting(|s| s.theme = DefaultTheme.to_string())
        {
            text.push_str(&format!(
                "; {} is in use again",
                self.themes.name_of(DefaultTheme)
            ));
        }
        self.sync_theme();
        if let Some(page) = self.page.as_mut() {
            page.cursor = page.cursor.saturating_sub(1);
        }
        self.say(text, ToastKind::Info);
    }

    /// `/theme`: no argument opens the theme list; a name or id applies that theme.
    pub(crate) fn theme_command(&mut self, arg: Option<String>) {
        match arg {
            None => self.open_page(Some(Section::Themes)),
            Some(typed) => match self.themes.find(&typed) {
                Some(id) => {
                    self.apply_theme(&id);
                }
                None => self.say(
                    format!(
                        "no theme {:?}; the themes are {}",
                        crate::text::truncate(&crate::text::sanitize(&typed, false), 24),
                        self.themes.ids().join(", ")
                    ),
                    ToastKind::Warning,
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_pasted_and_dropped_paths_are_cleaned() {
        let home = Some("/home/u");
        for (raw, want) in [
            ("/tmp/a.json", "/tmp/a.json"),
            ("  /tmp/a.json \n", "/tmp/a.json"),
            ("'/tmp/my theme.json'", "/tmp/my theme.json"),
            ("\"/tmp/my theme.json\"\r\n", "/tmp/my theme.json"),
            ("/tmp/my\\ theme\\ \\(2\\).json", "/tmp/my theme (2).json"),
            ("file:///tmp/my%20theme.json", "/tmp/my theme.json"),
            ("file://localhost/tmp/x.json", "/tmp/x.json"),
            ("~/themes/x.json", "/home/u/themes/x.json"),
            ("~", "/home/u"),
            ("~other/x.json", "~other/x.json"),
            ("relative/x.json", "relative/x.json"),
            ("C:\\Users\\x.json", "C:\\Users\\x.json"),
            ("100%.json", "100%.json"),
            ("", ""),
            ("   ", ""),
        ] {
            assert_eq!(clean_path(raw, home), want, "{raw:?}");
        }
        assert_eq!(clean_path("~/x", None), "~/x");
    }
}
