/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The app state machine. It owns every opened session, folds their events, maps input to
//! actions and actions to session calls. It never touches the terminal: drawing goes to a
//! ratatui `Frame` (see `draw`), and bytes for the terminal (OSC 52) are queued in `outputs`
//! for the loop to write.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use harness::api::{SessionPhase, runtime_info_t, runtime_t, session_t};
use harness::approval::{ApprovalDecision, ApprovalMode};
use harness::event::SessionEvent;
use harness::organize::organization_t;
use harness::settings::settings_t;
use harness::store::session_summary_t;
use ratatui::layout::{Position, Rect};

use crate::clipboard::osc52;
use crate::commands::{self, Command};
use crate::composer::{EditOp, composer_t};
use crate::conversation::delete_plan_t;
use crate::folders::{MoveSource, grab_t, prompt_t};
use crate::input::{
    Action, Focus, Modal, MouseIntent, hit_registry_t, key_ctx_t, map_key, map_mouse, wheel_areas_t,
};
use crate::layout::{SidebarMode, layout_t};
use crate::menu::menu_t;
use crate::selection::{ClickKind, selection_t};
use crate::settings_page::{PageKeys, settings_page_t};
use crate::sidebar::{Activity, SidebarItemKind, sidebar_item_t};
use crate::text::{sanitize, truncate};
use crate::theme::{ColorMode, theme_t};
use crate::theme_set::theme_set_t;
use crate::transcript::transcript_view_t;
use crate::view_model::transcript_t;

/// A lone Escape waits this long for a following event before it acts (D48).
pub const EscapeDelayMs: u64 = 70;
/// A second Ctrl+C within this time quits.
pub const QuitArmMs: u64 = 2000;
/// The session list is read again at most this often, and after each turn ends.
pub const SummaryRefreshMs: u64 = 5000;
const ToastMs: u64 = 3200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct app_options_t {
    pub animate: bool,
    pub mouse: bool,
    pub color: ColorMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct toast_t {
    pub text: String,
    pub kind: ToastKind,
    pub until: u64,
}

/// Scroll position of a transcript. `follow` keeps the bottom in view as content arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct scroll_t {
    pub top: usize,
    pub follow: bool,
    pub unseen: usize,
    /// Total rows and viewport height of the last frame.
    pub total: usize,
    pub viewport: usize,
    /// The block and row within it at the top of the viewport, kept across re-wraps.
    pub anchor: Option<(u64, usize)>,
}

impl Default for scroll_t {
    fn default() -> Self {
        scroll_t {
            top: 0,
            follow: true,
            unseen: 0,
            total: 0,
            viewport: 0,
            anchor: None,
        }
    }
}

impl scroll_t {
    pub fn max_top(&self) -> usize {
        self.total.saturating_sub(self.viewport)
    }

    pub fn scroll_by(&mut self, delta: i64) {
        let top = (self.top as i64).saturating_add(delta).max(0) as usize;
        self.top = top.min(self.max_top());
        self.follow = self.top >= self.max_top();
        if self.follow {
            self.unseen = 0;
        }
        self.anchor = None;
    }

    pub fn to_top(&mut self) {
        self.top = 0;
        self.follow = self.max_top() == 0;
        self.anchor = None;
    }

    pub fn to_bottom(&mut self) {
        self.top = self.max_top();
        self.follow = true;
        self.unseen = 0;
        self.anchor = None;
    }

    /// Thumb position from a press at `offset` of a `track` rows tall.
    pub fn to_track(&mut self, offset: u16, track: u16) {
        let span = usize::from(track.saturating_sub(1)).max(1);
        let top = usize::from(offset).saturating_mul(self.max_top()) / span;
        self.top = top.min(self.max_top());
        self.follow = self.top >= self.max_top();
        if self.follow {
            self.unseen = 0;
        }
        self.anchor = None;
    }
}

/// Whether a tab that is not shown has news for the user (D62).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    None,
    /// A turn ended while the tab was not shown.
    Finished,
    /// An approval or a question arrived while the tab was not shown.
    NeedsYou,
}

/// One open tab: a session and everything the UI keeps for it. Each tab keeps its own draft,
/// scroll position, selected block and expanded blocks, so switching tabs loses none of them.
pub struct live_session_t {
    pub session: Box<dyn session_t>,
    pub transcript: transcript_t,
    pub view: transcript_view_t,
    pub scroll: scroll_t,
    pub selected_block: Option<u64>,
    pub phase: SessionPhase,
    pub queued: Vec<String>,
    /// Approvals and questions already answered from this UI, so a double press sends once.
    pub resolved: HashSet<String>,
    /// App-clock time the title card's logo started its reveal (D56). `None` when it shows
    /// settled: animation off, a replayed session, the reveal done, or the card off screen.
    pub logo_from: Option<u64>,
    /// The composer text of this tab while another tab is shown. The shown tab's text lives in
    /// `app_t::composer`, and this field is empty then.
    pub draft: composer_t,
    pub attention: Attention,
}

impl live_session_t {
    pub(crate) fn new(session: Box<dyn session_t>) -> Self {
        let phase = session.phase();
        live_session_t {
            session,
            transcript: transcript_t::new(),
            view: transcript_view_t::default(),
            scroll: scroll_t::default(),
            selected_block: None,
            phase,
            queued: Vec::new(),
            resolved: HashSet::new(),
            logo_from: None,
            draft: composer_t::new(),
            attention: Attention::None,
        }
    }

    pub fn busy(&self) -> bool {
        self.phase.busy()
    }

    /// True once the user sent a message in this conversation.
    pub fn has_messages(&self) -> bool {
        self.transcript
            .blocks
            .iter()
            .any(|b| matches!(b.kind, crate::view_model::BlockKind::User { .. }))
    }

