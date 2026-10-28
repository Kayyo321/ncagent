/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Toast cards: the short notes at the top right of the transcript, and their motion.
//!
//! A toast appears with the title card's reveal (D56): blocks shaped like its words, then a
//! front that sweeps from left to right and settles each cell into its glyph, rail and tone
//! included. It stays readable for `ToastMs`, then dissolves away by the same process run
//! backwards: the front sweeps back from right to left, so the last text to appear is the first
//! to go, the cells turn back into coarse blocks, and the blocks thin out to nothing. Each takes
//! 300 to 450 ms by the length of the text. A click dismisses a toast at once and starts its
//! dissolve. A new toast replaces the one showing: the old card is cut and the new one reveals,
//! so two cards never overlap.
//!
//! With animation off (the setting or `--speed instant`) a toast appears and goes at once. The
//! last frame of the reveal is the static card, cell for cell. Every frame is a function of the
//! app clock, so a resize or a jump of the clock never leaves a card half dissolved.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::{ToastKind, app_t, toast_t};
use crate::canvas::{put_line, put_str};
use crate::effect::{hash, render_reveal_timed, reveal_timing_t, unit};
use crate::input::Action;
use crate::text::{line_t, str_width, truncate};
use crate::theme::{mix, theme_t};

/// How long a toast stays readable once revealed.
pub const ToastMs: u64 = 3200;
/// The reveal and the dissolve each take this long at least, and at most `MaxToastFxMs`.
pub const MinToastFxMs: u64 = 300;
pub const MaxToastFxMs: u64 = 450;
/// The share of the dissolve spent sweeping back; the rest thins the blocks out.
const SweepShare: u64 = 70;

/// A new toast shown at `clock`.
pub fn new_toast(text: String, kind: ToastKind, clock: u64, animate: bool) -> toast_t {
    let fx = if animate { effect_ms(&text) } else { 0 };
    toast_t {
        text,
        kind,
        shown: clock,
        in_ms: fx,
        out_ms: fx,
        until: clock.saturating_add(fx).saturating_add(ToastMs),
    }
}

/// The length of a toast's reveal and dissolve: longer for a longer text, within bounds.
pub fn effect_ms(text: &str) -> u64 {
    (240 + 3 * str_width(text) as u64).clamp(MinToastFxMs, MaxToastFxMs)
}

impl toast_t {
    /// When the dissolve is over and the toast is gone.
    pub fn gone(&self) -> u64 {
        self.until.saturating_add(self.out_ms)
    }

    fn seed(&self) -> u64 {
        let text: Vec<u64> = self.text.bytes().map(u64::from).collect();
        hash(&[hash(&text), self.shown, 0x7057])
    }
}

/// What a toast shows at one instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastFrame {
    /// Not drawn: gone, or not yet shown.
    Hidden,
    /// The static card.
    Card,
    /// A frame of the reveal or the dissolve: the card's line with effect cells. A space with
    /// no background is see-through.
    Effect(line_t),
}

/// The static card of a toast: a rail in the toast's colour, then the text on an inset tone.
pub fn card_line(t: &toast_t, max_text: usize, theme: &theme_t) -> line_t {
    let color = match t.kind {
        ToastKind::Info => theme.agent,
        ToastKind::Warning => theme.approval,
        ToastKind::Error => theme.error,
    };
    let mut line = line_t::plain("▌", Style::new().fg(color).bg(theme.inset));
    line.push(
        format!(" {} ", truncate(&t.text, max_text)),
        Style::new().fg(theme.fg).bg(theme.inset),
    );
    line
}

