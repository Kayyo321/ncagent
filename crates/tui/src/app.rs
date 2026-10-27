/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The app state machine. It owns every opened session, folds their events, maps input to
//! actions and actions to session calls. It never touches the terminal: drawing goes to a
//! ratatui `Frame` (see `draw`), and bytes for the terminal (OSC 52) are queued in `outputs`
//! for the loop to write.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEventKind};
use harness::api::{SessionPhase, runtime_info_t, runtime_t, session_t};
use harness::approval::{ApprovalDecision, ApprovalMode};
use harness::event::SessionEvent;
use harness::store::session_summary_t;
use ratatui::layout::Rect;

use crate::clipboard::osc52;
use crate::commands::{self, Command};
use crate::composer::{EditOp, composer_t};
use crate::input::{
    Action, Focus, Modal, MouseIntent, hit_registry_t, key_ctx_t, map_key, map_mouse,
};
use crate::layout::{DockWidth, SidebarMode, layout_t};
use crate::sidebar::{Activity, SidebarItemKind, sidebar_item_t};
use crate::text::{format_age, sanitize, truncate};
use crate::theme::{ColorMode, theme_t};
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

/// One opened session and everything the UI keeps for it.
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
}

impl live_session_t {
    fn new(session: Box<dyn session_t>) -> Self {
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
        }
    }

    pub fn busy(&self) -> bool {
        self.phase.busy()
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

    fn activity(&self) -> Activity {
        match self.phase {
            SessionPhase::Idle => Activity::Idle,
            SessionPhase::AwaitingApproval | SessionPhase::AwaitingAnswer => Activity::Waiting,
            _ => Activity::Generating,
        }
    }
}

pub struct app_t {
    pub runtime: Arc<dyn runtime_t>,
    pub info: runtime_info_t,
    pub theme: theme_t,
    pub options: app_options_t,
    pub sessions: Vec<live_session_t>,
    pub active: usize,
    pub summaries: Vec<session_summary_t>,
    summaries_at: u64,
    summaries_dirty: bool,
    pub composer: composer_t,
    pub focus: Focus,
    pub sidebar_pinned: bool,
    pub drawer_open: bool,
    pub sidebar_items: Vec<sidebar_item_t>,
    pub sidebar_sel: usize,
    pub sidebar_scroll: usize,
    pub collapsed: BTreeSet<String>,
    pub help_open: bool,
    pub help_scroll: usize,
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
        let session = runtime.new_session().map_err(|e| start_error_t {
            message: format!("could not start a session: {e}"),
        })?;
        let mut app = app_t {
            runtime,
            info,
            theme: theme_t::dark(),
            animate: options.animate,
            options,
            sessions: vec![live_session_t::new(session)],
            active: 0,
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
            collapsed: BTreeSet::new(),
            help_open: false,
            help_scroll: 0,
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
        app.refresh_summaries();
        app.rebuild_sidebar_items();
        Ok(app)
    }

    pub fn active(&self) -> Option<&live_session_t> {
        self.sessions.get(self.active)
    }

