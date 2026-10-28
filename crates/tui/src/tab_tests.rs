/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Tab tests (D62, D63): the tab bar drawn on ratatui's `TestBackend` and driven by synthetic
//! crossterm events, against the scripted fixture; and two conversations generating at once
//! against the real `controller_runtime_t` with a gated test backend, so a call stays open
//! until the test lets it finish.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use harness::api::{SessionError, SessionPhase, runtime_t};
use harness::approval::{ApprovalDecision, ApprovalMode, ApprovalSource};
use harness::backend::{
    BackendError, BackendKind, CallOutcome, call_request_t, cancelled_t, capabilities_t,
    completion_t, level_progress_t, model_identity_t, sampler_t, text_codec_t,
};
use harness::event::{SessionEvent, TurnOutcome};
use harness::runtime::{MaxAttempts, PassBudget, controller_runtime_t, runtime_config_t};
use harness::settings::{OpenIn, SettingsError, settings_t};
use harness::store::memory_store_t;
use harness::tools::{ApprovalClass, unavailable_tool_runner_t};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

use crate::app::{Attention, app_options_t, app_t};
use crate::app_tests::{rig_t, start_turn};
use crate::canvas::buffer_lines;
use crate::draw::draw;
use crate::folder_tests::{ok, saved, session_index, toast};
use crate::input::{Action, Focus, TabAction, Target};
use crate::scripted::{ScriptCall, preview_info, scripted_runtime_t};
use crate::theme::{ColorMode, theme_t};
use crate::view_model::tests::{call_start, level_done};

fn alt(rig: &mut rig_t, c: char) {
    rig.key(KeyCode::Char(c), KeyModifiers::ALT);
}

fn ids(rig: &rig_t) -> Vec<String> {
    rig.app
        .sessions
        .iter()
        .map(|s| s.session.id().to_string())
        .collect()
}

fn active_id(rig: &rig_t) -> String {
    rig.app
        .active()
        .map(|s| s.session.id().to_string())
        .unwrap_or_default()
}

/// A rig with stored conversations s-1 to s-4 and nothing else open.
fn stored_rig(width: u16, height: u16, setup: impl FnOnce(&scripted_runtime_t)) -> rig_t {
    rig_t::with(width, height, false, true, |rt| {
        for i in 1..=4u64 {
            let id = format!("s-{i}");
            rt.store(&id, saved(&id, &format!("topic {i}"), 9_000 - i * 1_000));
        }
        setup(rt);
    })
}

/// A second app on the same runtime: what the next start of the program sees.
fn relaunch(rt: &Arc<scripted_runtime_t>, width: u16, height: u16) -> rig_t {
    let dyn_rt: Arc<dyn runtime_t> = rt.clone();
    let app = app_t::new(
        dyn_rt,
        app_options_t {
            animate: false,
            mouse: true,
            color: ColorMode::TrueColor,
        },
        0,
        10_000_000,
    )
    .unwrap_or_else(|e| panic!("app: {}", e.message));
    let term = Terminal::new(TestBackend::new(width, height)).unwrap_or_else(|e| match e {});
    let mut rig = rig_t {
        rt: Arc::clone(rt),
        app,
        term,
        clock: 0,
    };
    rig.frame();
    rig
}

/// The glyph and colour of a tab's activity mark, if it shows one.
fn tab_mark(rig: &rig_t, tab: usize) -> Option<(String, Color)> {
    let r = rig.app.hits.find_target(&Target::Tab(tab))?;
    let buf = rig.term.backend().buffer();
    (r.x + 2..r.right()).find_map(|x| {
        let cell = buf.cell((x, r.y))?;
        matches!(cell.symbol(), "●" | "◆").then(|| (cell.symbol().to_string(), cell.fg))
    })
}

