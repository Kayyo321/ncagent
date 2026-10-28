/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Folder tests: the sidebar tree drawn on ratatui's `TestBackend`, driven by synthetic keys
//! and mouse events through the hit registry, against the scripted fixture, whose folders use
//! the real rules of `harness::organize` (D50 to D55).

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use harness::api::SessionError;
use harness::event::{SessionEvent, TurnOutcome, stamped_event_t};
use harness::organize::{MaxFolderDepth, OrganizeError, OrganizeOp};
use ratatui::style::Color;

use crate::app::EscapeDelayMs;
use crate::app_tests::rig_t;
use crate::input::{Action, Focus, Modal, Target};
use crate::sidebar::{MoveMark, SidebarItemKind};
use crate::theme::{mix, theme_t};
use crate::view_model::tests::session_start;

pub(crate) fn ok<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(err) => panic!("expected Ok, found {err:?}"),
    }
}

pub(crate) fn create(name: &str, parent: Option<&str>) -> OrganizeOp {
    OrganizeOp::CreateFolder {
        name: name.into(),
        parent: parent.map(str::to_string),
    }
}

pub(crate) fn file(session: &str, folder: Option<&str>) -> OrganizeOp {
    OrganizeOp::MoveSession {
        session_id: session.into(),
        folder: folder.map(str::to_string),
    }
}

/// A saved session with one finished turn, its events stamped from `at_ms`.
pub(crate) fn saved(id: &str, title: &str, at_ms: u64) -> Vec<stamped_event_t> {
    let mut start = session_start();
    if let SessionEvent::SessionStart { session_id, .. } = &mut start {
        *session_id = id.into();
    }
    let events = vec![
        start,
        SessionEvent::TurnStart { turn: 1 },
        SessionEvent::UserMessage {
            turn: 1,
            text: title.into(),
            from_queue: false,
        },
        SessionEvent::TurnEnd {
            turn: 1,
            outcome: TurnOutcome::Completed,
            calls: 0,
            passes: 0,
        },
    ];
    events
        .into_iter()
        .enumerate()
        .map(|(seq, event)| stamped_event_t {
            seq: seq as u64,
            at_ms: at_ms + seq as u64 * 10,
            event,
        })
        .collect()
}

/// Research (f1) holding Papers (f2), and Ideas (f3); "attention paper" in Papers, "lab notes"
/// in Research, "free chat" unfiled.
pub(crate) fn tree_rig(width: u16, height: u16, mouse: bool) -> rig_t {
    rig_t::with(width, height, false, mouse, |rt| {
        rt.store("s-paper", saved("s-paper", "attention paper", 3_000));
        rt.store("s-note", saved("s-note", "lab notes", 2_000));
        rt.store("s-free", saved("s-free", "free chat", 1_000));
        ok(rt.seed_organization(&[
            create("Research", None),
            create("Papers", Some("f1")),
            create("Ideas", None),
            file("s-paper", Some("f2")),
            file("s-note", Some("f1")),
        ]));
    })
}

/// Parity-table setup: a Research folder with a Papers sub-folder.
pub(crate) fn seed_research(r: &mut rig_t) {
    ok(r.rt.seed_organization(&[create("Research", None), create("Papers", Some("f1"))]));
    r.app.load_organization();
    r.app.rebuild_sidebar_items();
    r.frame();
}

/// Parity-table setup: the saved conversation filed in Research.
pub(crate) fn file_saved_in_research(r: &mut rig_t) {
    ok(r.rt.seed_organization(&[file("saved-1", Some("f1"))]));
    r.app.load_organization();
    r.app.rebuild_sidebar_items();
    r.frame();
}

pub(crate) fn folder_index(r: &rig_t, name: &str) -> usize {
    r.app
        .sidebar_items
        .iter()
        .position(|i| matches!(i.kind, SidebarItemKind::Folder { .. }) && i.title == name)
        .unwrap_or_else(|| panic!("no folder {name}: {:?}", r.app.sidebar_items))
}

pub(crate) fn session_index(r: &rig_t, id: &str) -> usize {
    r.app
        .sidebar_items
        .iter()
        .position(|i| matches!(&i.kind, SidebarItemKind::Session { id: other } if other == id))
        .unwrap_or_else(|| panic!("no session {id}: {:?}", r.app.sidebar_items))
}

