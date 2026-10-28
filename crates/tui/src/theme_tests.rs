/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Theme tests (D64): the default palette is the one of D47, colour names and the derivation
//! rules, building custom themes and each refusal, legible contrast in every built-in theme,
//! and every built-in drawing the key screens in 24-bit and 256 colours.
//!
//! Contrast is the WCAG 2.x ratio (`theme::contrast_ratio`). The minimums, by role:
//!
//! | role | pairs | minimum |
//! |---|---|---|
//! | body text | `fg` on every panel tone, `user` on `panel` | 4.5 |
//! | dim text | `fg_dim` on `bg`, `side`, `panel`, `dock`, `inset` | 3.0 |
//! | faint text (hints, decorations) | `fg_faint` on `bg`, `side`, `dock` | 1.9 |
//! | accents (speaker colours, status) | each accent on `bg`, and the main ones on `side` | 3.0 |
//! | chip text | `bg` on `approval`, `error`, `agent`, `success` | 3.0 |
//! | the logo and the effect | `logo_hi`, `logo_lo`, `glint` on `bg` | 3.0 |
//!
//! 4.5 is WCAG AA for body text. Dim text and accents are short labels and large rails, so the
//! AA large-text minimum of 3.0 applies. Faint text is for hints that repeat what is said
//! elsewhere; 1.9 is the lowest the default theme reaches today (it must not change, D64), and
//! every other theme meets it.
//!
//! These minimums hold for `ncagent-dark`, `light`, `navy`, `astral`, `ember` and `mono`.
//! `high_contrast` must meet higher ones: 7.0 (WCAG AAA) for body text, dim text and the logo,
//! and 4.5 for faint text, accents and chip text, so none of its dim tones falls under AA.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyModifiers, MouseButton};
use harness::settings::{DefaultTheme, custom_theme_t, settings_t, valid_theme_id};
use ratatui::style::Color;

use crate::app_tests::{rig_t, start_turn};
use crate::builtin_themes::BuiltinThemes;
use crate::folder_tests::{ok, saved, session_index};
use crate::theme::{BaseColors, ColorNames, contrast_ratio, derive, derived_from, theme_t};
use crate::theme_set::{MaxThemeDepth, ThemeErrorKind, theme_set_t};
use crate::view_model::tests::full_session;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The palette of D47 as it was before themes were data, written out so a change shows.
fn palette_before_themes() -> Vec<(&'static str, Color)> {
    vec![
        ("bg", rgb(0, 0, 0)),
        ("side", rgb(12, 12, 15)),
        ("panel", rgb(22, 22, 27)),
        ("dock", rgb(19, 19, 24)),
        ("inset", rgb(30, 30, 37)),
        ("hover", rgb(34, 34, 42)),
        ("select", rgb(40, 40, 52)),
        ("marked", rgb(20, 36, 58)),
        ("fg", rgb(236, 236, 240)),
        ("fg_dim", rgb(146, 146, 158)),
        ("fg_faint", rgb(84, 84, 96)),
        ("user", rgb(250, 250, 252)),
        ("agent", rgb(142, 202, 255)),
        ("ice", rgb(196, 228, 255)),
        ("ice_deep", rgb(58, 92, 128)),
        ("tool", rgb(226, 118, 236)),
        ("tool_dim", rgb(122, 70, 132)),
        ("approval", rgb(255, 182, 72)),
        ("question", rgb(252, 212, 112)),
        ("error", rgb(255, 94, 98)),
        ("success", rgb(112, 228, 152)),
        ("composer_rail", rgb(176, 148, 255)),
        ("code", rgb(236, 206, 164)),
        ("link", rgb(142, 202, 255)),
        // These were literals in the title card and the effect.
        ("logo_hi", rgb(255, 255, 255)),
        ("logo_lo", rgb(116, 176, 238)),
        ("glint", rgb(246, 249, 255)),
    ]
}

