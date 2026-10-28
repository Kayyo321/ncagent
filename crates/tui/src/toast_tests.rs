/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Toast motion tests: the reveal and the dissolve at chosen clock times on ratatui's
//! `TestBackend`, replacement, animation off, a resize or a clock jump mid-effect, and a click
//! that dismisses.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::app::ToastKind;
use crate::app_tests::rig_t;
use crate::effect::is_effect_glyph;
use crate::input::Action;
use crate::toast::{MaxToastFxMs, MinToastFxMs, ToastMs, effect_ms};

const Text: &str = "this is the only tab";

/// One cell: symbol, colours and modifiers.
type cell_t = (String, Color, Color, Modifier);

fn cells(r: &rig_t, rect: Rect) -> Vec<cell_t> {
    let buf = r.term.backend().buffer();
    (rect.x..rect.right())
        .filter_map(|x| buf.cell((x, rect.y)))
        .map(|c| (c.symbol().to_string(), c.fg, c.bg, c.modifier))
        .collect()
}

fn text(cells: &[cell_t]) -> String {
    cells.iter().map(|c| c.0.as_str()).collect()
}

/// Where the static card of `Text` is drawn in a 120-column rig: the transcript's top row,
/// right-aligned two columns from its edge.
fn card_rect(r: &rig_t) -> Rect {
    let area = r.app.layout.transcript;
    let w = (Text.chars().count() + 3) as u16;
    Rect::new(area.right() - w - 2, area.y, w, 1)
}

/// A rig in which `alt+w` on the lone tab says `Text`.
fn say_only_tab(animate: bool) -> rig_t {
    let mut r = rig_t::new(120, 30, animate);
    r.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(r.app.toast.as_ref().map(|t| t.text.as_str()), Some(Text));
    r
}

#[test]
fn the_effect_takes_three_to_four_hundred_fifty_ms_by_length() {
    assert_eq!(effect_ms("ok"), MinToastFxMs);
    assert_eq!(effect_ms(&"x".repeat(400)), MaxToastFxMs);
    let mid = effect_ms(Text);
    assert!((MinToastFxMs..=MaxToastFxMs).contains(&mid));
}

#[test]
fn the_reveal_ends_on_the_static_card_exactly() {
    let still = say_only_tab(false);
    let rect = card_rect(&still);
    let card = cells(&still, rect);
    assert_eq!(text(&card), format!("▌ {Text} "));

    let mut r = say_only_tab(true);
    let t = r.app.toast.clone().unwrap_or_else(|| panic!("toast"));
    assert!(t.in_ms >= MinToastFxMs && t.until == t.shown + t.in_ms + ToastMs);
    let first = cells(&r, rect);
    assert_ne!(first, card, "the first frame is not the card");
    assert!(
        !text(&first).contains("only"),
        "no word shows yet: {}",
        text(&first)
    );
    r.advance(t.in_ms / 2);
    let mid = cells(&r, rect);
    assert_ne!(mid, card);
    assert!(
        text(&mid).chars().any(|c| c != ' ' && is_effect_glyph(c)),
        "effect glyphs mid-way: {}",
        text(&mid)
    );
    r.advance(t.in_ms / 2 + 1);
    assert_eq!(cells(&r, rect), card, "styles included");
    // It stays readable for its whole time.
    r.advance(ToastMs - 10);
    assert_eq!(cells(&r, rect), card);
}

