/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The settings page (PROJECT.md, D65): its state, its keys and what each of its actions does.
//! Drawing is in `settings_draw`, the setting rules in `settings_spec`, themes in
//! `theme_actions`.
//!
//! The page is shown in the main panel as a closable pseudo-tab at the end of the tab bar. It is
//! never saved with the open tabs and is not a conversation. While it shows, the conversation
//! under it keeps running and its composer is not drawn; showing a conversation tab hides the
//! page, and closing the page gives the view back to that conversation.
//!
//! Every change is applied at once and saved through `app_t::update_settings`. A refused save
//! is a toast, and the value in memory stays the saved one.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use harness::settings::settings_t;

use crate::app::{ToastKind, app_t};
use crate::composer::{EditOp, composer_t};
use crate::input::{Action, Focus};
use crate::layout::SidebarMode;
use crate::settings_spec::{
    Control, Section, Sections, SettingKey, changed_count, keys_of, reset_all,
};
use crate::theme_set::theme_set_t;

/// Which part of the page has the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The list of sections.
    List,
    /// The rows of the shown section.
    Rows,
}

/// A read-only fact of the About section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fact {
    Model,
    Backend,
    Notice,
    State,
    Contract,
    Shape,
    Colors,
    Mouse,
}

/// One row of a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageRow {
    Setting(SettingKey),
    /// A theme of the theme list, by id.
    Theme(String),
    /// The import button, or the path prompt while it is open.
    Import,
    Fact(Fact),
    /// The count of changed settings and the reset button.
    ResetAll,
}

impl PageRow {
    /// Facts are read and scrolled past, never chosen.
    pub fn focusable(&self) -> bool {
        !matches!(self, PageRow::Fact(_))
    }
}

/// A change that waits for a yes on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageConfirm {
    ResetAll,
    /// Remove the imported theme with this id.
    Remove(String),
}

/// What the page's keys and clicks do. Each mouse action has a key: see the comments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageAction {
    /// Open the page, or bring it back (`ctrl+o`, `f2`, `/settings`, sidebar `s`, the
    /// `⚙ settings` button).
    Open,
    /// Open the page on a section (`/theme` with no argument opens the themes).
    OpenAt(Section),
    /// The page's tab in the tab bar. Keys: `ctrl+o`, or `alt+.` past the last tab.
    Show,
    /// The `×` of the page's tab. Keys: `esc` on the page, `alt+w` while it shows.
    Close,
    /// A click on a section. Keys: the list pane and `j` `k`, or `1` to `6`.
    Section(Section),
    /// `tab` and `shift+tab`: section list, rows, then the sidebar when it shows.
    Cycle(i32),
    /// `j` `k` `↑` `↓`: the next section in the list, or the next row.
    Move(i32),
    /// `enter` `space`: change the row, or go from the list to the rows.
    Activate,
    /// `←` `→` `h` `l`: the previous or next option; off and on for a toggle.
    Step(i32),
    /// A click on a row. Keys: `j` `k`.
    Row(usize),
    /// A click on the toggle chip of row `n`. Keys: the row, then `enter`.
    Toggle(usize),
    /// A click on option `o` of row `n`. Keys: the row, then `←` `→`.
    Pick(usize, usize),
    /// The reset chip of row `n`. Keys: the row, then `r`.
    Reset(usize),
    /// `r`: reset the cursor row.
    ResetCursor,
    /// A click on a theme in the list. Keys: the row, then `enter`.
    ApplyTheme(String),
    /// The `remove` chip of an imported theme. Keys: the row, then `d` or `delete`.
    RemoveTheme(String),
    /// `d` `delete`: remove the imported theme under the cursor.
    RemoveCursor,
    /// A click on a button row (`import theme…`, `reset all settings…`): the row, then what
    /// `enter` does there. Keys: the row, then `enter`.
    Press(usize),
    PromptEdit(EditOp),
    /// `enter` in the path prompt, or its `import` chip.
    PromptSubmit,
    /// `enter` or `y` on a waiting confirmation, or its chip.
    ConfirmYes,
    /// `n` on a waiting confirmation, `esc` on it or on the path prompt, or their chips: drop
    /// what waits.
    ConfirmNo,
    /// The wheel over the page. Keys: `pgup` `pgdn`, `j` `k`.
    Scroll(i32),
}

