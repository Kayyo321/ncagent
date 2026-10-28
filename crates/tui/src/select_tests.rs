/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Selection, context menu and delete tests (D57 to D61): the whole UI on ratatui's
//! `TestBackend`, driven by synthetic crossterm events through the hit registry, against the
//! scripted fixture, whose folders and conversation settings use the real rules of
//! `harness::organize`.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton};
use harness::api::{SessionError, SessionPhase};
use harness::approval::ApprovalMode;
use harness::organize::{MetaChange, OrganizeOp, TagColor};
use harness::store::StoreError;
use ratatui::layout::Rect;

use crate::app::EscapeDelayMs;
use crate::app_tests::{rig_t, start_turn};
use crate::folder_tests::{
    cell_bg, create, file, folder_index, ok, saved, session_index, toast, tree_rig,
};
use crate::input::{Action, Focus, MenuAction, Modal, SelectAction};
use crate::scripted::ScriptCall;
use crate::selection::RowId;
use crate::theme::theme_t;

/// Five unfiled conversations, newest first a to e, and an empty folder Box.
fn flat_rig(width: u16, height: u16) -> rig_t {
    rig_t::with(width, height, false, true, |rt| {
        for (i, id) in ["s-a", "s-b", "s-c", "s-d", "s-e"].iter().enumerate() {
            let title = format!("talk {}", &id[2..]);
            rt.store(id, saved(id, &title, 9_000 - i as u64 * 1_000));
        }
        ok(rt.seed_organization(&[create("Box", None)]));
    })
}

fn row(r: &rig_t, id: &str) -> usize {
    session_index(r, id)
}

fn selected_ids(r: &rig_t) -> Vec<String> {
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

fn menu_labels(r: &rig_t) -> Vec<String> {
    r.app
        .menu
        .as_ref()
        .map(|m| m.items.iter().map(|i| i.label.clone()).collect())
        .unwrap_or_default()
}

/// Click the menu item with this label.
fn pick(r: &mut rig_t, label: &str) {
    let i = menu_labels(r)
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("no item {label}: {:?}", menu_labels(r)));
    r.click_action(&Action::Menu(MenuAction::Pick(i)));
}

/// Click the item of the open submenu with this label.
fn pick_sub(r: &mut rig_t, label: &str) {
    let labels: Vec<String> = r
        .app
        .menu
        .as_ref()
        .and_then(|m| m.sub.and_then(|p| m.items.get(p)))
        .map(|p| p.sub.iter().map(|i| i.label.clone()).collect())
        .unwrap_or_default();
    let i = labels
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("no sub item {label}: {labels:?}"));
    r.click_action(&Action::Menu(MenuAction::PickSub(i)));
}

fn last_op(r: &rig_t) -> OrganizeOp {
    r.rt.applied()
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("nothing applied"))
}