/// Six tabs: generating, waiting for an approval, running a tool, finished in the background,
/// a question in the background, and the shown one, idle.
fn busy_tabs(width: u16) -> rig_t {
    let mut rig = rig_t::new(width, 30, false);
    for _ in 0..5 {
        rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    }
    assert_eq!(rig.app.sessions.len(), 6);
    let mut gen_events = start_turn("hello");
    gen_events.push(call_start(1, 0, 0));
    gen_events.push(level_done(1, 0, 0, 0, None));
    rig.handle(0).push(gen_events);
    let mut approval = start_turn("please approve");
    approval.push(SessionEvent::ToolCall {
        turn: 1,
        tool_call_id: "t1".into(),
        tool: "replace_lines".into(),
        class: ApprovalClass::Write,
        args: vec![],
    });
    approval.push(SessionEvent::ApprovalRequested {
        turn: 1,
        tool_call_id: "t1".into(),
        tool: "replace_lines".into(),
        class: ApprovalClass::Write,
        summary: "lines".into(),
        reason: "write".into(),
    });
    rig.handle(1).push(approval);
    let mut tool = start_turn("tool");
    tool.push(SessionEvent::ToolCall {
        turn: 1,
        tool_call_id: "t2".into(),
        tool: "list_dir".into(),
        class: ApprovalClass::Read,
        args: vec![],
    });
    tool.push(SessionEvent::Approval {
        tool_call_id: "t2".into(),
        decision: ApprovalDecision::Once,
        source: ApprovalSource::Mode,
    });
    rig.handle(2).push(tool);
    let mut done = start_turn("done");
    done.push(SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Completed,
        calls: 0,
        passes: 0,
    });
    rig.handle(3).push(done);
    let mut ask = start_turn("ask me");
    ask.push(SessionEvent::QuestionAsked {
        turn: 1,
        question_id: "q1".into(),
        question: "Which?".into(),
        options: vec!["a".into()],
    });
    rig.handle(4).push(ask);
    rig.frame();
    rig
}

#[test]
fn the_tab_bar_shows_each_activity_at_every_width_and_scrolls_when_full() {
    let theme = theme_t::dark();
    for width in [130u16, 80, 50, 40] {
        let mut rig = busy_tabs(width);
        assert_eq!(rig.app.active, 5);
        let bar = rig.screen().lines().next().unwrap_or("").to_string();
        for c in ['┌', '┐', '└', '┘', '│', '─'] {
            assert!(!bar.contains(c), "box character at {width}: {bar}");
        }
        assert!(
            rig.app.hits.find_target(&Target::Tab(5)).is_some(),
            "the shown tab is in view at {width}: {bar}"
        );
        assert!(rig.app.hits.find(&Action::Tab(TabAction::New)).is_some());
        if width >= 130 {
            assert!(bar.contains("please approve"), "{bar}");
            assert!(!bar.contains('‹'), "no scrolling at {width}: {bar}");
            let (g, c) = tab_mark(&rig, 0).unwrap_or_default();
            assert_eq!(g, "●");
            assert_ne!(c, theme.tool);
            assert_eq!(tab_mark(&rig, 1), Some(("◆".into(), theme.approval)));
            assert_eq!(tab_mark(&rig, 2), Some(("●".into(), theme.tool)));
            assert_eq!(tab_mark(&rig, 3), Some(("●".into(), theme.success)));
            assert_eq!(tab_mark(&rig, 4), Some(("◆".into(), theme.approval)));
            assert_eq!(tab_mark(&rig, 5), None);
        }
        if width <= 50 {
            assert!(bar.starts_with('‹') && bar.ends_with('›'), "{width}: {bar}");
            // The chips scroll the bar back to the first tab.
            for _ in 0..6 {
                if rig.app.hits.find_target(&Target::Tab(0)).is_some() {
                    break;
                }
                rig.click_action(&Action::Tab(TabAction::Scroll(-1)));
            }
            assert!(
                rig.app.hits.find_target(&Target::Tab(0)).is_some(),
                "{}",
                rig.screen()
            );
            // The wheel over the bar scrolls it too.
            rig.mouse(MouseEventKind::ScrollDown, 10, 0);
            assert!(rig.app.tab_scroll > 0);
            // Showing another tab brings it into view.
            alt(&mut rig, '0');
            assert!(rig.app.hits.find_target(&Target::Tab(5)).is_some());
        }
        // The shown tab is lifted: a tone above the bar, with a rail.
        let r = rig
            .app
            .hits
            .find_target(&Target::Tab(rig.app.active))
            .unwrap_or_default();
        let cell = rig.term.backend().buffer().cell((r.x, r.y)).cloned();
        assert_eq!(cell.as_ref().map(|c| c.symbol()), Some("▌"), "{width}");
        assert_ne!(cell.map(|c| c.bg), Some(theme.side), "{width}");
    }
}