/// What the page's keys are doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageKeys {
    Browse,
    /// The import path prompt has the keys.
    Prompt,
    /// A confirmation waits.
    Confirm,
}

/// The settings page while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct settings_page_t {
    pub section: Section,
    pub pane: Pane,
    /// The cursor row of the shown section.
    pub cursor: usize,
    /// The first line of the rows drawn.
    pub scroll: usize,
    /// Keep the cursor row in view. Off after the wheel moved the rows.
    pub follow: bool,
    /// The theme file path being typed.
    pub prompt: Option<composer_t>,
    pub confirm: Option<PageConfirm>,
    /// Where each theme (a list row or a chip) was drawn in the last frame, for the hover
    /// preview.
    pub preview_areas: Vec<(ratatui::layout::Rect, String)>,
}

impl settings_page_t {
    pub fn new(section: Section) -> Self {
        settings_page_t {
            section,
            pane: Pane::Rows,
            cursor: 0,
            scroll: 0,
            follow: true,
            prompt: None,
            confirm: None,
            preview_areas: Vec::new(),
        }
    }

    pub fn keys(&self) -> PageKeys {
        if self.prompt.is_some() {
            PageKeys::Prompt
        } else if self.confirm.is_some() {
            PageKeys::Confirm
        } else {
            PageKeys::Browse
        }
    }
}

/// The rows of a section, from the settings and the theme set.
pub fn page_rows(section: Section, themes: &theme_set_t) -> Vec<PageRow> {
    match section {
        Section::Themes => themes
            .ids()
            .into_iter()
            .map(PageRow::Theme)
            .chain(std::iter::once(PageRow::Import))
            .collect(),
        Section::About => [
            Fact::Model,
            Fact::Backend,
            Fact::Notice,
            Fact::State,
            Fact::Contract,
            Fact::Shape,
            Fact::Colors,
            Fact::Mouse,
        ]
        .into_iter()
        .map(PageRow::Fact)
        .chain(std::iter::once(PageRow::ResetAll))
        .collect(),
        other => keys_of(other).into_iter().map(PageRow::Setting).collect(),
    }
}

/// The keys of the page when it has the focus. Keys that work everywhere (`esc`, `ctrl+c`, the
/// tab keys) are mapped before this.
pub fn page_key(key: &KeyEvent, mode: PageKeys) -> Option<Action> {
    let c = key.modifiers.contains(KeyModifiers::CONTROL);
    let a = key.modifiers.contains(KeyModifiers::ALT);
    let page = |p| Some(Action::Page(p));
    match mode {
        PageKeys::Prompt => match key.code {
            KeyCode::Enter if !a => page(PageAction::PromptSubmit),
            KeyCode::Tab | KeyCode::BackTab => None,
            _ => crate::input::editing(key)
                .filter(|op| !matches!(op, EditOp::Newline | EditOp::Up | EditOp::Down))
                .map(|op| Action::Page(PageAction::PromptEdit(op))),
        },
        PageKeys::Confirm => match key.code {
            KeyCode::Enter | KeyCode::Char('y') if !c => page(PageAction::ConfirmYes),
            KeyCode::Char('n') if !c => page(PageAction::ConfirmNo),
            _ => None,
        },
        PageKeys::Browse => match key.code {
            KeyCode::Tab => page(PageAction::Cycle(1)),
            KeyCode::BackTab => page(PageAction::Cycle(-1)),
            KeyCode::Down | KeyCode::Char('j') => page(PageAction::Move(1)),
            KeyCode::Up | KeyCode::Char('k') => page(PageAction::Move(-1)),
            KeyCode::Enter | KeyCode::Char(' ') if !c => page(PageAction::Activate),
            KeyCode::Left | KeyCode::Char('h') if !a && !c => page(PageAction::Step(-1)),
            KeyCode::Right | KeyCode::Char('l') if !a && !c => page(PageAction::Step(1)),
            KeyCode::Char('r') if !c && !a => page(PageAction::ResetCursor),
            KeyCode::Char('d') | KeyCode::Delete if !c && !a => page(PageAction::RemoveCursor),
            KeyCode::Char(d @ '1'..='6') if !c && !a => Sections
                .get((d as usize).saturating_sub('1' as usize))
                .map(|s| Action::Page(PageAction::Section(*s))),
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            _ => None,
        },
    }
}

