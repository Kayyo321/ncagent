/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Tabs: several conversations open and running at once (PROJECT.md, D62).
//!
//! `app_t::sessions` is the tab list, in tab order, and `app_t::active` the shown tab. Every
//! tab is pumped on every frame, so a tab that is not shown keeps running. A conversation is
//! never open in two tabs. The shown tab's draft lives in the composer; switching tabs swaps
//! it with the draft kept in the other tab, so each tab keeps its own text, scroll position and
//! expanded blocks.
//!
//! Closing a busy tab cancels its run first. The session is then kept in `closing` and pumped
//! until it stops, so its log ends with the cancel, and only then dropped.
//!
//! With `restore_tabs` the open tabs and the shown one are saved through the runtime whenever
//! they change, and reopened at the next start (D63).

use std::collections::HashSet;

use harness::event::SessionEvent;
use harness::settings::{MaxSavedTabs, OpenIn};
use harness::store::valid_session_id;

use crate::app::{Attention, ToastKind, app_t, live_session_t};
use crate::composer::composer_t;
use crate::input::{Focus, TabAction};

/// Where a conversation opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenPlace {
    /// As the `open_in` setting says: a new tab, or the shown tab when it is idle.
    Setting,
    /// Always a tab of its own.
    NewTab,
}

impl app_t {
    /// The tab that holds this conversation.
    pub fn tab_of(&self, id: &str) -> Option<usize> {
        self.sessions.iter().position(|s| s.session.id() == id)
    }

    /// The id of the shown conversation, as a list for the functions that act on several.
    pub fn shown_ids(&self) -> Vec<String> {
        self.active()
            .map(|s| s.session.id().to_string())
            .into_iter()
            .collect()
    }

    /// True when the conversation is working: busy in a tab, or still stopping after its tab
    /// closed. Such a conversation cannot be deleted (D59).
    pub fn is_running(&self, id: &str) -> bool {
        self.sessions
            .iter()
            .chain(self.closing.iter())
            .any(|s| s.session.id() == id && s.busy())
    }

    /// True when the shown tab is a New chat nobody has used, draft included.
    pub fn active_untouched(&self) -> bool {
        self.active()
            .is_some_and(|s| s.untouched() && self.composer.is_empty())
    }

    /// Show tab `index`, keeping the draft of the tab that was shown.
    pub fn switch_tab(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        // The bar brings the shown tab into view on the next frame, even after a scroll.
        self.tab_shown = None;
        self.hide_page();
        if index != self.active {
            let draft = std::mem::take(&mut self.composer);
            if let Some(out) = self.sessions.get_mut(self.active) {
                out.draft = draft;
            }
            self.active = index;
            if let Some(shown) = self.sessions.get_mut(index) {
                self.composer = std::mem::take(&mut shown.draft);
            }
        }
        if let Some(shown) = self.sessions.get_mut(index) {
            shown.attention = Attention::None;
        }
        self.sync_widgets();
        self.rebuild_sidebar_items();
        self.tabs_changed();
    }

    fn push_tab(&mut self, live: live_session_t) {
        self.sessions.push(live);
        self.switch_tab(self.sessions.len() - 1);
    }

    /// A new chat in a tab of its own (`ctrl+t`, the `+` chip, `/tab`).
    pub fn new_tab(&mut self) {
        match self.runtime.new_session() {
            Ok(session) => {
                let mut live = live_session_t::new(session);
                live.logo_from = self.animate.then_some(self.clock);
                self.apply_approval(&mut live);
                self.push_tab(live);
                self.focus = Focus::Input;
                self.drawer_open = false;
                self.summaries_dirty = true;
            }
            Err(err) => self.say(
                format!("could not start a session: {err}"),
                ToastKind::Error,
            ),
        }
    }

