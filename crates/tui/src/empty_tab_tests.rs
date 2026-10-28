/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Empty conversations delete themselves when their tab goes (see `app_t::discard_if_empty`):
//! closing a tab by every path, replacing an unused tab, and quitting. Anything with history,
//! a draft, its own settings or a folder is kept.

use crossterm::event::{KeyCode, KeyModifiers};
use harness::api::SessionError;
use harness::event::{SessionEvent, TurnOutcome};
use harness::organize::TagColor;
use harness::store::StoreError;

use crate::app_tests::{rig_t, start_turn};
use crate::folder_tests::{create, file, ok, saved, toast};
use crate::input::{Action, Focus, TabAction};
use crate::sidebar::SidebarItemKind;

fn rig() -> rig_t {
    rig_t::with(130, 40, false, true, |rt| {
        rt.store("saved-1", saved("saved-1", "an older chat", 9_000_000));
    })
}

/// The id of tab `i`.
fn id_of(r: &rig_t, i: usize) -> String {
    r.app
        .sessions
        .get(i)
        .map(|s| s.session.id().to_string())
        .unwrap_or_default()
}

fn listed(r: &rig_t, id: &str) -> bool {
    r.app
        .sidebar_items
        .iter()
        .any(|it| matches!(&it.kind, SidebarItemKind::Session { id: other } if other == id))
}

fn finished_turn(text: &str) -> Vec<SessionEvent> {
    let mut events = start_turn(text);
    events.push(SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Completed,
        calls: 0,
        passes: 0,
    });
    events
}

/// A second tab, shown, with nothing in it. Returns its id.
fn new_empty_tab(r: &mut rig_t) -> String {
    r.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    id_of(r, r.app.active)
}

#[test]
fn closing_an_empty_tab_deletes_it_by_every_path() {
    let mut r = rig();
    r.push(finished_turn("keep me"));
    let kept = id_of(&r, 0);
    // alt+w, the × chip, /close, and the tab menu's Close.
    let paths: [fn(&mut rig_t); 4] = [
        |r| r.key(KeyCode::Char('w'), KeyModifiers::ALT),
        |r| {
            let i = r.app.active;
            r.click_action(&Action::Tab(TabAction::Close(i)));
        },
        |r| {
            r.typed("/close");
            r.key(KeyCode::Enter, KeyModifiers::NONE);
        },
        |r| {
            r.key(KeyCode::F(10), KeyModifiers::SHIFT);
            let close = r
                .app
                .menu
                .as_ref()
                .and_then(|m| m.items.iter().position(|i| i.label == "Close"))
                .unwrap_or_else(|| panic!("no close item: {:?}", r.app.menu));
            r.app
                .apply(Action::Menu(crate::input::MenuAction::Pick(close)));
            r.frame();
        },
    ];
    for (n, close) in paths.iter().enumerate() {
        let id = new_empty_tab(&mut r);
        assert!(listed(&r, &id), "{n}: listed while open");
        close(&mut r);
        assert!(r.app.tab_of(&id).is_none(), "{n}: closed");
        assert!(r.rt.deleted().contains(&id), "{n}: {:?}", r.rt.deleted());
        assert!(!listed(&r, &id), "{n}: the row is gone at once");
        assert!(
            !toast(&r).contains("delete"),
            "{n}: no toast: {}",
            toast(&r)
        );
    }
    assert!(!r.rt.deleted().contains(&kept));
    // The row does not come back with the next read of the list.
    r.advance(10_000);
    assert!(r.rt.deleted().iter().all(|id| !listed(&r, id)));
}

#[test]
fn a_tab_with_history_a_draft_or_its_own_settings_is_kept() {
    type setup_t = fn(&mut rig_t);
    let cases: [(&str, setup_t); 7] = [
        ("message", |r| r.push(finished_turn("hello"))),
        ("draft", |r| r.typed("half a thought")),
        ("pin", |r| {
            r.typed("/pin");
            r.key(KeyCode::Enter, KeyModifiers::NONE);
        }),
        ("colour", |r| {
            let ids = r.app.shown_ids();
            r.app.set_color(&ids, Some(TagColor::Teal));
        }),
        ("rename", |r| {
            r.typed("/rename My plan");
            r.key(KeyCode::Enter, KeyModifiers::NONE);
        }),
        ("folder", |r| {
            let id = id_of(r, r.app.active);
            ok(r.rt.seed_organization(&[create("Research", None), file(&id, Some("f1"))]));
            r.app.load_organization();
        }),
        ("busy", |r| r.push(start_turn("still going"))),
    ];
    for (name, setup) in cases {
        let mut r = rig();
        let id = new_empty_tab(&mut r);
        setup(&mut r);
        r.key(KeyCode::Char('w'), KeyModifiers::ALT);
        assert!(r.app.tab_of(&id).is_none(), "{name}: closed");
        assert!(!r.rt.deleted().contains(&id), "{name}: kept");
    }
}