#[test]
fn ctrl_shift_and_ctrl_shift_clicks_chain_like_a_file_explorer() {
    let mut rig = flat_rig(130, 44);
    let tabs = rig.app.sessions.len();
    rig.click_row(row(&rig, "s-a"));
    assert_eq!(rig.app.active().map(|s| s.session.id()), Some("s-a"));
    assert!(
        !rig.app.selection.shown(),
        "a plain click is no selection tone"
    );
    rig.click_row_with(row(&rig, "s-c"), KeyModifiers::CONTROL);
    assert_eq!(selected_ids(&rig), ["s-a", "s-c"]);
    rig.click_row_with(row(&rig, "s-e"), KeyModifiers::SHIFT);
    assert_eq!(
        selected_ids(&rig),
        ["s-c", "s-d", "s-e"],
        "shift replaces from the anchor"
    );
    rig.click_row_with(
        row(&rig, "s-a"),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert_eq!(selected_ids(&rig), ["s-a", "s-b", "s-c", "s-d", "s-e"]);
    rig.click_row_with(row(&rig, "s-d"), KeyModifiers::CONTROL);
    assert_eq!(selected_ids(&rig), ["s-a", "s-b", "s-c", "s-e"]);
    // alt+click toggles too, for terminals that keep ctrl+click.
    rig.click_row_with(row(&rig, "s-b"), KeyModifiers::ALT);
    assert_eq!(selected_ids(&rig), ["s-a", "s-c", "s-e"]);
    // None of the modified clicks opened anything.
    assert_eq!(rig.app.sessions.len(), tabs);
    assert_eq!(rig.app.active().map(|s| s.session.id()), Some("s-a"));
    // The selection tone and the line with the count.
    let screen = rig.screen();
    assert!(screen.contains("3 selected"), "{screen}");
    let theme = theme_t::dark();
    let c = rig.row_rect(row(&rig, "s-c"));
    assert_eq!(cell_bg(&rig, c.x + 6, c.y), theme.marked);
    let d = rig.row_rect(row(&rig, "s-d"));
    assert_ne!(cell_bg(&rig, d.x + 6, d.y), theme.marked);
    // The clear chip, then esc in the sidebar, clear it.
    rig.click_action(&Action::Select(SelectAction::Clear));
    assert!(rig.app.selection.is_empty());
    assert!(!rig.screen().contains("selected"));
}

#[test]
fn the_keys_select_like_the_mouse() {
    let mut rig = flat_rig(130, 44);
    rig.select_by_keys(row(&rig, "s-b"));
    rig.key(KeyCode::Down, KeyModifiers::SHIFT);
    rig.key(KeyCode::Char('J'), KeyModifiers::SHIFT);
    assert_eq!(selected_ids(&rig), ["s-b", "s-c", "s-d"]);
    rig.key(KeyCode::Char('K'), KeyModifiers::SHIFT);
    assert_eq!(selected_ids(&rig), ["s-b", "s-c"]);
    rig.ch('j');
    rig.ch('j');
    rig.key(KeyCode::Char(' '), KeyModifiers::CONTROL);
    assert_eq!(selected_ids(&rig), ["s-b", "s-c", "s-e"]);
    assert_eq!(rig.app.modal(), Modal::None, "ctrl+space opens nothing");
    rig.key(KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(selected_ids(&rig).len(), rig.app.visible_rows().len());
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    assert!(rig.app.selection.is_empty());
    assert_eq!(rig.app.focus, Focus::Sidebar, "esc cleared, focus stayed");
}

#[test]
fn the_check_chip_selects_without_modifier_keys() {
    let mut rig = flat_rig(130, 44);
    for id in ["s-b", "s-d"] {
        let i = row(&rig, id);
        rig.hover_row(i);
        assert!(rig.screen().contains("☐"), "{}", rig.screen());
        rig.click_action(&Action::Select(SelectAction::ToggleAt(i)));
    }
    assert_eq!(selected_ids(&rig), ["s-b", "s-d"]);
    rig.hover_row(row(&rig, "s-b"));
    assert!(rig.screen().contains("☑"), "{}", rig.screen());
    // The menu chip of the selection line opens the menu for both.
    rig.click_action(&Action::Menu(MenuAction::OpenSelection));
    assert_eq!(rig.app.modal(), Modal::Menu);
    assert!(rig.screen().contains("2 selected"));
    assert!(menu_labels(&rig).contains(&"Pin all".to_string()));
}

#[test]
fn dragging_a_selected_row_moves_the_whole_selection_in_one_batch() {
    let mut rig = flat_rig(130, 44);
    rig.click_row_with(row(&rig, "s-a"), KeyModifiers::CONTROL);
    rig.click_row_with(row(&rig, "s-c"), KeyModifiers::CONTROL);
    let applied = rig.rt.applied().len();
    rig.drag_rows(row(&rig, "s-c"), folder_index(&rig, "Box"));
    for id in ["s-a", "s-c"] {
        assert_eq!(rig.app.organization.folder_of(id), Some("f1"), "{id}");
    }
    assert_eq!(rig.app.organization.folder_of("s-b"), None);
    assert_eq!(rig.rt.applied().len(), applied + 1, "one change");
    assert!(matches!(last_op(&rig), OrganizeOp::Batch(ops) if ops.len() == 2));
    assert_eq!(selected_ids(&rig), ["s-a", "s-c"], "the selection stays");
    // An unselected row drags alone.
    rig.drag_rows(row(&rig, "s-b"), folder_index(&rig, "Box"));
    assert_eq!(rig.app.organization.folder_of("s-b"), Some("f1"));
    assert_eq!(rig.app.organization.folder_of("s-d"), None);
}

#[test]
fn a_selection_move_with_one_refused_item_moves_nothing() {
    let mut rig = tree_rig(130, 44, true);
    // Research and Ideas together cannot go into Papers, which is inside Research.
    rig.click_row_with(folder_index(&rig, "Research"), KeyModifiers::CONTROL);
    rig.click_row_with(folder_index(&rig, "Ideas"), KeyModifiers::CONTROL);
    let before = rig.app.organization.clone();
    rig.drag_rows(folder_index(&rig, "Ideas"), folder_index(&rig, "Papers"));
    assert_eq!(rig.app.organization, before);
    assert!(toast(&rig).contains("cannot move"), "{}", toast(&rig));
    // Research with its own conversation: the conversation rides along with its folder.
    rig.app.selection.clear();
    rig.app.rebuild_sidebar_items();
    rig.frame();
    rig.click_row_with(folder_index(&rig, "Research"), KeyModifiers::CONTROL);
    rig.click_row_with(session_index(&rig, "s-note"), KeyModifiers::CONTROL);
    rig.drag_rows(session_index(&rig, "s-note"), folder_index(&rig, "Ideas"));
    assert_eq!(
        rig.app
            .organization
            .folder("f1")
            .and_then(|f| f.parent.clone()),
        Some("f3".to_string())
    );
    assert_eq!(rig.app.organization.folder_of("s-note"), Some("f1"));
}

#[test]
fn pick_up_carries_the_selection() {
    let mut rig = flat_rig(130, 44);
    rig.select_by_keys(row(&rig, "s-d"));
    rig.key(KeyCode::Down, KeyModifiers::SHIFT);
    rig.ch('m');
    assert_eq!(rig.app.modal(), Modal::Move);
    assert!(rig.screen().contains("moving 2 items"), "{}", rig.screen());
    rig.choose_destination(folder_index(&rig, "Box"));
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.organization.folder_of("s-d"), Some("f1"));
    assert_eq!(rig.app.organization.folder_of("s-e"), Some("f1"));
}

#[test]
fn right_click_opens_the_conversation_menu_and_pin_takes_effect() {
    let mut rig = flat_rig(130, 44);
    let i = row(&rig, "s-c");
    rig.right_click_row(i);
    assert_eq!(rig.app.modal(), Modal::Menu);
    assert_eq!(
        menu_labels(&rig),
        [
            "Open",
            "Open in new tab",
            "Rename…",
            "Pin",
            "Colour tag",
            "Approval mode",
            "Archive",
            "Move to folder…",
            "Copy id",
            "Delete…"
        ]
    );
    let screen = rig.screen();
    assert!(screen.contains("“talk c”"), "{screen}");
    // The menu opens at the pointer: below it, or above it near the bottom.
    let r = rig.row_rect(i);
    let menu = rig.app.overlay_areas.first().copied().unwrap_or_default();
    assert_eq!(menu.x, r.x + 3, "{screen}");
    assert!(menu.y == r.y || menu.bottom() == r.y + 1, "{menu:?} {r:?}");
    assert_eq!(selected_ids(&rig), ["s-c"], "the row became the selection");
    pick(&mut rig, "Pin");
    assert_eq!(rig.app.modal(), Modal::None);
    assert!(rig.app.organization.meta("s-c").pinned);
    // Pinned conversations lead the top level, with the pin glyph.
    let first = rig
        .app
        .sidebar_items
        .iter()
        .position(|i| i.in_tree())
        .unwrap_or(0);
    assert_eq!(rig.app.row_id(first), Some(RowId::Session("s-c".into())));
    assert!(rig.screen().contains("✦ talk c"), "{}", rig.screen());
    // The menu now offers Unpin.
    rig.right_click_row(row(&rig, "s-c"));
    assert!(menu_labels(&rig).contains(&"Unpin".to_string()));
}

#[test]
fn menu_keys_move_open_submenus_go_back_and_close() {
    let mut rig = flat_rig(130, 44);
    rig.select_by_keys(row(&rig, "s-b"));
    rig.ch('.');
    assert_eq!(rig.app.modal(), Modal::Menu);
    // Walk down to the colour tag: open, open in new tab, rename, pin, colour tag.
    for _ in 0..4 {
        rig.ch('j');
    }
    assert_eq!(rig.app.menu.as_ref().map(|m| m.sel), Some(4));
    rig.key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(rig.app.menu.as_ref().and_then(|m| m.sub), Some(4));
    rig.key(KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(rig.app.menu.as_ref().and_then(|m| m.sub), None);
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    rig.ch('j');
    rig.ch('j');
    rig.ch('j');
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rig.app.organization.meta("s-b").color,
        Some(TagColor::Green)
    );
    assert_eq!(rig.app.modal(), Modal::None);
    // shift+f10 opens the same menu; esc closes it after the delay.
    rig.key(KeyCode::F(10), KeyModifiers::SHIFT);
    assert_eq!(rig.app.modal(), Modal::Menu);
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    assert_eq!(rig.app.modal(), Modal::None);
    // A digit picks: 4 is Pin.
    rig.ch('.');
    rig.ch('4');
    assert!(rig.app.organization.meta("s-b").pinned);
}

#[test]
fn a_selection_menu_offers_what_applies_to_all_and_applies_one_batch() {
    let mut rig = flat_rig(130, 44);
    for id in ["s-a", "s-b", "s-d"] {
        rig.click_row_with(row(&rig, id), KeyModifiers::CONTROL);
    }
    rig.right_click_row(row(&rig, "s-b"));
    assert_eq!(
        selected_ids(&rig),
        ["s-a", "s-b", "s-d"],
        "kept on a selected row"
    );
    assert_eq!(
        menu_labels(&rig),
        [
            "Open in tabs",
            "Pin all",
            "Colour tag",
            "Archive all",
            "Move to folder…",
            "Delete…"
        ]
    );
    pick(&mut rig, "Colour tag");
    pick_sub(&mut rig, "teal");
    match last_op(&rig) {
        OrganizeOp::Batch(ops) => {
            assert_eq!(ops.len(), 3);
            assert!(ops.iter().all(|op| matches!(
                op,
                OrganizeOp::SetSessionMeta {
                    change: MetaChange::Color(Some(TagColor::Teal)),
                    ..
                }
            )));
        }
        other => panic!("expected one batch, found {other:?}"),
    }
    assert_eq!(
        selected_ids(&rig),
        ["s-a", "s-b", "s-d"],
        "kept after the change"
    );
    // Open in tabs opens each one, and shows the first.
    rig.right_click_row(row(&rig, "s-a"));
    pick(&mut rig, "Open in tabs");
    assert!(rig.app.tab_of("s-a").is_some() && rig.app.tab_of("s-d").is_some());
    assert_eq!(rig.app.active().map(|s| s.session.id()), Some("s-a"));
    // With a folder in it, only move and delete remain.
    rig.click_row_with(folder_index(&rig, "Box"), KeyModifiers::CONTROL);
    rig.right_click_row(folder_index(&rig, "Box"));
    assert_eq!(menu_labels(&rig), ["Move to folder…", "Delete…"]);
}

#[test]
fn the_folder_menu_reuses_the_folder_actions() {
    let mut rig = tree_rig(130, 44, true);
    rig.right_click_row(folder_index(&rig, "Research"));
    assert_eq!(
        menu_labels(&rig),
        ["New sub-folder", "Rename…", "Move to folder…", "Delete…"]
    );
    pick(&mut rig, "Delete…");
    assert_eq!(rig.app.confirm.as_deref(), Some("f1"), "the inline confirm");
    rig.key(KeyCode::Char('n'), KeyModifiers::NONE);
    rig.right_click_row(folder_index(&rig, "Research"));
    pick(&mut rig, "New sub-folder");
    assert_eq!(rig.app.modal(), Modal::Prompt);
}

#[test]
fn rename_writes_the_title_override_everywhere_and_empty_clears_it() {
    let mut rig = flat_rig(130, 44);
    rig.click_row(row(&rig, "s-c"));
    rig.right_click_row(row(&rig, "s-c"));
    pick(&mut rig, "Rename…");
    assert_eq!(rig.app.modal(), Modal::Prompt);
    assert!(rig.screen().contains("rename conversation"));
    rig.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    rig.typed("Quarterly plan");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rig.app.organization.meta("s-c").title.as_deref(),
        Some("Quarterly plan")
    );
    let screen = rig.screen();
    let tab_row = screen.lines().next().unwrap_or("");
    assert!(tab_row.contains("Quarterly plan"), "tab: {tab_row}");
    let i = row(&rig, "s-c");
    assert_eq!(rig.app.sidebar_items[i].title, "Quarterly plan");
    // /rename with no title opens the prompt; an empty title clears the override.
    rig.app.apply(Action::SetFocus(Focus::Input));
    rig.typed("/rename");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    rig.key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.organization.meta("s-c").title, None);
    assert_eq!(rig.app.conversation_title("s-c"), "talk c");
    // A title given to /rename is set at once.
    rig.app.apply(Action::SetFocus(Focus::Input));
    rig.typed("/rename x");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.conversation_title("s-c"), "x");
}

