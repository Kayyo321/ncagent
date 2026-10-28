/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! App-level tests: the whole UI drawn on ratatui's `TestBackend`, driven by synthetic
//! crossterm events, against the scripted fixture. Nothing here uses a real terminal, so
//! these tests cover the mapping from events to actions and frames, not terminal emulators.

use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use harness::api::{SessionPhase, runtime_t};
use harness::approval::{ApprovalDecision, ApprovalMode};
use harness::event::{CancelScope, SessionEvent, TurnOutcome};
use harness::tools::ApprovalClass;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::app::{EscapeDelayMs, app_options_t, app_t};
use crate::canvas::buffer_lines;
use crate::draw::draw;
use crate::folder_tests::{file_saved_in_research, folder_index, seed_research};
use crate::input::{Action, Focus, MenuAction, Modal, SelectAction, TabAction, Target};
use crate::scripted::{ScriptCall, preview_info, script_handle_t, scripted_runtime_t};
use crate::settings_page::PageAction;
use crate::settings_spec::Section;
use crate::theme::ColorMode;
use crate::view_model::tests::{call_end, call_start, full_session, level_done, session_start};

pub(crate) struct rig_t {
    pub(crate) rt: Arc<scripted_runtime_t>,
    pub(crate) app: app_t,
    pub(crate) term: Terminal<TestBackend>,
    pub(crate) clock: u64,
}

impl rig_t {
    pub(crate) fn new(width: u16, height: u16, animate: bool) -> Self {
        Self::with(width, height, animate, true, |_| {})
    }

    pub(crate) fn with(
        width: u16,
        height: u16,
        animate: bool,
        mouse: bool,
        setup: impl FnOnce(&scripted_runtime_t),
    ) -> Self {
        let rt = Arc::new(scripted_runtime_t::manual(preview_info("/home/u/proj")));
        setup(&rt);
        let dyn_rt: Arc<dyn runtime_t> = rt.clone();
        let app = app_t::new(
            dyn_rt,
            app_options_t {
                animate,
                mouse,
                color: ColorMode::TrueColor,
            },
            0,
            10_000_000,
        )
        .unwrap_or_else(|e| panic!("app: {}", e.message));
        let term = Terminal::new(TestBackend::new(width, height)).unwrap_or_else(|e| match e {});
        let mut rig = rig_t {
            rt,
            app,
            term,
            clock: 0,
        };
        rig.frame();
        rig
    }

    pub(crate) fn handle(&self, i: usize) -> script_handle_t {
        self.rt.handle(i).unwrap_or_else(|| panic!("no handle {i}"))
    }

    /// The handle of the session shown in tab `tab`, found by its id.
    pub(crate) fn tab_handle(&self, tab: usize) -> script_handle_t {
        let id = self
            .app
            .sessions
            .get(tab)
            .map(|s| s.session.id().to_string())
            .unwrap_or_default();
        (0..256)
            .filter_map(|i| self.rt.handle(i))
            .find(|h| h.id == id)
            .unwrap_or_else(|| panic!("no handle for tab {tab} ({id})"))
    }

    pub(crate) fn push(&mut self, events: Vec<SessionEvent>) {
        self.tab_handle(self.app.active).push(events);
        self.frame();
    }

    pub(crate) fn advance(&mut self, ms: u64) {
        self.clock += ms;
        self.frame();
    }

    pub(crate) fn frame(&mut self) -> Vec<String> {
        self.app.tick(self.clock, 10_000_000 + self.clock);
        let app = &mut self.app;
        let _ = self.term.draw(|f| draw(app, f));
        buffer_lines(self.term.backend().buffer())
    }

    pub(crate) fn screen(&self) -> String {
        buffer_lines(self.term.backend().buffer()).join("\n")
    }

    pub(crate) fn resize(&mut self, w: u16, h: u16) {
        self.term.backend_mut().resize(w, h);
        let _ = self.term.resize(Rect::new(0, 0, w, h));
        self.app.handle_event(&Event::Resize(w, h));
        self.frame();
    }

    pub(crate) fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.app
            .handle_event(&Event::Key(KeyEvent::new(code, mods)));
        self.frame();
    }

    pub(crate) fn ch(&mut self, c: char) {
        self.key(KeyCode::Char(c), KeyModifiers::NONE);
    }

    pub(crate) fn typed(&mut self, s: &str) {
        for c in s.chars() {
            self.app.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            )));
        }
        self.frame();
    }

    pub(crate) fn mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) {
        self.app.handle_event(&Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }));
        self.frame();
    }

    pub(crate) fn click(&mut self, x: u16, y: u16) {
        self.mouse(MouseEventKind::Down(MouseButton::Left), x, y);
        self.mouse(MouseEventKind::Up(MouseButton::Left), x, y);
    }

    pub(crate) fn mouse_mods(&mut self, kind: MouseEventKind, x: u16, y: u16, mods: KeyModifiers) {
        self.app.handle_event(&Event::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: mods,
        }));
        self.frame();
    }

    /// Press and release `button` at a cell with modifiers.
    pub(crate) fn click_with(&mut self, button: MouseButton, x: u16, y: u16, mods: KeyModifiers) {
        self.mouse_mods(MouseEventKind::Down(button), x, y, mods);
        self.mouse_mods(MouseEventKind::Up(button), x, y, mods);
    }

    /// A click on sidebar row `index` with modifiers.
    pub(crate) fn click_row_with(&mut self, index: usize, mods: KeyModifiers) {
        let r = self.row_rect(index);
        self.click_with(MouseButton::Left, r.x + 3, r.y, mods);
    }

    /// A right click on sidebar row `index`.
    pub(crate) fn right_click_row(&mut self, index: usize) {
        let r = self.row_rect(index);
        self.click_with(MouseButton::Right, r.x + 3, r.y, KeyModifiers::NONE);
    }

    pub(crate) fn rect_of(&self, action: &Action) -> Rect {
        self.app
            .hits
            .find(action)
            .unwrap_or_else(|| panic!("no hit target for {action:?}\n{}", self.screen()))
    }

    pub(crate) fn click_action(&mut self, action: &Action) {
        let r = self.rect_of(action);
        self.click(r.x, r.y);
    }

    pub(crate) fn calls(&self) -> Vec<ScriptCall> {
        self.tab_handle(self.app.active).calls()
    }

    /// The rectangle of sidebar row `index` in the last frame.
    pub(crate) fn row_rect(&self, index: usize) -> Rect {
        self.app
            .hits
            .find_target(&Target::Row(index))
            .unwrap_or_else(|| panic!("row {index} is not drawn\n{}", self.screen()))
    }

    /// Move the pointer over sidebar row `index`, so its chips show.
    pub(crate) fn hover_row(&mut self, index: usize) {
        let r = self.row_rect(index);
        self.mouse(MouseEventKind::Moved, r.x + 3, r.y);
    }

    /// Focus the sidebar and walk the selection to row `index` with `j` and `k`.
    pub(crate) fn select_by_keys(&mut self, index: usize) {
        if self.app.focus != Focus::Sidebar {
            self.app.apply(Action::SetFocus(Focus::Sidebar));
            self.frame();
        }
        for _ in 0..80 {
            let sel = self.app.sidebar_sel;
            if sel == index {
                return;
            }
            self.ch(if sel > index { 'k' } else { 'j' });
        }
        panic!("could not select row {index}\n{}", self.screen());
    }

    /// Pick-up mode: step with `j` and `k` until row `index` is the destination.
    pub(crate) fn choose_destination(&mut self, index: usize) {
        for _ in 0..80 {
            let sel = self.app.sidebar_sel;
            if sel == index {
                return;
            }
            self.ch(if sel > index { 'k' } else { 'j' });
        }
        panic!(
            "row {index} is not an allowed destination\n{}",
            self.screen()
        );
    }

    /// Press on row `from`, drag to row `to`, release there.
    pub(crate) fn drag_rows(&mut self, from: usize, to: usize) {
        let a = self.row_rect(from);
        self.mouse(MouseEventKind::Down(MouseButton::Left), a.x + 3, a.y);
        let b = self.row_rect(to);
        self.mouse(MouseEventKind::Drag(MouseButton::Left), b.x + 3, b.y);
        let b = self.row_rect(to);
        self.mouse(MouseEventKind::Up(MouseButton::Left), b.x + 3, b.y);
    }

    /// Press and release on sidebar row `index`.
    pub(crate) fn click_row(&mut self, index: usize) {
        let r = self.row_rect(index);
        self.click(r.x + 3, r.y);
    }
}