/// Row and column of the first appearance of `needle` on screen, in cells.
pub(crate) fn locate(r: &rig_t, needle: &str) -> (usize, usize) {
    let screen = r.screen();
    for (row, line) in screen.lines().enumerate() {
        if let Some(b) = line.find(needle) {
            return (row, line.get(..b).unwrap_or("").chars().count());
        }
    }
    panic!("{needle:?} not on screen\n{screen}");
}

fn sidebar_bg(drawer: bool) -> Color {
    let t = theme_t::dark();
    if drawer { t.dock } else { t.side }
}

pub(crate) fn cell_bg(r: &rig_t, x: u16, y: u16) -> Color {
    r.term
        .backend()
        .buffer()
        .cell((x, y))
        .map(|c| c.bg)
        .unwrap_or(Color::Reset)
}

pub(crate) fn toast(r: &rig_t) -> String {
    r.app
        .toast
        .as_ref()
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

#[test]
fn tree_lists_folders_then_unfiled_with_indentation_at_each_width() {
    for (width, drawer) in [(100u16, false), (60, true), (40, true)] {
        let mut rig = tree_rig(width, 44, true);
        if drawer {
            rig.key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        }
        let screen = rig.screen();
        let (r_ideas, c_ideas) = locate(&rig, "▾ Ideas");
        let (r_research, c_research) = locate(&rig, "▾ Research");
        let (r_papers, c_papers) = locate(&rig, "▾ Papers");
        let (r_paper, c_paper) = locate(&rig, "attention");
        let (r_note, c_note) = locate(&rig, "lab notes");
        let (r_free, c_free) = locate(&rig, "free chat");
        // Folders by name, sub-folders before conversations, then unfiled conversations.
        assert!(r_ideas < r_research, "width {width}\n{screen}");
        assert!(r_research < r_papers && r_papers < r_paper, "{screen}");
        assert!(r_paper < r_note && r_note < r_free, "{screen}");
        // Each level indents two columns; a conversation lines up with its sibling folders'
        // names.
        assert_eq!(c_ideas, c_research, "{screen}");
        assert_eq!(c_papers, c_research + 2, "{screen}");
        assert_eq!(c_paper, c_papers + 4, "{screen}");
        assert_eq!(c_note, c_papers + 2, "{screen}");
        assert_eq!(c_free, c_research + 2, "{screen}");
        // Recursive counts on the folder rows, and the guide rails, not box lines.
        let research_line = screen.lines().nth(r_research).unwrap_or("");
        assert!(research_line.trim_end().ends_with('2'), "{research_line}");
        let papers_line = screen.lines().nth(r_papers).unwrap_or("");
        assert!(papers_line.contains('▏'), "{papers_line}");
        assert!(papers_line.trim_end().ends_with('1'), "{papers_line}");
        assert!(screen.contains("+ folder"), "{screen}");
        for c in ['┌', '┐', '└', '┘', '│', '─', '├'] {
            assert!(!screen.contains(c), "box char {c:?}\n{screen}");
        }
        // No project header rows any more (D54).
        assert!(
            !rig.app
                .sidebar_items
                .iter()
                .any(|i| i.title == "proj" && i.kind != SidebarItemKind::Root)
        );
    }
}

#[test]
fn folding_hides_the_contents_and_keeps_the_count() {
    let mut rig = tree_rig(120, 44, true);
    let i = folder_index(&rig, "Research");
    rig.click_row(i);
    let screen = rig.screen();
    assert!(screen.contains("▸ Research"), "{screen}");
    assert!(!screen.contains("Papers"), "{screen}");
    assert!(!screen.contains("lab notes"), "{screen}");
    assert_eq!(
        rig.app.sidebar_items.get(i).map(|x| x.detail.as_str()),
        Some("2")
    );
    // → unfolds, ← folds, by keys.
    rig.key(KeyCode::Right, KeyModifiers::NONE);
    assert!(rig.screen().contains("Papers"));
    rig.key(KeyCode::Left, KeyModifiers::NONE);
    assert!(!rig.screen().contains("Papers"));
}

#[test]
fn a_project_name_shows_only_when_it_differs() {
    let mut rig = rig_t::with(120, 40, false, true, |rt| {
        let mut events = saved("s-other", "elsewhere", 1_000);
        if let Some(SessionEvent::SessionStart { project, .. }) =
            events.first_mut().map(|e| &mut e.event)
        {
            *project = "otherproj".into();
        }
        rt.store("s-other", events);
        rt.store("s-here", saved("s-here", "right here", 2_000));
    });
    rig.frame();
    let other = session_index(&rig, "s-other");
    let here = session_index(&rig, "s-here");
    let detail = |i: usize| rig.app.sidebar_items.get(i).map(|x| x.detail.clone());
    assert!(detail(other).is_some_and(|d| d.starts_with("otherproj · ")));
    assert!(detail(here).is_some_and(|d| !d.contains("proj")));
}

#[test]
fn ctrl_f_names_a_new_folder_and_refusals_keep_the_prompt() {
    let mut rig = tree_rig(120, 44, true);
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.modal(), Modal::Prompt);
    assert_eq!(rig.app.focus, Focus::Sidebar);
    let screen = rig.screen();
    assert!(screen.contains("new folder at top level"), "{screen}");
    assert!(screen.contains("enter create"), "{screen}");
    // Empty name: refused, the prompt stays.
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(toast(&rig), "invalid folder name: the name is empty");
    assert_eq!(rig.app.modal(), Modal::Prompt);
    // A sibling's name, ignoring case: refused.
    rig.typed("research");
    assert!(rig.screen().contains("research"));
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(toast(&rig).contains("is already there"), "{}", toast(&rig));
    assert_eq!(rig.app.modal(), Modal::Prompt);
    // Too long.
    rig.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    rig.typed(&"x".repeat(41));
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(toast(&rig).contains("longer than 40"), "{}", toast(&rig));
    // A good name: created at the top level and selected.
    rig.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    rig.typed("Zeta notes");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.modal(), Modal::None);
    let created = rig.app.organization.find_child(None, "Zeta notes").cloned();
    assert!(created.is_some());
    let i = folder_index(&rig, "Zeta notes");
    assert_eq!(rig.app.sidebar_sel, i);
    assert!(rig.screen().contains("created folder “Zeta notes”"));
}