impl app_t {
    /// The page, when it is the view of the main panel.
    pub fn shown_page(&self) -> Option<&crate::settings_page::settings_page_t> {
        self.page.as_ref().filter(|_| self.page_shown)
    }

    /// The rows of the section the page shows.
    pub fn page_rows(&self) -> Vec<PageRow> {
        match &self.page {
            Some(p) => page_rows(p.section, &self.themes),
            None => Vec::new(),
        }
    }

    /// The row under the page's cursor.
    pub fn page_cursor_row(&self) -> Option<PageRow> {
        let cursor = self.page.as_ref()?.cursor;
        self.page_rows().get(cursor).cloned()
    }

    /// Open the page, or show it again with the keys on it.
    pub fn open_page(&mut self, section: Option<Section>) {
        self.menu = None;
        self.help_open = false;
        self.cancel_sidebar_modal();
        let page = self
            .page
            .get_or_insert_with(|| settings_page_t::new(section.unwrap_or(Section::Appearance)));
        if let Some(section) = section
            && page.section != section
        {
            page.section = section;
            page.cursor = 0;
            page.scroll = 0;
        }
        page.pane = Pane::Rows;
        page.follow = true;
        self.page_shown = true;
        self.focus = Focus::Settings;
        if self.layout.sidebar_mode() == SidebarMode::Drawer {
            self.drawer_open = false;
        }
        self.page_settle_cursor(1);
    }

    /// Close the page. The conversation tab under it is shown again.
    pub fn close_page(&mut self) {
        self.page = None;
        self.page_shown = false;
        if self.focus == Focus::Settings {
            self.focus = Focus::Input;
        }
    }

    /// Hide the page behind a conversation tab. It stays in the tab bar.
    pub(crate) fn hide_page(&mut self) {
        self.page_shown = false;
        if self.focus == Focus::Settings {
            self.focus = Focus::Input;
        }
    }

    /// `esc` on the page: the prompt or the confirmation goes first, then the page closes.
    pub(crate) fn page_escape(&mut self) -> bool {
        let Some(page) = self.page.as_mut() else {
            return false;
        };
        if page.prompt.take().is_some() || page.confirm.take().is_some() {
            return true;
        }
        self.close_page();
        true
    }

    /// Move the cursor off a fact, in direction `d` first, and keep it in range.
    fn page_settle_cursor(&mut self, d: i32) {
        let rows = self.page_rows();
        let Some(page) = self.page.as_mut() else {
            return;
        };
        let n = rows.len();
        if n == 0 {
            page.cursor = 0;
            return;
        }
        let start = page.cursor.min(n - 1);
        let ok = |i: &usize| rows.get(*i).is_some_and(PageRow::focusable);
        let found = if d < 0 {
            (0..=start).rev().find(ok).or_else(|| (start..n).find(ok))
        } else {
            (start..n).find(ok).or_else(|| (0..=start).rev().find(ok))
        };
        page.cursor = found.unwrap_or(start);
    }

    fn page_set_cursor(&mut self, row: usize) {
        if let Some(page) = self.page.as_mut() {
            page.cursor = row;
            page.pane = Pane::Rows;
            page.follow = true;
        }
        self.focus = Focus::Settings;
    }

    /// Change the settings from the page: applied at once and saved (D65). Returns true when
    /// the change is now in effect.
    pub fn change_setting(&mut self, change: impl FnOnce(&mut settings_t)) -> bool {
        let tabs_were_kept = self.settings.restore_tabs;
        let links_were_on = self.settings.link_hyperlinks;
        let ok = self.update_settings(change);
        self.settings_changed();
        self.links_changed(links_were_on);
        if ok && self.settings.restore_tabs && !tabs_were_kept {
            // Keep the tabs open now, not only after the next change of tabs.
            self.tabs_changed();
        }
        ok
    }