pub(crate) fn start_turn(user: &str) -> Vec<SessionEvent> {
    vec![
        session_start(),
        SessionEvent::TurnStart { turn: 1 },
        SessionEvent::UserMessage {
            turn: 1,
            text: user.into(),
            from_queue: false,
        },
    ]
}

fn approval_events() -> Vec<SessionEvent> {
    let mut e = start_turn("please approve");
    e.push(SessionEvent::ToolCall {
        turn: 1,
        tool_call_id: "t1".into(),
        tool: "replace_lines".into(),
        class: ApprovalClass::Write,
        args: vec![],
    });
    e.push(SessionEvent::ApprovalRequested {
        turn: 1,
        tool_call_id: "t1".into(),
        tool: "replace_lines".into(),
        class: ApprovalClass::Write,
        summary: "replace lines 3-4 of src/lib.rs".into(),
        reason: "write tools ask in ask mode".into(),
    });
    e
}

fn question_events() -> Vec<SessionEvent> {
    let mut e = start_turn("ask me");
    e.push(SessionEvent::QuestionAsked {
        turn: 1,
        question_id: "q1".into(),
        question: "Which one?".into(),
        options: vec!["first".into(), "second".into(), "third".into()],
    });
    e
}

fn generating_events() -> Vec<SessionEvent> {
    let mut e = start_turn("hello");
    e.push(call_start(1, 0, 0));
    e.push(level_done(1, 0, 0, 0, Some("secret")));
    e.push(level_done(1, 0, 0, 1, Some("secret words here")));
    e
}

fn ascii_free_of_letters(screen: &str, word: &str) -> bool {
    !screen.contains(word)
}

#[test]
fn full_session_renders_speakers_demo_and_reply() {
    let mut rig = rig_t::new(140, 120, true);
    let events: Vec<SessionEvent> = full_session().into_iter().map(|s| s.event).collect();
    rig.push(events);
    rig.advance(10_000);
    let screen = rig.screen();
    for needle in [
        "you",
        "please approve",
        "ncagent · demo model",
        "replace_lines",
        "allowed once by you",
        "attempt 1 invalid",
        "ncagent asks",
        "end of turn 1",
        "completed",
        "All good.",
        "DEMO",
        "UI PREVIEW (scripted events)",
        "conversations",
        "your turn",
    ] {
        assert!(screen.contains(needle), "missing {needle:?}\n{screen}");
    }
    // The logo is on the title card.
    assert!(screen.contains("█████▄ ▄█████ █████▄"), "{screen}");
    // No box-drawing frame anywhere (D47).
    for c in ['┌', '┐', '└', '┘', '│', '─'] {
        assert!(!screen.contains(c), "box char {c:?}\n{screen}");
    }
}

#[test]
fn generating_shows_strip_and_hides_drafts() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(generating_events());
    rig.handle(0).set_phase(SessionPhase::Generating);
    for _ in 0..10 {
        rig.advance(37);
        let screen = rig.screen();
        assert!(ascii_free_of_letters(&screen, "secret"), "{screen}");
        assert!(ascii_free_of_letters(&screen, "words"), "{screen}");
    }
    let screen = rig.screen();
    assert!(screen.contains("400"), "{screen}");
    assert!(screen.contains("410"), "{screen}");
    assert!(
        screen.contains("pass times: simulator wall clock"),
        "{screen}"
    );
    assert!(screen.contains("ncagent's turn"), "{screen}");
    assert!(screen.contains("generating level 3/3"), "{screen}");
}

#[test]
fn dissolve_reveals_text_only_after_the_end() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(generating_events());
    rig.advance(100);
    rig.push(vec![call_end(1, 0, 0, "Zanzibar quokka", true)]);
    let first = rig.screen();
    assert!(!first.contains("Zanzibar"), "{first}");
    assert!(!first.contains("quokka"), "{first}");
    rig.advance(4_000);
    assert!(rig.screen().contains("Zanzibar quokka"));
}

#[test]
fn instant_mode_shows_the_final_text_at_once() {
    let mut rig = rig_t::new(120, 40, false);
    rig.push(generating_events());
    rig.push(vec![call_end(1, 0, 0, "Zanzibar quokka", true)]);
    assert!(rig.screen().contains("Zanzibar quokka"));
}

#[test]
fn approval_keys_resolve_exactly_once() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(approval_events());
    assert_eq!(rig.app.modal(), Modal::Approval);
    assert!(rig.screen().contains("approval needed"));
    rig.ch('y');
    rig.ch('y');
    rig.ch('n');
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::ResolveApproval(
            "t1".into(),
            ApprovalDecision::Once
        )]
    );
    rig.push(vec![SessionEvent::Approval {
        tool_call_id: "t1".into(),
        decision: ApprovalDecision::Once,
        source: harness::approval::ApprovalSource::User,
    }]);
    assert_eq!(rig.app.modal(), Modal::None);
}

#[test]
fn approval_buttons_by_arrows_and_click() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(approval_events());
    rig.key(KeyCode::Right, KeyModifiers::NONE);
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::ResolveApproval(
            "t1".into(),
            ApprovalDecision::Session
        )]
    );

    let mut rig = rig_t::new(120, 40, true);
    rig.push(approval_events());
    rig.click_action(&Action::Approve(ApprovalDecision::Deny));
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::ResolveApproval(
            "t1".into(),
            ApprovalDecision::Deny
        )]
    );
}

#[test]
fn approval_closes_on_cancel_and_turn_end() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(approval_events());
    rig.push(vec![SessionEvent::Cancel {
        turn: 1,
        call: None,
        levels_done: None,
        scope: CancelScope::Approval,
    }]);
    assert_eq!(rig.app.modal(), Modal::None);

    let mut rig = rig_t::new(120, 40, true);
    rig.push(approval_events());
    rig.push(vec![SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Cancelled,
        calls: 0,
        passes: 0,
    }]);
    assert_eq!(rig.app.modal(), Modal::None);
    // Keys no longer resolve anything.
    rig.ch('y');
    assert!(rig.calls().is_empty());
}

#[test]
fn question_by_number_click_and_free_text() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(question_events());
    assert_eq!(rig.app.modal(), Modal::Question);
    rig.ch('2');
    rig.ch('3');
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::AnswerQuestion(
            "q1".into(),
            "second".into(),
            Some(1)
        )]
    );

    let mut rig = rig_t::new(120, 40, true);
    rig.push(question_events());
    rig.click_action(&Action::PickOption(2));
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::AnswerQuestion(
            "q1".into(),
            "third".into(),
            Some(2)
        )]
    );

    let mut rig = rig_t::new(120, 40, true);
    rig.push(question_events());
    rig.typed("my own");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::AnswerQuestion(
            "q1".into(),
            "my own".into(),
            None
        )]
    );
    rig.push(vec![SessionEvent::AskAnswer {
        turn: 1,
        question_id: "q1".into(),
        answer: "my own".into(),
        option_index: None,
    }]);
    assert_eq!(rig.app.modal(), Modal::None);

    let mut rig = rig_t::new(120, 40, true);
    rig.push(question_events());
    rig.push(vec![SessionEvent::Cancel {
        turn: 1,
        call: None,
        levels_done: None,
        scope: CancelScope::Question,
    }]);
    assert_eq!(rig.app.modal(), Modal::None);
}

