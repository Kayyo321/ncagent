/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The tab bar across the top of the main panel (PROJECT.md, D62).
//!
//! No lines: the bar is one tone step above the transcript, each tab a step above the bar, and
//! the shown tab a step above that with a left rail in its colour tag (ice blue without one),
//! like a speaker's rail in the transcript. A tab shows its colour tag, a pin, its title, what
//! it is doing (ice blue while it generates, orchid while a tool runs, amber while it waits for
//! the user) and a `×` chip. A tab that is not shown and finished its turn gets a green dot, and
//! one that needs the user gets an amber diamond, until it is shown.
//!
//! When the tabs do not fit, their titles shrink first, then the `×` chips of the tabs that are
//! not shown go, and last the bar scrolls between `‹` and `›` chips, keeping the shown tab in
//! view.
//!
//! While the settings page is open it has a tab of its own after the conversations, `⚙
//! settings`, which never scrolls away (D65). It is not a conversation and is never saved.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::app::Attention;
use crate::canvas::{fill, put_str};
use crate::input::{Action, TabAction, Target, hit_registry_t};
use crate::settings_page::PageAction;
use crate::sidebar::Activity;
use crate::tags::PinGlyph;
use crate::text::{str_width, truncate};
use crate::theme::{mix, theme_t};

/// Longest title a tab shows, in columns.
pub const MaxTabTitle: usize = 24;
/// Titles shrink to this before the bar scrolls.
pub const MinTabTitle: usize = 6;

/// What one tab shows.
#[derive(Clone, Debug, PartialEq)]
pub struct tab_view_t {
    pub title: String,
    /// The colour of its tag, when it has one.
    pub color: Option<Color>,
    pub pinned: bool,
    pub activity: Activity,
    pub attention: Attention,
    pub active: bool,
}

pub struct tab_bar_input_t<'a> {
    pub tabs: &'a [tab_view_t],
    /// The first tab to draw when the bar scrolls.
    pub scroll: usize,
    /// Bring the shown tab into view (it changed since the last frame).
    pub follow: bool,
    pub hover: Option<(u16, u16)>,
    pub clock: u64,
    pub animate: bool,
    /// The settings page's tab: `Some(true)` while the page is the view, `Some(false)` while it
    /// is open behind a conversation, `None` when it is closed.
    pub page: Option<bool>,
}

/// The settings page's tab: rail, space, the label, space, the close chip.
const PageLabel: &str = "⚙ settings";
fn page_tab_width() -> usize {
    1 + 1 + str_width(PageLabel) + 1 + 2
}

/// How the tabs are fitted to the bar.
struct fit_t {
    cap: usize,
    /// Every tab has its `×` chip, not only the shown one.
    closes: bool,
    /// The first tab drawn, and whether the bar scrolls.
    start: usize,
    scrolls: bool,
}

fn indicator(tab: &tab_view_t) -> bool {
    tab.activity != Activity::Idle || tab.attention != Attention::None
}

/// Columns of one tab: rail, space, pin, title, space, activity dot, close chip.
fn tab_width(tab: &tab_view_t, cap: usize, close: bool) -> usize {
    1 + 1
        + if tab.pinned { 2 } else { 0 }
        + str_width(&tab.title).clamp(1, cap.max(1))
        + 1
        + if indicator(tab) { 2 } else { 0 }
        + if close { 2 } else { 0 }
}

/// Columns of tabs `from..to` with one column between them.
fn span(tabs: &[tab_view_t], from: usize, to: usize, cap: usize, closes: bool) -> usize {
    tabs.iter()
        .take(to)
        .skip(from)
        .map(|t| tab_width(t, cap, closes || t.active) + 1)
        .sum()
}

/// The `+` chip and the gap before it.
const PlusWidth: usize = 4;
/// The `‹` and `›` chips.
const ArrowWidth: usize = 2;