#[test]
fn a_paste_lands_in_the_prompt_on_one_line() {
    let mut rig = tree_rig(120, 44, true);
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    rig.app
        .handle_event(&crossterm::event::Event::Paste("Read\nlater".into()));
    rig.frame();
    assert_eq!(
        rig.app.prompt.as_ref().map(|p| p.editor.text().to_string()),
        Some("Read later".to_string())
    );
    assert!(rig.app.composer.is_empty());
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.app
            .organization
            .find_child(None, "Read later")
            .is_some()
    );
}

#[test]
fn esc_and_ctrl_c_close_the_prompt() {
    let mut rig = tree_rig(120, 44, true);
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    rig.typed("draft");
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(rig.app.organization.folders().len(), 3);
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    rig.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.modal(), Modal::None);
    assert!(!rig.app.quit);
}

#[test]
fn f_in_the_sidebar_makes_a_sub_folder_of_the_selection() {
    let mut rig = tree_rig(120, 44, true);
    let i = folder_index(&rig, "Research");
    rig.select_by_keys(i);
    rig.ch('f');
    let screen = rig.screen();
    assert!(screen.contains("new folder in Research"), "{screen}");
    // The prompt row sits under Research, one level in.
    let (row_caption, col_caption) = locate(&rig, "new folder in Research");
    let (row_research, col_research) = locate(&rig, "▾ Research");
    assert_eq!(row_caption, row_research + 1, "{screen}");
    assert_eq!(col_caption, col_research + 2, "{screen}");
    rig.typed("Notes");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    let notes = rig
        .app
        .organization
        .find_child(Some("f1"), "Notes")
        .cloned();
    assert!(notes.is_some(), "{:?}", rig.app.organization);

    // With a conversation selected, f makes a top-level folder.
    let s = session_index(&rig, "s-free");
    rig.select_by_keys(s);
    rig.ch('f');
    assert!(rig.screen().contains("new folder at top level"));
}

#[test]
fn a_folder_at_the_depth_limit_refuses_a_sub_folder_at_once() {
    let mut rig = rig_t::with(120, 44, false, true, |rt| {
        let mut parent: Option<String> = None;
        for level in 0..MaxFolderDepth {
            ok(rt.seed_organization(&[create(&format!("l{level}"), parent.as_deref())]));
            parent = Some(format!("f{}", level + 1));
        }
    });
    rig.frame();
    let deepest = folder_index(&rig, &format!("l{}", MaxFolderDepth - 1));
    rig.select_by_keys(deepest);
    rig.ch('f');
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(toast(&rig), "folders nest at most 5 levels deep");
    // The + chip is not offered on that row either.
    rig.hover_row(deepest);
    assert!(
        rig.app
            .hits
            .find(&Action::NewSubfolderAt(deepest))
            .is_none()
    );
    assert!(
        rig.app
            .hits
            .find(&Action::RenameFolderAt(deepest))
            .is_some()
    );
}