    /// New chat (`ctrl+n`, the button, `/new`): a new tab, unless the shown tab is a New chat
    /// nobody has used, which is used again and plays its title card once more.
    pub fn new_chat(&mut self) {
        if !self.active_untouched() {
            self.new_tab();
            return;
        }
        self.hide_page();
        let (animate, clock) = (self.animate, self.clock);
        if let Some(live) = self.active_mut() {
            live.logo_from = animate.then_some(clock);
            live.scroll.to_top();
        }
        self.focus = Focus::Input;
        self.drawer_open = false;
    }

    /// Read a conversation back from the runtime, or take it back from `closing`.
    fn load_live(&mut self, id: &str) -> Result<live_session_t, String> {
        if let Some(pos) = self.closing.iter().position(|s| s.session.id() == id) {
            return Ok(self.closing.remove(pos));
        }
        let opened = self.runtime.open_session(id).map_err(|e| e.to_string())?;
        let mut live = live_session_t::new(opened.session);
        for e in &opened.events {
            live.transcript.fold(e, None);
        }
        live.phase = live.session.phase();
        live.queued = live.session.queued();
        if opened.skipped_lines > 0 {
            self.say(
                format!(
                    "skipped {} unreadable line(s) of the saved session",
                    opened.skipped_lines
                ),
                ToastKind::Warning,
            );
        }
        Ok(live)
    }

    /// Open a conversation: its tab when it is open, else a tab as `place` says. A New chat
    /// nobody has used is replaced rather than left behind. With `open_in` set to the current
    /// tab, an idle shown tab with an empty draft is replaced too.
    pub fn open_conversation(&mut self, id: &str, place: OpenPlace) {
        self.drawer_open = false;
        self.hide_page();
        if let Some(i) = self.tab_of(id) {
            self.switch_tab(i);
            return;
        }
        let mut live = match self.load_live(id) {
            Ok(live) => live,
            Err(err) => {
                self.say(
                    format!("could not open the session: {err}"),
                    ToastKind::Error,
                );
                return;
            }
        };
        self.apply_approval(&mut live);
        let reuse = self.active().is_some_and(|shown| {
            self.composer.is_empty()
                && !shown.busy()
                && (shown.untouched()
                    || (place == OpenPlace::Setting && self.settings.open_in == OpenIn::CurrentTab))
        });
        match self.sessions.get_mut(self.active) {
            Some(slot) if reuse => {
                *slot = live;
                self.sync_widgets();
                self.rebuild_sidebar_items();
                self.tabs_changed();
            }
            _ => self.push_tab(live),
        }
    }

    /// Close tab `index`. A busy tab is cancelled first and named in a toast. The last tab is
    /// replaced by a New chat, so there is always one; a lone unused New chat stays.
    pub fn close_tab(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        if self.sessions.len() == 1 && self.active_untouched() {
            self.say("this is the only tab", ToastKind::Info);
            return;
        }
        let was_active = index == self.active;
        if was_active {
            // The closed tab's draft goes with it.
            self.composer = composer_t::new();
        }
        let mut live = self.sessions.remove(index);
        if live.busy() {
            let title = self.conversation_title(live.session.id());
            live.session.cancel();
            live.phase = live.session.phase();
            self.closing.push(live);
            self.say(format!("stopped “{title}”"), ToastKind::Info);
        }
        if self.sessions.is_empty() {
            self.active = 0;
            self.new_tab();
            return;
        }
        if was_active {
            let next = index.min(self.sessions.len() - 1);
            self.active = next;
            if let Some(shown) = self.sessions.get_mut(next) {
                self.composer = std::mem::take(&mut shown.draft);
                shown.attention = Attention::None;
            }
        } else if index < self.active {
            self.active -= 1;
        }
        self.sync_widgets();
        self.rebuild_sidebar_items();
        self.tabs_changed();
    }

    /// Close the tabs of these conversations, wherever they are.
    pub fn close_tabs_of(&mut self, ids: &[String]) {
        for id in ids {
            if let Some(i) = self.tab_of(id) {
                self.close_tab(i);
            }
        }
    }

