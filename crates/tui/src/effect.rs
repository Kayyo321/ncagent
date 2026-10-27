/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The NCM generation effect.
//!
//! An NCM reply is not streamed token by token (PROJECT.md, "What NCM changes for an agent",
//! fact 1). Each level pass refines every position at once, coarse to fine, and the text exists
//! only when the last level is done. So the UI never types words out. While a call runs, the
//! reply is a set of textured blocks, one per word as the current level shapes it; a new level
//! re-shapes the blocks behind a sweeping front; the final block of text then dissolves in from
//! the left.
//!
//! A level's draft, when the backend supplies one, is used only for block widths. Its glyphs
//! are never drawn. Everything here is a pure function of the inputs and an explicit clock in
//! milliseconds, so frames are deterministic and testable.

use ratatui::style::{Color, Modifier, Style};

use crate::text::{clusters, line_t, span_t, str_width};
use crate::theme::{mix, theme_t};

/// Duration of the front that re-shapes blocks when a level arrives.
pub const SweepMs: u64 = 420;
/// Bounds of the final dissolve front, scaled by the reply's length.
pub const DissolveMinMs: u64 = 1200;
pub const DissolveMaxMs: u64 = 2000;
/// How long one cell takes to go from block to glyph once the front reaches it.
pub const CellFadeMs: u64 = 380;
/// Largest per-cell delay added to the front, so the edge looks organic.
pub const JitterMs: u64 = 160;
/// Period after which a block cell re-rolls its texture.
pub const ShimmerMs: u64 = 520;
/// Columns of skew per row, so the front runs slightly diagonally.
const RowSkew: usize = 3;
/// Safety caps for hostile drafts.
pub const MaxBlocks: usize = 4000;
pub const MaxEffectRows: usize = 400;
/// Rows of blocks guessed for a level with no draft.
const GuessRows: usize = 3;

/// SplitMix64 finaliser.
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn hash(parts: &[u64]) -> u64 {
    parts
        .iter()
        .fold(0x517C_C1B7_2722_0A95, |acc, p| mix64(acc ^ mix64(*p)))
}

/// A hash as a number in `0.0..1.0`.
pub fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Block widths from a draft: one block per whitespace-separated token, as wide as the token.
pub fn shapes_from_draft(draft: &str, width: usize) -> Vec<usize> {
    let width = width.max(1);
    draft
        .split_whitespace()
        .take(MaxBlocks)
        .map(|token| str_width(token).clamp(1, width))
        .collect()
}

/// Block widths for a level without a draft: `level_len` blocks, capped to what fits a few
/// rows, with seeded widths. Coarse levels get few wide blocks, fine levels many narrow ones.
pub fn shapes_from_len(level_len: u32, level: u32, seed: u64, width: usize) -> Vec<usize> {
    let width = width.max(1);
    let total = width.saturating_mul(GuessRows);
    let count = (level_len as usize).clamp(1, (total / 3).clamp(1, MaxBlocks));
    let average = (total / count).max(2) as f32;
    (0..count)
        .map(|i| {
            let r = unit(hash(&[seed, u64::from(level), i as u64, 0x5A]));
            let w = (average * (0.45 + 1.1 * r)).round() as usize;
            w.clamp(1, width)
        })
        .collect()
}

/// A few short blocks shown after a call starts and before the first level is done.
pub fn placeholder_shapes(seed: u64, width: usize) -> Vec<usize> {
    let width = width.max(1);
    (0..6)
        .map(|i| (3 + (hash(&[seed, i, 0xB1]) % 8) as usize).min(width))
        .collect()
}

/// One cell of a laid-out block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct block_cell_t {
    pub block: usize,
    pub index: usize,
    pub width: usize,
}

/// Blocks wrapped into rows with a one-column gap, as a dense grid of cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct shape_grid_t {
    pub rows: Vec<Vec<Option<block_cell_t>>>,
}

impl shape_grid_t {
    pub fn at(&self, row: usize, col: usize) -> Option<block_cell_t> {
        self.rows
            .get(row)
            .and_then(|r| r.get(col))
            .copied()
            .flatten()
    }

    /// Widths of the blocks as placed, in order (for tests and the sidebar).
    pub fn block_widths(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for row in &self.rows {
            for cell in row.iter().flatten() {
                if cell.index == 0 {
                    out.push(cell.width);
                }
            }
        }
        out
    }
}