#[test]
fn escape_cancels_only_after_the_delay() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(generating_events());
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(rig.calls().is_empty());
    rig.advance(EscapeDelayMs);
    assert_eq!(rig.calls(), vec![ScriptCall::Cancel]);

    // An Escape followed by other input (a wheel reported as Escape) does nothing.
    let mut rig = rig_t::new(120, 40, true);
    rig.push(generating_events());
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.mouse(MouseEventKind::ScrollUp, 10, 10);
    rig.advance(EscapeDelayMs * 3);
    assert!(rig.calls().is_empty());
}

#[test]
fn ctrl_c_and_strip_click_cancel_when_busy_only() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(generating_events());
    rig.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(rig.calls(), vec![ScriptCall::Cancel]);

    let mut rig = rig_t::new(120, 40, true);
    rig.push(generating_events());
    let strip = rig
        .app
        .hits
        .hits
        .iter()
        .find(|h| {
            h.target == Target::Action(Action::Cancel)
                && h.rect.y < rig.app.layout.transcript.bottom()
        })
        .map(|h| h.rect)
        .unwrap_or_else(|| panic!("no strip\n{}", rig.screen()));
    rig.click(strip.x + 2, strip.y);
    assert_eq!(rig.calls(), vec![ScriptCall::Cancel]);

    // Idle: none of the three cancels.
    let mut rig = rig_t::new(120, 40, true);
    rig.push(start_turn("hi"));
    rig.push(vec![SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Completed,
        calls: 0,
        passes: 0,
    }]);
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    rig.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    rig.typed("/cancel");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.calls().iter().all(|c| *c != ScriptCall::Cancel),
        "{:?}",
        rig.calls()
    );
}