    pub(crate) fn apply_tab(&mut self, action: TabAction) {
        let n = self.sessions.len();
        // The settings page, while open, is one more tab after the last (D65).
        let page = usize::from(self.page.is_some());
        match action {
            TabAction::New => self.new_tab(),
            TabAction::Close(i) => self.close_tab(i),
            TabAction::CloseActive if self.page_shown => self.close_page(),
            TabAction::CloseActive => self.close_tab(self.active),
            TabAction::Select(i) if i < n => self.switch_tab(i),
            TabAction::Select(i) => self.say(format!("there is no tab {}", i + 1), ToastKind::Info),
            TabAction::Step(d) if n + page > 1 => {
                let at = if self.page_shown { n } else { self.active };
                let next = (at as i64 + i64::from(d)).rem_euclid((n + page) as i64) as usize;
                if next == n {
                    self.open_page(None);
                } else {
                    self.switch_tab(next);
                }
            }
            TabAction::Step(_) => {}
            TabAction::Last if page > 0 => self.open_page(None),
            TabAction::Last => self.switch_tab(n.saturating_sub(1)),
            TabAction::Scroll(d) => {
                let top = n.saturating_sub(1) as i64;
                self.tab_scroll = (self.tab_scroll as i64 + i64::from(d)).clamp(0, top) as usize;
            }
        }
    }

    /// Pump the tabs that were closed while running until they stop, then drop them. True when
    /// one of them ended its turn.
    pub(crate) fn pump_closing(&mut self) -> bool {
        let mut ended = false;
        let clock = self.clock;
        for live in self.closing.iter_mut() {
            for e in live.session.pump() {
                ended |= matches!(e.event, SessionEvent::TurnEnd { .. });
                live.transcript.fold(&e, Some(clock));
            }
            live.phase = live.session.phase();
        }
        self.closing.retain(live_session_t::busy);
        ended
    }

    /// Save the open tabs and the shown one when `restore_tabs` is on and they changed. At most
    /// `MaxSavedTabs` are kept, around the shown tab.
    pub(crate) fn tabs_changed(&mut self) {
        if !self.settings.restore_tabs {
            return;
        }
        let mut ids: Vec<String> = Vec::new();
        for live in &self.sessions {
            let id = live.session.id();
            if valid_session_id(id) && !ids.iter().any(|known| known == id) {
                ids.push(id.to_string());
            }
        }
        let shown = self.active().map(|s| s.session.id().to_string());
        let at = shown
            .as_ref()
            .and_then(|id| ids.iter().position(|known| known == id))
            .unwrap_or(0);
        if ids.len() > MaxSavedTabs {
            let start = (at + 1).saturating_sub(MaxSavedTabs);
            ids = ids.into_iter().skip(start).take(MaxSavedTabs).collect();
        }
        let shown = shown.filter(|id| ids.contains(id));
        if ids == self.settings.open_tabs && shown == self.settings.active_tab {
            return;
        }
        self.update_settings_quietly(|s| {
            s.open_tabs = ids;
            s.active_tab = shown;
        });
    }

    /// Reopen the tabs saved by the last run. Conversations that no longer exist or do not
    /// open are skipped, and one toast counts them.
    pub(crate) fn restore_tabs(&mut self) {
        if !self.settings.restore_tabs || self.settings.open_tabs.is_empty() {
            return;
        }
        let wanted = self.settings.open_tabs.clone();
        let shown = self.settings.active_tab.clone();
        let known: HashSet<String> = self.summaries.iter().map(|s| s.id.clone()).collect();
        let mut skipped = 0usize;
        for id in wanted.iter().take(MaxSavedTabs) {
            if !known.contains(id) || self.tab_of(id).is_some() {
                skipped += 1;
                continue;
            }
            match self.load_live(id) {
                Ok(mut live) => {
                    self.apply_approval(&mut live);
                    self.sessions.push(live);
                }
                Err(_) => skipped += 1,
            }
        }
        self.active = shown
            .and_then(|id| self.tab_of(&id))
            .unwrap_or(self.sessions.len().saturating_sub(1));
        if skipped > 0 {
            self.say(
                format!("{skipped} saved tab(s) could not be reopened"),
                ToastKind::Warning,
            );
        }
    }
}