fn fit(input: &tab_bar_input_t<'_>, width: usize) -> fit_t {
    let tabs = input.tabs;
    let n = tabs.len();
    let page = if input.page.is_some() {
        page_tab_width() + 1
    } else {
        0
    };
    let room = width.saturating_sub(1 + PlusWidth + page);
    for closes in [true, false] {
        for cap in (MinTabTitle..=MaxTabTitle).rev() {
            if span(tabs, 0, n, cap, closes) <= room {
                return fit_t {
                    cap,
                    closes,
                    start: 0,
                    scrolls: false,
                };
            }
        }
    }
    let room = room.saturating_sub(2 * ArrowWidth);
    let active = tabs.iter().position(|t| t.active).unwrap_or(0);
    let mut start = input.scroll.min(n.saturating_sub(1));
    if input.follow {
        start = start.min(active);
        while start < active && span(tabs, start, active + 1, MinTabTitle, false) > room {
            start += 1;
        }
    }
    fit_t {
        cap: MinTabTitle,
        closes: false,
        start,
        scrolls: true,
    }
}

/// Draw the tab bar. Returns the first tab drawn, for the next frame's scroll.
pub fn draw_tab_bar(
    buf: &mut Buffer,
    area: Rect,
    input: &tab_bar_input_t<'_>,
    theme: &theme_t,
    hits: &mut hit_registry_t,
) -> usize {
    if area.height == 0 || area.width < 8 {
        return input.scroll;
    }
    let bar = theme.side;
    fill(buf, area, Style::new().bg(bar));
    let width = usize::from(area.width);
    let fitted = fit(input, width);
    let y = area.y;
    let right = area.right();
    let hovered = |r: Rect| {
        input
            .hover
            .is_some_and(|(hx, hy)| r.contains(Position::new(hx, hy)))
    };
    let pulse = if input.animate {
        (input.clock as f32 / 300.0).sin() * 0.5 + 0.5
    } else {
        1.0
    };

    let mut x = area.x.saturating_add(1);
    let page_w = page_tab_width() as u16;
    let mut limit = right.saturating_sub(PlusWidth as u16);
    if input.page.is_some() {
        limit = limit.saturating_sub(page_w + 1);
    }
    if fitted.scrolls {
        let back = Rect::new(area.x, y, ArrowWidth as u16, 1);
        let can = fitted.start > 0;
        let fg = match (can, hovered(back)) {
            (true, true) => theme.fg,
            (true, false) => theme.fg_dim,
            (false, _) => theme.fg_faint,
        };
        put_str(buf, back.x, y, right, "‹ ", Style::new().fg(fg).bg(bar));
        hits.add(back, Action::Tab(TabAction::Scroll(-1)));
        x = area.x.saturating_add(ArrowWidth as u16);
        limit = limit.saturating_sub(ArrowWidth as u16);
    }

    let mut last_drawn = fitted.start;
    let mut more_right = false;
    for (i, tab) in input.tabs.iter().enumerate().skip(fitted.start) {
        let close = fitted.closes || tab.active;
        let w = tab_width(tab, fitted.cap, close) as u16;
        if x.saturating_add(w) > limit {
            more_right = true;
            break;
        }
        let ctx = tab_ctx_t {
            input,
            cap: fitted.cap,
            pulse,
            theme,
        };
        draw_tab(buf, Rect::new(x, y, w, 1), tab, i, close, &ctx, hits);
        last_drawn = i;
        x = x.saturating_add(w + 1);
    }

    if let Some(shown) = input.page {
        draw_page_tab(buf, Rect::new(x, y, page_w, 1), shown, input, theme, hits);
        x = x.saturating_add(page_w + 1);
    }

    // The `+` chip follows the last tab drawn.
    let plus = Rect::new(x, y, 3, 1);
    let plus_bg = if hovered(plus) {
        theme.select
    } else {
        theme.inset
    };
    put_str(
        buf,
        plus.x,
        y,
        right,
        " + ",
        Style::new()
            .fg(theme.ice)
            .bg(plus_bg)
            .add_modifier(Modifier::BOLD),
    );
    hits.add(plus, Action::Tab(TabAction::New));

    if fitted.scrolls {
        let fwd = Rect::new(
            right.saturating_sub(ArrowWidth as u16),
            y,
            ArrowWidth as u16,
            1,
        );
        let can = more_right || last_drawn + 1 < input.tabs.len();
        let fg = match (can, hovered(fwd)) {
            (true, true) => theme.fg,
            (true, false) => theme.fg_dim,
            (false, _) => theme.fg_faint,
        };
        put_str(buf, fwd.x, y, right, " ›", Style::new().fg(fg).bg(bar));
        hits.add(fwd, Action::Tab(TabAction::Scroll(1)));
    }
    fitted.start
}