#[test]
fn background_tabs_keep_folding_and_say_when_they_need_you() {
    let mut rig = rig_t::new(130, 40, false);
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.active, 1);
    let mut events = start_turn("background work");
    events.push(call_start(1, 0, 0));
    events.push(level_done(1, 0, 0, 0, None));
    rig.handle(0).push(events);
    rig.frame();
    assert!(
        rig.app.sessions[0].transcript.blocks.len() > 2,
        "folded while hidden"
    );
    assert_eq!(rig.app.sessions[0].attention, Attention::None);
    rig.handle(0).push([SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Completed,
        calls: 1,
        passes: 1,
    }]);
    rig.frame();
    assert_eq!(rig.app.sessions[0].attention, Attention::Finished);
    rig.handle(0).push([SessionEvent::QuestionAsked {
        turn: 1,
        question_id: "q".into(),
        question: "?".into(),
        options: vec![],
    }]);
    rig.frame();
    assert_eq!(rig.app.sessions[0].attention, Attention::NeedsYou);
    alt(&mut rig, '1');
    assert_eq!(rig.app.active, 0);
    assert_eq!(rig.app.sessions[0].attention, Attention::None, "seen");
    assert!(rig.screen().contains("background work"));
}

#[test]
fn tab_keys_and_clicks_switch_open_and_close() {
    let mut rig = stored_rig(130, 40, |_| {});
    rig.click_row(session_index(&rig, "s-1"));
    rig.click_row(session_index(&rig, "s-2"));
    rig.click_row(session_index(&rig, "s-3"));
    assert_eq!(
        ids(&rig),
        ["s-1", "s-2", "s-3"],
        "the unused New chat was replaced"
    );
    assert_eq!(rig.app.active, 2);
    alt(&mut rig, '.');
    assert_eq!(rig.app.active, 0, "next wraps");
    alt(&mut rig, ',');
    assert_eq!(rig.app.active, 2, "previous wraps");
    rig.key(KeyCode::PageDown, KeyModifiers::CONTROL);
    assert_eq!(rig.app.active, 0);
    rig.key(KeyCode::PageUp, KeyModifiers::CONTROL);
    assert_eq!(rig.app.active, 2);
    alt(&mut rig, '2');
    assert_eq!(active_id(&rig), "s-2");
    alt(&mut rig, '0');
    assert_eq!(active_id(&rig), "s-3");
    alt(&mut rig, '9');
    assert_eq!(toast(&rig), "there is no tab 9");
    // A click shows a tab; a middle click and the × chip close one.
    let r = rig
        .app
        .hits
        .find_target(&Target::Tab(0))
        .unwrap_or_default();
    rig.click(r.x + 3, r.y);
    assert_eq!(active_id(&rig), "s-1");
    let r = rig
        .app
        .hits
        .find_target(&Target::Tab(1))
        .unwrap_or_default();
    rig.click_with(MouseButton::Middle, r.x + 3, r.y, KeyModifiers::NONE);
    assert_eq!(ids(&rig), ["s-1", "s-3"]);
    rig.click_action(&Action::Tab(TabAction::Close(0)));
    assert_eq!(ids(&rig), ["s-3"]);
    // Clicking an open conversation shows its tab instead of opening it twice.
    rig.click_row(session_index(&rig, "s-3"));
    assert_eq!(ids(&rig), ["s-3"]);
    // + and ctrl+t open a new chat in a new tab; ctrl+n reuses an unused one.
    rig.click_action(&Action::Tab(TabAction::New));
    assert_eq!(rig.app.sessions.len(), 2);
    rig.key(KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.sessions.len(), 2, "the unused New chat is reused");
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.sessions.len(), 3);
    // alt+w closes the shown tab; /close too; the last unused tab stays.
    rig.key(KeyCode::Char('w'), KeyModifiers::ALT);
    rig.typed("/close");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(ids(&rig), ["s-3"]);
    rig.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(
        rig.app.sessions.len(),
        1,
        "a New chat replaces the last tab"
    );
    assert!(rig.app.active_untouched());
    rig.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(toast(&rig), "this is the only tab");
    // ctrl+w still deletes a word in the composer.
    rig.typed("two words");
    rig.key(KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.composer.text(), "two ");
}

