/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Link tests (D76, D77): the drawn link rectangles, the hyperlink overlay of a real frame, the
//! confirmation and each of its choices, the keyboard's link cursor, the status line, refused
//! addresses and the opener's failure. The opener is the recording fake: nothing is started.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};
use harness::api::SessionError;
use harness::event::{SessionEvent, TurnOutcome};
use harness::settings::{LinkAddresses, SettingsError};
use ratatui::style::Modifier;

use crate::app::EscapeDelayMs;
use crate::app_tests::{rig_t, start_turn};
use crate::folder_tests::ok;
use crate::input::{Action, Focus, Modal};
use crate::link_ui::LinkAction;
use crate::opener::{OpenError, opener_error_t, recording_opener_t};
use crate::settings_page::{PageAction, PageRow};
use crate::settings_spec::{Section, SettingKey};
use crate::view_model::BlockKind;
use crate::view_model::tests::{call_end, call_start};

pub(crate) const Paper: &str = "https://arxiv.org/abs/2610.14162";
pub(crate) const Docs: &str = "https://docs.rs/ratatui";

/// A finished turn whose reply holds `text`.
pub(crate) fn reply(text: &str) -> Vec<SessionEvent> {
    let mut e = start_turn("links please");
    e.push(call_start(1, 0, 0));
    e.push(call_end(1, 0, 0, text, true));
    e.push(SessionEvent::TurnEnd {
        turn: 1,
        outcome: TurnOutcome::Completed,
        calls: 1,
        passes: 3,
    });
    e
}

pub(crate) fn two_links() -> String {
    format!("Read the [NCM paper]({Paper}) and the [ratatui docs]({Docs}) today.")
}

/// A rig with the reply drawn and a recording opener in place.
fn rig(width: u16, height: u16, text: &str) -> (rig_t, Arc<recording_opener_t>) {
    let mut r = rig_t::new(width, height, false);
    let opener = Arc::new(recording_opener_t::default());
    r.app.opener = opener.clone();
    r.push(reply(text));
    (r, opener)
}

/// The text drawn in a rectangle of the last frame.
fn drawn(r: &rig_t, rect: ratatui::layout::Rect) -> String {
    let buf = r.term.backend().buffer();
    (rect.x..rect.right())
        .filter_map(|x| buf.cell((x, rect.y)).map(|c| c.symbol().to_string()))
        .collect()
}

/// Each address with the text of its rectangles, joined.
fn labels(r: &rig_t) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for l in &r.app.links {
        let text = drawn(r, l.rect);
        match out.iter_mut().find(|(u, _)| *u == l.url) {
            Some((_, t)) => t.push_str(&text),
            None => out.push((l.url.clone(), text)),
        }
    }
    out
}

fn reply_block(r: &rig_t) -> u64 {
    r.app
        .active()
        .and_then(|s| {
            s.transcript
                .blocks
                .iter()
                .find(|b| matches!(b.kind, BlockKind::Reply(_)))
                .map(|b| b.id)
        })
        .unwrap_or(u64::MAX)
}

#[test]
fn link_rectangles_cover_exactly_the_label_at_every_width_and_scroll() {
    for width in [130u16, 80, 50, 40] {
        let (mut r, _) = rig(width, 40, &two_links());
        let got = labels(&r);
        let joined = |u: &str| {
            got.iter()
                .find(|(x, _)| x == u)
                .map(|(_, t)| t.replace(' ', ""))
                .unwrap_or_default()
        };
        assert_eq!(joined(Paper), "NCMpaper", "{width}: {got:?}");
        assert_eq!(joined(Docs), "ratatuidocs", "{width}: {got:?}");
        let screen = r.screen();
        assert!(
            !screen.contains("(https://"),
            "{width}: the address is hidden\n{screen}"
        );
        // Scrolled: more turns below, then up a page; the rectangles follow.
        for turn in 2..12u32 {
            r.push(vec![
                SessionEvent::TurnStart { turn },
                SessionEvent::UserMessage {
                    turn,
                    text: format!("filler message {turn} with words to wrap around"),
                    from_queue: false,
                },
                SessionEvent::TurnEnd {
                    turn,
                    outcome: TurnOutcome::Completed,
                    calls: 0,
                    passes: 0,
                },
            ]);
        }
        for _ in 0..12 {
            r.key(KeyCode::PageUp, KeyModifiers::NONE);
            if !r.app.links.is_empty() {
                break;
            }
        }
        for (url, text) in labels(&r) {
            let want = if url == Paper {
                "NCMpaper"
            } else {
                "ratatuidocs"
            };
            assert!(
                want.starts_with(&text.replace(' ', "")) || want.ends_with(&text.replace(' ', "")),
                "{width} scrolled: {url} {text:?}"
            );
        }
    }
}