    /// Bring what the settings drive up to date: the effect's on/off and the sidebar list.
    pub(crate) fn settings_changed(&mut self) {
        self.animate = self.options.animate && self.settings.animation;
        self.rebuild_sidebar_items();
    }

    /// Flip a toggle, or say why it cannot change.
    fn page_toggle(&mut self, key: SettingKey, value: Option<bool>) {
        if key == SettingKey::Animation && !self.options.animate {
            self.say(
                "animation is off by the command line (--speed instant)",
                ToastKind::Info,
            );
            return;
        }
        let Some(now) = key.toggle(&self.settings) else {
            return;
        };
        let next = value.unwrap_or(!now);
        self.change_setting(|s| {
            key.set_toggle(s, next);
        });
    }

    fn page_choose(&mut self, key: SettingKey, index: usize) {
        match key.control() {
            Control::Theme => {
                if let Some(id) = self.themes.ids().get(index).cloned() {
                    self.apply_theme(&id);
                }
            }
            Control::Choice if !key.option_enabled(index) => self.say(
                format!(
                    "{} can be chosen only on the command line",
                    key.options().get(index).copied().unwrap_or("this option")
                ),
                ToastKind::Info,
            ),
            Control::Choice => {
                self.change_setting(|s| {
                    key.set_choice(s, index);
                });
            }
            Control::Toggle => self.page_toggle(key, None),
        }
    }

    /// The theme `d` steps from the one in use, wrapping.
    fn theme_stepped(&self, d: i32) -> Option<usize> {
        let ids = self.themes.ids();
        if ids.is_empty() {
            return None;
        }
        let at = ids
            .iter()
            .position(|id| *id == self.settings.theme)
            .unwrap_or(0);
        Some((at as i64 + i64::from(d.signum())).rem_euclid(ids.len() as i64) as usize)
    }

    fn page_step_row(&mut self, row: &PageRow, d: i32) {
        let PageRow::Setting(key) = row else {
            return;
        };
        match key.control() {
            Control::Toggle => self.page_toggle(*key, Some(d > 0)),
            Control::Choice => {
                if let Some(i) = key.stepped(&self.settings, d) {
                    self.page_choose(*key, i);
                }
            }
            Control::Theme => {
                if let Some(i) = self.theme_stepped(d) {
                    self.page_choose(*key, i);
                }
            }
        }
    }

    fn page_activate(&mut self, row: &PageRow) {
        match row {
            PageRow::Setting(key) => match key.control() {
                Control::Toggle => self.page_toggle(*key, None),
                _ => self.page_step_row(row, 1),
            },
            PageRow::Theme(id) => {
                let id = id.clone();
                self.apply_theme(&id);
            }
            PageRow::Import => self.page_import_start(),
            PageRow::ResetAll => {
                if changed_count(&self.settings) == 0 {
                    self.say("every setting is at its default", ToastKind::Info);
                } else if let Some(page) = self.page.as_mut() {
                    page.confirm = Some(PageConfirm::ResetAll);
                }
            }
            PageRow::Fact(_) => {}
        }
    }

    fn page_import_start(&mut self) {
        let rows = self.page_rows();
        let at = rows.iter().position(|r| *r == PageRow::Import);
        if let Some(page) = self.page.as_mut() {
            page.confirm = None;
            page.prompt = Some(composer_t::new());
            if let Some(at) = at {
                page.cursor = at;
            }
            page.pane = Pane::Rows;
            page.follow = true;
        }
        self.focus = Focus::Settings;
    }