    /// A New chat nobody used yet: idle, with no message and nothing queued. Its draft is
    /// checked by the caller, because the shown tab's draft is in the composer.
    pub fn untouched(&self) -> bool {
        !self.busy() && !self.has_messages() && self.queued.is_empty()
    }

    pub fn title(&self) -> String {
        self.transcript
            .blocks
            .iter()
            .find_map(|b| match &b.kind {
                crate::view_model::BlockKind::User { text, .. } => {
                    Some(sanitize(text.lines().next().unwrap_or(""), false))
                }
                _ => None,
            })
            .unwrap_or_else(|| "New chat".to_string())
    }

    pub fn activity(&self) -> Activity {
        match self.phase {
            SessionPhase::Idle => Activity::Idle,
            SessionPhase::AwaitingApproval | SessionPhase::AwaitingAnswer => Activity::Waiting,
            SessionPhase::RunningTool => Activity::Tool,
            SessionPhase::Generating | SessionPhase::Cancelling => Activity::Generating,
        }
    }
}

pub struct app_t {
    pub runtime: Arc<dyn runtime_t>,
    pub info: runtime_info_t,
    /// The palette every frame draws with: the theme of the settings, or the page's preview.
    pub theme: theme_t,
    /// The id `theme` was resolved from. Cleared to make the next frame resolve it again.
    pub theme_id: String,
    /// The built-in themes and the imported ones (D64).
    pub themes: theme_set_t,
    pub options: app_options_t,
    /// The user's settings as last loaded or saved (D63).
    pub settings: settings_t,
    /// False when the settings file did not load: the defaults are in use, and nothing the app
    /// saves on its own overwrites the file (D70).
    pub settings_ok: bool,
    /// The open tabs, in tab order (D62).
    pub sessions: Vec<live_session_t>,
    /// The shown tab.
    pub active: usize,
    /// Tabs closed while they ran: cancelled, and pumped until they stop so their logs end
    /// cleanly, then dropped.
    pub closing: Vec<live_session_t>,
    /// The first tab drawn when the tab bar overflows.
    pub tab_scroll: usize,
    /// The tab the bar last scrolled into view, so a change of tab brings the new one in.
    pub tab_shown: Option<usize>,
    pub summaries: Vec<session_summary_t>,
    summaries_at: u64,
    pub(crate) summaries_dirty: bool,
    pub composer: composer_t,
    pub focus: Focus,
    pub sidebar_pinned: bool,
    pub drawer_open: bool,
    pub sidebar_items: Vec<sidebar_item_t>,
    pub sidebar_sel: usize,
    pub sidebar_scroll: usize,
    /// The rows of the conversation tree in the last frame, for drag auto-scroll.
    pub sidebar_list: Rect,
    /// The folders and which conversation is in which (D50), as of the last change.
    pub organization: organization_t,
    /// Folded folders, by id. UI state only: every folder starts unfolded (D54).
    pub folded: BTreeSet<String>,
    /// The folder-name prompt, when open.
    pub prompt: Option<prompt_t>,
    /// The folder whose delete waits for confirmation in the sidebar.
    pub confirm: Option<String>,
    /// The delete of conversations (and folders) that waits in the dialog (D59).
    pub delete_plan: Option<delete_plan_t>,
    /// The rows selected in the sidebar (D60).
    pub selection: selection_t,
    /// The context menu, when open (D61).
    pub menu: Option<menu_t>,
    /// Where the menu and the delete dialog were drawn in the last frame. A press outside
    /// them closes them.
    pub overlay_areas: Vec<Rect>,
    /// Pick-up mode: what is being moved (D55).
    pub pickup: Option<MoveSource>,
    /// A sidebar row held by the mouse.
    pub grab: Option<grab_t>,
    pub help_open: bool,
    pub help_scroll: usize,
    /// The settings page, while it is open (D65).
    pub page: Option<settings_page_t>,
    /// The page is the view of the main panel. It can be open and hidden behind a tab.
    pub page_shown: bool,
    pub approval_sel: usize,
    approval_for: Option<String>,
    pub question_sel: usize,
    question_for: Option<String>,
    pub answer: composer_t,
    pub clock: u64,
    pub now_ms: u64,
    pending_escape: Option<u64>,
    quit_armed_until: Option<u64>,
    pub quit: bool,
    pub toast: Option<toast_t>,
    pub hits: hit_registry_t,
    pub hover: Option<(u16, u16)>,
    pub dragging: Option<Rect>,
    pub outputs: Vec<Vec<u8>>,
    pub size: (u16, u16),
    pub layout: layout_t,
    /// The generation effect, the logo reveal and the pulses: allowed by the command line and
    /// turned on in the settings (D63).
    pub animate: bool,
    pub redraw: bool,
}

/// Why the app could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct start_error_t {
    pub message: String,
}

