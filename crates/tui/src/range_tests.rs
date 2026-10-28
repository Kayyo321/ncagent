/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Range selection like a file explorer (D60): the anchor's fallbacks, shift and ctrl+shift
//! clicks, modifiers read from the press, and the modifier-free chip drag for terminals that
//! keep shift+click for themselves. Synthetic mouse events on the real app.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};

use crate::app::EscapeDelayMs;
use crate::app_tests::rig_t;
use crate::folder_tests::{create, file, ok, saved, session_index};
use crate::input::{Action, SelectAction, TabAction};
use crate::selection::RowId;
use crate::sidebar::SidebarItemKind;

/// Six unfiled conversations, newest first a to f.
fn flat(height: u16) -> rig_t {
    rig_t::with(130, height, false, true, |rt| {
        for (i, id) in ["s-a", "s-b", "s-c", "s-d", "s-e", "s-f"]
            .iter()
            .enumerate()
        {
            rt.store(
                id,
                saved(id, &format!("talk {}", &id[2..]), 9_000 - i as u64 * 100),
            );
        }
    })
}

fn row(r: &rig_t, id: &str) -> usize {
    session_index(r, id)
}

fn selected(r: &rig_t) -> Vec<String> {
    r.app
        .selection
        .in_order(&r.app.visible_rows())
        .into_iter()
        .map(|id| match id {
            RowId::Session(s) => s,
            RowId::Folder(f) => format!("folder:{f}"),
        })
        .collect()
}

/// The ids shown with the selection tone in the sidebar, top to bottom.
fn marked(r: &rig_t) -> Vec<String> {
    r.app
        .sidebar_items
        .iter()
        .filter(|i| i.selected)
        .filter_map(|i| match &i.kind {
            SidebarItemKind::Session { id } => Some(id.clone()),
            SidebarItemKind::Folder { id, .. } => Some(format!("folder:{id}")),
            _ => None,
        })
        .collect()
}