    fn page_remove_start(&mut self, id: &str) {
        if !self.themes.custom.contains_key(id) {
            self.say(
                "only an imported theme can be removed; built-in themes stay",
                ToastKind::Info,
            );
            return;
        }
        let rows = self.page_rows();
        let at = rows
            .iter()
            .position(|r| *r == PageRow::Theme(id.to_string()));
        if let Some(page) = self.page.as_mut() {
            page.prompt = None;
            page.confirm = Some(PageConfirm::Remove(id.to_string()));
            if let Some(at) = at {
                page.cursor = at;
            }
        }
    }

    fn page_confirm(&mut self) {
        let Some(confirm) = self.page.as_mut().and_then(|p| p.confirm.take()) else {
            return;
        };
        match confirm {
            PageConfirm::ResetAll => {
                let n = changed_count(&self.settings);
                if self.change_setting(reset_all) {
                    self.say(
                        format!("reset {n} setting(s) to the defaults"),
                        ToastKind::Info,
                    );
                }
            }
            PageConfirm::Remove(id) => self.remove_custom_theme(&id),
        }
    }

    pub(crate) fn apply_page(&mut self, action: PageAction) {
        if !matches!(action, PageAction::Open | PageAction::OpenAt(_)) && self.page.is_none() {
            return;
        }
        if !matches!(
            action,
            PageAction::Open
                | PageAction::OpenAt(_)
                | PageAction::Show
                | PageAction::Close
                | PageAction::Scroll(_)
        ) {
            self.page_shown = true;
            self.focus = Focus::Settings;
        }
        match action {
            PageAction::Open => self.open_page(None),
            PageAction::OpenAt(section) => self.open_page(Some(section)),
            PageAction::Show => self.open_page(None),
            PageAction::Close => self.close_page(),
            PageAction::Section(section) => {
                if let Some(page) = self.page.as_mut() {
                    if page.section != section {
                        page.section = section;
                        page.cursor = 0;
                        page.scroll = 0;
                        page.prompt = None;
                        page.confirm = None;
                    }
                    page.pane = Pane::List;
                    page.follow = true;
                }
                self.page_settle_cursor(1);
            }
            PageAction::Cycle(d) => self.page_cycle(d),
            PageAction::Move(d) => {
                let pane = self.page.as_ref().map(|p| p.pane);
                match pane {
                    Some(Pane::List) => {
                        let next = self.page.as_ref().map(|p| p.section.step(d));
                        if let (Some(next), Some(page)) = (next, self.page.as_mut())
                            && next != page.section
                        {
                            page.section = next;
                            page.cursor = 0;
                            page.scroll = 0;
                            page.prompt = None;
                            page.confirm = None;
                        }
                        self.page_settle_cursor(1);
                    }
                    Some(Pane::Rows) => {
                        let len = self.page_rows().len();
                        if let Some(page) = self.page.as_mut() {
                            page.confirm = None;
                            let i = (page.cursor as i64 + i64::from(d))
                                .clamp(0, len.saturating_sub(1) as i64);
                            page.cursor = i as usize;
                            page.follow = true;
                        }
                        self.page_settle_cursor(d);
                    }
                    None => {}
                }
            }
            PageAction::Activate => {
                if self.page.as_ref().is_some_and(|p| p.pane == Pane::List) {
                    if let Some(page) = self.page.as_mut() {
                        page.pane = Pane::Rows;
                        page.follow = true;
                    }
                    self.page_settle_cursor(1);
                } else if let Some(row) = self.page_cursor_row() {
                    self.page_activate(&row);
                }
            }
            PageAction::Step(d) => {
                if self.page.as_ref().is_some_and(|p| p.pane == Pane::List) {
                    if d > 0
                        && let Some(page) = self.page.as_mut()
                    {
                        page.pane = Pane::Rows;
                    }
                } else if let Some(row) = self.page_cursor_row() {
                    self.page_step_row(&row, d);
                }
            }
            PageAction::Row(i) => {
                if self.page_rows().get(i).is_some_and(PageRow::focusable) {
                    self.page_set_cursor(i);
                }
            }
            PageAction::Toggle(i) => {
                if let Some(PageRow::Setting(key)) = self.page_rows().get(i).cloned() {
                    self.page_set_cursor(i);
                    self.page_toggle(key, None);
                }
            }
            PageAction::Pick(i, option) => {
                if let Some(PageRow::Setting(key)) = self.page_rows().get(i).cloned() {
                    self.page_set_cursor(i);
                    self.page_choose(key, option);
                }
            }
            PageAction::Reset(i) => {
                if let Some(PageRow::Setting(key)) = self.page_rows().get(i).cloned() {
                    self.page_set_cursor(i);
                    self.change_setting(|s| key.reset(s));
                }
            }
            PageAction::ResetCursor => {
                if let Some(PageRow::Setting(key)) = self.page_cursor_row() {
                    if key.is_default(&self.settings) {
                        self.say(
                            format!("{} is at its default", key.label()),
                            ToastKind::Info,
                        );
                    } else {
                        self.change_setting(|s| key.reset(s));
                    }
                }
            }
            PageAction::ApplyTheme(id) => {
                let rows = self.page_rows();
                if let Some(i) = rows.iter().position(|r| *r == PageRow::Theme(id.clone())) {
                    self.page_set_cursor(i);
                }
                self.apply_theme(&id);
            }
            PageAction::RemoveTheme(id) => self.page_remove_start(&id),
            PageAction::RemoveCursor => {
                if let Some(PageRow::Theme(id)) = self.page_cursor_row() {
                    self.page_remove_start(&id);
                }
            }
            PageAction::Press(i) => {
                if let Some(row) = self.page_rows().get(i).cloned() {
                    self.page_set_cursor(i);
                    self.page_activate(&row);
                }
            }
            PageAction::PromptEdit(op) => {
                if let Some(p) = self.page.as_mut().and_then(|p| p.prompt.as_mut()) {
                    p.apply(op);
                }
            }
            PageAction::PromptSubmit => {
                let text = self
                    .page
                    .as_ref()
                    .and_then(|p| p.prompt.as_ref())
                    .map(|p| p.text().to_string());
                if let Some(text) = text
                    && self.import_theme(&text)
                    && let Some(page) = self.page.as_mut()
                {
                    page.prompt = None;
                }
            }
            PageAction::ConfirmYes => self.page_confirm(),
            PageAction::ConfirmNo => {
                if let Some(page) = self.page.as_mut() {
                    page.confirm = None;
                    page.prompt = None;
                }
            }
            PageAction::Scroll(d) => {
                if let Some(page) = self.page.as_mut() {
                    page.scroll = (page.scroll as i64 + i64::from(d)).max(0) as usize;
                    page.follow = false;
                }
            }
        }
    }