/// The settings page's tab, drawn like a conversation tab with an ice rail while it shows.
fn draw_page_tab(
    buf: &mut Buffer,
    rect: Rect,
    shown: bool,
    input: &tab_bar_input_t<'_>,
    theme: &theme_t,
    hits: &mut hit_registry_t,
) {
    let over = |r: Rect| {
        input
            .hover
            .is_some_and(|(hx, hy)| r.contains(Position::new(hx, hy)))
    };
    let bg = if shown {
        mix(theme.select, theme.agent, 0.07)
    } else if over(rect) {
        theme.hover
    } else {
        theme.panel
    };
    let st = Style::new().bg(bg);
    let (y, end) = (rect.y, rect.right());
    fill(buf, rect, st);
    if shown {
        put_str(buf, rect.x, y, end, "▌", st.fg(theme.ice));
    }
    hits.add(rect, Action::Page(PageAction::Show));
    let x = put_str(
        buf,
        rect.x.saturating_add(2),
        y,
        end,
        "⚙",
        st.fg(if shown { theme.ice } else { theme.fg_dim }),
    );
    let label = if shown {
        st.fg(theme.fg).add_modifier(Modifier::BOLD)
    } else {
        st.fg(theme.fg_dim)
    };
    let x = put_str(buf, x.saturating_add(1), y, end, "settings", label);
    let chip = Rect::new(x.saturating_add(1), y, 2, 1);
    let fg = if over(chip) {
        theme.error
    } else {
        theme.fg_faint
    };
    put_str(buf, chip.x, y, end, "×", st.fg(fg));
    hits.add(chip, Action::Page(PageAction::Close));
}

/// The tone of a tab: the shown one is lifted most, a hovered one a little.
fn tab_bg(tab: &tab_view_t, hovered: bool, theme: &theme_t) -> Color {
    if tab.active {
        mix(theme.select, theme.agent, 0.07)
    } else if hovered {
        theme.hover
    } else {
        theme.panel
    }
}

/// What a tab is doing, as a glyph and a colour.
fn activity_mark(tab: &tab_view_t, pulse: f32, theme: &theme_t) -> Option<(&'static str, Color)> {
    match (tab.activity, tab.attention) {
        (Activity::Generating, _) => Some(("●", mix(theme.ice_deep, theme.ice, pulse))),
        (Activity::Tool, _) => Some(("●", theme.tool)),
        (Activity::Waiting, Attention::NeedsYou) => Some(("◆", theme.approval)),
        (Activity::Waiting, _) => Some(("●", theme.approval)),
        (Activity::Idle, Attention::NeedsYou) => Some(("◆", theme.approval)),
        (Activity::Idle, Attention::Finished) => Some(("●", theme.success)),
        (Activity::Idle, Attention::None) => None,
    }
}

/// What every tab of one frame draws with.
struct tab_ctx_t<'a, 'b> {
    input: &'a tab_bar_input_t<'b>,
    cap: usize,
    pulse: f32,
    theme: &'a theme_t,
}