#[test]
fn archive_hides_the_row_unless_archived_rows_show() {
    let mut rig = flat_rig(130, 44);
    rig.right_click_row(row(&rig, "s-b"));
    pick(&mut rig, "Archive");
    assert!(rig.app.organization.meta("s-b").archived);
    assert!(
        !rig.app
            .visible_rows()
            .contains(&RowId::Session("s-b".into()))
    );
    assert!(toast(&rig).contains("hidden"), "{}", toast(&rig));
    assert!(rig.app.update_settings(|s| s.show_archived = true));
    rig.app.rebuild_sidebar_items();
    rig.frame();
    let i = row(&rig, "s-b");
    assert!(rig.app.sidebar_items[i].archived);
    assert!(rig.screen().contains("archived"), "{}", rig.screen());
    assert!(
        ok(rig.rt.saved_settings()).show_archived,
        "the setting was saved"
    );
    rig.right_click_row(i);
    pick(&mut rig, "Unarchive");
    assert!(!rig.app.organization.meta("s-b").archived);
}

#[test]
fn the_approval_mode_applies_to_the_open_session_and_full_is_explained() {
    let mut rig = flat_rig(130, 44);
    rig.click_row(row(&rig, "s-a"));
    let tab = rig.app.active;
    rig.right_click_row(row(&rig, "s-a"));
    pick(&mut rig, "Approval mode");
    pick_sub(&mut rig, "edit");
    assert_eq!(
        rig.app.organization.meta("s-a").approval,
        Some(ApprovalMode::Edit)
    );
    let handle = rig.tab_handle(tab);
    assert!(
        handle
            .calls()
            .contains(&ScriptCall::SetApprovalMode(ApprovalMode::Edit))
    );
    rig.right_click_row(row(&rig, "s-a"));
    pick(&mut rig, "Approval mode");
    pick_sub(&mut rig, "full");
    assert!(toast(&rig).contains("command line"), "{}", toast(&rig));
    assert_eq!(rig.app.modal(), Modal::Menu, "a dim item runs nothing");
    pick_sub(&mut rig, "default");
    assert_eq!(rig.app.organization.meta("s-a").approval, None);
    assert!(
        handle
            .calls()
            .contains(&ScriptCall::SetApprovalMode(ApprovalMode::Ask))
    );
}