/// Each mouse target has a keyboard path with the same effect.
#[test]
fn keyboard_parity_table() {
    type check_t = fn(&rig_t) -> String;
    struct case_t {
        name: &'static str,
        setup: fn(&mut rig_t),
        mouse: fn(&mut rig_t),
        keys: fn(&mut rig_t),
        observe: check_t,
    }
    fn noop(_: &mut rig_t) {}
    let cases: Vec<case_t> = vec![
        case_t {
            name: "new chat button / ctrl+n",
            setup: |r| r.push(start_turn("used")),
            mouse: |r| r.click_action(&Action::NewChat),
            keys: |r| r.key(KeyCode::Char('n'), KeyModifiers::CONTROL),
            observe: |r| format!("{} {}", r.app.sessions.len(), r.app.active),
        },
        case_t {
            name: "sidebar chip / ctrl+b",
            setup: noop,
            mouse: |r| r.click_action(&Action::ToggleSidebar),
            keys: |r| r.key(KeyCode::Char('b'), KeyModifiers::CONTROL),
            observe: |r| format!("{}", r.app.sidebar_pinned),
        },
        case_t {
            name: "help chip / ?",
            setup: noop,
            mouse: |r| r.click_action(&Action::ToggleHelp),
            keys: |r| r.ch('?'),
            observe: |r| format!("{}", r.app.help_open),
        },
        case_t {
            name: "approve once button / y",
            setup: |r| r.push(approval_events()),
            mouse: |r| r.click_action(&Action::Approve(ApprovalDecision::Once)),
            keys: |r| r.ch('y'),
            observe: |r| format!("{:?}", r.calls()),
        },
        case_t {
            name: "approve session button / a",
            setup: |r| r.push(approval_events()),
            mouse: |r| r.click_action(&Action::Approve(ApprovalDecision::Session)),
            keys: |r| r.ch('a'),
            observe: |r| format!("{:?}", r.calls()),
        },
        case_t {
            name: "deny button / n",
            setup: |r| r.push(approval_events()),
            mouse: |r| r.click_action(&Action::Approve(ApprovalDecision::Deny)),
            keys: |r| r.ch('n'),
            observe: |r| format!("{:?}", r.calls()),
        },
        case_t {
            name: "question option / number key",
            setup: |r| r.push(question_events()),
            mouse: |r| r.click_action(&Action::PickOption(0)),
            keys: |r| r.ch('1'),
            observe: |r| format!("{:?}", r.calls()),
        },
        case_t {
            name: "level strip / ctrl+c",
            setup: |r| r.push(generating_events()),
            mouse: |r| {
                let rect = r
                    .app
                    .hits
                    .hits
                    .iter()
                    .find(|h| h.target == Target::Action(Action::Cancel))
                    .map(|h| h.rect)
                    .unwrap_or_default();
                r.click(rect.x, rect.y);
            },
            keys: |r| r.key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            observe: |r| format!("{:?}", r.calls()),
        },
        case_t {
            name: "tool block / navigate + enter",
            setup: |r| {
                r.push(full_session().into_iter().map(|s| s.event).collect());
                r.advance(10_000);
            },
            mouse: |r| {
                let id = tool_block_id(r);
                r.click_action(&Action::ToggleBlock(id));
            },
            keys: |r| {
                let id = tool_block_id(r);
                r.app.apply(Action::SetFocus(Focus::Transcript));
                // Walk the selection to the tool block with j/k, then press enter.
                r.key(KeyCode::Char('g'), KeyModifiers::NONE);
                for _ in 0..40 {
                    let sel = r.app.active().and_then(|s| s.selected_block);
                    if sel == Some(id) {
                        break;
                    }
                    let up = sel.is_some_and(|s| s > id);
                    r.ch(if up { 'k' } else { 'j' });
                }
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| {
                let id = tool_block_id(r);
                let s = r.app.active().unwrap_or_else(|| panic!("session"));
                let b = s.transcript.blocks.iter().find(|b| b.id == id);
                format!("{:?}", b.map(|b| b.expanded))
            },
        },
        case_t {
            name: "scrollbar to the top / ctrl+home",
            setup: |r| {
                r.push(full_session().into_iter().map(|s| s.event).collect());
                r.advance(10_000);
            },
            mouse: |r| {
                let sb = r.app.layout.scrollbar;
                r.mouse(MouseEventKind::Down(MouseButton::Left), sb.x, sb.y);
                r.mouse(MouseEventKind::Up(MouseButton::Left), sb.x, sb.y);
            },
            keys: |r| r.key(KeyCode::Home, KeyModifiers::CONTROL),
            observe: |r| {
                format!(
                    "{:?}",
                    r.app.active().map(|s| (s.scroll.top, s.scroll.follow))
                )
            },
        },
        case_t {
            name: "jump-to-latest chip / ctrl+end",
            setup: |r| {
                r.push(full_session().into_iter().map(|s| s.event).collect());
                r.advance(10_000);
                r.key(KeyCode::Home, KeyModifiers::CONTROL);
            },
            mouse: |r| r.click_action(&Action::JumpLatest),
            keys: |r| r.key(KeyCode::End, KeyModifiers::CONTROL),
            observe: |r| {
                format!(
                    "{:?}",
                    r.app.active().map(|s| (s.scroll.top, s.scroll.follow))
                )
            },
        },
        case_t {
            name: "slash popup entry / typed command + enter",
            setup: |r| r.ch('/'),
            mouse: |r| r.click_action(&Action::RunCommand("/help")),
            keys: |r| {
                r.typed("help");
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{} {:?}", r.app.help_open, r.app.composer.text()),
        },
        case_t {
            name: "clear-queue chip / ctrl+x",
            setup: |r| {
                r.push(generating_events());
                r.typed("later");
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            mouse: |r| r.click_action(&Action::ClearQueue),
            keys: |r| r.key(KeyCode::Char('x'), KeyModifiers::CONTROL),
            observe: |r| format!("{:?}", r.calls()),
        },
        case_t {
            name: "composer click / i",
            setup: |r| r.app.apply(Action::SetFocus(Focus::Transcript)),
            mouse: |r| r.click_action(&Action::SetFocus(Focus::Input)),
            keys: |r| r.ch('i'),
            observe: |r| format!("{:?}", r.app.focus),
        },
        case_t {
            name: "saved conversation / sidebar keys",
            setup: |_| {},
            mouse: |r| {
                let index = saved_index(r);
                r.click_row(index);
            },
            keys: |r| {
                let index = saved_index(r);
                r.app.apply(Action::SetFocus(Focus::Sidebar));
                r.frame();
                for _ in 0..40 {
                    if r.app.sidebar_sel == index {
                        break;
                    }
                    r.ch('j');
                }
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{:?}", r.app.active().map(|s| s.session.id().to_string())),
        },
        case_t {
            name: "+ folder button / ctrl+f",
            setup: noop,
            mouse: |r| r.click_action(&Action::NewFolder),
            keys: |r| r.key(KeyCode::Char('f'), KeyModifiers::CONTROL),
            observe: |r| format!("{:?} {:?}", r.app.prompt, r.app.focus),
        },
        case_t {
            name: "folder + chip / select and f",
            setup: seed_research,
            mouse: |r| {
                let i = folder_index(r, "Research");
                r.hover_row(i);
                r.click_action(&Action::NewSubfolderAt(i));
            },
            keys: |r| {
                let i = folder_index(r, "Research");
                r.select_by_keys(i);
                r.ch('f');
            },
            observe: |r| format!("{:?}", r.app.prompt),
        },
        case_t {
            name: "rename chip / select and r",
            setup: seed_research,
            mouse: |r| {
                let i = folder_index(r, "Research");
                r.hover_row(i);
                r.click_action(&Action::RenameFolderAt(i));
            },
            keys: |r| {
                let i = folder_index(r, "Research");
                r.select_by_keys(i);
                r.ch('r');
            },
            observe: |r| format!("{:?}", r.app.prompt),
        },
        case_t {
            name: "delete chip / select and d",
            setup: seed_research,
            mouse: |r| {
                let i = folder_index(r, "Research");
                r.hover_row(i);
                r.click_action(&Action::DeleteFolderAt(i));
            },
            keys: |r| {
                let i = folder_index(r, "Research");
                r.select_by_keys(i);
                r.ch('d');
            },
            observe: |r| format!("{:?}", r.app.confirm),
        },
        case_t {
            name: "delete confirm button / enter",
            setup: |r| {
                seed_research(r);
                let i = folder_index(r, "Research");
                r.select_by_keys(i);
                r.ch('d');
            },
            mouse: |r| r.click_action(&Action::ConfirmYes),
            keys: |r| r.key(KeyCode::Enter, KeyModifiers::NONE),
            observe: |r| format!("{:?}", r.app.organization),
        },
        case_t {
            name: "prompt cancel hint / esc",
            setup: |r| r.key(KeyCode::Char('f'), KeyModifiers::CONTROL),
            mouse: |r| r.click_action(&Action::SidebarCancel),
            keys: |r| {
                r.key(KeyCode::Esc, KeyModifiers::NONE);
                r.advance(EscapeDelayMs);
            },
            observe: |r| format!("{:?}", r.app.prompt),
        },
        case_t {
            name: "move chip / select and m",
            setup: seed_research,
            mouse: |r| {
                let i = saved_index(r);
                r.hover_row(i);
                r.click_action(&Action::PickUpAt(i));
            },
            keys: |r| {
                let i = saved_index(r);
                r.select_by_keys(i);
                r.ch('m');
            },
            observe: |r| format!("{:?} {}", r.app.pickup, r.app.sidebar_sel),
        },
        case_t {
            name: "drag onto a folder / m, choose, enter",
            setup: seed_research,
            mouse: |r| {
                let from = saved_index(r);
                let to = folder_index(r, "Research");
                r.drag_rows(from, to);
            },
            keys: |r| {
                let from = saved_index(r);
                r.select_by_keys(from);
                r.ch('m');
                let to = folder_index(r, "Research");
                r.choose_destination(to);
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{:?}", r.app.organization.folder_of("saved-1")),
        },
        case_t {
            name: "drag onto conversations / m, choose top level, enter",
            setup: |r| {
                seed_research(r);
                file_saved_in_research(r);
            },
            mouse: |r| {
                let from = saved_index(r);
                r.drag_rows(from, 1);
            },
            keys: |r| {
                let from = saved_index(r);
                r.select_by_keys(from);
                r.ch('m');
                r.choose_destination(1);
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{:?}", r.app.organization.folder_of("saved-1")),
        },
        case_t {
            name: "folder row click / select and enter",
            setup: seed_research,
            mouse: |r| {
                let i = folder_index(r, "Research");
                r.click_row(i);
            },
            keys: |r| {
                let i = folder_index(r, "Research");
                r.select_by_keys(i);
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{:?}", r.app.folded),
        },
        case_t {
            name: "tab click / alt+1",
            setup: |r| r.key(KeyCode::Char('t'), KeyModifiers::CONTROL),
            mouse: |r| {
                let t = r.app.hits.find_target(&Target::Tab(0)).unwrap_or_default();
                r.click(t.x + 3, t.y);
            },
            keys: |r| r.key(KeyCode::Char('1'), KeyModifiers::ALT),
            observe: |r| format!("{}", r.app.active),
        },
        case_t {
            name: "tab × chip / alt+w",
            setup: |r| r.key(KeyCode::Char('t'), KeyModifiers::CONTROL),
            mouse: |r| r.click_action(&Action::Tab(TabAction::Close(1))),
            keys: |r| r.key(KeyCode::Char('w'), KeyModifiers::ALT),
            observe: |r| format!("{} {}", r.app.sessions.len(), r.app.active),
        },
        case_t {
            name: "tab middle click / alt+w",
            setup: |r| r.key(KeyCode::Char('t'), KeyModifiers::CONTROL),
            mouse: |r| {
                let t = r.app.hits.find_target(&Target::Tab(1)).unwrap_or_default();
                r.click_with(MouseButton::Middle, t.x + 3, t.y, KeyModifiers::NONE);
            },
            keys: |r| r.key(KeyCode::Char('w'), KeyModifiers::ALT),
            observe: |r| format!("{} {}", r.app.sessions.len(), r.app.active),
        },
        case_t {
            name: "tab + chip / ctrl+t",
            setup: noop,
            mouse: |r| r.click_action(&Action::Tab(TabAction::New)),
            keys: |r| r.key(KeyCode::Char('t'), KeyModifiers::CONTROL),
            observe: |r| format!("{} {}", r.app.sessions.len(), r.app.active),
        },
        case_t {
            name: "tab right click / shift+f10",
            setup: noop,
            mouse: |r| {
                let t = r.app.hits.find_target(&Target::Tab(0)).unwrap_or_default();
                r.click_with(MouseButton::Right, t.x + 3, t.y, KeyModifiers::NONE);
            },
            keys: |r| r.key(KeyCode::F(10), KeyModifiers::SHIFT),
            observe: menu_view,
        },
        case_t {
            name: "row right click / . in the sidebar",
            setup: noop,
            mouse: |r| {
                let i = saved_index(r);
                r.right_click_row(i);
            },
            keys: |r| {
                let i = saved_index(r);
                r.select_by_keys(i);
                r.ch('.');
            },
            observe: menu_view,
        },
        case_t {
            name: "menu item click / menu keys",
            setup: |r| {
                let i = saved_index(r);
                r.select_by_keys(i);
                r.ch('.');
            },
            mouse: |r| r.click_action(&Action::Menu(MenuAction::Pick(3))),
            keys: |r| {
                for _ in 0..3 {
                    r.ch('j');
                }
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{:?}", r.app.organization.meta("saved-1")),
        },
        case_t {
            name: "check chip / ctrl+space",
            setup: noop,
            mouse: |r| {
                let i = saved_index(r);
                r.hover_row(i);
                r.click_action(&Action::Select(SelectAction::ToggleAt(i)));
            },
            keys: |r| {
                let i = saved_index(r);
                r.select_by_keys(i);
                r.key(KeyCode::Char(' '), KeyModifiers::CONTROL);
            },
            observe: |r| format!("{:?} {}", r.app.selection, r.app.sidebar_sel),
        },
        case_t {
            name: "ctrl+click / ctrl+space",
            setup: noop,
            mouse: |r| {
                let i = saved_index(r);
                r.click_row_with(i, KeyModifiers::CONTROL);
            },
            keys: |r| {
                let i = saved_index(r);
                r.select_by_keys(i);
                r.key(KeyCode::Char(' '), KeyModifiers::CONTROL);
            },
            observe: |r| format!("{:?} {}", r.app.selection, r.app.sidebar_sel),
        },
        case_t {
            name: "shift+click / shift+arrow",
            setup: noop,
            mouse: |r| {
                let (a, b) = (saved_index(r), other_index(r));
                r.click_row_with(a, KeyModifiers::CONTROL);
                r.click_row_with(b, KeyModifiers::SHIFT);
            },
            keys: |r| {
                let (a, b) = (saved_index(r), other_index(r));
                r.select_by_keys(a);
                r.key(KeyCode::Char(' '), KeyModifiers::CONTROL);
                let dir = if b > a { KeyCode::Down } else { KeyCode::Up };
                r.key(dir, KeyModifiers::SHIFT);
            },
            observe: |r| format!("{:?} {}", r.app.selection, r.app.sidebar_sel),
        },
        case_t {
            name: "selection clear chip / esc",
            setup: select_two,
            mouse: |r| r.click_action(&Action::Select(SelectAction::Clear)),
            keys: |r| {
                r.key(KeyCode::Esc, KeyModifiers::NONE);
                r.advance(EscapeDelayMs);
            },
            observe: |r| format!("{:?}", r.app.selection),
        },
        case_t {
            name: "selection menu chip / shift+f10",
            setup: select_two,
            mouse: |r| r.click_action(&Action::Menu(MenuAction::OpenSelection)),
            keys: |r| r.key(KeyCode::F(10), KeyModifiers::SHIFT),
            observe: menu_view,
        },
        case_t {
            name: "delete dialog button / enter",
            setup: |r| {
                let i = saved_index(r);
                r.select_by_keys(i);
                r.ch('d');
            },
            mouse: |r| r.click_action(&Action::ConfirmYes),
            keys: |r| r.key(KeyCode::Enter, KeyModifiers::NONE),
            observe: |r| format!("{:?} {:?}", r.rt.deleted(), r.app.delete_plan),
        },
        case_t {
            name: "delete dialog cancel button / n",
            setup: |r| {
                let i = saved_index(r);
                r.select_by_keys(i);
                r.ch('d');
            },
            mouse: |r| r.click_action(&Action::SidebarCancel),
            keys: |r| r.ch('n'),
            observe: |r| format!("{:?} {:?}", r.rt.deleted(), r.app.delete_plan),
        },
        case_t {
            name: "settings button / ctrl+o",
            setup: noop,
            mouse: |r| r.click_action(&Action::Page(PageAction::Open)),
            keys: |r| r.key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            observe: |r| format!("{} {:?}", r.app.page_shown, r.app.focus),
        },
        case_t {
            name: "settings section / digit",
            setup: |r| r.key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            mouse: |r| r.click_action(&Action::Page(PageAction::Section(Section::Sidebar))),
            keys: |r| r.ch('2'),
            observe: |r| format!("{:?}", r.app.page.as_ref().map(|p| p.section)),
        },
        case_t {
            name: "settings toggle chip / j enter",
            setup: |r| r.key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            mouse: |r| r.click_action(&Action::Page(PageAction::Toggle(1))),
            keys: |r| {
                r.ch('j');
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{} {}", r.app.settings.animation, r.app.animate),
        },
        case_t {
            name: "settings choice chip / j j right",
            setup: |r| r.key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            mouse: |r| r.click_action(&Action::Page(PageAction::Pick(2, 1))),
            keys: |r| {
                r.ch('j');
                r.ch('j');
                r.key(KeyCode::Right, KeyModifiers::NONE);
            },
            observe: |r| format!("{:?}", r.app.settings.density),
        },
        case_t {
            name: "settings theme chip / right",
            setup: |r| r.key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            mouse: |r| r.click_action(&Action::Page(PageAction::Pick(0, 1))),
            keys: |r| r.key(KeyCode::Right, KeyModifiers::NONE),
            observe: |r| format!("{} {:?}", r.app.settings.theme, r.app.theme.bg),
        },
        case_t {
            name: "settings reset chip / r",
            setup: |r| {
                r.key(KeyCode::Char('o'), KeyModifiers::CONTROL);
                r.app.apply(Action::Page(PageAction::Pick(2, 1)));
                r.app.apply(Action::Page(PageAction::Row(0)));
                r.frame();
            },
            mouse: |r| r.click_action(&Action::Page(PageAction::Reset(2))),
            keys: |r| {
                r.ch('j');
                r.ch('j');
                r.ch('r');
            },
            observe: |r| format!("{:?}", r.app.settings.density),
        },
        case_t {
            name: "settings theme row / enter",
            setup: |r| {
                r.key(KeyCode::Char('o'), KeyModifiers::CONTROL);
                r.ch('5');
                r.key(KeyCode::Tab, KeyModifiers::NONE);
            },
            mouse: |r| r.click_action(&Action::Page(PageAction::ApplyTheme("navy".into()))),
            keys: |r| {
                r.ch('j');
                r.ch('j');
                r.key(KeyCode::Enter, KeyModifiers::NONE);
                r.ch('k');
                r.ch('k');
            },
            observe: |r| r.app.settings.theme.clone(),
        },
        case_t {
            name: "settings import button / enter",
            setup: |r| {
                r.key(KeyCode::Char('o'), KeyModifiers::CONTROL);
                r.ch('5');
                r.key(KeyCode::Tab, KeyModifiers::NONE);
            },
            mouse: |r| {
                let at = crate::builtin_themes::BuiltinThemes.len();
                r.click_action(&Action::Page(PageAction::Press(at)));
            },
            keys: |r| {
                for _ in 0..crate::builtin_themes::BuiltinThemes.len() {
                    r.ch('j');
                }
                r.key(KeyCode::Enter, KeyModifiers::NONE);
            },
            observe: |r| format!("{:?}", r.app.page.as_ref().map(|p| p.prompt.is_some())),
        },
        case_t {
            name: "settings reset all button / enter",
            setup: |r| {
                r.key(KeyCode::Char('o'), KeyModifiers::CONTROL);
                r.app.apply(Action::Page(PageAction::Pick(2, 1)));
                r.ch('6');
                r.key(KeyCode::Tab, KeyModifiers::NONE);
            },
            mouse: |r| {
                let at = r.app.page_rows().len() - 1;
                r.click_action(&Action::Page(PageAction::Press(at)));
            },
            keys: |r| r.key(KeyCode::Enter, KeyModifiers::NONE),
            observe: |r| format!("{:?}", r.app.page.as_ref().map(|p| p.confirm.clone())),
        },
        case_t {
            name: "settings tab close / alt+w",
            setup: |r| r.key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            mouse: |r| r.click_action(&Action::Page(PageAction::Close)),
            keys: |r| r.key(KeyCode::Char('w'), KeyModifiers::ALT),
            observe: |r| format!("{} {}", r.app.page.is_some(), r.app.sessions.len()),
        },
        case_t {
            name: "settings tab / alt+.",
            setup: |r| {
                r.key(KeyCode::Char('o'), KeyModifiers::CONTROL);
                r.app.apply(Action::Tab(TabAction::Select(0)));
                r.frame();
            },
            mouse: |r| r.click_action(&Action::Page(PageAction::Show)),
            keys: |r| r.key(KeyCode::Char('.'), KeyModifiers::ALT),
            observe: |r| format!("{} {:?}", r.app.page_shown, r.app.focus),
        },
    ];
    for case in cases {
        let make = || {
            rig_t::with(140, 50, true, true, |rt| {
                let events = full_session()
                    .into_iter()
                    .map(|mut s| {
                        if let SessionEvent::SessionStart { session_id, .. } = &mut s.event {
                            *session_id = "saved-1".into();
                        }
                        s
                    })
                    .collect();
                rt.store("saved-1", events);
            })
        };
        let mut by_mouse = make();
        (case.setup)(&mut by_mouse);
        (case.mouse)(&mut by_mouse);
        let mut by_keys = make();
        (case.setup)(&mut by_keys);
        (case.keys)(&mut by_keys);
        let (m, k) = ((case.observe)(&by_mouse), (case.observe)(&by_keys));
        assert_eq!(m, k, "parity failed: {}", case.name);
        let baseline = {
            let mut r = make();
            (case.setup)(&mut r);
            (case.observe)(&r)
        };
        assert_ne!(m, baseline, "the action had no effect: {}", case.name);
    }
}

/// What an open menu offers, without where it was drawn.
fn menu_view(r: &rig_t) -> String {
    match &r.app.menu {
        Some(m) => format!(
            "{:?} {} {:?}",
            m.scope,
            m.caption,
            m.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>()
        ),
        None => "closed".to_string(),
    }
}

/// Parity setup: the two conversations selected with ctrl+click, the sidebar focused.
fn select_two(r: &mut rig_t) {
    let (a, b) = (saved_index(r), other_index(r));
    r.click_row_with(a, KeyModifiers::CONTROL);
    r.click_row_with(b, KeyModifiers::CONTROL);
}

/// The sidebar row of the open New chat, next to the saved conversation.
fn other_index(r: &rig_t) -> usize {
    r.app
        .sidebar_items
        .iter()
        .position(|i| matches!(&i.kind, crate::sidebar::SidebarItemKind::Session { id } if id != "saved-1"))
        .unwrap_or_else(|| panic!("no other conversation: {:?}", r.app.sidebar_items))
}

fn tool_block_id(r: &rig_t) -> u64 {
    r.app
        .active()
        .and_then(|s| {
            s.transcript
                .blocks
                .iter()
                .find(|b| matches!(b.kind, crate::view_model::BlockKind::Tool(_)))
                .map(|b| b.id)
        })
        .unwrap_or(u64::MAX)
}

pub(crate) fn saved_index(r: &rig_t) -> usize {
    r.app
        .sidebar_items
        .iter()
        .position(|i| matches!(&i.kind, crate::sidebar::SidebarItemKind::Session { id } if id == "saved-1"))
        .unwrap_or_else(|| panic!("saved session not listed: {:?}", r.app.sidebar_items))
}

/// Every action the mouse can reach in a busy frame belongs to the parity table above.
#[test]
fn every_click_target_has_a_keyboard_path() {
    let mut rig = rig_t::new(140, 50, true);
    rig.push(full_session().into_iter().map(|s| s.event).collect());
    rig.advance(10_000);
    rig.push(approval_events().into_iter().skip(1).collect());
    rig.ch('/');
    for hit in &rig.app.hits.hits {
        assert!(
            keyboard_path(&hit.target),
            "click target without a keyboard path: {:?}",
            hit.target
        );
    }
}

/// True when what a click on `target` does also has a key. The parity tables check each pair;
/// this list is what those tables cover.
pub(crate) fn keyboard_path(target: &Target) -> bool {
    match target {
        // Sidebar rows: j k and enter, m to move, shift and ctrl keys to select. Tabs: alt+1
        // to alt+9, alt+0, alt+. and alt+, (the tab tests cover each).
        Target::Scrollbar | Target::Row(_) | Target::Tab(_) => true,
        Target::Action(a) => matches!(
            a,
            Action::NewChat
                | Action::ToggleSidebar
                | Action::ToggleHelp
                | Action::Approve(_)
                | Action::PickOption(_)
                | Action::Cancel
                | Action::ToggleBlock(_)
                | Action::SelectBlockId(_)
                | Action::JumpLatest
                | Action::RunCommand(_)
                | Action::ClearQueue
                | Action::SetFocus(_)
                | Action::SidebarActivateAt(_)
                | Action::FocusNext
                | Action::Submit
                | Action::QuestionSubmit
                | Action::ToggleSelected
                | Action::SidebarActivate
                | Action::NewFolder
                | Action::NewSubfolder
                | Action::NewSubfolderAt(_)
                | Action::RenameFolderAt(_)
                | Action::DeleteFolderAt(_)
                | Action::DeleteRows
                | Action::PickUp
                | Action::PickUpAt(_)
                | Action::MoveDrop
                | Action::PromptSubmit
                | Action::ConfirmYes
                | Action::SidebarCancel
                | Action::Tab(_)
                | Action::Select(_)
                | Action::Menu(_)
                | Action::Page(_)
                // A toast goes on its own after its time; a click only ends it sooner.
                | Action::DismissToast
        ),
    }
}

#[test]
fn layouts_at_many_widths() {
    for (w, docked) in [
        (40u16, false),
        (59, false),
        (60, false),
        (80, false),
        (99, false),
        (100, true),
        (140, true),
    ] {
        let mut rig = rig_t::new(w, 30, false);
        rig.push(full_session().into_iter().map(|s| s.event).collect());
        let screen = rig.screen();
        assert_eq!(
            screen.contains("conversations"),
            docked,
            "width {w}\n{screen}"
        );
        assert!(screen.contains("DEMO"), "width {w}\n{screen}");
        if !docked {
            rig.key(KeyCode::Char('b'), KeyModifiers::CONTROL);
            assert!(
                rig.screen().contains("New chat"),
                "drawer at {w}\n{}",
                rig.screen()
            );
            assert_eq!(rig.app.focus, Focus::Sidebar);
            rig.key(KeyCode::Char('b'), KeyModifiers::CONTROL);
            assert!(!rig.screen().contains("conversations"));
        }
    }
}

#[test]
fn too_small_then_recovers() {
    let mut rig = rig_t::new(39, 11, true);
    let screen = rig.screen();
    assert!(screen.contains("terminal too small (39x11,"), "{screen}");
    assert!(screen.contains("40x12)"), "{screen}");
    rig.resize(40, 12);
    assert!(!rig.screen().contains("too small"));
    rig.resize(10, 3);
    assert!(rig.screen().contains("too small"));
    rig.resize(1, 1);
    rig.resize(0, 0);
    rig.resize(90, 30);
    assert!(rig.screen().contains("your turn"));
}

#[test]
fn resize_while_scrolled_keeps_the_top_block() {
    let mut rig = rig_t::new(120, 30, false);
    let mut events = vec![session_start()];
    for turn in 1..=40u32 {
        events.push(SessionEvent::TurnStart { turn });
        events.push(SessionEvent::UserMessage {
            turn,
            text: format!("message number {turn} with some words to wrap around the narrow width"),
            from_queue: false,
        });
        events.push(SessionEvent::TurnEnd {
            turn,
            outcome: TurnOutcome::Completed,
            calls: 0,
            passes: 0,
        });
    }
    rig.push(events);
    rig.key(KeyCode::PageUp, KeyModifiers::NONE);
    rig.key(KeyCode::PageUp, KeyModifiers::NONE);
    let top_block = |r: &rig_t| {
        let s = r.app.active().unwrap_or_else(|| panic!("session"));
        s.view
            .block_at(s.scroll.top)
            .and_then(|i| s.transcript.blocks.get(i))
            .map(|b| b.id)
    };
    let before = top_block(&rig);
    assert!(!rig.app.active().is_some_and(|s| s.scroll.follow));
    rig.resize(70, 30);
    assert_eq!(top_block(&rig), before);
    // And with a modal open.
    rig.push(approval_events().into_iter().skip(1).collect());
    rig.key(KeyCode::Up, KeyModifiers::NONE);
    let before = top_block(&rig);
    rig.resize(140, 40);
    assert_eq!(top_block(&rig), before);
    assert_eq!(rig.app.modal(), Modal::Approval);
}

#[test]
fn long_transcript_redraws_from_cache() {
    let mut rig = rig_t::new(140, 50, false);
    let mut events = vec![session_start()];
    for turn in 1..=3000u32 {
        events.push(SessionEvent::TurnStart { turn });
        events.push(SessionEvent::UserMessage {
            turn,
            text: format!("question {turn}"),
            from_queue: false,
        });
        events.push(call_start(turn, 0, 0));
        events.push(level_done(turn, 0, 0, 0, None));
        events.push(call_end(turn, 0, 0, "A reply with **bold** text.", true));
        events.push(SessionEvent::TurnEnd {
            turn,
            outcome: TurnOutcome::Completed,
            calls: 1,
            passes: 1,
        });
    }
    rig.push(events);
    let total = rig.app.active().map(|s| s.view.total).unwrap_or(0);
    assert!(total > 5000, "{total}");
    let start = Instant::now();
    for _ in 0..5 {
        rig.advance(16);
    }
    let per_frame = start.elapsed() / 5;
    let rendered = rig
        .app
        .active()
        .map(|s| s.view.rendered)
        .unwrap_or(usize::MAX);
    assert!(rendered <= 2, "re-rendered {rendered} blocks");
    assert!(per_frame.as_millis() < 250, "{per_frame:?}");
}

#[test]
fn mouse_off_still_works_by_keyboard() {
    let mut rig = rig_t::with(120, 40, true, false, |_| {});
    rig.push(approval_events());
    let r = rig.rect_of(&Action::Approve(ApprovalDecision::Deny));
    rig.click(r.x, r.y);
    assert!(rig.calls().is_empty());
    rig.ch('n');
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::ResolveApproval(
            "t1".into(),
            ApprovalDecision::Deny
        )]
    );
}

#[test]
fn approval_full_is_refused_with_its_message() {
    let mut rig = rig_t::new(120, 40, true);
    rig.typed("/approval full");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::SetApprovalMode(ApprovalMode::Full)]
    );
    // The toast has revealed.
    rig.advance(crate::toast::MaxToastFxMs);
    assert!(
        rig.screen()
            .contains("must be selected on the command line"),
        "{}",
        rig.screen()
    );
    rig.typed("/approval edit");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.screen().contains("approval mode is now edit"),
        "{}",
        rig.screen()
    );
}

#[test]
fn copy_writes_osc52_to_the_output_queue() {
    let mut rig = rig_t::new(120, 40, false);
    rig.push(full_session().into_iter().map(|s| s.event).collect());
    rig.key(KeyCode::Char('y'), KeyModifiers::CONTROL);
    let expected = crate::clipboard::osc52("## Done\n\nAll **good**.");
    assert_eq!(rig.app.outputs, vec![expected]);
}

#[test]
fn quitting() {
    let mut rig = rig_t::new(100, 30, true);
    rig.typed("/quit");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(rig.app.quit);

    let mut rig = rig_t::new(100, 30, true);
    rig.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!rig.app.quit);
    rig.advance(crate::toast::MaxToastFxMs);
    assert!(rig.screen().contains("press ctrl+c again to quit"));
    rig.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(rig.app.quit);

    let mut rig = rig_t::new(100, 30, true);
    rig.key(KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert!(rig.app.quit);
}

#[test]
fn typing_during_a_turn_queues_and_shows_chips() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(generating_events());
    rig.typed("follow up");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.calls(), vec![ScriptCall::Submit("follow up".into())]);
    let screen = rig.screen();
    assert!(screen.contains("queued"), "{screen}");
    assert!(screen.contains("follow up"), "{screen}");
    assert!(screen.contains("ctrl+x clear"), "{screen}");
}

#[test]
fn saved_session_opens_from_the_sidebar_settled() {
    let mut rig = rig_t::with(140, 60, true, true, |rt| {
        let events = full_session()
            .into_iter()
            .map(|mut s| {
                if let SessionEvent::SessionStart { session_id, .. } = &mut s.event {
                    *session_id = "saved-1".into();
                }
                s
            })
            .collect();
        rt.store("saved-1", events);
    });
    assert!(rig.screen().contains("please approve"), "{}", rig.screen());
    let index = saved_index(&rig);
    rig.click_row(index);
    assert_eq!(
        rig.app
            .active()
            .map(|s| s.session.id().to_string())
            .as_deref(),
        Some("saved-1")
    );
    // Replayed events render settled: the final text is there without waiting.
    assert!(rig.screen().contains("All good."), "{}", rig.screen());
}

#[test]
fn background_sessions_keep_pumping() {
    let mut rig = rig_t::new(140, 40, true);
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.active, 1);
    rig.handle(0).push(generating_events());
    rig.frame();
    let first = rig
        .app
        .sessions
        .first()
        .map(|s| s.transcript.blocks.len())
        .unwrap_or(0);
    assert!(first > 2);
    assert!(
        rig.app
            .sidebar_items
            .iter()
            .any(|i| i.activity == crate::sidebar::Activity::Generating)
    );
}

#[test]
fn indexed_colour_mode_leaves_no_rgb() {
    let rt = Arc::new(scripted_runtime_t::manual(preview_info("/x/y")));
    let dyn_rt: Arc<dyn runtime_t> = rt.clone();
    let mut app = app_t::new(
        dyn_rt,
        app_options_t {
            animate: true,
            mouse: true,
            color: ColorMode::Indexed256,
        },
        0,
        0,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    if let Some(h) = rt.handle(0) {
        h.push(full_session().into_iter().map(|s| s.event));
    }
    app.tick(10, 10);
    let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap_or_else(|e| match e {});
    let _ = term.draw(|f| draw(&mut app, f));
    for cell in &term.backend().buffer().content {
        assert!(!matches!(cell.fg, Color::Rgb(..)));
        assert!(!matches!(cell.bg, Color::Rgb(..)));
    }
}

#[test]
fn help_lists_keys_mouse_and_native_selection() {
    let mut rig = rig_t::new(120, 60, true);
    rig.ch('?');
    let mut seen = rig.screen();
    for _ in 0..40 {
        rig.ch('j');
        seen.push_str(&rig.screen());
    }
    for needle in [
        "ncagent help",
        "ctrl+y",
        "shift+drag",
        "level strip",
        "/approval",
        "approve",
    ] {
        assert!(seen.contains(needle), "missing {needle}\n{seen}");
    }
    assert!(rig.app.help_open);
    rig.ch('x');
    assert!(!rig.app.help_open);
}

#[test]
fn hostile_event_text_never_reaches_the_screen_as_escapes() {
    let mut rig = rig_t::new(100, 40, false);
    let mut e = start_turn("\u{1b}[2J\u{1b}]52;c;AAAA\u{7}evil");
    e.push(SessionEvent::ToolResult {
        turn: 1,
        tool_call_id: "x".into(),
        tool: "read_file\u{1b}[31m".into(),
        ok: true,
        output: "\u{1b}]0;title\u{7}line\n".repeat(500),
        truncated: true,
        simulated: true,
    });
    e.push(SessionEvent::Error {
        turn: Some(1),
        scope: harness::event::ErrorScope::Backend,
        name: "x".repeat(10_000),
        message: "日本語🙂e\u{301}".repeat(300),
    });
    rig.push(e);
    for cell in &rig.term.backend().buffer().content {
        assert!(!cell.symbol().contains('\u{1b}'));
        assert!(!cell.symbol().contains('\u{7}'));
    }
}

#[test]
fn scrollbar_drag_and_wheel() {
    let mut rig = rig_t::new(120, 30, false);
    rig.push(full_session().into_iter().map(|s| s.event).collect());
    let sb = rig.app.layout.scrollbar;
    rig.mouse(MouseEventKind::Down(MouseButton::Left), sb.x, sb.y);
    assert_eq!(rig.app.active().map(|s| s.scroll.top), Some(0));
    assert!(rig.app.dragging.is_some());
    rig.mouse(
        MouseEventKind::Drag(MouseButton::Left),
        sb.x,
        sb.bottom() + 5,
    );
    assert!(rig.app.active().is_some_and(|s| s.scroll.follow));
    rig.mouse(MouseEventKind::Up(MouseButton::Left), sb.x, sb.y);
    assert!(rig.app.dragging.is_none());
    let t = rig.app.layout.transcript;
    rig.mouse(MouseEventKind::ScrollUp, t.x + 5, t.y + 5);
    let s = rig.app.active().map(|s| s.scroll).unwrap_or_default();
    assert!(!s.follow);
    assert_eq!(s.top, s.max_top() - 3);
    assert!(rig.screen().contains("jump to latest"));
}

#[test]
fn scroll_maths() {
    let mut s = crate::app::scroll_t {
        total: 100,
        viewport: 20,
        ..Default::default()
    };
    s.to_track(0, 20);
    assert_eq!((s.top, s.follow), (0, false));
    s.to_track(19, 20);
    assert_eq!((s.top, s.follow), (80, true));
    s.to_track(u16::MAX, 20);
    assert_eq!(s.top, 80);
    s.scroll_by(-1000);
    assert_eq!(s.top, 0);
    s.scroll_by(i64::MAX);
    assert_eq!((s.top, s.follow), (80, true));
    let mut tiny = crate::app::scroll_t {
        total: 3,
        viewport: 20,
        ..Default::default()
    };
    tiny.to_track(5, 0);
    assert_eq!(tiny.top, 0);
}

#[test]
fn escape_toggles_navigate_mode_when_idle() {
    let mut rig = rig_t::new(120, 40, true);
    rig.push(full_session().into_iter().map(|s| s.event).collect());
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    assert_eq!(rig.app.focus, Focus::Transcript);
    assert!(rig.app.active().is_some_and(|s| s.selected_block.is_some()));
    rig.ch('k');
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    assert_eq!(rig.app.focus, Focus::Input);
    assert!(rig.calls().is_empty());
}

#[test]
fn tab_cycles_focus_through_visible_areas() {
    let mut rig = rig_t::new(140, 40, true);
    let mut seen = vec![rig.app.focus];
    for _ in 0..3 {
        rig.key(KeyCode::Tab, KeyModifiers::NONE);
        seen.push(rig.app.focus);
    }
    assert_eq!(
        seen,
        vec![
            Focus::Input,
            Focus::Transcript,
            Focus::Sidebar,
            Focus::Input
        ]
    );
    let mut rig = rig_t::new(80, 40, true);
    rig.key(KeyCode::Tab, KeyModifiers::NONE);
    rig.key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(rig.app.focus, Focus::Input);
}

#[test]
fn sticky_label_names_the_speaker_of_the_top_line() {
    let mut rig = rig_t::new(100, 30, false);
    let long: String = (1..=60).map(|i| format!("line {i}\n")).collect();
    let mut events = start_turn(&long);
    events.push(SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Completed,
        calls: 0,
        passes: 0,
    });
    rig.push(events);
    let start =
        rig.app
            .active()
            .and_then(|s| {
                let i =
                    s.transcript.blocks.iter().position(|b| {
                        matches!(b.kind, crate::view_model::BlockKind::User { .. })
                    })?;
                s.view.starts.get(i).copied()
            })
            .unwrap_or(0);
    rig.key(KeyCode::Home, KeyModifiers::CONTROL);
    rig.app.apply(Action::ScrollBy(start as i32 + 10));
    rig.frame();
    let first = usize::from(rig.app.layout.transcript.y);
    let top = rig.screen().lines().nth(first).unwrap_or("").to_string();
    assert!(top.contains("you · turn 1"), "{top}");
    assert!(top.contains("continued"), "{top}");
}

/// The screen row of the logo's second row and its column, in a settled frame.
fn logo_row(screen: &str) -> (usize, usize) {
    let want = crate::banner::logo_rows()[1].clone();
    screen
        .lines()
        .enumerate()
        .find_map(|(i, l)| l.find(&want).map(|b| (i, l[..b].chars().count())))
        .unwrap_or_else(|| panic!("no logo\n{screen}"))
}

#[test]
fn startup_logo_reveals_left_to_right_then_equals_the_static_card() {
    let still = rig_t::new(140, 40, false);
    let still_screen = still.screen();
    let (y, x) = logo_row(&still_screen);
    let still_line: Vec<char> = still_screen.lines().nth(y).unwrap_or("").chars().collect();

    let mut rig = rig_t::new(140, 40, true);
    let first: Vec<char> = rig.screen().lines().nth(y).unwrap_or("").chars().collect();
    assert_ne!(
        first, still_line,
        "the first frame is the effect, not the card"
    );
    assert!(
        rig.app
            .active()
            .is_some_and(|s| s.view.any_live(&s.transcript))
    );
    rig.advance(450);
    let mid: Vec<char> = rig.screen().lines().nth(y).unwrap_or("").chars().collect();
    assert_eq!(
        mid[x..x + 13],
        still_line[x..x + 13],
        "the left is sharp first"
    );
    assert_ne!(mid, still_line, "the right is still settling");
    rig.advance(550);
    // Settled: the whole frame, styles included, is today's static card.
    assert_eq!(rig.screen(), still_screen);
    assert_eq!(
        rig.term.backend().buffer().content,
        still.term.backend().buffer().content
    );
    assert!(rig.app.active().is_some_and(|s| s.logo_from.is_none()));
}

#[test]
fn logo_reveal_is_skipped_without_animation_or_off_screen() {
    // Instant mode: the first frame is the card.
    let rig = rig_t::new(140, 40, false);
    assert!(rig.app.active().is_some_and(|s| s.logo_from.is_none()));
    logo_row(&rig.screen());
    // Turning the effect off mid-reveal settles it at once.
    let mut rig = rig_t::new(140, 40, true);
    rig.advance(100);
    rig.typed("/animate");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    logo_row(&rig.screen());
    // A card scrolled out of view stops its reveal, so it never plays off screen.
    let mut rig = rig_t::new(140, 40, true);
    let mut events = vec![session_start()];
    for turn in 1..=30u32 {
        events.push(SessionEvent::TurnStart { turn });
        events.push(SessionEvent::UserMessage {
            turn,
            text: format!("message {turn}"),
            from_queue: false,
        });
    }
    rig.push(events);
    assert!(rig.app.active().is_some_and(|s| s.logo_from.is_none()));
    rig.key(KeyCode::Home, KeyModifiers::CONTROL);
    logo_row(&rig.screen());
    // A new chat plays it again for its own card.
    let mut rig = rig_t::new(140, 40, true);
    rig.advance(2_000);
    rig.key(KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert!(rig.app.active().is_some_and(|s| s.logo_from.is_some()));
}

#[test]
fn run_refuses_without_a_terminal() {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        // Under a real terminal this would start the UI; the check is covered elsewhere.
        return;
    }
    let rt: Arc<dyn runtime_t> = Arc::new(scripted_runtime_t::manual(preview_info("/p")));
    let result = crate::run(rt, crate::tui_options_t::default());
    assert_eq!(result, Err(crate::TuiError::NotATerminal));
}