pub fn layout_shapes(widths: &[usize], width: usize) -> shape_grid_t {
    let width = width.max(1);
    let mut rows: Vec<Vec<Option<block_cell_t>>> = Vec::new();
    let mut row: Vec<Option<block_cell_t>> = Vec::new();
    for (block, w) in widths.iter().enumerate() {
        let w = (*w).clamp(1, width);
        let gap = usize::from(!row.is_empty());
        if row.len() + gap + w > width && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            if rows.len() >= MaxEffectRows {
                break;
            }
        }
        if !row.is_empty() {
            row.push(None);
        }
        for index in 0..w {
            row.push(Some(block_cell_t {
                block,
                index,
                width: w,
            }));
        }
    }
    if !row.is_empty() && rows.len() < MaxEffectRows {
        rows.push(row);
    }
    shape_grid_t { rows }
}

/// How fine a level is, from 0 (placeholder) to 1 (last level).
fn fineness(level: Option<u32>, level_count: u32) -> f32 {
    match level {
        None => 0.0,
        Some(l) => (l.saturating_add(1) as f32 / level_count.max(1) as f32).clamp(0.0, 1.0),
    }
}

fn braille(h: u64, dots: u32) -> char {
    let dots = dots.clamp(1, 8);
    let mut mask = 0u32;
    let mut k = 0u32;
    while mask.count_ones() < dots && k < 21 {
        mask |= 1 << ((h >> (k * 3)) & 7);
        k += 1;
    }
    let mut fill = 0;
    while mask.count_ones() < dots && fill < 8 {
        mask |= 1 << fill;
        fill += 1;
    }
    char::from_u32(0x2800 + mask).unwrap_or('⣿')
}

/// The texture of one block cell: a solid or shade glyph or a braille cell, and its colour.
/// Coarse levels are dense and bright; finer levels are dottier and cooler.
pub fn texture(
    seed: u64,
    cell: block_cell_t,
    level: Option<u32>,
    level_count: u32,
    clock_ms: u64,
    theme: &theme_t,
) -> (char, Color) {
    let f = fineness(level, level_count);
    let phase = hash(&[seed, cell.block as u64, cell.index as u64, 0x11]) % ShimmerMs;
    let bucket = (clock_ms.saturating_add(phase)) / ShimmerMs;
    let h = hash(&[
        seed,
        cell.block as u64,
        cell.index as u64,
        bucket,
        u64::from(level.unwrap_or(u32::MAX)),
    ]);
    let r = unit(h);
    let edge = cell.width >= 3 && (cell.index == 0 || cell.index + 1 == cell.width);
    let solid = if level.is_none() {
        0.0
    } else {
        0.58 * (1.0 - f) + 0.06
    };
    let shade = if level.is_none() {
        0.0
    } else {
        0.24 * (1.0 - f) + 0.10
    };
    let symbol = if r < solid {
        if edge { '▓' } else { '█' }
    } else if r < solid + shade {
        if (h >> 8) & 1 == 0 { '▓' } else { '▒' }
    } else {
        let dots = if level.is_none() {
            2 + ((h >> 20) % 3) as u32
        } else {
            (2.0 + 6.0 * (1.0 - f) + unit(mix64(h)) * 2.0).round() as u32
        };
        braille(mix64(h ^ 0xD07), dots)
    };
    let white = Color::Rgb(246, 249, 255);
    let base = mix(white, theme.ice, 0.25 + 0.65 * f);
    let dim = 0.82 + 0.18 * unit(mix64(h ^ 0xC0));
    let color = mix(theme.bg, base, dim);
    (symbol, color)
}

/// What the effect needs about one level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct level_input_t {
    pub level: u32,
    pub level_len: u32,
    pub draft: Option<String>,
    /// App-clock time the level arrived. `None` for a replayed event (no transition).
    pub arrived_ms: Option<u64>,
}

/// One attempt of a call, as the effect sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct effect_input_t<'a> {
    pub seed: u64,
    pub level_count: u32,
    pub levels: &'a [level_input_t],
    /// App-clock time the call started (placeholder breathing). `None` when replayed.
    pub started_ms: Option<u64>,
    /// The final layout, when the call ended, and when its end arrived.
    pub end: Option<(&'a [line_t], Option<u64>)>,
}