fn shift_click(r: &mut rig_t, id: &str) {
    let i = row(r, id);
    r.click_row_with(i, KeyModifiers::SHIFT);
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn plain_click_then_shift_click_selects_everything_between_both_ways() {
    let mut r = flat(44);
    r.click_row(row(&r, "s-b"));
    shift_click(&mut r, "s-e");
    assert_eq!(selected(&r), ids(&["s-b", "s-c", "s-d", "s-e"]));
    assert_eq!(marked(&r), selected(&r), "the rows between are highlighted");
    // From the same anchor upwards: the range replaces the first one.
    shift_click(&mut r, "s-a");
    assert_eq!(selected(&r), ids(&["s-a", "s-b"]));
    assert_eq!(
        r.app.selection.anchor(),
        Some(&RowId::Session("s-b".into()))
    );
}

#[test]
fn without_an_anchor_the_open_conversation_is_the_start() {
    let mut r = flat(44);
    // Nothing was clicked: the open New chat is the anchor.
    let open = r
        .app
        .active()
        .map(|s| s.session.id().to_string())
        .unwrap_or_default();
    shift_click(&mut r, "s-c");
    let got = selected(&r);
    assert!(got.len() >= 2, "more than the clicked row: {got:?}");
    assert!(
        got.contains(&open) && got.contains(&"s-c".to_string()),
        "{got:?}"
    );
    // Opening a conversation through its tab moves the anchor to it.
    r.app.clear_selection();
    r.frame();
    r.click_row(row(&r, "s-b"));
    r.click_row(row(&r, "s-d"));
    let b = r.app.tab_of("s-b").unwrap_or(usize::MAX);
    r.app.apply(Action::Tab(TabAction::Select(b)));
    r.frame();
    shift_click(&mut r, "s-f");
    assert_eq!(selected(&r), ids(&["s-b", "s-c", "s-d", "s-e", "s-f"]));
}

#[test]
fn a_cursor_move_moves_the_anchor() {
    let mut r = flat(44);
    r.click_row(row(&r, "s-a"));
    r.ch('j');
    r.ch('j');
    let at = r.app.row_id(r.app.sidebar_sel);
    shift_click(&mut r, "s-f");
    let got = selected(&r);
    assert_eq!(got.last().map(String::as_str), Some("s-f"));
    assert_eq!(r.app.selection.anchor(), at.as_ref());
    assert!(
        !got.contains(&"s-a".to_string()),
        "the range starts at the cursor: {got:?}"
    );
}

#[test]
fn ctrl_then_shift_and_ctrl_shift_follow_the_explorer_rules() {
    let mut r = flat(44);
    r.click_row_with(row(&r, "s-a"), KeyModifiers::CONTROL);
    r.click_row_with(row(&r, "s-c"), KeyModifiers::CONTROL);
    shift_click(&mut r, "s-e");
    assert_eq!(
        selected(&r),
        ids(&["s-c", "s-d", "s-e"]),
        "shift replaces from C"
    );
    let mut r = flat(44);
    r.click_row_with(row(&r, "s-a"), KeyModifiers::CONTROL);
    r.click_row_with(row(&r, "s-c"), KeyModifiers::CONTROL);
    let e = row(&r, "s-e");
    r.click_row_with(e, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert_eq!(
        selected(&r),
        ids(&["s-a", "s-c", "s-d", "s-e"]),
        "ctrl+shift adds C..E"
    );
    // alt+shift works as ctrl+shift (alt stands for ctrl where a terminal keeps ctrl+click).
    let a = row(&r, "s-f");
    r.click_row_with(a, KeyModifiers::ALT | KeyModifiers::SHIFT);
    assert_eq!(selected(&r), ids(&["s-a", "s-c", "s-d", "s-e", "s-f"]));
}

#[test]
fn shift_up_and_down_use_the_same_anchor() {
    let mut r = flat(44);
    r.click_row(row(&r, "s-c"));
    r.key(KeyCode::Down, KeyModifiers::SHIFT);
    r.key(KeyCode::Down, KeyModifiers::SHIFT);
    assert_eq!(selected(&r), ids(&["s-c", "s-d", "s-e"]));
    // Without any click, from the cursor row.
    let mut r = flat(44);
    r.app.apply(Action::SetFocus(crate::input::Focus::Sidebar));
    r.app.sidebar_sel = row(&r, "s-d");
    r.frame();
    r.key(KeyCode::Up, KeyModifiers::SHIFT);
    assert_eq!(selected(&r), ids(&["s-c", "s-d"]));
}

/// Research (f1) holding Papers (f2) and s-note; s-paper in Papers; s-free unfiled.
fn tree(height: u16) -> rig_t {
    rig_t::with(130, height, false, true, |rt| {
        rt.store("s-paper", saved("s-paper", "attention paper", 3_000));
        rt.store("s-note", saved("s-note", "lab notes", 2_000));
        rt.store("s-free", saved("s-free", "free chat", 1_000));
        ok(rt.seed_organization(&[
            create("Research", None),
            create("Papers", Some("f1")),
            file("s-paper", Some("f2")),
            file("s-note", Some("f1")),
        ]));
    })
}

#[test]
fn a_folded_folder_in_a_range_brings_its_row_not_its_hidden_children() {
    let mut r = tree(44);
    r.app.folded.insert("f2".into());
    r.app.rebuild_sidebar_items();
    r.frame();
    let f1 = r
        .app
        .row_index(&RowId::Folder("f1".into()))
        .unwrap_or(usize::MAX);
    r.click_row_with(f1, KeyModifiers::CONTROL);
    shift_click(&mut r, "s-free");
    let got = selected(&r);
    assert!(got.contains(&"folder:f2".to_string()), "{got:?}");
    assert!(got.contains(&"s-note".to_string()) && got.contains(&"s-free".to_string()));
    assert!(
        !got.contains(&"s-paper".to_string()),
        "hidden in Papers: {got:?}"
    );
}

#[test]
fn a_hidden_anchor_falls_back_instead_of_selecting_only_the_clicked_row() {
    let mut r = tree(44);
    r.click_row_with(row(&r, "s-paper"), KeyModifiers::CONTROL);
    assert_eq!(
        r.app.selection.anchor(),
        Some(&RowId::Session("s-paper".into()))
    );
    // Papers folds: the anchor is no longer listed.
    r.app.folded.insert("f2".into());
    r.app.rebuild_sidebar_items();
    r.frame();
    shift_click(&mut r, "s-free");
    let got = selected(&r);
    assert!(got.len() >= 2, "{got:?}");
    assert!(got.contains(&"s-free".to_string()));
}

#[test]
fn the_press_decides_the_click_and_a_modified_press_never_moves_a_row() {
    let mut r = flat(44);
    r.click_row(row(&r, "s-b"));
    // Shift on the press only, as terminals that drop modifiers on the release send it.
    let d = r.row_rect(row(&r, "s-d"));
    r.mouse_mods(
        MouseEventKind::Down(MouseButton::Left),
        d.x + 3,
        d.y,
        KeyModifiers::SHIFT,
    );
    r.mouse_mods(
        MouseEventKind::Up(MouseButton::Left),
        d.x + 3,
        d.y,
        KeyModifiers::NONE,
    );
    assert_eq!(selected(&r), ids(&["s-b", "s-c", "s-d"]));
    // A shift press dragged onto a folder neither moves nor selects anything.
    let mut r = tree(44);
    let applied = r.rt.applied().len();
    let free = r.row_rect(row(&r, "s-free"));
    let folder = r
        .app
        .row_index(&RowId::Folder("f1".into()))
        .map(|i| r.row_rect(i))
        .unwrap_or_default();
    r.mouse_mods(
        MouseEventKind::Down(MouseButton::Left),
        free.x + 3,
        free.y,
        KeyModifiers::SHIFT,
    );
    r.mouse_mods(
        MouseEventKind::Drag(MouseButton::Left),
        folder.x + 3,
        folder.y,
        KeyModifiers::SHIFT,
    );
    r.mouse_mods(
        MouseEventKind::Up(MouseButton::Left),
        folder.x + 3,
        folder.y,
        KeyModifiers::SHIFT,
    );
    assert_eq!(r.rt.applied().len(), applied, "nothing moved");
    assert!(r.app.grab.is_none());
}

/// Press the selection chip of `id`, after hovering its row so the chip shows.
fn press_chip(r: &mut rig_t, id: &str) {
    let i = row(r, id);
    r.hover_row(i);
    let chip = r.rect_of(&Action::Select(SelectAction::ToggleAt(i)));
    r.mouse(MouseEventKind::Down(MouseButton::Left), chip.x + 1, chip.y);
}

fn drag_over(r: &mut rig_t, id: &str) {
    let rect = r.row_rect(row(r, id));
    r.mouse(MouseEventKind::Drag(MouseButton::Left), rect.x + 5, rect.y);
}

fn release_over(r: &mut rig_t, id: &str) {
    let rect = r.row_rect(row(r, id));
    r.mouse(MouseEventKind::Up(MouseButton::Left), rect.x + 5, rect.y);
}

#[test]
fn dragging_from_a_chip_paints_a_range_down_and_up() {
    let mut r = flat(44);
    press_chip(&mut r, "s-b");
    assert_eq!(selected(&r), ids(&["s-b"]));
    drag_over(&mut r, "s-d");
    assert_eq!(
        marked(&r),
        ids(&["s-b", "s-c", "s-d"]),
        "live while dragging"
    );
    drag_over(&mut r, "s-e");
    release_over(&mut r, "s-e");
    assert!(r.app.paint.is_none());
    assert_eq!(selected(&r), ids(&["s-b", "s-c", "s-d", "s-e"]));
    assert_eq!(
        r.app.selection.anchor(),
        Some(&RowId::Session("s-b".into()))
    );
    // Up, from a row that is not selected: added to what is there.
    press_chip(&mut r, "s-f");
    drag_over(&mut r, "s-e");
    release_over(&mut r, "s-e");
    assert_eq!(selected(&r), ids(&["s-b", "s-c", "s-d", "s-e", "s-f"]));
    // Every row shows its chip while a selection shows, and a hint says why.
    let screen = r.screen();
    assert!(screen.contains("drag ☐"), "{screen}");
    let a = row(&r, "s-a");
    assert!(
        r.app
            .hits
            .find(&Action::Select(SelectAction::ToggleAt(a)))
            .is_some()
    );
}

#[test]
fn painting_from_a_selected_row_unselects_and_esc_cancels() {
    let mut r = flat(44);
    r.click_row(row(&r, "s-a"));
    r.app.apply(Action::Select(SelectAction::All));
    r.frame();
    let all = selected(&r);
    press_chip(&mut r, "s-c");
    drag_over(&mut r, "s-d");
    release_over(&mut r, "s-d");
    let got = selected(&r);
    assert!(
        !got.contains(&"s-c".into()) && !got.contains(&"s-d".into()),
        "{got:?}"
    );
    assert_eq!(got.len(), all.len() - 2);
    // esc during a paint puts the selection back.
    let before = selected(&r);
    press_chip(&mut r, "s-a");
    drag_over(&mut r, "s-b");
    assert_ne!(selected(&r), before);
    r.key(KeyCode::Esc, KeyModifiers::NONE);
    r.advance(EscapeDelayMs + 1);
    assert!(r.app.paint.is_none());
    assert_eq!(selected(&r), before);
    // A press and release on the same chip toggles that row.
    press_chip(&mut r, "s-c");
    release_over(&mut r, "s-c");
    assert!(selected(&r).contains(&"s-c".to_string()));
}

#[test]
fn a_paint_scrolls_the_list_at_its_edges() {
    let mut r = rig_t::with(130, 24, false, true, |rt| {
        for i in 0..30u64 {
            let id = format!("s-{i:02}");
            rt.store(&id, saved(&id, &format!("talk {i:02}"), 90_000 - i * 100));
        }
    });
    press_chip(&mut r, "s-01");
    let list = r.app.sidebar_list;
    let before = r.app.sidebar_scroll;
    for _ in 0..5 {
        r.mouse(
            MouseEventKind::Drag(MouseButton::Left),
            list.x + 5,
            list.bottom().saturating_sub(1),
        );
    }
    assert!(r.app.sidebar_scroll > before, "scrolled down");
    assert!(selected(&r).len() > 3, "{:?}", selected(&r));
}