#[test]
fn the_overlay_of_a_real_frame_writes_both_links_and_skips_covered_ones() {
    let (mut r, _) = rig(130, 40, &two_links());
    let buf = r.term.backend().buffer().clone();
    let bytes = r.app.link_bytes(&buf);
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert_eq!(text.matches("\x1b]8;;https://").count(), 2, "{text:?}");
    assert!(text.contains(&format!("\x1b]8;;{Paper}\x1b\\")));
    assert!(text.contains("NCM"));
    // The same frame again writes nothing.
    assert!(r.app.link_bytes(&buf).is_empty());
    // A link under the confirmation dialog or a toast is skipped.
    r.app.link_overlay.reset();
    let rect = r.app.links.first().map(|l| l.rect).unwrap_or_default();
    r.app.covers.push(rect);
    let bytes = r.app.link_bytes(&buf);
    assert_eq!(
        String::from_utf8_lossy(&bytes)
            .matches("\x1b]8;;https://")
            .count(),
        1
    );
    // A toast's rectangle is a cover.
    r.app
        .say("a toast over the transcript", crate::app::ToastKind::Info);
    r.frame();
    assert!(
        r.app
            .covers
            .iter()
            .any(|c| c.y == r.app.layout.transcript.y)
    );
}

/// Toggles the link setting `key` on the page, which must show its row.
fn toggle_on_page(r: &mut rig_t, key: SettingKey) {
    r.app.open_page(Some(Section::Appearance));
    r.frame();
    let row = r
        .app
        .page_rows()
        .iter()
        .position(|x| *x == PageRow::Setting(key))
        .unwrap_or_else(|| panic!("no row for {key:?}"));
    r.click_action(&Action::Page(PageAction::Toggle(row)));
    r.app.close_page();
    r.frame();
}

#[test]
fn hyperlinks_off_stops_osc8_on_the_next_frame_and_on_restarts_it() {
    let (mut r, _) = rig(130, 40, &two_links());
    let buf = r.term.backend().buffer().clone();
    assert_eq!(
        String::from_utf8_lossy(&r.app.link_bytes(&buf))
            .matches("\x1b]8;;https://")
            .count(),
        2
    );
    // `/links off`: the next frame is a full redraw, and writes no OSC 8 at all.
    r.typed("/links off");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(!r.app.settings.link_hyperlinks);
    assert!(r.app.redraw, "a full redraw clears the old hyperlinks");
    r.frame();
    let buf = r.term.backend().buffer().clone();
    let off = r.app.link_bytes(&buf);
    assert!(off.is_empty(), "{:?}", String::from_utf8_lossy(&off));
    assert!(!String::from_utf8_lossy(&off).contains("\x1b]8;"));
    // The page's switch turns them on again: the links are written over the next frame.
    toggle_on_page(&mut r, SettingKey::LinkHyperlinks);
    assert!(r.app.settings.link_hyperlinks);
    let buf = r.term.backend().buffer().clone();
    let on = String::from_utf8_lossy(&r.app.link_bytes(&buf)).to_string();
    assert_eq!(on.matches("\x1b]8;;https://").count(), 2, "{on:?}");
    assert!(on.contains(&format!("\x1b]8;;{Paper}\x1b\\")));
    // Off again by the page: nothing once more.
    toggle_on_page(&mut r, SettingKey::LinkHyperlinks);
    let buf = r.term.backend().buffer().clone();
    assert!(r.app.link_bytes(&buf).is_empty());
}