#[test]
fn rename_with_r_is_prefilled() {
    let mut rig = tree_rig(120, 44, true);
    let i = folder_index(&rig, "Ideas");
    rig.select_by_keys(i);
    rig.ch('r');
    assert!(rig.screen().contains("rename folder"));
    assert_eq!(
        rig.app.prompt.as_ref().map(|p| p.editor.text().to_string()),
        Some("Ideas".to_string())
    );
    // The prompt replaces the row in place.
    assert!(!rig.screen().contains("▾ Ideas"), "{}", rig.screen());
    rig.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    rig.typed("Thoughts");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rig.app.organization.folder("f3").map(|f| f.name.as_str()),
        Some("Thoughts")
    );
    assert!(rig.screen().contains("▾ Thoughts"));
    // r on a conversation renames the conversation (D57), prefilled with its title.
    let s = session_index(&rig, "s-free");
    rig.select_by_keys(s);
    rig.ch('r');
    assert_eq!(rig.app.modal(), Modal::Prompt);
    assert!(
        rig.screen().contains("rename conversation"),
        "{}",
        rig.screen()
    );
    assert_eq!(
        rig.app.prompt.as_ref().map(|p| p.editor.text().to_string()),
        Some("free chat".to_string())
    );
    // On the new-chat button there is nothing to rename.
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    rig.select_by_keys(0);
    rig.ch('r');
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(
        toast(&rig),
        "select a folder or a conversation to rename it"
    );
}

#[test]
fn delete_asks_inline_and_keeps_the_conversations() {
    let mut rig = tree_rig(120, 44, true);
    let i = folder_index(&rig, "Research");
    rig.select_by_keys(i);
    rig.ch('d');
    assert_eq!(rig.app.modal(), Modal::Confirm);
    let screen = rig.screen();
    assert!(screen.contains("delete folder “Research”?"), "{screen}");
    assert!(screen.contains("contents move up"), "{screen}");
    assert!(screen.contains("enter confirm"), "{screen}");
    assert!(screen.contains("esc cancel"), "{screen}");
    // n keeps it.
    rig.ch('n');
    assert_eq!(rig.app.modal(), Modal::None);
    assert!(rig.app.organization.folder("f1").is_some());
    rig.ch('d');
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(rig.app.organization.folder("f1").is_none());
    // Papers moved to the top level, lab notes became unfiled, and both still show.
    assert_eq!(
        rig.app.organization.folder("f2").map(|f| f.parent.clone()),
        Some(None)
    );
    assert_eq!(rig.app.organization.folder_of("s-note"), None);
    let screen = rig.screen();
    assert!(screen.contains("lab notes"), "{screen}");
    assert!(screen.contains("attention"), "{screen}");
    assert_eq!(
        toast(&rig),
        "deleted folder “Research”; its contents moved up"
    );
}

#[test]
fn delete_by_the_row_chip_and_the_confirm_button() {
    let mut rig = tree_rig(120, 44, true);
    let i = folder_index(&rig, "Ideas");
    rig.hover_row(i);
    rig.click_action(&Action::DeleteFolderAt(i));
    assert_eq!(rig.app.confirm.as_deref(), Some("f3"));
    rig.click_action(&Action::ConfirmYes);
    assert!(rig.app.organization.folder("f3").is_none());
}

#[test]
fn chips_show_on_hover_only() {
    let mut rig = tree_rig(120, 44, true);
    let i = folder_index(&rig, "Research");
    assert!(rig.app.hits.find(&Action::RenameFolderAt(i)).is_none());
    rig.hover_row(i);
    for action in [
        Action::NewSubfolderAt(i),
        Action::RenameFolderAt(i),
        Action::PickUpAt(i),
        Action::DeleteFolderAt(i),
    ] {
        assert!(rig.app.hits.find(&action).is_some(), "{action:?}");
    }
    let screen = rig.screen();
    let row = screen
        .lines()
        .nth(locate(&rig, "▾ Research").0)
        .unwrap_or("");
    assert!(
        row.contains(" + ") && row.contains(" ✎ ") && row.contains(" ✕ "),
        "{row}"
    );
    let s = session_index(&rig, "s-free");
    rig.hover_row(s);
    assert!(rig.app.hits.find(&Action::PickUpAt(s)).is_some());
    assert!(rig.app.hits.find(&Action::RenameFolderAt(s)).is_none());
}