#[test]
fn copy_id_sends_osc52() {
    let mut rig = flat_rig(130, 44);
    rig.right_click_row(row(&rig, "s-e"));
    pick(&mut rig, "Copy id");
    assert_eq!(rig.app.outputs, vec![crate::clipboard::osc52("s-e")]);
}

#[test]
fn a_press_outside_the_menu_closes_it_and_does_nothing_else() {
    let mut rig = flat_rig(130, 44);
    rig.right_click_row(row(&rig, "s-e"));
    let tabs = rig.app.sessions.len();
    let r = rig.rect_of(&Action::NewChat);
    rig.click(r.x, r.y);
    assert_eq!(rig.app.modal(), Modal::None);
    assert_eq!(
        rig.app.sessions.len(),
        tabs,
        "the click did not reach the button"
    );
    // A right press elsewhere opens the menu there instead. The menu starts three columns into
    // its row, so a press at the row's first columns is outside it.
    rig.right_click_row(row(&rig, "s-e"));
    let a = rig.row_rect(row(&rig, "s-a"));
    rig.click_with(MouseButton::Right, a.x + 1, a.y, KeyModifiers::NONE);
    assert!(rig.screen().contains("“talk a”"), "{}", rig.screen());
}

#[test]
fn the_menu_stays_on_screen_and_a_submenu_opens_inline_when_narrow() {
    // Near the bottom right corner it flips up and left.
    let mut rig = flat_rig(130, 44);
    let i = row(&rig, "s-e");
    let r = rig.row_rect(i);
    rig.click_with(MouseButton::Right, 129, r.y, KeyModifiers::NONE);
    let screen = Rect::new(0, 0, 130, 44);
    let main = rig.app.overlay_areas.first().copied().unwrap_or_default();
    assert_eq!(main.intersection(screen), main);
    assert!(main.right() <= 130 && main.x < 129, "{main:?}");
    pick(&mut rig, "Colour tag");
    let sub = rig.app.overlay_areas.get(1).copied().unwrap_or_default();
    assert!(
        sub.right() <= main.x,
        "the submenu opens to the left: {sub:?} {main:?}"
    );
    let shown = rig.screen();
    assert!(shown.contains("purple"), "{shown}");

    // At 40 columns there is no room beside it: the submenu opens inline.
    let mut rig = flat_rig(40, 40);
    rig.key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    let i = row(&rig, "s-a");
    rig.right_click_row(i);
    pick(&mut rig, "Colour tag");
    assert_eq!(rig.app.overlay_areas.len(), 1, "one panel");
    let shown = rig.screen();
    for c in TagColor::all() {
        assert!(shown.contains(c.name()), "{} missing\n{shown}", c.name());
    }
    assert!(shown.contains('▾'), "{shown}");
    for ch in ['┌', '┐', '└', '┘', '│', '─'] {
        assert!(!shown.contains(ch), "box character {ch}\n{shown}");
    }
    pick_sub(&mut rig, "pink");
    assert_eq!(rig.app.organization.meta("s-a").color, Some(TagColor::Pink));
}