#[test]
fn each_tab_keeps_its_draft_scroll_and_expanded_blocks() {
    let mut rig = stored_rig(130, 30, |_| {});
    rig.click_row(session_index(&rig, "s-1"));
    rig.app.apply(Action::SetFocus(Focus::Input));
    rig.typed("draft one");
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert!(rig.app.composer.is_empty(), "the new tab has its own draft");
    rig.typed("draft two");
    alt(&mut rig, '1');
    assert_eq!(rig.app.composer.text(), "draft one");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::Submit("draft one".into())],
        "typing goes to the shown tab only"
    );
    assert!(rig.tab_handle(1).calls().is_empty());
    alt(&mut rig, '2');
    assert_eq!(rig.app.composer.text(), "draft two");
    // Scroll and an expanded block survive a round trip.
    let mut long = start_turn("long");
    for turn in 2..=30u32 {
        long.push(SessionEvent::TurnStart { turn });
        long.push(SessionEvent::UserMessage {
            turn,
            text: format!("message {turn}"),
            from_queue: false,
        });
    }
    rig.push(long);
    rig.key(KeyCode::Home, KeyModifiers::CONTROL);
    let id = rig
        .app
        .active()
        .and_then(|s| s.transcript.blocks.get(1).map(|b| b.id))
        .unwrap_or(0);
    rig.app.apply(Action::ToggleBlock(id));
    let before = rig.app.active().map(|s| {
        (
            s.scroll.top,
            s.scroll.follow,
            s.transcript.blocks[1].expanded,
        )
    });
    alt(&mut rig, '1');
    alt(&mut rig, '2');
    let after = rig.app.active().map(|s| {
        (
            s.scroll.top,
            s.scroll.follow,
            s.transcript.blocks[1].expanded,
        )
    });
    assert_eq!(before, after);
}

#[test]
fn closing_a_busy_tab_stops_its_run_first() {
    let mut rig = stored_rig(130, 30, |_| {});
    rig.click_row(session_index(&rig, "s-1"));
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    alt(&mut rig, '1');
    rig.push(start_turn("running"));
    let handle = rig.tab_handle(0);
    handle.set_phase(SessionPhase::Generating);
    rig.frame();
    assert!(rig.app.is_running("s-1"));
    rig.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(handle.calls(), vec![ScriptCall::Cancel]);
    assert_eq!(toast(&rig), "stopped “topic 1”");
    assert_eq!(rig.app.sessions.len(), 1);
    assert_eq!(rig.app.closing.len(), 1, "kept until it stops");
    assert!(
        rig.app.is_running("s-1"),
        "still stopping: cannot be deleted"
    );
    handle.push([SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Cancelled,
        calls: 0,
        passes: 0,
    }]);
    handle.set_phase(SessionPhase::Idle);
    rig.frame();
    assert!(rig.app.closing.is_empty());
    assert!(!rig.app.is_running("s-1"));
}