#[test]
fn drag_a_conversation_into_a_folder() {
    let mut rig = tree_rig(120, 44, true);
    let from = session_index(&rig, "s-free");
    let to = folder_index(&rig, "Ideas");
    let a = rig.row_rect(from);
    rig.mouse(MouseEventKind::Down(MouseButton::Left), a.x + 3, a.y);
    // Still on the row: no drag yet.
    rig.mouse(MouseEventKind::Drag(MouseButton::Left), a.x + 5, a.y);
    assert!(rig.app.grab.as_ref().is_some_and(|g| !g.moved));
    let b = rig.row_rect(to);
    rig.mouse(MouseEventKind::Drag(MouseButton::Left), b.x + 3, b.y);
    let screen = rig.screen();
    assert!(screen.contains("moving free chat"), "{screen}");
    assert!(screen.contains("⇢ free chat"), "ghost\n{screen}");
    assert!(screen.contains("move here"), "{screen}");
    assert!(screen.contains("↑ top level"), "{screen}");
    // The destination is lit with the accent tone and a rail.
    let t = theme_t::dark();
    assert_eq!(
        cell_bg(&rig, b.x + 8, b.y),
        mix(sidebar_bg(false), t.agent, 0.24)
    );
    let rail = rig
        .term
        .backend()
        .buffer()
        .cell((b.x, b.y))
        .map(|c| c.symbol().to_string());
    assert_eq!(rail.as_deref(), Some("▌"));
    // Conversation rows are not destinations while something moves.
    assert_eq!(
        rig.app.sidebar_items.get(from).map(|i| i.mark.clone()),
        Some(MoveMark::Source)
    );
    rig.mouse(MouseEventKind::Up(MouseButton::Left), b.x + 3, b.y);
    assert_eq!(rig.app.organization.folder_of("s-free"), Some("f3"));
    assert!(rig.app.grab.is_none());
    // The moved row stays selected and visible in its new folder.
    let moved = session_index(&rig, "s-free");
    assert_eq!(rig.app.sidebar_sel, moved);
    assert_eq!(rig.app.sidebar_items.get(moved).map(|i| i.depth), Some(1));
    assert!(rig.screen().contains("moved “free chat” into “Ideas”"));
    assert!(!rig.screen().contains("moving free chat"));
}

#[test]
fn drag_a_folder_into_a_folder() {
    let mut rig = tree_rig(120, 44, true);
    rig.drag_rows(folder_index(&rig, "Ideas"), folder_index(&rig, "Research"));
    assert_eq!(
        rig.app
            .organization
            .folder("f3")
            .and_then(|f| f.parent.clone()),
        Some("f1".to_string())
    );
    let ideas = folder_index(&rig, "Ideas");
    assert_eq!(rig.app.sidebar_items.get(ideas).map(|i| i.depth), Some(1));
}

#[test]
fn release_on_a_descendant_is_refused_with_the_reason() {
    let mut rig = tree_rig(120, 44, true);
    let from = folder_index(&rig, "Research");
    let to = folder_index(&rig, "Papers");
    let a = rig.row_rect(from);
    rig.mouse(MouseEventKind::Down(MouseButton::Left), a.x + 3, a.y);
    let b = rig.row_rect(to);
    rig.mouse(MouseEventKind::Drag(MouseButton::Left), b.x + 3, b.y);
    let t = theme_t::dark();
    assert_eq!(
        cell_bg(&rig, b.x + 8, b.y),
        mix(sidebar_bg(false), t.error, 0.18)
    );
    let papers_row = rig
        .screen()
        .lines()
        .nth(usize::from(b.y))
        .unwrap_or("")
        .to_string();
    assert!(papers_row.contains("inside itself"), "{papers_row}");
    // Ideas is still allowed, the source row is marked.
    let ideas = folder_index(&rig, "Ideas");
    assert_eq!(
        rig.app.sidebar_items.get(ideas).map(|i| i.mark.clone()),
        Some(MoveMark::Allowed)
    );
    rig.mouse(MouseEventKind::Up(MouseButton::Left), b.x + 3, b.y);
    assert_eq!(
        rig.app
            .organization
            .folder("f1")
            .and_then(|f| f.parent.clone()),
        None
    );
    assert_eq!(
        toast(&rig),
        "cannot move “Research” there: folder f1 cannot go inside itself"
    );
}