impl app_t {
    pub fn new(
        runtime: Arc<dyn runtime_t>,
        options: app_options_t,
        clock: u64,
        now_ms: u64,
    ) -> Result<Self, start_error_t> {
        let info = runtime.info().clone();
        let mut app = app_t {
            runtime,
            info,
            theme: theme_t::dark(),
            theme_id: String::new(),
            themes: theme_set_t::default(),
            animate: options.animate,
            options,
            settings: settings_t::default(),
            settings_ok: false,
            sessions: Vec::new(),
            active: 0,
            closing: Vec::new(),
            tab_scroll: 0,
            tab_shown: None,
            summaries: Vec::new(),
            summaries_at: 0,
            summaries_dirty: true,
            composer: composer_t::new(),
            focus: Focus::Input,
            sidebar_pinned: true,
            drawer_open: false,
            sidebar_items: Vec::new(),
            sidebar_sel: 0,
            sidebar_scroll: 0,
            sidebar_list: Rect::default(),
            organization: organization_t::new(),
            folded: BTreeSet::new(),
            prompt: None,
            confirm: None,
            delete_plan: None,
            selection: selection_t::default(),
            menu: None,
            overlay_areas: Vec::new(),
            pickup: None,
            grab: None,
            help_open: false,
            help_scroll: 0,
            page: None,
            page_shown: false,
            approval_sel: 0,
            approval_for: None,
            question_sel: 0,
            question_for: None,
            answer: composer_t::new(),
            clock,
            now_ms,
            pending_escape: None,
            quit_armed_until: None,
            quit: false,
            toast: None,
            hits: hit_registry_t::default(),
            hover: None,
            dragging: None,
            outputs: Vec::new(),
            size: (0, 0),
            layout: layout_t::default(),
            redraw: false,
        };
        app.load_settings();
        let notes = app.load_themes();
        app.settings_changed();
        if !notes.is_empty() {
            app.say_more(notes.join(" · "), ToastKind::Warning);
        }
        app.refresh_summaries();
        app.load_organization();
        app.restore_tabs();
        if app.sessions.is_empty() {
            let session = app.runtime.new_session().map_err(|e| start_error_t {
                message: format!("could not start a session: {e}"),
            })?;
            let mut first = live_session_t::new(session);
            first.logo_from = app.animate.then_some(clock);
            app.apply_approval(&mut first);
            app.sessions.push(first);
            app.active = 0;
        }
        app.tabs_changed();
        app.rebuild_sidebar_items();
        Ok(app)
    }

    pub fn active(&self) -> Option<&live_session_t> {
        self.sessions.get(self.active)
    }

    pub(crate) fn active_mut(&mut self) -> Option<&mut live_session_t> {
        self.sessions.get_mut(self.active)
    }

    pub fn busy(&self) -> bool {
        self.active().is_some_and(live_session_t::busy)
    }

    pub fn simulated(&self) -> bool {
        self.info.identity.simulated()
    }

    pub fn mode(&self) -> ApprovalMode {
        self.active()
            .and_then(|s| s.transcript.mode)
            .unwrap_or(self.info.approval_mode)
    }

    pub fn say(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toast = Some(toast_t {
            text: text.into(),
            kind,
            until: self.clock.saturating_add(ToastMs),
        });
    }

    /// A toast that keeps the one showing, for messages at start that come one after another.
    pub(crate) fn say_more(&mut self, text: impl Into<String>, kind: ToastKind) {
        let text = text.into();
        match self.toast.take() {
            Some(t) if self.clock <= t.until => {
                let kind = match (t.kind, kind) {
                    (ToastKind::Error, _) | (_, ToastKind::Error) => ToastKind::Error,
                    (ToastKind::Warning, _) | (_, ToastKind::Warning) => ToastKind::Warning,
                    _ => ToastKind::Info,
                };
                self.say(format!("{} · {text}", t.text), kind);
            }
            _ => self.say(text, kind),
        }
    }

    pub(crate) fn refresh_summaries(&mut self) {
        match self.runtime.list_sessions() {
            Ok(list) => self.summaries = list,
            Err(err) => self.say(
                format!("could not list sessions: {err}"),
                ToastKind::Warning,
            ),
        }
        self.summaries_at = self.clock;
        self.summaries_dirty = false;
    }

    /// Advance the app clock, pump every opened session and fold what arrived.
    pub fn tick(&mut self, clock: u64, now_ms: u64) {
        self.clock = clock;
        self.now_ms = now_ms;
        let mut turn_ended = false;
        let active = self.active;
        // Every tab is pumped on every frame, shown or not, so several conversations run at
        // once (D62). A tab that is not shown marks what it needs from the user.
        for (index, live) in self.sessions.iter_mut().enumerate() {
            let events = live.session.pump();
            for e in &events {
                live.transcript.fold(e, Some(clock));
                if matches!(
                    e.event,
                    SessionEvent::TurnEnd { .. } | SessionEvent::UserMessage { .. }
                ) {
                    turn_ended = true;
                }
                if index != active {
                    match e.event {
                        SessionEvent::ApprovalRequested { .. }
                        | SessionEvent::QuestionAsked { .. } => {
                            live.attention = Attention::NeedsYou;
                        }
                        SessionEvent::TurnEnd { .. } if live.attention != Attention::NeedsYou => {
                            live.attention = Attention::Finished;
                        }
                        _ => {}
                    }
                }
            }
            live.phase = live.session.phase();
            live.queued = live.session.queued();
        }
        if self.pump_closing() {
            turn_ended = true;
        }
        if turn_ended {
            self.summaries_dirty = true;
        }
        if self.summaries_dirty || clock.saturating_sub(self.summaries_at) >= SummaryRefreshMs {
            self.refresh_summaries();
        }
        self.sync_widgets();
        if let Some(at) = self.pending_escape
            && clock.saturating_sub(at) >= EscapeDelayMs
        {
            self.pending_escape = None;
            self.resolve_escape();
        }
        if self.quit_armed_until.is_some_and(|t| clock > t) {
            self.quit_armed_until = None;
        }
        if self.toast.as_ref().is_some_and(|t| clock > t.until) {
            self.toast = None;
        }
        self.rebuild_sidebar_items();
    }