#[test]
fn open_in_new_tab_or_in_the_current_idle_tab() {
    let mut rig = stored_rig(130, 30, |_| {});
    rig.click_row(session_index(&rig, "s-1"));
    rig.click_row(session_index(&rig, "s-2"));
    assert_eq!(ids(&rig), ["s-1", "s-2"]);
    assert!(rig.app.update_settings(|s| s.open_in = OpenIn::CurrentTab));
    rig.click_row(session_index(&rig, "s-3"));
    assert_eq!(ids(&rig), ["s-1", "s-3"], "the idle shown tab was replaced");
    // A busy tab, or one with a draft, is not replaced.
    rig.push(start_turn("busy"));
    rig.tab_handle(1).set_phase(SessionPhase::Generating);
    rig.frame();
    rig.click_row(session_index(&rig, "s-4"));
    assert_eq!(ids(&rig), ["s-1", "s-3", "s-4"]);
    rig.app.apply(Action::SetFocus(Focus::Input));
    rig.typed("keep me");
    rig.click_row(session_index(&rig, "s-2"));
    assert_eq!(ids(&rig), ["s-1", "s-3", "s-4", "s-2"]);
    // Open in new tab always gets a tab of its own.
    alt(&mut rig, '1');
    rig.app
        .open_conversation("s-2", crate::tabs::OpenPlace::NewTab);
    assert_eq!(
        active_id(&rig),
        "s-2",
        "an open one is shown, not opened twice"
    );
}

#[test]
fn the_open_tabs_are_saved_and_reopened_at_the_next_start() {
    let mut rig = stored_rig(130, 30, |_| {});
    rig.click_row(session_index(&rig, "s-2"));
    rig.click_row(session_index(&rig, "s-4"));
    rig.click_row(session_index(&rig, "s-1"));
    alt(&mut rig, '2');
    let saved = ok(rig.rt.saved_settings());
    assert_eq!(saved.open_tabs, ["s-2", "s-4", "s-1"]);
    assert_eq!(saved.active_tab.as_deref(), Some("s-4"));
    drop(rig.app);
    let rt = Arc::clone(&rig.rt);
    let next = relaunch(&rt, 130, 30);
    assert_eq!(ids(&next), ["s-2", "s-4", "s-1"]);
    assert_eq!(active_id(&next), "s-4");
    assert!(next.screen().contains("topic 4"));

    // Unknown ids are skipped and counted; with nothing left a New chat opens.
    ok(rt.seed_settings(&settings_t {
        open_tabs: vec!["s-gone".into(), "s-3".into()],
        active_tab: Some("s-gone".into()),
        ..settings_t::default()
    }));
    let next = relaunch(&rt, 130, 30);
    assert_eq!(ids(&next), ["s-3"]);
    assert_eq!(toast(&next), "1 saved tab(s) could not be reopened");
    ok(rt.seed_settings(&settings_t {
        open_tabs: vec!["s-gone".into()],
        active_tab: None,
        ..settings_t::default()
    }));
    let next = relaunch(&rt, 130, 30);
    assert_eq!(next.app.sessions.len(), 1);
    assert!(next.app.active_untouched());

    // With restore_tabs off nothing is saved or reopened.
    ok(rt.seed_settings(&settings_t {
        restore_tabs: false,
        open_tabs: vec!["s-1".into()],
        active_tab: None,
        ..settings_t::default()
    }));
    let mut next = relaunch(&rt, 130, 30);
    assert!(next.app.active_untouched());
    next.click_row(session_index(&next, "s-2"));
    assert_eq!(ok(rt.saved_settings()).open_tabs, ["s-1"]);
}

#[test]
fn settings_that_do_not_load_give_the_defaults_and_are_not_overwritten() {
    let mut rig = stored_rig(130, 30, |rt| {
        rt.fail_settings(Some(SessionError::Settings(SettingsError::Invalid {
            message: "bad tab".into(),
        })));
    });
    assert!(
        toast(&rig).contains("could not read settings"),
        "{}",
        toast(&rig)
    );
    assert_eq!(rig.app.settings, settings_t::default());
    assert!(!rig.app.settings_ok);
    rig.click_row(session_index(&rig, "s-1"));
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert!(
        ok(rig.rt.saved_settings()).open_tabs.is_empty(),
        "nothing saved quietly"
    );
    // A change the user asks for still tries, and says when it fails.
    assert!(!rig.app.update_settings(|s| s.show_archived = true));
    assert!(
        toast(&rig).contains("could not save settings"),
        "{}",
        toast(&rig)
    );
    rig.rt.fail_settings(None);
    assert!(rig.app.update_settings(|s| s.show_archived = true));
    assert!(ok(rig.rt.saved_settings()).show_archived);
}