/// A rendered effect frame. `done` is true when it equals the final lines.
#[derive(Clone, Debug, PartialEq)]
pub struct effect_frame_t {
    pub lines: Vec<line_t>,
    pub done: bool,
}

/// One column of the final layout.
#[derive(Clone, Debug, PartialEq)]
enum final_cell_t {
    Glyph {
        text: String,
        width: usize,
        style: Style,
    },
    /// Covered by the wide glyph to its left.
    Covered,
    Blank {
        style: Style,
    },
}

fn final_grid(lines: &[line_t]) -> Vec<Vec<final_cell_t>> {
    lines
        .iter()
        .map(|line| {
            let mut row = Vec::new();
            for span in &line.spans {
                let style = match (span.style.bg, line.fill) {
                    (None, Some(fill)) => span.style.bg(fill),
                    _ => span.style,
                };
                for c in clusters(&span.text) {
                    if c.width == 0 {
                        continue;
                    }
                    let text = span.text.get(c.start..c.end).unwrap_or(" ").to_string();
                    if text == " " {
                        row.push(final_cell_t::Blank { style });
                    } else {
                        row.push(final_cell_t::Glyph {
                            text,
                            width: c.width,
                            style,
                        });
                        for _ in 1..c.width {
                            row.push(final_cell_t::Covered);
                        }
                    }
                }
            }
            row
        })
        .collect()
}

/// Length of the dissolve front for a reply of `glyphs` visible cells.
pub fn dissolve_ms(glyphs: usize) -> u64 {
    (900 + 3 * glyphs as u64).clamp(DissolveMinMs, DissolveMaxMs)
}

/// The app-clock time at which the dissolve of a call that ended at `end_ms` is complete.
pub fn dissolve_end(end_ms: u64, final_lines: &[line_t]) -> u64 {
    let glyphs = count_glyphs(final_lines);
    end_ms
        .saturating_add(dissolve_ms(glyphs))
        .saturating_add(JitterMs)
        .saturating_add(CellFadeMs)
}

fn count_glyphs(lines: &[line_t]) -> usize {
    lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.text.chars().filter(|c| !c.is_whitespace()).count())
        .sum()
}

/// Position of a cell along the front, in `0.0..=1.0`.
fn front_position(row: usize, col: usize, rows: usize, width: usize) -> f32 {
    let span = width + rows.saturating_sub(1) * RowSkew;
    (col + row * RowSkew) as f32 / span.max(1) as f32
}

fn swept(
    arrived: Option<u64>,
    now: u64,
    row: usize,
    col: usize,
    rows: usize,
    width: usize,
) -> bool {
    match arrived {
        None => true,
        Some(a) if now >= a.saturating_add(SweepMs) => true,
        Some(a) => {
            let progress = now.saturating_sub(a) as f32 / SweepMs as f32;
            front_position(row, col, rows, width) <= progress
        }
    }
}

fn level_grid(input: &effect_input_t<'_>, index: usize, width: usize) -> shape_grid_t {
    match input.levels.get(index) {
        Some(level) => {
            let shapes = match &level.draft {
                Some(draft) if !draft.trim().is_empty() => shapes_from_draft(draft, width),
                _ => shapes_from_len(level.level_len, level.level, input.seed, width),
            };
            layout_shapes(&shapes, width)
        }
        None => layout_shapes(&placeholder_shapes(input.seed, width), width),
    }
}

fn texture_span(
    input: &effect_input_t<'_>,
    cell: Option<block_cell_t>,
    level: Option<u32>,
    clock: u64,
    theme: &theme_t,
) -> span_t {
    match cell {
        Some(cell) => {
            let (symbol, color) = texture(input.seed, cell, level, input.level_count, clock, theme);
            span_t::new(symbol.to_string(), Style::new().fg(color))
        }
        None => span_t::new(" ", Style::new()),
    }
}