#[test]
fn release_on_conversations_unfiles() {
    let mut rig = tree_rig(120, 44, true);
    rig.drag_rows(session_index(&rig, "s-paper"), 1);
    assert_eq!(rig.app.organization.folder_of("s-paper"), None);
    let moved = session_index(&rig, "s-paper");
    assert_eq!(rig.app.sidebar_items.get(moved).map(|i| i.depth), Some(0));
    // And a sub-folder to the top level.
    rig.drag_rows(folder_index(&rig, "Papers"), 1);
    assert_eq!(
        rig.app.organization.folder("f2").map(|f| f.parent.clone()),
        Some(None)
    );
}

#[test]
fn a_click_without_movement_still_opens_and_folds() {
    let mut rig = tree_rig(120, 44, true);
    let s = session_index(&rig, "s-note");
    rig.click_row(s);
    assert_eq!(
        rig.app
            .active()
            .map(|x| x.session.id().to_string())
            .as_deref(),
        Some("s-note")
    );
    let f = folder_index(&rig, "Ideas");
    rig.click_row(f);
    assert!(rig.app.folded.contains("f3"));
    assert!(rig.app.organization.folder_of("s-note") == Some("f1"));
    // Pressing, wandering off and coming back is a move back onto itself: nothing happens.
    let a = rig.row_rect(s);
    let away = rig.row_rect(1);
    rig.mouse(MouseEventKind::Down(MouseButton::Left), a.x + 3, a.y);
    rig.mouse(MouseEventKind::Drag(MouseButton::Left), away.x + 3, away.y);
    rig.mouse(MouseEventKind::Drag(MouseButton::Left), a.x + 3, a.y);
    rig.mouse(MouseEventKind::Up(MouseButton::Left), a.x + 3, a.y);
    assert_eq!(rig.app.organization.folder_of("s-note"), Some("f1"));
    assert_eq!(toast(&rig), "move cancelled");
}

#[test]
fn esc_cancels_a_drag_at_once() {
    let mut rig = tree_rig(120, 44, true);
    let a = rig.row_rect(session_index(&rig, "s-free"));
    let b = rig.row_rect(folder_index(&rig, "Ideas"));
    rig.mouse(MouseEventKind::Down(MouseButton::Left), a.x + 3, a.y);
    rig.mouse(MouseEventKind::Drag(MouseButton::Left), b.x + 3, b.y);
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(rig.app.grab.is_none());
    assert!(!rig.screen().contains("moving free chat"));
    rig.mouse(MouseEventKind::Up(MouseButton::Left), b.x + 3, b.y);
    assert_eq!(rig.app.organization.folder_of("s-free"), None);
}

#[test]
fn dragging_near_the_bottom_scrolls_the_tree() {
    let mut rig = rig_t::with(120, 34, false, true, |rt| {
        for n in 0..30u64 {
            let id = format!("s-{n}");
            rt.store(
                &id,
                saved(&id, &format!("chat number {n}"), 1_000 + n * 100),
            );
        }
        ok(rt.seed_organization(&[create("Archive", None)]));
    });
    rig.frame();
    let list = rig.app.sidebar_list;
    assert!(list.height > 0);
    let from = rig.row_rect(session_index(&rig, "s-29"));
    rig.mouse(MouseEventKind::Down(MouseButton::Left), from.x + 3, from.y);
    let before = rig.app.sidebar_scroll;
    for _ in 0..5 {
        rig.mouse(
            MouseEventKind::Drag(MouseButton::Left),
            list.x + 4,
            list.bottom().saturating_sub(1),
        );
    }
    assert!(rig.app.sidebar_scroll > before, "{}", rig.screen());
    // Back to the top, and drop on the folder.
    for _ in 0..40 {
        rig.mouse(MouseEventKind::Drag(MouseButton::Left), list.x + 4, list.y);
    }
    let to = rig.row_rect(folder_index(&rig, "Archive"));
    rig.mouse(MouseEventKind::Drag(MouseButton::Left), to.x + 3, to.y);
    rig.mouse(MouseEventKind::Up(MouseButton::Left), to.x + 3, to.y);
    assert_eq!(rig.app.organization.folder_of("s-29"), Some("f1"));
}

