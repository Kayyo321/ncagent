/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Colours in one place (D47, D64). White on black by default, with background-tone steps
//! instead of borders. A theme is data: `theme_t` is a set of named colours, the built-in themes
//! are in `builtin_themes`, and a custom theme sets some of the names (see `theme_set`).
//! When the terminal lacks 24-bit colour every `Rgb` is mapped to the 256-colour palette at the
//! end of each frame, so the rest of the crate always speaks `Rgb`, whatever the theme.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// How colours reach the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    Indexed256,
}

/// `COLORTERM` decides: `truecolor` or `24bit` keeps `Rgb`, anything else maps to 256 colours.
pub fn color_mode_from_env(value: Option<&str>) -> ColorMode {
    match value.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if v == "truecolor" || v == "24bit" => ColorMode::TrueColor,
        _ => ColorMode::Indexed256,
    }
}

pub fn detect_color_mode() -> ColorMode {
    let value = std::env::var("COLORTERM").ok();
    color_mode_from_env(value.as_deref())
}

const CubeLevels: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn nearest_cube_index(v: u8) -> usize {
    let mut best = 0usize;
    let mut best_d = u32::MAX;
    for (i, level) in CubeLevels.iter().enumerate() {
        let d = (i32::from(*level) - i32::from(v)).unsigned_abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

fn dist2(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let dr = (i32::from(a.0) - i32::from(b.0)).unsigned_abs();
    let dg = (i32::from(a.1) - i32::from(b.1)).unsigned_abs();
    let db = (i32::from(a.2) - i32::from(b.2)).unsigned_abs();
    dr * dr + dg * dg + db * db
}

/// The xterm 256-colour index nearest to an RGB colour: the 6x6x6 cube (16..=231) or the grey
/// ramp (232..=255), whichever is closer.
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (
        nearest_cube_index(r),
        nearest_cube_index(g),
        nearest_cube_index(b),
    );
    let cube_rgb = (CubeLevels[ri], CubeLevels[gi], CubeLevels[bi]);
    // Indices are at most 5, so the sum stays below 232.
    let cube_index = 16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8;
    let average = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
    let grey_step = (average.saturating_sub(8) / 10).min(23) as u8;
    let grey_value = 8 + 10 * grey_step;
    let grey_rgb = (grey_value, grey_value, grey_value);
    if dist2((r, g, b), grey_rgb) < dist2((r, g, b), cube_rgb) {
        232 + grey_step
    } else {
        cube_index
    }
}

pub fn quantize(color: Color, mode: ColorMode) -> Color {
    match (mode, color) {
        (ColorMode::Indexed256, Color::Rgb(r, g, b)) => Color::Indexed(rgb_to_256(r, g, b)),
        (_, other) => other,
    }
}

/// Map every cell of a finished frame to the colour mode.
pub fn quantize_buffer(buf: &mut Buffer, mode: ColorMode) {
    if mode == ColorMode::TrueColor {
        return;
    }
    for cell in buf.content.iter_mut() {
        cell.fg = quantize(cell.fg, mode);
        cell.bg = quantize(cell.bg, mode);
    }
}

/// Linear blend of two RGB colours. `t` is clamped to 0..=1. Non-RGB colours return `a`.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let f = |x: u8, y: u8| -> u8 {
                let v = f32::from(x) + (f32::from(y) - f32::from(x)) * t;
                v.round().clamp(0.0, 255.0) as u8
            };
            Color::Rgb(f(ar, br), f(ag, bg), f(ab, bb))
        }
        _ => a,
    }
}