#[test]
fn the_dissolve_runs_back_to_nothing() {
    let still = say_only_tab(false);
    let rect = card_rect(&still);
    let card = cells(&still, rect);
    let mut r = say_only_tab(true);
    let t = r.app.toast.clone().unwrap_or_else(|| panic!("toast"));
    r.advance(t.until - t.shown + t.out_ms / 3);
    let early = cells(&r, rect);
    assert_ne!(early, card);
    assert!(text(&early).chars().any(|c| c != ' ' && is_effect_glyph(c)));
    // The left of the text stays sharp longest: the sweep runs back from the right.
    assert!(text(&early).starts_with("▌ this"), "{}", text(&early));
    r.advance(t.out_ms / 2);
    let late = cells(&r, rect);
    assert!(!text(&late).contains("this"), "{}", text(&late));
    r.advance(t.out_ms);
    assert!(r.app.toast.is_none());
    assert!(!text(&cells(&r, rect)).contains('▌'), "nothing is left");
}

#[test]
fn a_new_toast_replaces_the_old_one_without_two_cards() {
    let mut r = say_only_tab(true);
    r.advance(200);
    r.app.say("second note", ToastKind::Warning);
    r.frame();
    let t = r.app.toast.clone().unwrap_or_else(|| panic!("toast"));
    assert_eq!(t.text, "second note");
    assert_eq!(t.shown, r.clock, "it reveals from now");
    for _ in 0..8 {
        r.advance(60);
        let screen = r.screen();
        let top = screen
            .lines()
            .nth(usize::from(r.app.layout.transcript.y))
            .unwrap_or("");
        assert!(!top.contains("only tab"), "the old card is cut: {top}");
    }
    assert!(r.screen().contains("second note"));
}

#[test]
fn with_animation_off_a_toast_is_instant() {
    let mut r = say_only_tab(false);
    let t = r.app.toast.clone().unwrap_or_else(|| panic!("toast"));
    assert_eq!((t.in_ms, t.out_ms), (0, 0));
    assert!(r.screen().contains(Text), "the first frame is the card");
    r.advance(ToastMs + 1);
    assert!(r.app.toast.is_none());
    assert!(!r.screen().contains(Text));
    // The setting turns it off too, with the command line allowing animation.
    let mut r = rig_t::new(120, 30, true);
    r.app.change_setting(|s| s.animation = false);
    r.key(KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(r.app.toast.as_ref().map(|t| t.in_ms), Some(0));
    assert!(r.screen().contains(Text));
}

#[test]
fn a_resize_or_a_clock_jump_never_leaves_a_half_dissolved_card() {
    let mut r = say_only_tab(true);
    r.advance(100);
    r.resize(90, 24);
    let t = r.app.toast.clone().unwrap_or_else(|| panic!("toast"));
    r.advance(t.in_ms);
    let screen = r.screen();
    assert!(
        screen.contains(&format!("▌ {Text} ")),
        "settled after the resize\n{screen}"
    );
    // Mid-dissolve, a resize, then the end: gone.
    r.advance(t.until - r.clock + t.out_ms / 2);
    r.resize(120, 30);
    r.advance(t.out_ms);
    assert!(r.app.toast.is_none());
    assert!(!r.screen().contains("▌ this"));
    // A jump far past the end.
    let mut r = say_only_tab(true);
    r.advance(1_000_000);
    assert!(r.app.toast.is_none());
    assert!(!r.screen().contains("only tab"));
}

#[test]
fn a_click_on_the_toast_starts_its_dissolve() {
    let mut r = say_only_tab(true);
    let t = r.app.toast.clone().unwrap_or_else(|| panic!("toast"));
    r.advance(t.in_ms + 10);
    let rect = r.rect_of(&Action::DismissToast);
    r.click(rect.x + 3, rect.y);
    let after = r
        .app
        .toast
        .clone()
        .unwrap_or_else(|| panic!("still dissolving"));
    assert_eq!(after.until, r.clock, "the readable time ended at the click");
    assert!(after.until < t.until);
    r.advance(after.out_ms + 1);
    assert!(r.app.toast.is_none());
    // With animation off a click removes it at the next frame.
    let mut r = say_only_tab(false);
    let rect = r.rect_of(&Action::DismissToast);
    r.click(rect.x + 3, rect.y);
    r.advance(1);
    assert!(r.app.toast.is_none());
}