#[test]
fn pick_up_by_keys_skips_refused_rows_and_drops() {
    let mut rig = tree_rig(120, 44, true);
    let research = folder_index(&rig, "Research");
    rig.select_by_keys(research);
    rig.ch('m');
    assert_eq!(rig.app.modal(), Modal::Move);
    let screen = rig.screen();
    assert!(screen.contains("moving Research"), "{screen}");
    assert!(screen.contains("j/k choose"), "{screen}");
    assert!(screen.contains("enter drop"), "{screen}");
    // Its own sub-folder is refused and never chosen.
    let papers = folder_index(&rig, "Papers");
    assert!(matches!(
        rig.app.sidebar_items.get(papers).map(|i| &i.mark),
        Some(MoveMark::Refused { .. })
    ));
    for _ in 0..6 {
        rig.ch('j');
        assert_ne!(rig.app.sidebar_sel, papers);
    }
    for _ in 0..6 {
        rig.ch('k');
        assert_ne!(rig.app.sidebar_sel, papers);
    }
    let ideas = folder_index(&rig, "Ideas");
    rig.choose_destination(ideas);
    assert!(rig.screen().contains("move here"));
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(
        rig.app
            .organization
            .folder("f1")
            .and_then(|f| f.parent.clone()),
        Some("f3".to_string())
    );
    assert_eq!(rig.app.sidebar_sel, folder_index(&rig, "Research"));
}

#[test]
fn pick_up_cancels_with_esc_and_marks_the_row() {
    let mut rig = tree_rig(120, 44, true);
    let s = session_index(&rig, "s-free");
    rig.select_by_keys(s);
    rig.ch('m');
    assert_eq!(
        rig.app.sidebar_items.get(s).map(|i| i.mark.clone()),
        Some(MoveMark::Source)
    );
    let (row, _) = locate(&rig, "free chat");
    let line = rig.screen().lines().nth(row).unwrap_or("").to_string();
    assert!(line.contains('⇢'), "{line}");
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(rig.app.organization.folder_of("s-free"), None);
}

#[test]
fn pick_up_by_chip_then_click_a_folder() {
    let mut rig = tree_rig(120, 44, true);
    let s = session_index(&rig, "s-free");
    rig.hover_row(s);
    rig.click_action(&Action::PickUpAt(s));
    assert_eq!(rig.app.modal(), Modal::Move);
    // A click on a refused-free conversation row is not a destination.
    let other = session_index(&rig, "s-note");
    rig.click_row(other);
    assert_eq!(rig.app.modal(), Modal::Move);
    assert!(toast(&rig).starts_with("drop on a folder"));
    rig.click_row(folder_index(&rig, "Papers"));
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(rig.app.organization.folder_of("s-free"), Some("f2"));
}

#[test]
fn moving_without_folders_says_how_to_make_one() {
    let mut rig = rig_t::with(120, 40, false, true, |rt| {
        rt.store("s-a", saved("s-a", "lonely", 1_000));
    });
    rig.frame();
    rig.select_by_keys(session_index(&rig, "s-a"));
    rig.ch('m');
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(toast(&rig), "no folders yet; ctrl+f makes one");
}

#[test]
fn the_drawer_holds_the_prompt_and_the_tree_when_narrow() {
    let mut rig = tree_rig(60, 40, true);
    assert!(!rig.screen().contains("conversations"));
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert!(rig.app.drawer_open);
    let screen = rig.screen();
    assert!(screen.contains("conversations"), "{screen}");
    assert!(screen.contains("new folder at top level"), "{screen}");
    rig.typed("Narrow");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(rig.app.organization.find_child(None, "Narrow").is_some());
    assert!(rig.screen().contains("▾ Narrow"));
    // A drag works in the drawer too.
    rig.drag_rows(session_index(&rig, "s-free"), folder_index(&rig, "Narrow"));
    assert_eq!(rig.app.organization.folder_of("s-free"), Some("f4"));
    // Pick-up at 40 columns.
    rig.resize(40, 40);
    rig.app.apply(Action::SetFocus(Focus::Sidebar));
    rig.app.drawer_open = true;
    rig.frame();
    rig.select_by_keys(session_index(&rig, "s-note"));
    rig.ch('m');
    assert_eq!(rig.app.modal(), Modal::Move);
    rig.choose_destination(1);
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.organization.folder_of("s-note"), None);
}