/// Render the reply area of one attempt at app-clock `now`, `width` columns wide. With
/// `animate` false the frame is static: no shimmer, no fronts, and the final text at once.
pub fn render_effect(
    input: &effect_input_t<'_>,
    now: u64,
    width: usize,
    theme: &theme_t,
    animate: bool,
) -> effect_frame_t {
    let width = width.max(1);
    if let Some((final_lines, end_at)) = input.end {
        let end_at = if animate { end_at } else { None };
        return match end_at {
            Some(e) if now < dissolve_end(e, final_lines) => {
                dissolve(input, final_lines, e, now, width, theme)
            }
            _ => effect_frame_t {
                lines: final_lines.to_vec(),
                done: true,
            },
        };
    }
    let clock = if animate { now } else { 0 };
    let current = input.levels.len().checked_sub(1);
    let cur_grid = match current {
        Some(i) => level_grid(input, i, width),
        None => level_grid(input, usize::MAX, width),
    };
    let cur_level = current.and_then(|i| input.levels.get(i)).map(|l| l.level);
    let arrived = if animate {
        current
            .and_then(|i| input.levels.get(i))
            .and_then(|l| l.arrived_ms)
    } else {
        None
    };
    let (prev_grid, prev_level) = match (current, arrived) {
        (Some(i), Some(_)) => {
            let prev = i.checked_sub(1);
            let grid = match prev {
                Some(p) => level_grid(input, p, width),
                None => level_grid(input, usize::MAX, width),
            };
            (
                grid,
                prev.and_then(|p| input.levels.get(p)).map(|l| l.level),
            )
        }
        _ => (shape_grid_t::default(), None),
    };
    let rows = cur_grid.rows.len().max(prev_grid.rows.len());
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut line = line_t::blank();
        for col in 0..width {
            let (grid, level) = if swept(arrived, clock, row, col, rows, width) {
                (&cur_grid, cur_level)
            } else {
                (&prev_grid, prev_level)
            };
            let mut span = texture_span(input, grid.at(row, col), level, clock, theme);
            if current.is_none() && animate {
                // Before the first level: a slow breathing pulse.
                let since = clock.saturating_sub(input.started_ms.unwrap_or(0));
                let breath = ((since as f32 / 900.0).sin() * 0.5 + 0.5) * 0.5 + 0.35;
                if let Some(fg) = span.style.fg {
                    span.style = span.style.fg(mix(theme.bg, fg, breath));
                }
            }
            line.push(span.text, span.style);
        }
        trim_trailing_blanks(&mut line);
        lines.push(line);
    }
    effect_frame_t { lines, done: false }
}

fn trim_trailing_blanks(line: &mut line_t) {
    while let Some(last) = line.spans.last_mut() {
        if last.style == Style::new() {
            let trimmed = last.text.trim_end_matches(' ').len();
            last.text.truncate(trimmed);
            if last.text.is_empty() {
                line.spans.pop();
                continue;
            }
        }
        break;
    }
}