    fn active_mut(&mut self) -> Option<&mut live_session_t> {
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

    fn refresh_summaries(&mut self) {
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
        for live in self.sessions.iter_mut() {
            let events = live.session.pump();
            for e in &events {
                live.transcript.fold(e, Some(clock));
                if matches!(
                    e.event,
                    SessionEvent::TurnEnd { .. } | SessionEvent::UserMessage { .. }
                ) {
                    turn_ended = true;
                }
            }
            live.phase = live.session.phase();
            live.queued = live.session.queued();
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
    fn sync_widgets(&mut self) {
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
                }
                if let Some(action) = map_key(key, &self.key_ctx()) {
                    self.apply(action);
                }
            }
            Event::Mouse(mouse) => {
                self.pending_escape = None;
                if !self.options.mouse {
                    return;
                }
                let sidebar = if self.layout.sidebar_mode() == SidebarMode::Hidden {
                    Rect::default()
                } else {
                    self.layout.sidebar
                };
                match map_mouse(mouse, &self.hits, self.dragging, sidebar) {
                    MouseIntent::Act(action) => self.apply(action),
                    MouseIntent::StartDrag(action) => {
                        self.dragging = self.hits.at(mouse.column, mouse.row).map(|h| h.rect);
                        self.apply(action);
                    }
                    MouseIntent::Drag(action) => self.apply(action),
                    MouseIntent::EndDrag => self.dragging = None,
                    MouseIntent::Hover(x, y) => self.hover = Some((x, y)),
                    MouseIntent::Nothing => {}
                }
            }
            Event::Paste(text) => {
                self.pending_escape = None;
                match self.modal() {
                    Modal::Question => {
                        self.answer.apply(EditOp::Paste(text.clone()));
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
        if w < DockWidth {
            self.drawer_open = self.drawer_open && self.focus == Focus::Sidebar;
        }
    }

    fn resolve_escape(&mut self) {
        if self.help_open {
            self.help_open = false;
            return;
        }
        if self.busy() {
            self.cancel_turn();
            return;
        }
        if self.drawer_open {
            self.drawer_open = false;
            self.focus = Focus::Input;
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
                self.animate = !self.animate;
                let state = if self.animate { "on" } else { "off" };
                self.say(format!("generation effect {state}"), ToastKind::Info);
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
                self.focus = f;
                if f == Focus::Transcript {
                    self.select_last_block();
                }
            }
            Action::ScrollBy(d) => {
                if self.help_open {
                    self.help_scroll = (self.help_scroll as i64 + i64::from(d)).max(0) as usize;
                } else if let Some(s) = self.active_mut() {
                    s.scroll.scroll_by(i64::from(d));
                }
            }
            Action::PageBy(d) => {
                if let Some(s) = self.active_mut() {
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
            Action::SidebarMove(d) => {
                let n = self.sidebar_items.len();
                if n > 0 {
                    let next = (self.sidebar_sel as i64 + i64::from(d)).clamp(0, n as i64 - 1);
                    self.sidebar_sel = next as usize;
                }
            }
            Action::SidebarActivate => self.sidebar_activate(self.sidebar_sel),
            Action::SidebarActivateAt(i) => {
                self.focus = Focus::Sidebar;
                self.sidebar_sel = i;
                self.sidebar_activate(i);
            }
            Action::SidebarScroll(d) => {
                self.sidebar_scroll = (self.sidebar_scroll as i64 + i64::from(d)).max(0) as usize;
            }
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

    fn toggle_sidebar(&mut self) {
        if self.size.0 >= DockWidth {
            self.sidebar_pinned = !self.sidebar_pinned;
            if !self.sidebar_pinned && self.focus == Focus::Sidebar {
                self.focus = Focus::Input;
            }
        } else {
            self.drawer_open = !self.drawer_open;
            self.focus = if self.drawer_open {
                Focus::Sidebar
            } else {
                Focus::Input
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

    pub fn new_chat(&mut self) {
        match self.runtime.new_session() {
            Ok(session) => {
                self.sessions.push(live_session_t::new(session));
                self.active = self.sessions.len() - 1;
                self.focus = Focus::Input;
                self.drawer_open = false;
                self.summaries_dirty = true;
                self.sync_widgets();
                self.rebuild_sidebar_items();
            }
            Err(err) => self.say(
                format!("could not start a session: {err}"),
                ToastKind::Error,
            ),
        }
    }

    pub fn open_session(&mut self, id: &str) {
        if let Some(i) = self.sessions.iter().position(|s| s.session.id() == id) {
            self.active = i;
        } else {
            match self.runtime.open_session(id) {
                Ok(opened) => {
                    let mut live = live_session_t::new(opened.session);
                    for e in &opened.events {
                        live.transcript.fold(e, None);
                    }
                    live.phase = live.session.phase();
                    self.sessions.push(live);
                    self.active = self.sessions.len() - 1;
                    if opened.skipped_lines > 0 {
                        self.say(
                            format!(
                                "skipped {} unreadable line(s) of the saved session",
                                opened.skipped_lines
                            ),
                            ToastKind::Warning,
                        );
                    }
                }
                Err(err) => {
                    self.say(
                        format!("could not open the session: {err}"),
                        ToastKind::Error,
                    );
                    return;
                }
            }
        }
        self.drawer_open = false;
        self.sync_widgets();
        self.rebuild_sidebar_items();
    }

    fn sidebar_activate(&mut self, index: usize) {
        let Some(item) = self.sidebar_items.get(index).cloned() else {
            return;
        };
        match item.kind {
            SidebarItemKind::NewChat => self.new_chat(),
            SidebarItemKind::Project { name, .. } => {
                if !self.collapsed.remove(&name) {
                    self.collapsed.insert(name);
                }
                self.rebuild_sidebar_items();
            }
            SidebarItemKind::Session { id } => self.open_session(&id),
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

    /// The sidebar list: new chat, then projects (the current one first) with their sessions.
    pub fn rebuild_sidebar_items(&mut self) {
        let mut items = vec![sidebar_item_t {
            kind: SidebarItemKind::NewChat,
            title: "New chat".to_string(),
            detail: String::new(),
            active: false,
            activity: Activity::Idle,
            demo: false,
        }];
        struct entry_t {
            id: String,
            title: String,
            updated: u64,
            turns: u32,
            demo: bool,
        }
        let current_project = sanitize(&self.info.project, false);
        let mut groups: Vec<(String, Vec<entry_t>)> = Vec::new();
        let mut push =
            |project: String, e: entry_t| match groups.iter_mut().find(|(p, _)| *p == project) {
                Some((_, list)) => list.push(e),
                None => groups.push((project, vec![e])),
            };
        let mut seen: HashSet<String> = HashSet::new();
        for s in &self.summaries {
            seen.insert(s.id.clone());
            let title = if s.title.trim().is_empty() {
                "New chat".to_string()
            } else {
                sanitize(&s.title, false)
            };
            push(
                sanitize(&s.project, false),
                entry_t {
                    id: s.id.clone(),
                    title,
                    updated: s.updated_ms,
                    turns: s.turns,
                    demo: s.simulated,
                },
            );
        }
        for live in &self.sessions {
            let id = live.session.id().to_string();
            if seen.contains(&id) {
                continue;
            }
            let project = live
                .transcript
                .meta
                .as_ref()
                .map(|m| sanitize(&m.project, false))
                .unwrap_or_else(|| current_project.clone());
            push(
                project,
                entry_t {
                    id,
                    title: live.title(),
                    updated: if live.transcript.last_at_ms > 0 {
                        live.transcript.last_at_ms
                    } else {
                        self.now_ms
                    },
                    turns: live.transcript.turn.turn,
                    demo: self.info.identity.simulated(),
                },
            );
        }
        for (_, list) in groups.iter_mut() {
            list.sort_by_key(|e| std::cmp::Reverse(e.updated));
        }
        groups.sort_by(|a, b| {
            let a_cur = a.0 == current_project;
            let b_cur = b.0 == current_project;
            b_cur.cmp(&a_cur).then_with(|| {
                let a_t = a.1.first().map(|e| e.updated).unwrap_or(0);
                let b_t = b.1.first().map(|e| e.updated).unwrap_or(0);
                b_t.cmp(&a_t)
            })
        });
        let active_id = self.active().map(|s| s.session.id().to_string());
        for (project, list) in groups {
            let collapsed = self.collapsed.contains(&project);
            items.push(sidebar_item_t {
                kind: SidebarItemKind::Project {
                    name: project.clone(),
                    collapsed,
                },
                title: if project.is_empty() {
                    "(no project)".to_string()
                } else {
                    project.clone()
                },
                detail: list.len().to_string(),
                active: false,
                activity: Activity::Idle,
                demo: false,
            });
            if collapsed {
                continue;
            }
            for e in list {
                let live = self.sessions.iter().find(|s| s.session.id() == e.id);
                let activity = live.map(live_session_t::activity).unwrap_or(Activity::Idle);
                items.push(sidebar_item_t {
                    active: active_id.as_deref() == Some(e.id.as_str()),
                    detail: format!("{} · {}t", format_age(self.now_ms, e.updated), e.turns),
                    kind: SidebarItemKind::Session { id: e.id },
                    title: e.title,
                    activity,
                    demo: e.demo,
                });
            }
        }
        self.sidebar_sel = self.sidebar_sel.min(items.len().saturating_sub(1));
        self.sidebar_items = items;
    }
}