    /// Reset the approval and question widgets when a new request appears.
    pub(crate) fn sync_widgets(&mut self) {
        let (approval, question) = match self.active() {
            Some(s) => (
                s.transcript
                    .pending_approval()
                    .map(|(_, t)| t.tool_call_id.clone()),
                s.transcript
                    .pending_question()
                    .map(|(_, q)| q.question_id.clone()),
            ),
            None => (None, None),
        };
        if approval != self.approval_for {
            self.approval_for = approval;
            self.approval_sel = 0;
        }
        if question != self.question_for {
            self.question_for = question;
            self.question_sel = 0;
            self.answer = composer_t::new();
        }
    }

    pub fn modal(&self) -> Modal {
        if self.help_open {
            return Modal::Help;
        }
        if self.menu.is_some() {
            return Modal::Menu;
        }
        if self.delete_plan.is_some() {
            return Modal::Confirm;
        }
        if let Some(m) = self.sidebar_modal() {
            return m;
        }
        self.pending_modal()
    }

    /// The approval or question waiting in the active session, whatever else is open. The dock
    /// shows its widget even while a sidebar prompt has the keys.
    pub fn pending_modal(&self) -> Modal {
        // The conversation under the settings page waits in its tab; the page has the keys.
        if self.page_shown {
            return Modal::None;
        }
        let Some(s) = self.active() else {
            return Modal::None;
        };
        if let Some((_, t)) = s.transcript.pending_approval()
            && !s.resolved.contains(&t.tool_call_id)
        {
            return Modal::Approval;
        }
        if let Some((_, q)) = s.transcript.pending_question()
            && !s.resolved.contains(&q.question_id)
        {
            return Modal::Question;
        }
        Modal::None
    }

    pub fn popup(&self) -> Vec<commands::command_t> {
        if self.focus != Focus::Input || self.modal() != Modal::None {
            return Vec::new();
        }
        commands::completions(self.composer.text())
    }

    fn key_ctx(&self) -> key_ctx_t {
        let option_count = self
            .active()
            .and_then(|s| s.transcript.pending_question())
            .map(|(_, q)| q.options.len())
            .unwrap_or(0);
        key_ctx_t {
            focus: self.focus,
            modal: self.modal(),
            input_empty: self.composer.is_empty(),
            popup_open: !self.popup().is_empty(),
            answer_empty: self.answer.is_empty(),
            option_count,
            page: self
                .page
                .as_ref()
                .map(settings_page_t::keys)
                .unwrap_or(PageKeys::Browse),
        }
    }

