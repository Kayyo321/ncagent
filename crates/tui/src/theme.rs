/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Colours in one place (D47). White on black, with background-tone steps instead of borders.
//! When the terminal lacks 24-bit colour every `Rgb` is mapped to the 256-colour palette at the
//! end of each frame, so the rest of the crate always speaks `Rgb`.

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

/// The palette. Panels are a few steps above black; speakers own a rail colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct theme_t {
    pub bg: Color,
    /// Sidebar column.
    pub side: Color,
    /// The user's message panel.
    pub panel: Color,
    /// Composer and widgets docked at the bottom.
    pub dock: Color,
    /// Insets inside panels: code blocks, summaries, gauges.
    pub inset: Color,
    pub hover: Color,
    pub select: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_faint: Color,
    pub user: Color,
    pub agent: Color,
    pub ice: Color,
    pub ice_deep: Color,
    pub tool: Color,
    pub tool_dim: Color,
    pub approval: Color,
    pub question: Color,
    pub error: Color,
    pub success: Color,
    pub composer_rail: Color,
    pub code: Color,
    pub link: Color,
}

impl theme_t {
    pub fn dark() -> Self {
        theme_t {
            bg: Color::Rgb(0, 0, 0),
            side: Color::Rgb(12, 12, 15),
            panel: Color::Rgb(22, 22, 27),
            dock: Color::Rgb(19, 19, 24),
            inset: Color::Rgb(30, 30, 37),
            hover: Color::Rgb(34, 34, 42),
            select: Color::Rgb(40, 40, 52),
            fg: Color::Rgb(236, 236, 240),
            fg_dim: Color::Rgb(146, 146, 158),
            fg_faint: Color::Rgb(84, 84, 96),
            user: Color::Rgb(250, 250, 252),
            agent: Color::Rgb(142, 202, 255),
            ice: Color::Rgb(196, 228, 255),
            ice_deep: Color::Rgb(58, 92, 128),
            tool: Color::Rgb(226, 118, 236),
            tool_dim: Color::Rgb(122, 70, 132),
            approval: Color::Rgb(255, 182, 72),
            question: Color::Rgb(252, 212, 112),
            error: Color::Rgb(255, 94, 98),
            success: Color::Rgb(112, 228, 152),
            composer_rail: Color::Rgb(176, 148, 255),
            code: Color::Rgb(236, 206, 164),
            link: Color::Rgb(142, 202, 255),
        }
    }
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