#[test]
fn the_colour_tag_shows_on_the_row_and_the_tab() {
    let mut rig = flat_rig(130, 44);
    rig.click_row(row(&rig, "s-a"));
    rig.right_click_row(row(&rig, "s-a"));
    pick(&mut rig, "Colour tag");
    pick_sub(&mut rig, "orange");
    let orange = crate::tags::tag_rgb(TagColor::Orange);
    let r = rig.row_rect(row(&rig, "s-a"));
    let dot = (r.x..r.right()).find(|x| {
        rig.term
            .backend()
            .buffer()
            .cell((*x, r.y))
            .is_some_and(|c| c.symbol() == "●" && c.fg == orange)
    });
    assert!(dot.is_some(), "no orange dot on the row\n{}", rig.screen());
    let tab = rig
        .app
        .hits
        .find_target(&crate::input::Target::Tab(rig.app.active))
        .unwrap_or_default();
    let rail = rig.term.backend().buffer().cell((tab.x, tab.y)).cloned();
    assert_eq!(
        rail.map(|c| (c.symbol().to_string(), c.fg)),
        Some(("▌".into(), orange))
    );
}

#[test]
fn delete_asks_then_removes_the_logs_and_closes_the_tabs() {
    let mut rig = flat_rig(130, 44);
    rig.click_row(row(&rig, "s-b"));
    assert!(rig.app.tab_of("s-b").is_some());
    rig.click_row_with(row(&rig, "s-c"), KeyModifiers::CONTROL);
    rig.key(KeyCode::Delete, KeyModifiers::NONE);
    assert_eq!(rig.app.modal(), Modal::Confirm);
    let screen = rig.screen();
    assert!(screen.contains("delete 2 conversations?"), "{screen}");
    assert!(screen.contains("saved logs permanently"), "{screen}");
    assert!(screen.contains("enter delete"), "{screen}");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.rt.deleted(), ["s-b", "s-c"]);
    assert!(!rig.rt.is_stored("s-b") && !rig.rt.is_stored("s-c"));
    assert!(rig.app.tab_of("s-b").is_none(), "its tab closed");
    assert_eq!(toast(&rig), "2 deleted");
    // The cursor is on the row that followed them.
    assert_eq!(
        rig.app.row_id(rig.app.sidebar_sel),
        Some(RowId::Session("s-d".into()))
    );
    assert!(!rig.screen().contains("talk b"));
}