fn custom(id: &str, extends: &str, colors: &[(&str, &str)]) -> custom_theme_t {
    custom_theme_t {
        id: id.to_string(),
        name: format!("Theme {id}"),
        extends: extends.to_string(),
        colors: colors
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[test]
fn the_default_theme_is_the_palette_of_d47_exactly() {
    let dark = theme_t::dark();
    let before = palette_before_themes();
    assert_eq!(before.len(), ColorNames.len(), "every colour is pinned");
    for (name, color) in before {
        assert_eq!(dark.get(name), Some(color), "{name}");
    }
    assert_eq!(BuiltinThemes[0].id, DefaultTheme);
    assert_eq!(ok(theme_set_t::default().resolve(DefaultTheme)), dark);
}

#[test]
fn colour_names_get_and_set_every_field_and_nothing_else() {
    let mut seen = std::collections::BTreeSet::new();
    for name in theme_t::color_names() {
        assert!(seen.insert(*name), "{name} twice");
        let mut t = theme_t::dark();
        assert!(t.get(name).is_some(), "{name}");
        assert!(t.set(name, rgb(1, 2, 3)));
        assert_eq!(t.get(name), Some(rgb(1, 2, 3)));
    }
    let mut t = theme_t::dark();
    assert_eq!(t.get("nope"), None);
    assert!(!t.set("nope", rgb(1, 2, 3)));
    assert_eq!(t, theme_t::dark());
    // Every name is a base colour or derived from base colours only.
    for name in ColorNames {
        if BaseColors.contains(name) {
            assert!(derived_from(name).is_empty(), "{name}");
            assert_eq!(derive(name, &t), None, "{name}");
        } else {
            assert!(!derived_from(name).is_empty(), "{name} has no sources");
            assert!(derive(name, &t).is_some(), "{name}");
            for source in derived_from(name) {
                assert!(BaseColors.contains(source), "{name} from {source}");
            }
        }
    }
}

/// Every built-in id saves as the theme setting through the harness's own rules, so choosing
/// any of them is never refused (the rule allows `_` since the owner's change).
#[test]
fn every_built_in_id_saves_through_the_harness_rules() {
    let store = harness::settings::memory_settings_store_t::new();
    for b in BuiltinThemes {
        assert!(valid_theme_id(b.id), "{}", b.id);
        let s = settings_t {
            theme: b.id.to_string(),
            ..settings_t::default()
        };
        ok(harness::settings::settings_store_t::save(&store, &s));
    }
    let mut r = rig_t::new(130, 40, false);
    assert!(r.app.apply_theme("high_contrast"));
    assert_eq!(ok(r.rt.saved_settings()).theme, "high_contrast");
    r.frame();
    assert_eq!(r.app.theme, crate::builtin_themes::high_contrast());
}

#[test]
fn the_built_in_registry_has_unique_ids() {
    let mut ids = std::collections::BTreeSet::new();
    for b in BuiltinThemes {
        assert!(ids.insert(b.id), "{} twice", b.id);
        assert!(!b.name.is_empty());
        assert!(theme_set_t::is_builtin(b.id));
    }
    let set = theme_set_t::default();
    assert_eq!(set.find("ASTRAL").as_deref(), Some("astral"));
    assert_eq!(set.find("light").as_deref(), Some("light"));
    assert_eq!(set.find("Dark").as_deref(), Some("ncagent-dark"));
    assert_eq!(set.find("high contrast").as_deref(), Some("high_contrast"));
    assert_eq!(
        BuiltinThemes.map(|b| (b.id, b.name)),
        [
            ("ncagent-dark", "Dark"),
            ("light", "Light"),
            ("navy", "Navy"),
            ("astral", "Astral"),
            ("ember", "Ember"),
            ("mono", "Mono"),
            ("high_contrast", "High contrast"),
        ]
    );
    assert_eq!(set.find("nothing"), None);
}

/// The worst ratio of `pairs` (foreground, background) in `t`, and the pair.
fn worst(t: &theme_t, pairs: &[(&str, &str)]) -> (f64, String) {
    pairs
        .iter()
        .map(|(a, b)| {
            let ratio = contrast_ratio(
                t.get(a).unwrap_or(Color::Reset),
                t.get(b).unwrap_or(Color::Reset),
            );
            (ratio, format!("{a} on {b}"))
        })
        .fold(
            (f64::MAX, String::new()),
            |acc, x| if x.0 < acc.0 { x } else { acc },
        )
}

#[test]
fn every_built_in_theme_is_legible() {
    let body: Vec<(&str, &str)> = [
        "bg", "side", "panel", "dock", "inset", "hover", "select", "marked",
    ]
    .iter()
    .map(|b| ("fg", *b))
    .chain(std::iter::once(("user", "panel")))
    .collect();
    let dim: Vec<(&str, &str)> = ["bg", "side", "panel", "dock", "inset"]
        .iter()
        .map(|b| ("fg_dim", *b))
        .collect();
    let faint: Vec<(&str, &str)> = ["bg", "side", "dock"]
        .iter()
        .map(|b| ("fg_faint", *b))
        .collect();
    let accents: Vec<(&str, &str)> = [
        "agent",
        "tool",
        "approval",
        "question",
        "error",
        "success",
        "code",
        "link",
        "ice",
        "composer_rail",
    ]
    .iter()
    .map(|a| (*a, "bg"))
    .chain([
        ("agent", "side"),
        ("tool", "side"),
        ("code", "inset"),
        ("ice", "select"),
    ])
    .collect();
    let chips = [
        ("bg", "approval"),
        ("bg", "error"),
        ("bg", "agent"),
        ("bg", "success"),
    ];
    let logo = [("logo_hi", "bg"), ("logo_lo", "bg"), ("glint", "bg")];
    for b in BuiltinThemes {
        let t = (b.palette)();
        let strict = b.id == "high_contrast";
        for (role, pairs, min, strict_min) in [
            ("body", &body[..], 4.5, 7.0),
            ("dim", &dim[..], 3.0, 7.0),
            ("faint", &faint[..], 1.9, 4.5),
            ("accent", &accents[..], 3.0, 4.5),
            ("chip", &chips[..], 3.0, 4.5),
            ("logo", &logo[..], 3.0, 7.0),
        ] {
            let want = if strict { strict_min } else { min };
            let (ratio, pair) = worst(&t, pairs);
            assert!(
                ratio >= want,
                "{}: {role} {pair} is {ratio:.2}, under {want}",
                b.id
            );
        }
        // Tone steps stay visible: a panel is not the background.
        for tone in ["side", "panel", "inset", "select", "hover"] {
            assert_ne!(t.get(tone), t.get("bg"), "{} {tone}", b.id);
        }
    }
}

#[test]
fn a_custom_theme_overrides_names_and_rederives_what_it_did_not_set() {
    let set = theme_set_t::default();
    let light = custom(
        "bright",
        "ncagent-dark",
        &[("bg", "#ffffff"), ("fg", "#101010")],
    );
    let t = ok(theme_t::from_custom(&set, &light));
    assert_eq!(t.bg, rgb(255, 255, 255));
    assert_eq!(t.fg, rgb(16, 16, 16));
    // Panels and dim text follow the new bg and fg instead of keeping the dark tones.
    assert_eq!(Some(t.panel), derive("panel", &t));
    assert_eq!(Some(t.fg_dim), derive("fg_dim", &t));
    assert!(contrast_ratio(t.fg, t.panel) > 10.0);
    // A colour whose sources did not change keeps the base theme's value.
    assert_eq!(t.agent, theme_t::dark().agent);
    assert_eq!(t.tool, theme_t::dark().tool);
    assert_eq!(
        Some(t.tool_dim),
        derive("tool_dim", &t),
        "tool_dim follows bg"
    );
    // A derived colour the theme sets itself wins.
    let own = custom(
        "own",
        "ncagent-dark",
        &[("bg", "#ffffff"), ("panel", "#ff0000")],
    );
    let t = ok(theme_t::from_custom(&set, &own));
    assert_eq!(t.panel, rgb(255, 0, 0));
    assert_eq!(Some(t.side), derive("side", &t));
    // Extending a built-in starts from it.
    let warm = custom("warm", "astral", &[("tool", "#00ff00")]);
    let t = ok(theme_t::from_custom(&set, &warm));
    assert_eq!(t.bg, crate::builtin_themes::astral().bg);
    assert_eq!(t.tool, rgb(0, 255, 0));
    assert_eq!(Some(t.tool_dim), derive("tool_dim", &t));
}

#[test]
fn custom_themes_chain_through_each_other() {
    let base = custom("base", "light", &[("agent", "#123456")]);
    let top = custom("top", "base", &[("error", "#abcdef")]);
    let set = theme_set_t::new(vec![base.clone(), top.clone()]);
    let t = ok(set.resolve("top"));
    assert_eq!(t.bg, crate::builtin_themes::light().bg);
    assert_eq!(t.agent, rgb(0x12, 0x34, 0x56));
    assert_eq!(t.error, rgb(0xab, 0xcd, 0xef));
    let entries = set.entries();
    assert_eq!(entries.len(), BuiltinThemes.len() + 2);
    assert!(entries.iter().all(|e| e.palette.is_ok()));
    assert_eq!(set.name_of("top"), "Theme top");
}

#[test]
fn each_broken_theme_is_refused_with_a_named_error() {
    let set = theme_set_t::default();
    let unknown = custom("u", "ncagent-dark", &[("backgroud", "#000000")]);
    let err = theme_t::from_custom(&set, &unknown).err();
    assert!(
        matches!(&err, Some(e) if e.kind == ThemeErrorKind::UnknownColor { name: "backgroud".into() })
    );
    let text = err.map(|e| e.to_string()).unwrap_or_default();
    assert!(
        text.contains("backgroud") && text.contains("composer_rail"),
        "{text}"
    );

    let bad = custom("b", "ncagent-dark", &[("bg", "black")]);
    assert!(matches!(
        theme_t::from_custom(&set, &bad).map_err(|e| e.kind),
        Err(ThemeErrorKind::BadColor { .. })
    ));

    let orphan = custom("o", "gone", &[]);
    assert_eq!(
        theme_t::from_custom(&set, &orphan).map_err(|e| e.kind),
        Err(ThemeErrorKind::UnknownBase { id: "gone".into() })
    );

    let itself = custom("me", "me", &[]);
    assert!(matches!(
        theme_t::from_custom(&set, &itself).map_err(|e| e.kind),
        Err(ThemeErrorKind::Cycle { .. })
    ));
    let a = custom("a", "b", &[]);
    let b = custom("b", "a", &[]);
    let looped = theme_set_t::new(vec![a.clone(), b]);
    let err = theme_t::from_custom(&looped, &a).err();
    assert!(
        matches!(&err, Some(e) if e.kind == ThemeErrorKind::Cycle { chain: vec!["a".into(), "b".into(), "a".into()] }),
        "{err:?}"
    );

    let chain: Vec<custom_theme_t> = (0..MaxThemeDepth)
        .map(|i| {
            let base = if i + 1 == MaxThemeDepth {
                "ncagent-dark".to_string()
            } else {
                format!("t{}", i + 1)
            };
            custom(&format!("t{i}"), &base, &[])
        })
        .collect();
    let deep = theme_set_t::new(chain);
    assert!(matches!(
        deep.resolve("t0").map_err(|e| e.kind),
        Err(ThemeErrorKind::TooDeep { .. })
    ));
    assert!(deep.resolve("t1").is_ok(), "one less is allowed");

    let builtin = custom("light", "ncagent-dark", &[]);
    assert_eq!(
        theme_t::from_custom(&set, &builtin).map_err(|e| e.kind),
        Err(ThemeErrorKind::BuiltinId { id: "light".into() })
    );
    assert!(matches!(
        set.resolve("nope").map_err(|e| e.kind),
        Err(ThemeErrorKind::NotFound { .. })
    ));
}

/// A rig in theme `id`, with a finished session (tool, Markdown) and a second tab.
fn themed_rig(id: &str, width: u16) -> rig_t {
    let id = id.to_string();
    let mut rig = rig_t::with(width, 50, false, true, move |rt| {
        ok(rt.seed_settings(&settings_t {
            theme: id,
            ..settings_t::default()
        }));
        rt.store("saved-1", saved("saved-1", "an older chat", 9_000_000));
    });
    rig.push(full_session().into_iter().map(|s| s.event).collect());
    rig
}

#[test]
fn every_built_in_theme_draws_the_key_screens() {
    for b in BuiltinThemes {
        for width in [130u16, 80, 40] {
            let mut rig = themed_rig(b.id, width);
            assert_eq!(rig.app.theme, (b.palette)(), "{} in use", b.id);
            let screen = rig.screen();
            assert!(
                screen.contains("replace_lines"),
                "{} {width}\n{screen}",
                b.id
            );
            // The background is the theme's in the margin of the transcript.
            let buf = rig.term.backend().buffer();
            assert_eq!(
                buf.cell((0, 2)).map(|c| c.bg),
                Some((b.palette)().bg),
                "{} {width}",
                b.id
            );
            // A second tab, the menu of a sidebar row and the welcome card of a new chat.
            rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
            if width >= 100 {
                let i = session_index(&rig, "saved-1");
                rig.right_click_row(i);
                assert!(rig.app.menu.is_some(), "{} menu", b.id);
                rig.key(KeyCode::Esc, KeyModifiers::NONE);
            }
            rig.push(start_turn("hello"));
        }
    }
}

#[test]
fn every_theme_goes_through_the_256_colour_mapping() {
    for b in BuiltinThemes {
        let id = b.id.to_string();
        let mut rig = rig_t::with(130, 40, false, true, move |rt| {
            ok(rt.seed_settings(&settings_t {
                theme: id,
                ..settings_t::default()
            }));
        });
        rig.app.options.color = crate::theme::ColorMode::Indexed256;
        rig.push(full_session().into_iter().map(|s| s.event).collect());
        let buf = rig.term.backend().buffer();
        let rgb_left = buf
            .content
            .iter()
            .filter(|c| matches!(c.fg, Color::Rgb(..)) || matches!(c.bg, Color::Rgb(..)))
            .count();
        assert_eq!(rgb_left, 0, "{}", b.id);
    }
}

#[test]
fn the_logo_takes_the_themes_colours() {
    let rig = rig_t::with(130, 40, false, true, |rt| {
        ok(rt.seed_settings(&settings_t {
            theme: "light".to_string(),
            ..settings_t::default()
        }));
    });
    let light = crate::builtin_themes::light();
    let buf = rig.term.backend().buffer();
    let logo_cells = buf
        .content
        .iter()
        .filter(|c| c.symbol() != " " && c.fg == light.logo_hi)
        .count();
    assert!(
        logo_cells > 10,
        "the top logo row is drawn in logo_hi\n{}",
        rig.screen()
    );
    let white = buf
        .content
        .iter()
        .filter(|c| c.fg == rgb(255, 255, 255))
        .count();
    assert_eq!(white, 0, "nothing is drawn white on light");
}

#[test]
fn a_mouse_click_on_a_tab_still_works_in_a_light_theme() {
    let mut rig = themed_rig("light", 130);
    rig.key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.active, 1);
    let r = rig
        .app
        .hits
        .find_target(&crate::input::Target::Tab(0))
        .unwrap_or_else(|| panic!("tab 0\n{}", rig.screen()));
    rig.click_with(MouseButton::Left, r.x + 2, r.y, KeyModifiers::NONE);
    assert_eq!(rig.app.active, 0);
}