fn dissolve(
    input: &effect_input_t<'_>,
    final_lines: &[line_t],
    end_at: u64,
    now: u64,
    width: usize,
    theme: &theme_t,
) -> effect_frame_t {
    let target = final_grid(final_lines);
    let last = input.levels.len().checked_sub(1);
    let pre_grid = match last {
        Some(i) => level_grid(input, i, width),
        None => layout_shapes(&placeholder_shapes(input.seed, width), width),
    };
    let pre_level = last.and_then(|i| input.levels.get(i)).map(|l| l.level);
    let final_level = Some(input.level_count.saturating_sub(1));
    let rows = target.len().max(pre_grid.rows.len());
    let cols = target.iter().map(|r| r.len()).max().unwrap_or(0).max(width);
    let duration = dissolve_ms(count_glyphs(final_lines));
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut line = line_t::blank();
        let target_row = target.get(row);
        let mut col = 0usize;
        while col < cols {
            let fcell = target_row.and_then(|r| r.get(col));
            let u = front_position(row, col, rows, cols);
            let jitter = hash(&[input.seed, row as u64, col as u64, 0x7E]) % JitterMs.max(1);
            let reveal = end_at
                .saturating_add((u * duration as f32) as u64)
                .saturating_add(jitter);
            let reshaped = swept(Some(end_at), now, row, col, rows, cols);
            if now < reveal {
                // Not reached yet: blocks, the final word shapes once the reshape front passed.
                let block = if reshaped {
                    match fcell {
                        Some(final_cell_t::Glyph { width: w, .. }) => Some(block_cell_t {
                            block: row.wrapping_mul(1009).wrapping_add(col),
                            index: 1,
                            width: (*w).max(3),
                        }),
                        Some(final_cell_t::Covered) => Some(block_cell_t {
                            block: row.wrapping_mul(1009).wrapping_add(col),
                            index: 1,
                            width: 3,
                        }),
                        _ => None,
                    }
                } else {
                    pre_grid.at(row, col)
                };
                let level = if reshaped { final_level } else { pre_level };
                let span = texture_span(input, block, level, now, theme);
                line.push(span.text, span.style);
                col += 1;
                continue;
            }
            let p = now.saturating_sub(reveal) as f32 / CellFadeMs as f32;
            let h = hash(&[input.seed, row as u64, col as u64, now / 60]);
            match fcell {
                Some(final_cell_t::Glyph {
                    text,
                    width: w,
                    style,
                }) if p >= 0.6 => {
                    let t = ((p - 0.6) / 0.4).clamp(0.0, 1.0);
                    let fg = style.fg.unwrap_or(theme.fg);
                    let color = if p >= 1.0 { fg } else { mix(theme.ice, fg, t) };
                    let mut s = style.fg(color);
                    if p < 1.0 {
                        s = s.add_modifier(Modifier::BOLD);
                    }
                    line.push(text.clone(), s);
                    col += (*w).max(1);
                    continue;
                }
                Some(final_cell_t::Blank { style }) if p >= 0.6 => {
                    line.push(" ", *style);
                }
                Some(final_cell_t::Covered) if p >= 0.6 => {
                    // The glyph to the left was drawn when its own cell revealed. If its
                    // front lagged this one, keep the column blank.
                    line.push(" ", Style::new());
                }
                Some(final_cell_t::Glyph { .. }) | Some(final_cell_t::Covered) if p < 0.4 => {
                    let dots = (4.0 - p * 7.0).round().max(1.0) as u32;
                    let color = mix(theme.bg, theme.ice, 0.9 - p);
                    line.push(braille(h, dots).to_string(), Style::new().fg(color));
                }
                Some(final_cell_t::Glyph { .. }) | Some(final_cell_t::Covered) if p < 0.6 => {
                    if h.is_multiple_of(5) {
                        let color = mix(theme.bg, theme.ice, 0.35);
                        line.push(braille(h, 1).to_string(), Style::new().fg(color));
                    } else {
                        line.push(" ", Style::new());
                    }
                }
                _ => line.push(" ", Style::new()),
            }
            col += 1;
        }
        if let Some(fill) = final_lines.get(row).and_then(|l| l.fill) {
            line.fill = Some(fill);
        }
        trim_trailing_blanks(&mut line);
        lines.push(line);
    }
    effect_frame_t { lines, done: false }
}