#[test]
fn the_default_approval_mode_applies_to_new_conversations() {
    let mut rig = stored_rig(130, 30, |rt| {
        ok(rt.seed_settings(&settings_t {
            default_approval: ApprovalMode::Edit,
            ..settings_t::default()
        }));
    });
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::SetApprovalMode(ApprovalMode::Edit)],
        "the first New chat"
    );
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::SetApprovalMode(ApprovalMode::Edit)]
    );
    // A conversation's own mode wins when it opens.
    ok(rig
        .rt
        .seed_organization(&[harness::organize::OrganizeOp::SetSessionMeta {
            session_id: "s-1".into(),
            change: harness::organize::MetaChange::Approval(Some(ApprovalMode::Ask)),
        }]));
    rig.app.load_organization();
    rig.click_row(session_index(&rig, "s-1"));
    assert!(rig.calls().is_empty(), "already ask: nothing to change");
    rig.click_row(session_index(&rig, "s-2"));
    assert_eq!(
        rig.calls(),
        vec![ScriptCall::SetApprovalMode(ApprovalMode::Edit)]
    );
}

#[test]
fn a_run_started_in_full_keeps_full_for_new_conversations() {
    let mut info = preview_info("/p");
    info.approval_mode = ApprovalMode::Full;
    let rt = Arc::new(scripted_runtime_t::manual(info));
    ok(rt.seed_settings(&settings_t {
        default_approval: ApprovalMode::Edit,
        ..settings_t::default()
    }));
    let rig = relaunch(&rt, 100, 30);
    assert!(rig.calls().is_empty());
}

// Two conversations at once, with the real controller.

/// Ids 0 to 255 are bytes, 256 is EOS and 257 is PAD (D41a).
struct byte_codec_t;

impl text_codec_t for byte_codec_t {
    fn encode(&self, text: &str) -> Vec<u32> {
        text.bytes().map(u32::from).collect()
    }