#[test]
fn a_hostile_address_is_plain_text_and_never_a_target() {
    let hostile = [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "https://e.example/a\u{7}b",
        "https://exämple.com/",
    ];
    for bad in hostile {
        let text = format!("click [here]({bad}) now");
        let (r, opener) = rig(130, 40, &text);
        assert!(r.app.links.is_empty(), "{bad:?}");
        assert!(
            !r.app
                .hits
                .hits
                .iter()
                .any(|h| matches!(h.target, crate::input::Target::Action(Action::Link(_)))),
            "{bad:?}"
        );
        assert!(r.screen().contains("here"), "the text stays: {bad:?}");
        assert!(opener.opened().is_empty());
    }
    // Even a forged click on one never opens a dialog.
    let (mut r, opener) = rig(130, 40, "nothing");
    r.app.apply(Action::Link(LinkAction::Choose(
        "javascript:alert(1)".into(),
    )));
    assert!(r.app.link_dialog.is_none());
    r.app.apply(Action::Link(LinkAction::Open));
    assert!(opener.opened().is_empty());
}

#[test]
fn a_click_asks_and_each_choice_does_what_it_says() {
    // enter opens, y opens.
    for key in [KeyCode::Enter, KeyCode::Char('y')] {
        let (mut r, opener) = rig(130, 40, &two_links());
        let rect = r.rect_of(&Action::Link(LinkAction::Choose(Paper.into())));
        r.click(rect.x + 1, rect.y);
        assert_eq!(r.app.modal(), Modal::Link);
        assert!(
            opener.opened().is_empty(),
            "nothing opens before the answer"
        );
        r.key(key, KeyModifiers::NONE);
        assert_eq!(opener.opened(), [Paper]);
        assert!(r.app.link_dialog.is_none());
    }
    // c copies through OSC 52.
    let (mut r, opener) = rig(130, 40, &two_links());
    r.click_action(&Action::Link(LinkAction::Choose(Docs.into())));
    r.app.outputs.clear();
    r.ch('c');
    assert!(opener.opened().is_empty());
    let osc = String::from_utf8_lossy(&r.app.outputs.concat()).to_string();
    assert!(osc.starts_with("\x1b]52;c;"), "{osc:?}");
    assert!(crate::folder_tests::toast(&r).contains("link copied: https://docs.rs"));
    // esc and n cancel; so do the chips.
    let (mut r, opener) = rig(130, 40, &two_links());
    r.click_action(&Action::Link(LinkAction::Choose(Paper.into())));
    r.key(KeyCode::Esc, KeyModifiers::NONE);
    r.advance(EscapeDelayMs + 1);
    assert!(r.app.link_dialog.is_none());
    r.click_action(&Action::Link(LinkAction::Choose(Paper.into())));
    r.ch('n');
    assert!(r.app.link_dialog.is_none());
    r.click_action(&Action::Link(LinkAction::Choose(Paper.into())));
    r.click_action(&Action::Link(LinkAction::Cancel));
    assert!(r.app.link_dialog.is_none());
    r.click_action(&Action::Link(LinkAction::Choose(Paper.into())));
    r.click_action(&Action::Link(LinkAction::Open));
    assert_eq!(opener.opened(), [Paper]);
}