/// Whether a character is one the effect draws for a hidden word: block, shade, braille or
/// space. Tests use this to prove no glyph of a draft or unrevealed text leaks.
pub fn is_effect_glyph(c: char) -> bool {
    matches!(c, ' ' | '█' | '▓' | '▒') || ('\u{2800}'..='\u{28ff}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown;

    fn theme() -> theme_t {
        theme_t::dark()
    }

    fn level(level: u32, len: u32, draft: Option<&str>, at: Option<u64>) -> level_input_t {
        level_input_t {
            level,
            level_len: len,
            draft: draft.map(str::to_string),
            arrived_ms: at,
        }
    }

    fn all_text(frame: &effect_frame_t) -> String {
        frame
            .lines
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn hashing_is_deterministic() {
        assert_eq!(hash(&[1, 2, 3]), hash(&[1, 2, 3]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[3, 2, 1]));
        let u = unit(hash(&[9]));
        assert!((0.0..1.0).contains(&u));
    }

    #[test]
    fn block_widths_follow_draft_tokens() {
        let shapes = shapes_from_draft("ab  cde\n日本 x", 80);
        assert_eq!(shapes, vec![2, 3, 4, 1]);
        let grid = layout_shapes(&shapes, 80);
        assert_eq!(grid.block_widths(), vec![2, 3, 4, 1]);
        assert_eq!(grid.rows.len(), 1);
        // A token wider than the line is clamped, and blocks wrap.
        let grid = layout_shapes(&shapes_from_draft("aaaaaaaaaaaa bb cc", 5), 5);
        assert_eq!(grid.block_widths(), vec![5, 2, 2]);
        assert_eq!(grid.rows.len(), 2);
    }

    #[test]
    fn coarse_levels_have_fewer_wider_blocks() {
        let coarse = shapes_from_len(8, 0, 7, 80);
        let fine = shapes_from_len(64, 2, 7, 80);
        assert!(coarse.len() < fine.len());
        let avg = |v: &[usize]| v.iter().sum::<usize>() as f32 / v.len() as f32;
        assert!(avg(&coarse) > avg(&fine));
        assert!(shapes_from_len(1_000_000, 4, 7, 80).len() <= MaxBlocks);
        assert!(shapes_from_len(0, 0, 7, 0).iter().all(|w| *w >= 1));
    }

    #[test]
    fn texture_is_deterministic_and_finer_levels_are_sparser() {
        let t = theme();
        let cell = block_cell_t {
            block: 3,
            index: 1,
            width: 5,
        };
        assert_eq!(
            texture(5, cell, Some(0), 5, 1000, &t),
            texture(5, cell, Some(0), 5, 1000, &t)
        );
        let solid_count = |level: u32| {
            (0..400)
                .filter(|i| {
                    let c = block_cell_t {
                        block: *i,
                        index: 1,
                        width: 5,
                    };
                    matches!(texture(11, c, Some(level), 5, 0, &t).0, '█' | '▓' | '▒')
                })
                .count()
        };
        assert!(solid_count(0) > solid_count(4));
    }

    #[test]
    fn generation_frames_never_show_draft_glyphs() {
        let t = theme();
        let levels = vec![
            level(0, 8, Some("hello wonderful"), Some(0)),
            level(1, 32, Some("hello wonderful world"), Some(700)),
        ];
        for now in [0u64, 100, 300, 700, 800, 1000, 5000] {
            let input = effect_input_t {
                seed: 4242,
                level_count: 5,
                levels: &levels,
                started_ms: Some(0),
                end: None,
            };
            let frame = render_effect(&input, now, 40, &t, true);
            assert!(!frame.done);
            for c in all_text(&frame).chars() {
                assert!(is_effect_glyph(c) || c == '\n', "leaked {c:?} at {now}");
            }
        }
    }

    #[test]
    fn frames_are_deterministic_for_seed_and_clock() {
        let t = theme();
        let levels = vec![level(0, 8, None, Some(0)), level(1, 32, None, Some(500))];
        let input = effect_input_t {
            seed: 99,
            level_count: 4,
            levels: &levels,
            started_ms: Some(0),
            end: None,
        };
        let a = render_effect(&input, 650, 50, &t, true);
        let b = render_effect(&input, 650, 50, &t, true);
        assert_eq!(a, b);
        let other_seed = effect_input_t {
            seed: 100,
            ..input.clone()
        };
        assert_ne!(a, render_effect(&other_seed, 650, 50, &t, true));
    }

    #[test]
    fn level_transition_sweeps_left_to_right() {
        let t = theme();
        let levels = vec![
            level(
                0,
                2,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                Some(0),
            ),
            level(
                1,
                8,
                Some("a b c d e f g h i j k l m n o p q r s t"),
                Some(1000),
            ),
        ];
        let input = effect_input_t {
            seed: 1,
            level_count: 2,
            levels: &levels,
            started_ms: Some(0),
            end: None,
        };
        // Mid-sweep: the left side has gaps (new, fine layout) and the right side is solid.
        let mid = render_effect(&input, 1000 + SweepMs / 2, 40, &t, true);
        let row: Vec<char> = mid.lines[0].text().chars().collect();
        let left_gaps = row.iter().take(12).filter(|c| **c == ' ').count();
        let right_gaps = row.iter().skip(26).take(12).filter(|c| **c == ' ').count();
        assert!(left_gaps > 0, "{row:?}");
        assert_eq!(right_gaps, 0, "{row:?}");
    }

    #[test]
    fn dissolve_hides_text_until_the_front_and_ends_exactly() {
        let t = theme();
        let final_text = "Zebras quickly jog over lazy foxes";
        let final_lines = markdown::render(final_text, 40, &t);
        let levels = vec![level(
            0,
            8,
            Some("Zebras quickly jog over lazy foxes"),
            Some(0),
        )];
        let end_at = 2000u64;
        let input = effect_input_t {
            seed: 3,
            level_count: 1,
            levels: &levels,
            started_ms: Some(0),
            end: Some((&final_lines, Some(end_at))),
        };
        let finish = dissolve_end(end_at, &final_lines);
        // At the moment the call ends nothing is readable.
        let first = render_effect(&input, end_at, 40, &t, true);
        assert!(!first.done);
        for c in all_text(&first).chars() {
            assert!(is_effect_glyph(c), "leaked {c:?}");
        }
        // Mid-way, the left is readable before the right.
        let mid = render_effect(&input, end_at + 900, 40, &t, true);
        let text = all_text(&mid);
        assert!(text.starts_with("Zebras"), "{text}");
        assert!(!text.contains("foxes"), "{text}");
        // Just before the end it is not done; at the end it equals the Markdown render.
        assert!(!render_effect(&input, finish - 1, 40, &t, true).done);
        let done = render_effect(&input, finish, 40, &t, true);
        assert!(done.done);
        assert_eq!(done.lines, final_lines);
    }

    #[test]
    fn no_unrevealed_word_appears_in_any_frame() {
        let t = theme();
        let words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"];
        let final_text = words.join(" ");
        let final_lines = markdown::render(&final_text, 30, &t);
        let levels = vec![level(0, 4, None, Some(0))];
        let end_at = 1000u64;
        let input = effect_input_t {
            seed: 8,
            level_count: 1,
            levels: &levels,
            started_ms: Some(0),
            end: Some((&final_lines, Some(end_at))),
        };
        let finish = dissolve_end(end_at, &final_lines);
        let mut now = end_at;
        while now < finish {
            let frame = render_effect(&input, now, 30, &t, true);
            // Any letter drawn sits exactly where the final text has that letter.
            for (row, line) in frame.lines.iter().enumerate() {
                let want: Vec<char> = final_lines
                    .get(row)
                    .map(|l| l.text().chars().collect())
                    .unwrap_or_default();
                for (col, c) in line.text().chars().enumerate() {
                    if !is_effect_glyph(c) {
                        assert_eq!(want.get(col), Some(&c), "row {row} col {col} at {now}");
                    }
                }
            }
            now += 37;
        }
    }

    #[test]
    fn instant_mode_shows_final_text_at_once() {
        let t = theme();
        let final_lines = markdown::render("**done** now", 30, &t);
        let levels = vec![level(0, 4, None, Some(0))];
        let input = effect_input_t {
            seed: 8,
            level_count: 1,
            levels: &levels,
            started_ms: Some(0),
            end: Some((&final_lines, Some(10))),
        };
        let frame = render_effect(&input, 10, 30, &t, false);
        assert!(frame.done);
        assert_eq!(frame.lines, final_lines);
        // A replayed end (no arrival time) is also immediate.
        let replay = effect_input_t {
            end: Some((&final_lines, None)),
            ..input
        };
        assert!(render_effect(&replay, 10, 30, &t, true).done);
    }

    #[test]
    fn static_generation_does_not_shimmer() {
        let t = theme();
        let levels = vec![level(0, 8, None, Some(0))];
        let input = effect_input_t {
            seed: 8,
            level_count: 3,
            levels: &levels,
            started_ms: Some(0),
            end: None,
        };
        assert_eq!(
            render_effect(&input, 100, 30, &t, false),
            render_effect(&input, 9000, 30, &t, false)
        );
    }

    #[test]
    fn placeholder_before_first_level() {
        let t = theme();
        let input = effect_input_t {
            seed: 8,
            level_count: 3,
            levels: &[],
            started_ms: Some(0),
            end: None,
        };
        let frame = render_effect(&input, 250, 30, &t, true);
        assert!(!frame.lines.is_empty());
        assert!(
            all_text(&frame)
                .chars()
                .all(|c| is_effect_glyph(c) || c == '\n')
        );
    }

    #[test]
    fn dissolve_duration_scales_and_caps() {
        assert_eq!(dissolve_ms(0), DissolveMinMs);
        assert_eq!(dissolve_ms(100_000), DissolveMaxMs);
        assert!(dissolve_ms(250) > DissolveMinMs);
    }

    #[test]
    fn zero_width_and_huge_inputs_are_safe() {
        let t = theme();
        let draft = "x ".repeat(50_000);
        let levels = vec![level(0, 8, Some(&draft), Some(0))];
        let input = effect_input_t {
            seed: 8,
            level_count: 3,
            levels: &levels,
            started_ms: Some(0),
            end: None,
        };
        let frame = render_effect(&input, 10, 0, &t, true);
        assert!(frame.lines.len() <= MaxEffectRows);
    }
}