    fn decode(&self, ids: &[u32]) -> String {
        let bytes: Vec<u8> = ids.iter().filter_map(|id| u8::try_from(*id).ok()).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

const ReplyLen: u32 = 64;

/// A backend whose calls report their first level, then wait for the gate before they end.
/// Each reply echoes the newest user message, so a test can tell the conversations apart.
struct gate_backend_t {
    identity: model_identity_t,
    caps: capabilities_t,
    codec: byte_codec_t,
    open: AtomicBool,
    running: AtomicUsize,
    most: Mutex<usize>,
}

impl gate_backend_t {
    fn new() -> Self {
        gate_backend_t {
            identity: model_identity_t {
                id: "gate-test".into(),
                kind: BackendKind::Demo,
                export_hash: None,
                note: "test backend".into(),
            },
            caps: capabilities_t {
                prefix_window: 4096,
                reply_len: ReplyLen,
                level_count: 2,
                level_lens: vec![16, ReplyLen],
                eos_id: Some(256),
                pad_id: 257,
                vocab_size: 258,
                cut_at_eos: true,
                supports_drafts: false,
                default_sampler: sampler_t::greedy(),
            },
            codec: byte_codec_t,
            open: AtomicBool::new(false),
            running: AtomicUsize::new(0),
            most: Mutex::new(0),
        }
    }

    fn reply_for(&self, prefix: &[u32]) -> Vec<u32> {
        let text = self.codec.decode(prefix);
        let said = text
            .rsplit("User: ")
            .next()
            .and_then(|t| t.lines().next())
            .unwrap_or("")
            .trim()
            .to_string();
        let mut tokens = self.codec.encode(&format!("echo {said}"));
        tokens.truncate(ReplyLen as usize - 1);
        tokens.push(256);
        tokens.resize(ReplyLen as usize, 257);
        tokens
    }
}

impl harness::backend::backend_t for gate_backend_t {
    fn identity(&self) -> &model_identity_t {
        &self.identity
    }

    fn capabilities(&self) -> &capabilities_t {
        &self.caps
    }

    fn codec(&self) -> &dyn text_codec_t {
        &self.codec
    }

    fn run_call(
        &self,
        request: call_request_t,
        on_level: &mut dyn FnMut(level_progress_t),
    ) -> Result<CallOutcome, BackendError> {
        request.validate_for(&self.caps)?;
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut most) = self.most.lock() {
            *most = (*most).max(now);
        }
        on_level(level_progress_t {
            level: 0,
            level_count: 2,
            level_len: 16,
            codes: vec![0; 16],
            elapsed: Duration::from_millis(1),
            draft: None,
        });
        let started = Instant::now();
        while !self.open.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(10) {
            if request.cancel.is_cancelled() {
                self.running.fetch_sub(1, Ordering::SeqCst);
                return Ok(CallOutcome::Cancelled(cancelled_t { levels_done: 1 }));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        on_level(level_progress_t {
            level: 1,
            level_count: 2,
            level_len: ReplyLen,
            codes: vec![0; ReplyLen as usize],
            elapsed: Duration::from_millis(1),
            draft: None,
        });
        self.running.fetch_sub(1, Ordering::SeqCst);
        Ok(CallOutcome::Completed(completion_t {
            tokens: self.reply_for(&request.prefix),
        }))
    }
}

fn now_ms() -> u64 {
    1_000
}

#[test]
fn two_conversations_generate_at_once_and_a_hidden_tab_keeps_folding() {
    let backend = Arc::new(gate_backend_t::new());
    let config = runtime_config_t {
        cwd: "/srv/ncagent-test/project".into(),
        approval: ApprovalMode::Ask,
        session_seed: 7,
        max_attempts: MaxAttempts,
        pass_budget: PassBudget,
        version: "test".into(),
        notice: None,
        label: Some("test session".into()),
        state_note: None,
        hints: Vec::new(),
        clock: now_ms,
    };
    let runtime = ok(controller_runtime_t::new(
        backend.clone(),
        Arc::new(unavailable_tool_runner_t),
        Arc::new(memory_store_t::new()),
        config,
    ));
    let rt: Arc<dyn runtime_t> = Arc::new(runtime);
    let mut app = app_t::new(
        rt,
        app_options_t {
            animate: false,
            mouse: true,
            color: ColorMode::TrueColor,
        },
        0,
        1_000,
    )
    .unwrap_or_else(|e| panic!("app: {}", e.message));
    let mut term = Terminal::new(TestBackend::new(130, 40)).unwrap_or_else(|e| match e {});
    let mut clock = 0u64;
    let mut step = |app: &mut app_t| {
        clock += 5;
        app.tick(clock, 1_000 + clock);
        let _ = term.draw(|f| draw(app, f));
        std::thread::sleep(Duration::from_millis(2));
        buffer_lines(term.backend().buffer()).join("\n")
    };
    let typed = |app: &mut app_t, text: &str| {
        for c in text.chars() {
            app.handle_event(&Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            )));
        }
        app.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
    };
    typed(&mut app, "alpha");
    app.handle_event(&Event::Key(KeyEvent::new(
        KeyCode::Char('t'),
        KeyModifiers::CONTROL,
    )));
    typed(&mut app, "beta");
    assert_eq!(app.active, 1);
    // Both calls are in flight at the same moment.
    let started = Instant::now();
    while backend.running.load(Ordering::SeqCst) < 2 && started.elapsed() < Duration::from_secs(5) {
        step(&mut app);
    }
    assert_eq!(backend.running.load(Ordering::SeqCst), 2);
    step(&mut app);
    assert!(
        app.sessions
            .iter()
            .all(|s| s.phase == SessionPhase::Generating)
    );
    let screen = step(&mut app);
    let bar = screen.lines().next().unwrap_or("").to_string();
    assert_eq!(
        bar.matches('●').count(),
        2,
        "both tabs show activity: {bar}"
    );
    // Let both finish while the second tab stays shown.
    backend.open.store(true, Ordering::SeqCst);
    let started = Instant::now();
    while app.sessions.iter().any(|s| s.busy()) && started.elapsed() < Duration::from_secs(10) {
        step(&mut app);
    }
    assert_eq!(app.active, 1, "the shown tab never changed");
    assert_eq!(
        app.sessions[0].transcript.last_reply_text().as_deref(),
        Some("echo alpha"),
        "the hidden tab folded its own reply"
    );
    assert_eq!(
        app.sessions[1].transcript.last_reply_text().as_deref(),
        Some("echo beta")
    );
    assert_eq!(app.sessions[0].attention, Attention::Finished);
    assert_eq!(app.sessions[1].attention, Attention::None);
    assert_eq!(backend.most.lock().map(|m| *m).unwrap_or(0), 2);
    let r = app.hits.find_target(&Target::Tab(0)).unwrap_or_default();
    app.handle_event(&Event::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: r.x + 3,
        row: r.y,
        modifiers: KeyModifiers::NONE,
    }));
    let screen = step(&mut app);
    assert_eq!(app.active, 0);
    assert!(screen.contains("echo alpha"), "{screen}");
}