#[test]
fn the_dialog_shows_the_whole_address_and_the_host_in_bold_at_every_width() {
    let long = format!("https://arxiv.org/abs/2610.14162?{}", "x=1&".repeat(80));
    for width in [130u16, 80, 50, 40] {
        let text = format!("[paper]({long}) and [NCM paper]({Paper})");
        let (mut r, _) = rig(width, 30, &text);
        r.app.apply(Action::Link(LinkAction::Choose(Paper.into())));
        r.frame();
        let screen = r.screen();
        assert!(screen.contains("open this link"), "{width}\n{screen}");
        assert!(screen.contains("arxiv.org/abs/"), "{width}\n{screen}");
        assert!(screen.contains("enter open"), "{width}");
        // The host is bold.
        let (row, col) = screen
            .lines()
            .enumerate()
            .find_map(|(y, l)| l.find("host  ").map(|x| (y, l[..x].chars().count() + 6)))
            .unwrap_or_else(|| panic!("no host line\n{screen}"));
        let cell = r
            .term
            .backend()
            .buffer()
            .cell((col as u16, row as u16))
            .cloned();
        assert!(
            cell.is_some_and(|c| c.modifier.contains(Modifier::BOLD)),
            "{width}"
        );
        // A long address: both ends show.
        r.app.apply(Action::Link(LinkAction::Choose(long.clone())));
        r.frame();
        let screen = r.screen();
        assert!(screen.contains("https://arxiv.org"), "{width}\n{screen}");
        assert!(screen.contains("x=1&"), "{width}");
    }
}

#[test]
fn the_keys_choose_a_link_copy_it_and_open_it() {
    let (mut r, opener) = rig(130, 40, &two_links());
    let block = reply_block(&r);
    r.app.apply(Action::SetFocus(Focus::Transcript));
    r.app.apply(Action::SelectBlockId(block));
    r.frame();
    r.ch(']');
    assert_eq!(r.app.active_link().as_deref(), Some(Paper));
    let status = r.screen().lines().last().unwrap_or("").to_string();
    assert!(
        status.contains(Paper) && status.contains("enter open"),
        "{status}"
    );
    r.ch(']');
    assert_eq!(r.app.active_link().as_deref(), Some(Docs));
    r.app.outputs.clear();
    r.ch('y');
    assert!(!r.app.outputs.is_empty(), "y copies");
    assert!(opener.opened().is_empty());
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(r.app.link_dialog.as_deref(), Some(Docs));
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(opener.opened(), [Docs]);
    // Past the last link the cursor leaves the links, and enter toggles the block again.
    r.ch(']');
    assert_eq!(r.app.active_link(), None);
    r.ch('[');
    assert_eq!(r.app.active_link().as_deref(), Some(Docs));
}

#[test]
fn hover_shows_the_address_and_lights_the_link() {
    let (mut r, _) = rig(130, 40, &two_links());
    let rect = r.rect_of(&Action::Link(LinkAction::Choose(Paper.into())));
    r.mouse(MouseEventKind::Moved, rect.x + 1, rect.y);
    let status = r.screen().lines().last().unwrap_or("").to_string();
    assert!(status.contains(&format!("↗ {Paper}")), "{status}");
    assert!(status.contains("click to open"));
    let cell = r.term.backend().buffer().cell((rect.x, rect.y)).cloned();
    assert!(cell.is_some_and(|c| c.modifier.contains(Modifier::BOLD | Modifier::UNDERLINED)));
    // A long address is cut in the middle, keeping the host.
    let long = format!("https://example.com/{}/end", "segment/".repeat(40));
    let (mut r, _) = rig(90, 40, &format!("[long one]({long})"));
    let rect = r.rect_of(&Action::Link(LinkAction::Choose(long.clone())));
    r.mouse(MouseEventKind::Moved, rect.x + 1, rect.y);
    let status = r.screen().lines().last().unwrap_or("").to_string();
    assert!(
        status.contains("https://example.com/") && status.contains("…") && status.contains("/end"),
        "{status}"
    );
}

#[test]
fn a_failed_open_is_an_error_toast_and_copies_instead() {
    let (mut r, opener) = rig(130, 40, &two_links());
    opener.fail_with(Some(opener_error_t {
        kind: OpenError::Spawn {
            program: "xdg-open".into(),
            message: "No such file or directory (os error 2)".into(),
        },
    }));
    r.click_action(&Action::Link(LinkAction::Choose(Paper.into())));
    r.app.outputs.clear();
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    let t = crate::folder_tests::toast(&r);
    assert!(
        t.contains("could not open the link") && t.contains("xdg-open"),
        "{t}"
    );
    assert_eq!(
        r.app.toast.as_ref().map(|t| t.kind),
        Some(crate::app::ToastKind::Error)
    );
    assert!(String::from_utf8_lossy(&r.app.outputs.concat()).starts_with("\x1b]52;c;"));
}