#[test]
fn a_background_tab_with_a_draft_is_kept_and_without_one_is_deleted() {
    let mut r = rig();
    let first = id_of(&r, 0);
    r.typed("draft in the first tab");
    let second = new_empty_tab(&mut r);
    // The first tab is in the background now; its draft is saved with it.
    r.app.apply(Action::Tab(TabAction::Close(0)));
    r.frame();
    assert!(r.app.tab_of(&first).is_none());
    assert!(!r.rt.deleted().contains(&first), "its draft keeps it");
    let third = new_empty_tab(&mut r);
    let i = r.app.tab_of(&second).unwrap_or(usize::MAX);
    r.app.apply(Action::Tab(TabAction::Close(i)));
    r.frame();
    assert!(
        r.rt.deleted().contains(&second),
        "an empty background tab goes"
    );
    assert!(r.app.tab_of(&third).is_some());
}

#[test]
fn replacing_an_unused_tab_deletes_the_conversation_it_held() {
    let mut r = rig();
    let unused = id_of(&r, 0);
    let row = r
        .app
        .sidebar_items
        .iter()
        .position(|it| matches!(&it.kind, SidebarItemKind::Session { id } if id == "saved-1"))
        .unwrap_or(usize::MAX);
    r.click_row(row);
    assert_eq!(r.app.sessions.len(), 1, "the unused tab was replaced");
    assert_eq!(id_of(&r, 0), "saved-1");
    assert_eq!(r.rt.deleted(), std::slice::from_ref(&unused));
    assert!(!listed(&r, &unused));
    assert!(!r.rt.deleted().contains(&"saved-1".to_string()));
}

#[test]
fn quitting_deletes_the_empty_open_tabs_and_saves_only_the_rest() {
    let mut r = rig();
    let first = id_of(&r, 0);
    r.push(finished_turn("keep me"));
    let empty_a = new_empty_tab(&mut r);
    let empty_b = new_empty_tab(&mut r);
    let saved_before = ok(r.rt.saved_settings()).open_tabs;
    assert!(saved_before.contains(&empty_a) && saved_before.contains(&empty_b));
    r.typed("/quit");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(r.app.quit);
    r.app.shutdown();
    let deleted = r.rt.deleted();
    assert!(
        deleted.contains(&empty_a) && deleted.contains(&empty_b),
        "{deleted:?}"
    );
    assert!(!deleted.contains(&first));
    let s = ok(r.rt.saved_settings());
    assert_eq!(s.open_tabs, std::slice::from_ref(&first));
    assert_eq!(s.active_tab, Some(first));
}

#[test]
fn quitting_with_only_an_unused_tab_saves_no_tabs() {
    let mut r = rig();
    let only = id_of(&r, 0);
    r.app.shutdown();
    assert_eq!(r.rt.deleted(), [only]);
    let s = ok(r.rt.saved_settings());
    assert!(s.open_tabs.is_empty() && s.active_tab.is_none(), "{s:?}");
}

#[test]
fn a_failed_delete_is_one_error_toast_and_the_tab_still_closes() {
    let mut r = rig();
    r.push(finished_turn("keep me"));
    let id = new_empty_tab(&mut r);
    r.rt.fail_delete(Some(SessionError::Store(StoreError::Io {
        path: "/x/sessions".into(),
        message: "read-only file system".into(),
    })));
    r.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert!(r.app.tab_of(&id).is_none(), "closed regardless");
    let t = toast(&r);
    assert!(
        t.contains("could not delete the empty conversation") && t.contains("read-only"),
        "{t}"
    );
    assert_eq!(
        r.app.toast.as_ref().map(|t| t.kind),
        Some(crate::app::ToastKind::Error)
    );
}

#[test]
fn a_lone_unused_tab_still_cannot_be_closed() {
    let mut r = rig();
    let only = id_of(&r, 0);
    r.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(toast(&r), "this is the only tab");
    assert_eq!(id_of(&r, 0), only);
    assert!(r.rt.deleted().is_empty());
    // Closing the last used tab replaces it with a New chat, and keeps the used one.
    r.push(finished_turn("used"));
    r.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(r.app.sessions.len(), 1);
    assert!(r.rt.deleted().is_empty(), "{:?}", r.rt.deleted());
    assert_eq!(r.app.focus, Focus::Input);
}