#[test]
fn the_tab_menu_closes_others_and_to_the_right_and_renames() {
    let mut rig = stored_rig(130, 30, |_| {});
    for id in ["s-1", "s-2", "s-3", "s-4"] {
        rig.click_row(session_index(&rig, id));
    }
    let right_click_tab = |rig: &mut rig_t, i: usize| {
        let r = rig
            .app
            .hits
            .find_target(&Target::Tab(i))
            .unwrap_or_default();
        rig.click_with(MouseButton::Right, r.x + 3, r.y, KeyModifiers::NONE);
    };
    right_click_tab(&mut rig, 1);
    let shown = rig.screen();
    for label in [
        "Close others",
        "Close tabs to the right",
        "Rename…",
        "Colour tag",
    ] {
        assert!(shown.contains(label), "{label}\n{shown}");
    }
    assert!(shown.contains("tab “topic 2”"), "{shown}");
    let menu = rig.app.overlay_areas.first().copied().unwrap_or_default();
    assert!(menu.y >= 1, "under the bar: {menu:?}");
    let labels = |rig: &rig_t| -> Vec<String> {
        rig.app
            .menu
            .as_ref()
            .map(|m| m.items.iter().map(|i| i.label.clone()).collect())
            .unwrap_or_default()
    };
    let at = labels(&rig)
        .iter()
        .position(|l| l == "Close tabs to the right")
        .unwrap_or(0);
    rig.click_action(&Action::Menu(crate::input::MenuAction::Pick(at)));
    assert_eq!(ids(&rig), ["s-1", "s-2"]);
    right_click_tab(&mut rig, 0);
    let at = labels(&rig)
        .iter()
        .position(|l| l == "Close others")
        .unwrap_or(0);
    rig.click_action(&Action::Menu(crate::input::MenuAction::Pick(at)));
    assert_eq!(ids(&rig), ["s-1"]);
    // With one tab, the others and the right are dim.
    right_click_tab(&mut rig, 0);
    let menu = rig.app.menu.clone().unwrap_or_else(|| panic!("menu"));
    assert!(menu.items.iter().filter(|i| !i.enabled()).count() >= 2);
    // Rename from the tab opens the sidebar prompt for that conversation.
    let at = labels(&rig)
        .iter()
        .position(|l| l == "Rename…")
        .unwrap_or(0);
    rig.click_action(&Action::Menu(crate::input::MenuAction::Pick(at)));
    assert_eq!(rig.app.modal(), crate::input::Modal::Prompt);
    rig.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    rig.typed("From the tab");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.screen()
            .lines()
            .next()
            .unwrap_or("")
            .contains("From the tab")
    );
}