#[test]
fn resizing_to_narrow_keeps_an_open_prompt_in_view() {
    let mut rig = tree_rig(120, 40, true);
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    rig.resize(70, 40);
    assert!(rig.app.drawer_open);
    assert!(rig.screen().contains("new folder at top level"));
}

#[test]
fn a_folder_file_that_does_not_load_leaves_a_flat_list() {
    let mut rig = rig_t::with(120, 40, false, true, |rt| {
        rt.store("s-a", saved("s-a", "still listed", 1_000));
        rt.fail_organization(Some(SessionError::Organize(OrganizeError::Malformed {
            message: "line 1".into(),
        })));
    });
    let screen = rig.screen();
    assert!(
        screen.contains("could not read folders: folders file is not valid: line 1"),
        "{screen}"
    );
    assert!(screen.contains("still listed"), "{screen}");
    assert!(
        !rig.app
            .sidebar_items
            .iter()
            .any(|i| matches!(i.kind, SidebarItemKind::Folder { .. }))
    );
    // A change reports the same error and keeps the prompt.
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    rig.typed("x");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(toast(&rig), "folders file is not valid: line 1");
    assert_eq!(rig.app.modal(), Modal::Prompt);
}

#[test]
fn folder_command_creates_or_opens_the_prompt() {
    let mut rig = tree_rig(120, 44, true);
    rig.typed("/folder Reading list");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.app
            .organization
            .find_child(None, "Reading list")
            .is_some()
    );
    rig.app.apply(Action::SetFocus(Focus::Input));
    rig.typed("/folder");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.modal(), Modal::Prompt);
}

#[test]
fn without_the_mouse_every_folder_action_has_keys() {
    let mut rig = tree_rig(120, 44, false);
    // Clicks do nothing.
    let i = folder_index(&rig, "Research");
    let r = rig.row_rect(i);
    rig.click(r.x + 3, r.y);
    assert!(rig.app.folded.is_empty());
    // No chips are drawn for a mouse that is not there.
    rig.select_by_keys(i);
    assert!(rig.app.hits.find(&Action::RenameFolderAt(i)).is_none());
    // Fold, create, rename, move and delete, by keys only.
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(rig.app.folded.contains("f1"));
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    rig.ch('f');
    rig.typed("Sub");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(rig.app.organization.find_child(Some("f1"), "Sub").is_some());
    let sub = folder_index(&rig, "Sub");
    rig.select_by_keys(sub);
    rig.ch('r');
    rig.key(KeyCode::Backspace, KeyModifiers::NONE);
    rig.typed("bs");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.app
            .organization
            .find_child(Some("f1"), "Subs")
            .is_some()
    );
    let s = session_index(&rig, "s-free");
    rig.select_by_keys(s);
    rig.ch('m');
    rig.choose_destination(folder_index(&rig, "Subs"));
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.organization.folder_of("s-free"), Some("f4"));
    let subs = folder_index(&rig, "Subs");
    rig.select_by_keys(subs);
    rig.ch('d');
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.app
            .organization
            .find_child(Some("f1"), "Subs")
            .is_none()
    );
    assert_eq!(rig.app.organization.folder_of("s-free"), Some("f1"));
}

/// Every click target in a frame with folders, chips and a prompt has a keyboard path.
#[test]
fn every_folder_click_target_has_a_keyboard_path() {
    let mut rig = tree_rig(120, 44, true);
    rig.hover_row(folder_index(&rig, "Research"));
    let mut seen: Vec<Target> = rig.app.hits.hits.iter().map(|h| h.target.clone()).collect();
    rig.select_by_keys(folder_index(&rig, "Ideas"));
    rig.ch('d');
    seen.extend(rig.app.hits.hits.iter().map(|h| h.target.clone()));
    rig.ch('n');
    rig.key(KeyCode::Char('f'), KeyModifiers::CONTROL);
    seen.extend(rig.app.hits.hits.iter().map(|h| h.target.clone()));
    for target in seen {
        assert!(
            crate::app_tests::keyboard_path(&target),
            "click target without a keyboard path: {target:?}"
        );
    }
}