#[test]
fn links_inline_shows_every_address_and_hidden_hides_it_again() {
    let (mut r, _) = rig(130, 40, &two_links());
    assert!(!r.screen().contains("(https://"), "hidden by default");
    r.typed("/links inline");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        r.screen().contains(&format!("NCM paper ({Paper})")),
        "{}",
        r.screen()
    );
    assert_eq!(r.app.links.len(), 2, "still links");
    // The settings page changes the same block, which is already drawn.
    toggle_or_pick_addresses(&mut r, 0);
    assert!(!r.screen().contains("(https://"));
    assert_eq!(r.app.links.len(), 2, "still links");
    // A label that names another host keeps the address.
    let (r, _) = rig(130, 40, "[docs.rs](https://evil.example/x)");
    assert!(r.screen().contains("docs.rs (https://evil.example/x)"));
}

/// Picks option `option` of the link addresses row on the page, then shows the transcript.
fn toggle_or_pick_addresses(r: &mut rig_t, option: usize) {
    r.app.open_page(Some(Section::Appearance));
    r.frame();
    let row = r
        .app
        .page_rows()
        .iter()
        .position(|x| *x == PageRow::Setting(SettingKey::LinkAddresses))
        .unwrap_or_else(|| panic!("no link addresses row"));
    r.click_action(&Action::Page(PageAction::Pick(row, option)));
    r.app.close_page();
    r.frame();
}

#[test]
fn links_command_reports_both_settings_and_saves_them_across_a_restart() {
    let (mut r, _) = rig(130, 40, &two_links());
    r.typed("/links");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        crate::folder_tests::toast(&r),
        "links: terminal hyperlinks on, addresses hidden"
    );
    r.typed("/links inline");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    r.typed("/links off");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    let saved = ok(r.rt.saved_settings());
    assert!(!saved.link_hyperlinks);
    assert_eq!(saved.link_addresses, LinkAddresses::Inline);
    assert_eq!(r.app.settings, saved, "the app and the store agree");
    r.typed("/links");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        crate::folder_tests::toast(&r),
        "links: terminal hyperlinks off, addresses inline"
    );
    r.typed("/links sideways");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        crate::folder_tests::toast(&r).contains("usage: /links [osc8|off|inline|hidden]"),
        "{}",
        crate::folder_tests::toast(&r)
    );
    // A new run that loads the saved settings shows the same behaviour with no command.
    let mut again = rig_t::with(130, 40, false, true, |rt| {
        ok(rt.seed_settings(&saved));
    });
    again.push(reply(&two_links()));
    assert_eq!(again.app.settings, saved);
    assert!(again.screen().contains(&format!("NCM paper ({Paper})")));
    assert!(!again.app.settings.link_hyperlinks);
    assert!(
        again
            .app
            .link_bytes(&again.term.backend().buffer().clone())
            .is_empty()
    );
}

#[test]
fn a_refused_link_save_reverts_the_value_and_says_why() {
    let (mut r, _) = rig(130, 40, &two_links());
    r.rt.fail_settings(Some(SessionError::Settings(SettingsError::Io {
        path: "/x/settings.json".into(),
        message: "disk full".into(),
    })));
    r.typed("/links off");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(r.app.settings.link_hyperlinks, "the value stays");
    let t = crate::folder_tests::toast(&r);
    assert!(t.contains("disk full"), "{t}");
    assert!(!t.contains("plain text"), "no success text: {t}");
    r.typed("/links inline");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(r.app.settings.link_addresses, LinkAddresses::Hidden);
    assert!(!r.screen().contains("(https://"));
    r.rt.fail_settings(None);
    r.typed("/links off");
    r.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(!r.app.settings.link_hyperlinks);
    assert!(crate::folder_tests::toast(&r).contains("plain text"));
}