/// Declares the palette type with its colour names, so the list of names, `get` and `set`
/// cannot drift from the fields (D64).
macro_rules! palette_fields {
    ($($(#[$doc:meta])* $name:ident,)*) => {
        /// The palette. Panels are a few steps above the background; speakers own a rail colour.
        /// A theme is data: every field is a named colour that a custom theme may set (D64).
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct theme_t {
            $($(#[$doc])* pub $name: Color,)*
        }

        /// Every colour name of a theme, in field order. These are the keys a custom theme's
        /// `colors` map may use (D64).
        pub const ColorNames: &[&str] = &[$(stringify!($name)),*];

        impl theme_t {
            /// The colour called `name`, or `None` for a name that is not a theme colour.
            pub fn get(&self, name: &str) -> Option<Color> {
                match name {
                    $(stringify!($name) => Some(self.$name),)*
                    _ => None,
                }
            }

            /// Set the colour called `name`. False, and nothing changes, for an unknown name.
            pub fn set(&mut self, name: &str, color: Color) -> bool {
                match name {
                    $(stringify!($name) => {
                        self.$name = color;
                        true
                    })*
                    _ => false,
                }
            }
        }
    };
}

palette_fields! {
    bg,
    /// Sidebar column.
    side,
    /// The user's message panel.
    panel,
    /// Composer and widgets docked at the bottom.
    dock,
    /// Insets inside panels: code blocks, summaries, gauges.
    inset,
    hover,
    select,
    /// Rows in a selection of several (D60): a cool tone, apart from hover and the open row.
    marked,
    fg,
    fg_dim,
    fg_faint,
    user,
    agent,
    ice,
    ice_deep,
    tool,
    tool_dim,
    approval,
    question,
    error,
    success,
    composer_rail,
    code,
    link,
    /// The top row of the title card's logo; the rows below blend to `logo_lo`.
    logo_hi,
    logo_lo,
    /// The brightest glyphs of the generation effect, before they settle into `ice`.
    glint,
}

/// The colours a theme is built from. Every other colour is derived from these when a custom
/// theme changes one of its sources and does not set the derived colour itself (D64).
pub const BaseColors: &[&str] = &[
    "bg",
    "fg",
    "user",
    "agent",
    "tool",
    "approval",
    "question",
    "error",
    "success",
    "composer_rail",
    "code",
];

/// The base colours a derived colour is made from. Empty for a base colour.
pub fn derived_from(name: &str) -> &'static [&'static str] {
    match name {
        "side" | "panel" | "dock" | "inset" | "hover" | "fg_dim" | "fg_faint" => &["bg", "fg"],
        "select" => &["bg", "fg", "agent"],
        "marked" | "ice_deep" => &["bg", "agent"],
        "ice" | "glint" => &["agent", "fg"],
        "tool_dim" => &["bg", "tool"],
        "link" => &["agent"],
        "logo_hi" => &["user"],
        "logo_lo" => &["agent", "bg"],
        _ => &[],
    }
}

/// The value of a derived colour, from the base colours of `t`. `None` for a base colour or an
/// unknown name. The steps are fractions of the way from the background to the text, so a light
/// theme gets darker panels and a dark theme lighter ones.
pub fn derive(name: &str, t: &theme_t) -> Option<Color> {
    let toward_fg = |f: f32| mix(t.bg, t.fg, f);
    let ice = mix(t.agent, t.fg, 0.5);
    let color = match name {
        "side" => toward_fg(0.05),
        "panel" => toward_fg(0.095),
        "dock" => toward_fg(0.08),
        "inset" => toward_fg(0.13),
        "hover" => toward_fg(0.15),
        "select" => mix(toward_fg(0.16), t.agent, 0.08),
        "marked" => mix(t.bg, t.agent, 0.2),
        "fg_dim" => toward_fg(0.6),
        "fg_faint" => toward_fg(0.36),
        "ice" => ice,
        "ice_deep" => mix(t.bg, t.agent, 0.45),
        "tool_dim" => mix(t.bg, t.tool, 0.55),
        "link" => t.agent,
        "logo_hi" => t.user,
        "logo_lo" => mix(t.agent, t.bg, 0.1),
        "glint" => mix(ice, t.fg, 0.7),
        _ => return None,
    };
    Some(color)
}

impl theme_t {
    /// The default theme, `ncagent-dark` (D47).
    pub fn dark() -> Self {
        crate::builtin_themes::ncagent_dark()
    }

    /// Every colour name, in field order (D64).
    pub fn color_names() -> &'static [&'static str] {
        ColorNames
    }
}

/// `#rrggbb` to a colour. `None` for any other text.
pub fn parse_hex(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
    };
    Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?))
}

/// Relative luminance of an RGB colour (WCAG 2.x). Non-RGB colours count as black.
pub fn luminance(color: Color) -> f64 {
    let Color::Rgb(r, g, b) = color else {
        return 0.0;
    };
    let channel = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The WCAG contrast ratio of two colours, from 1 to 21.
pub fn contrast_ratio(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_detection() {
        assert_eq!(color_mode_from_env(Some("truecolor")), ColorMode::TrueColor);
        assert_eq!(color_mode_from_env(Some("24bit")), ColorMode::TrueColor);
        assert_eq!(
            color_mode_from_env(Some(" TrueColor ")),
            ColorMode::TrueColor
        );
        assert_eq!(color_mode_from_env(Some("")), ColorMode::Indexed256);
        assert_eq!(color_mode_from_env(Some("yes")), ColorMode::Indexed256);
        assert_eq!(color_mode_from_env(None), ColorMode::Indexed256);
    }

    #[test]
    fn cube_corners_and_greys() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(rgb_to_256(255, 0, 0), 196);
        assert_eq!(rgb_to_256(0, 255, 0), 46);
        assert_eq!(rgb_to_256(0, 0, 255), 21);
        // A mid grey is closer to the grey ramp than to the cube.
        assert_eq!(rgb_to_256(128, 128, 128), 244);
        assert_eq!(rgb_to_256(18, 18, 18), 233);
        // Near-black panel tones keep their order, so tone steps survive quantisation.
        let a = rgb_to_256(12, 12, 15);
        let b = rgb_to_256(30, 30, 37);
        assert!(a >= 232 && b >= 232 && a < b);
    }

    #[test]
    fn every_rgb_maps_into_the_extended_range() {
        for r in (0..=255u16).step_by(17) {
            for g in (0..=255u16).step_by(17) {
                for b in (0..=255u16).step_by(17) {
                    let i = rgb_to_256(r as u8, g as u8, b as u8);
                    assert!(i >= 16, "{r} {g} {b} -> {i}");
                }
            }
        }
    }

    #[test]
    fn quantize_only_touches_rgb_in_256_mode() {
        let c = Color::Rgb(255, 0, 0);
        assert_eq!(quantize(c, ColorMode::TrueColor), c);
        assert_eq!(quantize(c, ColorMode::Indexed256), Color::Indexed(196));
        assert_eq!(quantize(Color::Reset, ColorMode::Indexed256), Color::Reset);
    }

    #[test]
    fn mix_clamps() {
        let a = Color::Rgb(0, 0, 0);
        let b = Color::Rgb(200, 100, 50);
        assert_eq!(mix(a, b, 0.5), Color::Rgb(100, 50, 25));
        assert_eq!(mix(a, b, 2.0), b);
        assert_eq!(mix(a, b, f32::NAN), a);
    }
}