#[test]
fn delete_skips_a_running_conversation_and_says_so() {
    let mut rig = flat_rig(130, 44);
    rig.click_row(row(&rig, "s-a"));
    let tab = rig.app.active;
    rig.push(start_turn("busy"));
    rig.tab_handle(tab).set_phase(SessionPhase::Generating);
    rig.frame();
    rig.click_row_with(row(&rig, "s-e"), KeyModifiers::CONTROL);
    assert_eq!(selected_ids(&rig), ["s-a", "s-e"]);
    rig.ch('d');
    assert!(rig.screen().contains("still running and will be skipped"));
    rig.click_action(&Action::ConfirmYes);
    assert_eq!(rig.rt.deleted(), ["s-e"]);
    assert_eq!(toast(&rig), "1 deleted · 1 skipped: still running");
    // Only running ones: refused at once.
    rig.select_by_keys(row(&rig, "s-a"));
    rig.ch('d');
    assert_eq!(rig.app.modal(), Modal::None);
    assert!(toast(&rig).contains("still running"), "{}", toast(&rig));
}

#[test]
fn delete_without_confirmation_and_with_folders_in_the_selection() {
    let mut rig = tree_rig(130, 44, true);
    assert!(rig.app.update_settings(|s| s.confirm_delete = false));
    rig.click_row_with(folder_index(&rig, "Research"), KeyModifiers::CONTROL);
    rig.click_row_with(session_index(&rig, "s-free"), KeyModifiers::CONTROL);
    rig.ch('d');
    assert_eq!(rig.app.modal(), Modal::None, "no dialog");
    assert_eq!(rig.rt.deleted(), ["s-free"]);
    assert!(rig.app.organization.folder("f1").is_none());
    // Its contents moved up: Papers is a top-level folder, lab notes unfiled.
    assert_eq!(
        rig.app
            .organization
            .folder("f2")
            .and_then(|f| f.parent.clone()),
        None
    );
    assert_eq!(rig.app.organization.folder_of("s-note"), None);
    assert!(toast(&rig).contains("1 folder deleted"), "{}", toast(&rig));
}