    /// Handle one terminal event.
    pub fn handle_event(&mut self, event: &Event) {
        match event {
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    return;
                }
                if key.code != KeyCode::Esc {
                    // Input right after an Escape means the Escape was part of something else
                    // (crossterm issue 854): drop it.
                    self.pending_escape = None;
                } else if self.grab.as_ref().is_some_and(|g| g.moved) {
                    // A drag is live: the pointer keeps moving, and its events would drop a
                    // delayed Escape, so this one acts at once.
                    self.cancel_drag();
                    return;
                }
                if let Some(action) = map_key(key, &self.key_ctx()) {
                    if self.menu.is_some()
                        && !matches!(action, Action::Menu(_) | Action::Escape | Action::Interrupt)
                    {
                        self.menu = None;
                    }
                    self.apply(action);
                }
            }
            Event::Mouse(mouse) => {
                self.pending_escape = None;
                if !self.options.mouse {
                    return;
                }
                // A press outside the menu or the delete dialog closes it and does nothing else.
                // A right press then opens the menu of what is under it.
                if let MouseEventKind::Down(button) = mouse.kind
                    && (self.menu.is_some() || self.delete_plan.is_some())
                    && !self
                        .overlay_areas
                        .iter()
                        .any(|r| r.contains(Position::new(mouse.column, mouse.row)))
                {
                    self.menu = None;
                    self.delete_plan = None;
                    if button != MouseButton::Right {
                        return;
                    }
                }
                let areas = wheel_areas_t {
                    sidebar: if self.layout.sidebar_mode() == SidebarMode::Hidden {
                        Rect::default()
                    } else {
                        self.layout.sidebar
                    },
                    tabs: self.layout.tabs,
                };
                match map_mouse(mouse, &self.hits, self.dragging, areas) {
                    MouseIntent::Act(action) => self.apply(action),
                    MouseIntent::StartDrag(action) => {
                        self.dragging = self.hits.at(mouse.column, mouse.row).map(|h| h.rect);
                        self.apply(action);
                    }
                    MouseIntent::Drag(action) => self.apply(action),
                    MouseIntent::EndDrag => self.dragging = None,
                    MouseIntent::Hover(x, y) => self.hover = Some((x, y)),
                    MouseIntent::Grab(index, mods) => {
                        self.hover = Some((mouse.column, mouse.row));
                        self.grab_row(index, ClickKind::of(mods));
                    }
                    MouseIntent::Context(x, y) => {
                        self.hover = Some((x, y));
                        self.context_at(x, y);
                    }
                    MouseIntent::DragAt(x, y) => self.drag_to(x, y),
                    MouseIntent::Release(x, y) => self.release_at(x, y),
                    MouseIntent::Nothing => {}
                }
            }
            Event::Paste(text) => {
                self.pending_escape = None;
                if self.focus == Focus::Settings {
                    self.page_paste(text);
                    return;
                }
                match self.modal() {
                    Modal::Question => {
                        self.answer.apply(EditOp::Paste(text.clone()));
                    }
                    Modal::Prompt => {
                        let line = text.replace(['\r', '\n', '\t'], " ");
                        if let Some(p) = self.prompt.as_mut() {
                            p.editor.apply(EditOp::Paste(line));
                        }
                    }
                    Modal::None => {
                        self.focus = Focus::Input;
                        self.composer.apply(EditOp::Paste(text.clone()));
                    }
                    _ => {}
                }
            }
            Event::Resize(w, h) => self.resize(*w, *h),
            Event::FocusGained | Event::FocusLost => {}
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        if (w, h) == self.size {
            return;
        }
        // Keep the block at the top of the viewport in view across the re-wrap.
        for live in &mut self.sessions {
            if !live.scroll.follow
                && let Some(i) = live.view.block_at(live.scroll.top)
                && let (Some(block), Some(start)) =
                    (live.transcript.blocks.get(i), live.view.starts.get(i))
            {
                live.scroll.anchor = Some((block.id, live.scroll.top.saturating_sub(*start)));
            }
        }
        self.size = (w, h);
        if !self.can_dock() {
            self.drawer_open = self.drawer_open && self.focus == Focus::Sidebar;
            if self.sidebar_modal().is_some() {
                // The prompt, confirmation or pick-up stays in view in the drawer.
                self.drawer_open = true;
                self.focus = Focus::Sidebar;
            }
        }
    }

    fn resolve_escape(&mut self) {
        if self.help_open {
            self.help_open = false;
            return;
        }
        if self.menu.is_some() {
            self.menu_back();
            return;
        }
        if self.delete_plan.take().is_some() {
            return;
        }
        if self.cancel_sidebar_modal() {
            return;
        }
        if self.focus == Focus::Settings && self.page_escape() {
            return;
        }
        if self.focus == Focus::Sidebar && self.clear_selection() {
            self.rebuild_sidebar_items();
            return;
        }
        if self.page_shown {
            // The page is the view: esc closes the drawer over it, then the page, and never
            // stops the turn of the conversation under it.
            if self.drawer_open {
                self.drawer_open = false;
                self.focus = Focus::Settings;
            } else {
                self.close_page();
            }
            return;
        }
        if self.busy() {
            self.cancel_turn();
            return;
        }
        if self.drawer_open {
            self.drawer_open = false;
            self.focus = self.main_focus();
            return;
        }
        if !self.popup().is_empty() {
            self.composer.apply(EditOp::Clear);
            return;
        }
        self.focus = match self.focus {
            Focus::Input => {
                self.select_last_block();
                Focus::Transcript
            }
            _ => Focus::Input,
        };
    }

    fn cancel_turn(&mut self) {
        if let Some(s) = self.active_mut()
            && s.busy()
        {
            s.session.cancel();
        }
    }

    fn select_last_block(&mut self) {
        if let Some(s) = self.active_mut()
            && s.selected_block.is_none()
        {
            s.selected_block = s.transcript.blocks.last().map(|b| b.id);
        }
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Edit(op) => {
                self.focus = Focus::Input;
                if self.modal() != Modal::None {
                    return;
                }
                self.composer.apply(op);
            }
            Action::TypeInto(c) => {
                self.focus = Focus::Input;
                self.composer.apply(EditOp::Insert(c));
            }
            Action::Submit => self.submit(),
            Action::Escape => self.pending_escape = Some(self.clock),
            Action::Interrupt => {
                if self.menu.take().is_some() || self.delete_plan.take().is_some() {
                    return;
                }
                if self.focus == Focus::Settings
                    && let Some(page) = self.page.as_mut()
                    && (page.prompt.take().is_some() || page.confirm.take().is_some())
                {
                    return;
                }
                if self.cancel_sidebar_modal() {
                    return;
                }
                if self.busy() {
                    self.cancel_turn();
                } else if !self.composer.is_empty() {
                    self.composer.apply(EditOp::Clear);
                } else if self.quit_armed_until.is_some() {
                    self.quit = true;
                } else {
                    self.quit_armed_until = Some(self.clock.saturating_add(QuitArmMs));
                    self.say("press ctrl+c again to quit", ToastKind::Info);
                }
            }
            Action::Cancel => {
                if self.busy() {
                    self.cancel_turn();
                } else {
                    self.say("nothing is running", ToastKind::Info);
                }
            }
            Action::Quit => self.quit = true,
            Action::QuitIfEmpty => {
                if self.composer.is_empty() {
                    self.quit = true;
                } else {
                    self.composer.apply(EditOp::Delete);
                }
            }
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::NewChat => self.new_chat(),
            Action::CopyReply => self.copy_reply(),
            Action::Redraw => self.redraw = true,
            Action::ToggleHelp => {
                self.help_open = !self.help_open;
                self.help_scroll = 0;
            }
            Action::ToggleAnimation => {
                if !self.options.animate {
                    self.say(
                        "animation is off by the command line (--speed instant)",
                        ToastKind::Info,
                    );
                } else if self.change_setting(|s| s.animation = !s.animation) {
                    let state = if self.animate { "on" } else { "off" };
                    self.say(format!("animation {state} (saved)"), ToastKind::Info);
                }
            }
            Action::ClearQueue => {
                if let Some(s) = self.active_mut() {
                    if s.queued.is_empty() {
                        self.say("the queue is empty", ToastKind::Info);
                    } else {
                        s.session.clear_queue();
                        s.queued = s.session.queued();
                        self.say("cleared queued messages", ToastKind::Info);
                    }
                }
            }
            Action::FocusNext => self.cycle_focus(1),
            Action::FocusPrev => self.cycle_focus(-1),
            Action::SetFocus(f) => {
                if f != Focus::Sidebar {
                    self.cancel_sidebar_modal();
                }
                // While the page shows, the main panel is the page.
                let f = match f {
                    Focus::Input | Focus::Transcript if self.page_shown => Focus::Settings,
                    Focus::Settings if !self.page_shown => Focus::Input,
                    other => other,
                };
                self.focus = f;
                if f == Focus::Transcript {
                    self.select_last_block();
                }
            }
            Action::ScrollBy(d) => {
                if self.help_open {
                    self.help_scroll = (self.help_scroll as i64 + i64::from(d)).max(0) as usize;
                } else if self.page_shown {
                    self.apply_page(crate::settings_page::PageAction::Scroll(d));
                } else if let Some(s) = self.active_mut() {
                    s.scroll.scroll_by(i64::from(d));
                }
            }
            Action::PageBy(d) => {
                if self.page_shown {
                    let rows = i32::from(self.layout.transcript.height.saturating_sub(2).max(1));
                    self.apply_page(crate::settings_page::PageAction::Scroll(rows * d));
                } else if let Some(s) = self.active_mut() {
                    let page = s.scroll.viewport.saturating_sub(2).max(1) as i64;
                    s.scroll.scroll_by(page * i64::from(d));
                }
            }
            Action::ScrollTop => {
                if let Some(s) = self.active_mut() {
                    s.scroll.to_top();
                }
            }
            Action::JumpLatest => {
                if let Some(s) = self.active_mut() {
                    s.scroll.to_bottom();
                }
            }
            Action::ScrollbarTo { offset, track } => {
                if let Some(s) = self.active_mut() {
                    s.scroll.to_track(offset, track);
                }
            }
            Action::SelectBlock(d) => self.select_block(d),
            Action::SelectBlockId(id) => {
                self.focus = Focus::Transcript;
                if let Some(s) = self.active_mut() {
                    s.selected_block = Some(id);
                }
            }
            Action::ToggleSelected => {
                if let Some(s) = self.active_mut()
                    && let Some(id) = s.selected_block
                {
                    s.transcript.toggle_expanded(id);
                }
            }
            Action::ToggleBlock(id) => {
                self.focus = Focus::Transcript;
                if let Some(s) = self.active_mut() {
                    s.selected_block = Some(id);
                    s.transcript.toggle_expanded(id);
                }
            }
            Action::SidebarMove(d) => self.sidebar_step(d),
            Action::SidebarActivate => self.sidebar_activate(self.sidebar_sel),
            Action::SidebarActivateAt(i) => {
                self.focus = Focus::Sidebar;
                self.sidebar_sel = i;
                self.sidebar_activate(i);
            }
            Action::SidebarScroll(d) => {
                self.sidebar_scroll = (self.sidebar_scroll as i64 + i64::from(d)).max(0) as usize;
            }
            Action::SidebarFold(fold) => self.fold_selected(fold),
            Action::NewFolder => self.start_create(None),
            Action::NewSubfolder => {
                let parent = self.selected_folder();
                self.start_create(parent);
            }
            Action::NewSubfolderAt(i) => {
                self.select_row(i);
                self.apply(Action::NewSubfolder);
            }
            Action::RenameRow => self.start_rename(),
            Action::RenameFolderAt(i) => {
                self.select_row(i);
                self.start_rename();
            }
            Action::DeleteRows => {
                let targets = self.targets();
                self.start_delete_rows(targets);
            }
            Action::DeleteFolderAt(i) => {
                self.select_row(i);
                self.start_delete();
            }
            Action::TogglePin => {
                let ids = crate::conversation::session_ids(&self.targets());
                self.toggle_pin(&ids);
            }
            Action::PickUp => self.start_pickup(),
            Action::PickUpAt(i) => {
                self.select_row(i);
                self.start_pickup();
            }
            Action::MoveStep(d) => self.move_step(d),
            Action::MoveDrop => self.drop_at(self.sidebar_sel),
            Action::PromptEdit(op) => {
                if let Some(p) = self.prompt.as_mut() {
                    p.editor.apply(op);
                }
            }
            Action::PromptSubmit => self.prompt_submit(),
            Action::ConfirmYes => {
                if self.delete_plan.is_some() {
                    self.confirm_delete_plan();
                } else {
                    self.confirm_delete();
                }
            }
            Action::SidebarCancel => {
                if self.delete_plan.take().is_none() {
                    self.cancel_sidebar_modal();
                }
            }
            Action::Tab(tab) => self.apply_tab(tab),
            Action::Select(select) => {
                self.apply_select(select);
                self.rebuild_sidebar_items();
            }
            Action::Menu(menu) => self.apply_menu(menu),
            Action::Page(page) => self.apply_page(page),
            Action::Approve(decision) => self.approve(decision),
            Action::ApprovalMove(d) => {
                self.approval_sel =
                    ((self.approval_sel as i64 + i64::from(d)).rem_euclid(3)) as usize;
            }
            Action::ApprovalConfirm => {
                let decision = match self.approval_sel {
                    0 => ApprovalDecision::Once,
                    1 => ApprovalDecision::Session,
                    _ => ApprovalDecision::Deny,
                };
                self.approve(decision);
            }
            Action::PickOption(i) => self.pick_option(i),
            Action::QuestionMove(d) => {
                let n = self
                    .active()
                    .and_then(|s| s.transcript.pending_question())
                    .map(|(_, q)| q.options.len())
                    .unwrap_or(0);
                let slots = n as i64 + 1;
                self.question_sel =
                    ((self.question_sel as i64 + i64::from(d)).rem_euclid(slots)) as usize;
            }
            Action::QuestionSubmit => self.question_submit(),
            Action::QuestionEdit(op) => {
                if matches!(op, EditOp::Up | EditOp::Down) {
                    return;
                }
                self.answer.apply(op);
                let n = self
                    .active()
                    .and_then(|s| s.transcript.pending_question())
                    .map(|(_, q)| q.options.len())
                    .unwrap_or(0);
                if !self.answer.is_empty() {
                    self.question_sel = n;
                }
            }
            Action::PopupMove(d) => {
                let n = self.popup().len();
                if n > 0 {
                    let i = (self.composer.popup_index as i64 + i64::from(d)).rem_euclid(n as i64);
                    self.composer.popup_index = i as usize;
                }
            }
            Action::PopupComplete => {
                let list = self.popup();
                if let Some(c) =
                    list.get(self.composer.popup_index.min(list.len().saturating_sub(1)))
                {
                    let text = if c.args.is_empty() {
                        c.name.to_string()
                    } else {
                        format!("{} ", c.name)
                    };
                    self.composer.set_text(&text);
                }
            }
            Action::RunCommand(name) => {
                self.focus = Focus::Input;
                let spec = commands::Commands.iter().find(|c| c.name == name);
                match spec {
                    Some(c) if !c.args.is_empty() => self.composer.set_text(&format!("{name} ")),
                    _ => {
                        self.composer.take();
                        self.run_command_text(name);
                    }
                }
            }
        }
    }

    fn cycle_focus(&mut self, d: i32) {
        self.cancel_sidebar_modal();
        if self.page_shown {
            // The page has its own panes; from the sidebar, tab goes back onto it.
            self.focus_page_from_sidebar(d);
            return;
        }
        let sidebar_open = match self.layout.sidebar_mode() {
            SidebarMode::Hidden => false,
            SidebarMode::Docked | SidebarMode::Drawer => true,
        };
        let order: Vec<Focus> = if sidebar_open {
            vec![Focus::Input, Focus::Transcript, Focus::Sidebar]
        } else {
            vec![Focus::Input, Focus::Transcript]
        };
        let pos = order.iter().position(|f| *f == self.focus).unwrap_or(0) as i64;
        let next = (pos + i64::from(d)).rem_euclid(order.len() as i64) as usize;
        self.focus = order.get(next).copied().unwrap_or(Focus::Input);
        if self.focus == Focus::Transcript {
            self.select_last_block();
        }
    }

    fn select_block(&mut self, d: i32) {
        let Some(s) = self.active_mut() else {
            return;
        };
        let ids: Vec<u64> = s
            .transcript
            .blocks
            .iter()
            .filter(|b| !matches!(b.kind, crate::view_model::BlockKind::Banner))
            .map(|b| b.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let pos = s
            .selected_block
            .and_then(|id| ids.iter().position(|x| *x == id))
            .map(|p| p as i64 + i64::from(d))
            .unwrap_or(if d < 0 { ids.len() as i64 - 1 } else { 0 });
        let pos = pos.clamp(0, ids.len() as i64 - 1) as usize;
        let id = ids.get(pos).copied();
        s.selected_block = id;
        // Bring the selected block into view.
        if let Some(id) = id
            && let Some(i) = s.transcript.index_of(id)
            && let Some(start) = s.view.starts.get(i).copied()
        {
            let end = s.view.starts.get(i + 1).copied().unwrap_or(s.scroll.total);
            if start < s.scroll.top {
                s.scroll.top = start;
                s.scroll.follow = false;
            } else if end > s.scroll.top + s.scroll.viewport {
                let want = end.saturating_sub(s.scroll.viewport).min(start);
                s.scroll.top = want.min(s.scroll.max_top());
                s.scroll.follow = s.scroll.top >= s.scroll.max_top();
            }
            s.scroll.anchor = None;
        }
    }

    /// The focus of the main panel: the page when it shows, else the input.
    pub(crate) fn main_focus(&self) -> Focus {
        if self.page_shown {
            Focus::Settings
        } else {
            Focus::Input
        }
    }

    /// True when the sidebar docks at the current width, by the `sidebar` and `sidebar_width`
    /// settings (D63).
    pub fn can_dock(&self) -> bool {
        crate::layout::docks(
            self.size.0,
            self.settings.sidebar,
            self.settings.sidebar_width,
        )
    }

    fn toggle_sidebar(&mut self) {
        self.cancel_sidebar_modal();
        self.grab = None;
        if self.can_dock() {
            self.sidebar_pinned = !self.sidebar_pinned;
            if !self.sidebar_pinned && self.focus == Focus::Sidebar {
                self.focus = self.main_focus();
            }
        } else {
            self.drawer_open = !self.drawer_open;
            self.focus = if self.drawer_open {
                Focus::Sidebar
            } else {
                self.main_focus()
            };
        }
    }

    fn submit(&mut self) {
        if self.modal() != Modal::None {
            return;
        }
        let list = self.popup();
        let text = self.composer.text().to_string();
        if !list.is_empty() {
            let exact = list.iter().any(|c| c.name == text);
            let chosen = list
                .get(self.composer.popup_index.min(list.len().saturating_sub(1)))
                .copied();
            if !exact && let Some(c) = chosen {
                self.apply(Action::RunCommand(c.name));
                return;
            }
        }
        if text.trim().is_empty() {
            return;
        }
        if commands::parse(&text).is_some() {
            self.composer.take();
            self.run_command_text(&text);
            return;
        }
        let text = self.composer.take();
        if let Some(s) = self.active_mut() {
            s.session.submit(text);
            s.queued = s.session.queued();
            s.scroll.to_bottom();
        }
    }

    fn run_command_text(&mut self, text: &str) {
        match commands::parse(text) {
            Some(Ok(cmd)) => self.run_command(cmd),
            Some(Err(message)) => self.say(message, ToastKind::Warning),
            None => {}
        }
    }

    fn run_command(&mut self, cmd: Command) {
        match cmd {
            Command::Help => {
                self.help_open = true;
                self.help_scroll = 0;
            }
            Command::New => self.new_chat(),
            Command::Sidebar => self.toggle_sidebar(),
            Command::Approval(mode) if mode != ApprovalMode::Full => {
                // The same path as the menu: saved for the conversation and applied now.
                let ids = self.shown_ids();
                self.set_approval(&ids, Some(mode));
            }
            Command::Approval(mode) => {
                let result = self.active_mut().map(|s| s.session.set_approval_mode(mode));
                match result {
                    Some(Ok(())) => {
                        self.say(format!("approval mode: {}", mode.name()), ToastKind::Info)
                    }
                    Some(Err(err)) => self.say(format!("{err}"), ToastKind::Error),
                    None => {}
                }
            }
            Command::Cancel => self.apply(Action::Cancel),
            Command::ClearQueue => self.apply(Action::ClearQueue),
            Command::Copy => self.copy_reply(),
            Command::Animate => self.apply(Action::ToggleAnimation),
            Command::Settings => self.open_page(None),
            Command::Theme(arg) => self.theme_command(arg),
            Command::ThemeImport(path) => {
                self.import_theme(&path);
            }
            Command::Folder(None) => self.start_create(None),
            Command::Folder(Some(name)) => {
                self.create_folder_named(name, None);
            }
            Command::Tab => self.apply_tab(crate::input::TabAction::New),
            Command::Close => self.apply_tab(crate::input::TabAction::CloseActive),
            Command::Rename(title) => match self.active().map(|s| s.session.id().to_string()) {
                Some(id) => match title {
                    Some(title) => {
                        self.rename_conversation(&id, title);
                    }
                    None => self.start_rename_session(&id),
                },
                None => self.say("no conversation is open", ToastKind::Info),
            },
            Command::Pin => {
                let ids = self.shown_ids();
                self.toggle_pin(&ids);
            }
            Command::Archive => {
                let ids = self.shown_ids();
                self.toggle_archived(&ids);
            }
            Command::Delete => {
                let rows = self
                    .shown_ids()
                    .into_iter()
                    .map(crate::selection::RowId::Session)
                    .collect();
                self.start_delete_rows(rows);
            }
            Command::Quit => self.quit = true,
        }
    }

    fn copy_reply(&mut self) {
        let text = self.active().and_then(|s| s.transcript.last_reply_text());
        match text {
            Some(t) => {
                self.outputs.push(osc52(&t));
                self.say("copied the last reply (OSC 52)", ToastKind::Info);
            }
            None => self.say("no reply to copy yet", ToastKind::Info),
        }
    }

    fn sidebar_activate(&mut self, index: usize) {
        let Some(item) = self.sidebar_items.get(index).cloned() else {
            return;
        };
        match item.kind {
            SidebarItemKind::NewChat => self.new_chat(),
            SidebarItemKind::Root => {}
            SidebarItemKind::Folder { id, .. } => {
                if !self.folded.remove(&id) {
                    self.folded.insert(id);
                }
                self.rebuild_sidebar_items();
            }
            SidebarItemKind::Session { id } => {
                self.open_conversation(&id, crate::tabs::OpenPlace::Setting)
            }
        }
    }

    fn approve(&mut self, decision: ApprovalDecision) {
        let Some(s) = self.sessions.get_mut(self.active) else {
            return;
        };
        let Some(id) = s
            .transcript
            .pending_approval()
            .map(|(_, t)| t.tool_call_id.clone())
        else {
            return;
        };
        if s.resolved.insert(id.clone()) {
            s.session.resolve_approval(&id, decision);
        }
    }

    fn pick_option(&mut self, index: usize) {
        let Some(s) = self.sessions.get_mut(self.active) else {
            return;
        };
        let Some((qid, option)) = s.transcript.pending_question().and_then(|(_, q)| {
            q.options
                .get(index)
                .map(|o| (q.question_id.clone(), o.clone()))
        }) else {
            return;
        };
        if s.resolved.insert(qid.clone()) {
            s.session.answer_question(&qid, option, Some(index as u32));
        }
    }

    fn question_submit(&mut self) {
        let text = self.answer.text().trim().to_string();
        if !text.is_empty() {
            let Some(s) = self.sessions.get_mut(self.active) else {
                return;
            };
            let Some(qid) = s
                .transcript
                .pending_question()
                .map(|(_, q)| q.question_id.clone())
            else {
                return;
            };
            if s.resolved.insert(qid.clone()) {
                s.session.answer_question(&qid, text, None);
                self.answer = composer_t::new();
            }
            return;
        }
        self.pick_option(self.question_sel);
    }

    /// The phase in words, for the status row and the sidebar.
    pub fn phase_text(&self) -> String {
        let Some(s) = self.active() else {
            return "idle".to_string();
        };
        match s.phase {
            SessionPhase::Idle => "your turn".to_string(),
            SessionPhase::Generating => {
                match s.transcript.running_reply().and_then(|(_, r)| r.last()) {
                    Some(a) => format!(
                        "generating level {}/{}",
                        (a.levels.len() + 1).min(a.level_count.max(1) as usize),
                        a.level_count.max(1)
                    ),
                    None => "generating".to_string(),
                }
            }
            SessionPhase::AwaitingApproval => "waiting for your approval".to_string(),
            SessionPhase::RunningTool => {
                let tool = s
                    .transcript
                    .blocks
                    .iter()
                    .rev()
                    .find_map(|b| match &b.kind {
                        crate::view_model::BlockKind::Tool(t)
                            if t.status == crate::view_model::ToolStatus::Running =>
                        {
                            Some(sanitize(&t.tool, false))
                        }
                        _ => None,
                    });
                match tool {
                    Some(t) => format!("running {}", truncate(&t, 24)),
                    None => "running a tool".to_string(),
                }
            }
            SessionPhase::AwaitingAnswer => "waiting for your answer".to_string(),
            SessionPhase::Cancelling => "cancelling…".to_string(),
        }
    }
}