/// The frame of toast `t` at `now`, for a card `card`.
pub fn toast_frame(
    t: &toast_t,
    card: &line_t,
    now: u64,
    animate: bool,
    theme: &theme_t,
) -> ToastFrame {
    if now < t.shown || now > t.gone() {
        return ToastFrame::Hidden;
    }
    if now <= t.until {
        let in_end = t.shown.saturating_add(t.in_ms);
        if !animate || t.in_ms == 0 || now >= in_end {
            return ToastFrame::Card;
        }
        let frame = render_reveal_timed(
            std::slice::from_ref(card),
            0,
            t.seed(),
            t.shown,
            now,
            reveal_timing_t::scaled(t.in_ms),
            theme,
        );
        return match frame.lines.into_iter().next() {
            Some(line) if !frame.done => ToastFrame::Effect(line),
            _ => ToastFrame::Card,
        };
    }
    if !animate || t.out_ms == 0 {
        return ToastFrame::Hidden;
    }
    // The dissolve: the reveal run backwards, then the blocks thin out.
    let since = now - t.until;
    let sweep = (t.out_ms * SweepShare / 100).max(1);
    let timing = reveal_timing_t::scaled(t.in_ms.max(1));
    let at = if since < sweep {
        let left = timing
            .total_ms
            .saturating_sub(timing.total_ms * since / sweep);
        t.shown.saturating_add(left)
    } else {
        t.shown
    };
    let frame = render_reveal_timed(
        std::slice::from_ref(card),
        0,
        t.seed(),
        t.shown,
        at,
        timing,
        theme,
    );
    let mut line = match frame.lines.into_iter().next() {
        Some(line) if !frame.done => line,
        _ => card.clone(),
    };
    if since >= sweep {
        let q = (since - sweep) as f32 / (t.out_ms - sweep).max(1) as f32;
        line = thin_out(&line, q, t.seed(), theme);
    }
    ToastFrame::Effect(line)
}

/// Drop a share `q` of the drawn cells, chosen by a hash so the same cells go each frame, and
/// dim the rest toward the background.
fn thin_out(line: &line_t, q: f32, seed: u64, theme: &theme_t) -> line_t {
    let mut out = line_t::blank();
    let mut col = 0u64;
    for span in &line.spans {
        for ch in span.text.chars() {
            let keep = ch != ' ' && unit(hash(&[seed, col, 0x7417])) >= q;
            if keep {
                let fg = span.style.fg.map(|c| mix(c, theme.bg, q));
                let mut style = Style::new();
                if let Some(fg) = fg {
                    style = style.fg(fg);
                }
                out.push(ch.to_string(), style);
            } else {
                out.push(" ", Style::new());
            }
            col += 1;
        }
    }
    out
}

/// Write an effect line: a space with no background of its own is see-through, so the text
/// under the card shows until the front reaches it.
fn put_effect_line(buf: &mut Buffer, x: u16, y: u16, max_x: u16, line: &line_t) {
    let mut cx = x;
    for span in &line.spans {
        for ch in span.text.chars() {
            let text = ch.to_string();
            let w = str_width(&text).max(1) as u16;
            if cx.saturating_add(w) > max_x {
                return;
            }
            if ch != ' ' || span.style.bg.is_some() {
                put_str(buf, cx, y, max_x, &text, span.style);
            }
            cx = cx.saturating_add(w);
        }
    }
}

impl app_t {
    /// Start the dissolve of the toast now (a click on it).
    pub fn dismiss_toast(&mut self) {
        let clock = self.clock;
        if let Some(t) = self.toast.as_mut()
            && clock < t.until
        {
            t.until = clock;
        }
    }

    /// True while a toast reveals or dissolves, so the loop draws at the busy frame rate.
    pub fn toast_animating(&self) -> bool {
        self.animate
            && self.toast.as_ref().is_some_and(|t| {
                let in_end = t.shown.saturating_add(t.in_ms);
                (self.clock < in_end) || (self.clock >= t.until && self.clock <= t.gone())
            })
    }
}

/// Draw the toast at the top right of `area`, and make it a click target.
pub(crate) fn draw_toast(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    let theme = app.theme.clone();
    let Some(t) = &app.toast else {
        return;
    };
    let max = usize::from(area.width.saturating_sub(8)).max(8);
    let card = card_line(t, max, &theme);
    let w = card.width() as u16;
    let x = area.right().saturating_sub(w + 2);
    let y = area.y;
    match toast_frame(t, &card, app.clock, app.animate, &theme) {
        ToastFrame::Hidden => {}
        ToastFrame::Card => {
            put_line(buf, x, y, area.right(), &card, Style::new());
            app.hits.add(Rect::new(x, y, w, 1), Action::DismissToast);
            app.covers.push(Rect::new(x, y, w, 1));
        }
        ToastFrame::Effect(line) => {
            put_effect_line(buf, x, y, area.right(), &line);
            app.hits.add(Rect::new(x, y, w, 1), Action::DismissToast);
            app.covers.push(Rect::new(x, y, w, 1));
        }
    }
}