#[test]
fn the_delete_dialog_cancels_by_key_button_and_outside_click() {
    let mut rig = flat_rig(130, 44);
    rig.select_by_keys(row(&rig, "s-c"));
    rig.ch('d');
    rig.ch('n');
    assert_eq!(rig.app.modal(), Modal::None);
    rig.ch('d');
    rig.click_action(&Action::SidebarCancel);
    assert_eq!(rig.app.modal(), Modal::None);
    rig.ch('d');
    rig.click(1, 1);
    assert_eq!(rig.app.modal(), Modal::None);
    rig.ch('d');
    rig.key(KeyCode::Esc, KeyModifiers::NONE);
    rig.advance(EscapeDelayMs);
    assert_eq!(rig.app.modal(), Modal::None);
    assert!(rig.rt.deleted().is_empty(), "nothing was deleted");
    // A store error is reported and the conversation stays.
    rig.rt.fail_delete(Some(SessionError::Store(StoreError::Io {
        path: "x".into(),
        message: "disk full".into(),
    })));
    rig.select_by_keys(row(&rig, "s-c"));
    rig.ch('d');
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(toast(&rig).contains("disk full"), "{}", toast(&rig));
    assert!(rig.rt.is_stored("s-c"));
}

#[test]
fn slash_commands_act_on_the_open_conversation() {
    let mut rig = flat_rig(130, 44);
    rig.click_row(row(&rig, "s-d"));
    rig.app.apply(Action::SetFocus(Focus::Input));
    for cmd in ["/pin", "/archive"] {
        rig.typed(cmd);
        rig.key(KeyCode::Enter, KeyModifiers::NONE);
    }
    let meta = rig.app.organization.meta("s-d");
    assert!(meta.pinned && meta.archived);
    rig.typed("/rename  New name ");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.conversation_title("s-d"), "New name");
    rig.typed("/delete");
    rig.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        rig.screen().contains("delete “New name”?"),
        "{}",
        rig.screen()
    );
    rig.key(KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(rig.rt.deleted(), ["s-d"]);
    assert!(rig.app.tab_of("s-d").is_none());
    assert!(!rig.app.sessions.is_empty(), "there is always a tab");
}

#[test]
fn ordering_by_title_keeps_pinned_conversations_first() {
    let mut rig = flat_rig(130, 44);
    ok(rig.rt.seed_organization(&[
        OrganizeOp::SetSessionMeta {
            session_id: "s-d".into(),
            change: MetaChange::Pinned(true),
        },
        file("s-e", Some("f1")),
    ]));
    rig.app.load_organization();
    assert!(rig.app.update_settings(|s| {
        s.conversation_order = harness::settings::ConversationOrder::Title
    }));
    rig.app.rebuild_sidebar_items();
    let order: Vec<RowId> = rig.app.visible_rows();
    let names: Vec<String> = order
        .iter()
        .map(|r| match r {
            RowId::Session(s) => s.clone(),
            RowId::Folder(f) => f.clone(),
        })
        .collect();
    assert_eq!(names.first().map(String::as_str), Some("s-d"), "{names:?}");
    let pos = |id: &str| names.iter().position(|n| n == id).unwrap_or(usize::MAX);
    assert!(pos("f1") < pos("s-a"), "folders before unpinned: {names:?}");
    assert!(
        pos("s-a") < pos("s-b") && pos("s-b") < pos("s-c"),
        "{names:?}"
    );
}