fn draw_tab(
    buf: &mut Buffer,
    rect: Rect,
    tab: &tab_view_t,
    index: usize,
    close: bool,
    ctx: &tab_ctx_t<'_, '_>,
    hits: &mut hit_registry_t,
) {
    let (input, cap, pulse, theme) = (ctx.input, ctx.cap, ctx.pulse, ctx.theme);
    let over = input
        .hover
        .is_some_and(|(hx, hy)| rect.contains(Position::new(hx, hy)));
    let bg = tab_bg(tab, over, theme);
    let st = Style::new().bg(bg);
    let end = rect.right();
    let y = rect.y;
    fill(buf, rect, st);
    let rail = match (tab.color, tab.active) {
        (Some(c), _) => Some(c),
        (None, true) => Some(theme.agent),
        (None, false) => None,
    };
    if let Some(c) = rail {
        put_str(buf, rect.x, y, end, "▌", st.fg(c));
    }
    hits.add_target(rect, Target::Tab(index));
    let mut x = rect.x.saturating_add(2);
    if tab.pinned {
        x = put_str(
            buf,
            x,
            y,
            end,
            PinGlyph,
            st.fg(mix(bg, theme.question, 0.85)),
        );
        x = x.saturating_add(1);
    }
    let title_style = if tab.active {
        st.fg(theme.fg).add_modifier(Modifier::BOLD)
    } else if over {
        st.fg(theme.fg)
    } else {
        st.fg(theme.fg_dim)
    };
    x = put_str(buf, x, y, end, &truncate(&tab.title, cap), title_style);
    x = x.saturating_add(1);
    if let Some((glyph, color)) = activity_mark(tab, pulse, theme) {
        put_str(buf, x, y, end, glyph, st.fg(color));
        x = x.saturating_add(2);
    }
    if close {
        let chip = Rect::new(x, y, 2, 1);
        let fg = if input
            .hover
            .is_some_and(|(hx, hy)| chip.contains(Position::new(hx, hy)))
        {
            theme.error
        } else {
            theme.fg_faint
        };
        put_str(buf, x, y, end, "×", st.fg(fg));
        hits.add(chip, Action::Tab(TabAction::Close(index)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::buffer_lines;

    fn tab(title: &str, active: bool) -> tab_view_t {
        tab_view_t {
            title: title.to_string(),
            color: None,
            pinned: false,
            activity: Activity::Idle,
            attention: Attention::None,
            active,
        }
    }

    fn draw(tabs: &[tab_view_t], width: u16, scroll: usize) -> (String, hit_registry_t, usize) {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
        let mut hits = hit_registry_t::default();
        let input = tab_bar_input_t {
            tabs,
            scroll,
            follow: true,
            hover: None,
            clock: 0,
            animate: false,
            page: None,
        };
        let start = draw_tab_bar(
            &mut buf,
            Rect::new(0, 0, width, 1),
            &input,
            &theme_t::dark(),
            &mut hits,
        );
        (buffer_lines(&buf).join(""), hits, start)
    }

    #[test]
    fn titles_shrink_then_the_bar_scrolls_and_keeps_the_shown_tab() {
        let tabs: Vec<tab_view_t> = (0..8)
            .map(|i| tab(&format!("conversation number {i}"), i == 6))
            .collect();
        let (wide, _, _) = draw(&tabs[..2], 130, 0);
        assert!(wide.contains("conversation number 0"), "{wide}");
        let (narrow, hits, start) = draw(&tabs, 50, 0);
        assert!(narrow.starts_with("‹"), "{narrow}");
        assert!(narrow.ends_with("›"), "{narrow}");
        assert!(start > 0);
        assert!(hits.find_target(&Target::Tab(6)).is_some(), "{narrow}");
        assert!(hits.find(&Action::Tab(TabAction::Close(6))).is_some());
        assert!(hits.find(&Action::Tab(TabAction::Close(5))).is_none());
        assert!(hits.find(&Action::Tab(TabAction::New)).is_some());
    }

    #[test]
    fn activity_marks_follow_the_state() {
        let theme = theme_t::dark();
        let mut t = tab("x", false);
        assert_eq!(activity_mark(&t, 1.0, &theme), None);
        t.activity = Activity::Tool;
        assert_eq!(activity_mark(&t, 1.0, &theme), Some(("●", theme.tool)));
        t.activity = Activity::Waiting;
        t.attention = Attention::NeedsYou;
        assert_eq!(activity_mark(&t, 1.0, &theme), Some(("◆", theme.approval)));
        t.activity = Activity::Idle;
        t.attention = Attention::Finished;
        assert_eq!(activity_mark(&t, 1.0, &theme), Some(("●", theme.success)));
    }
}