    /// `tab` on the page: the section list, the rows, then the sidebar when it shows.
    fn page_cycle(&mut self, d: i32) {
        let sidebar = self.layout.sidebar_mode() != SidebarMode::Hidden;
        let Some(page) = self.page.as_mut() else {
            return;
        };
        page.prompt = None;
        page.confirm = None;
        match (page.pane, d > 0) {
            (Pane::List, true) => page.pane = Pane::Rows,
            (Pane::Rows, false) => page.pane = Pane::List,
            (Pane::Rows, true) | (Pane::List, false) if sidebar => {
                self.focus = Focus::Sidebar;
            }
            (Pane::Rows, true) => page.pane = Pane::List,
            (Pane::List, false) => page.pane = Pane::Rows,
        }
        page.follow = true;
        self.page_settle_cursor(1);
    }

    /// The tab key from the sidebar while the page shows: back onto the page.
    pub(crate) fn focus_page_from_sidebar(&mut self, d: i32) {
        if let Some(page) = self.page.as_mut() {
            page.pane = if d > 0 { Pane::List } else { Pane::Rows };
        }
        self.focus = Focus::Settings;
    }

    /// The paste of a path into the import prompt: one line, as typed.
    pub(crate) fn page_paste(&mut self, text: &str) -> bool {
        match self.page.as_mut().and_then(|p| p.prompt.as_mut()) {
            Some(prompt) if self.page_shown => {
                prompt.apply(EditOp::Paste(text.replace(['\r', '\n', '\t'], " ")));
                true
            }
            _ => false,
        }
    }
}
