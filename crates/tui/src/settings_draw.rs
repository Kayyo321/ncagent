/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Drawing the settings page (PROJECT.md, D65) in the visual language of D47: no lines, a tone
//! column for the section list, tone steps and rails for the cursor and the hovered row, and
//! small chips for every control.
//!
//! From 80 columns the section list is a column on the left and the rows of the section are on
//! the right. Below that the sections are a strip of chips across the top and the rows follow
//! in one column. The rows scroll; the cursor row stays in view unless the wheel moved them.

use harness::api::runtime_info_t;
use harness::settings::{Density, settings_t};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::app::app_t;
use crate::canvas::{fill, put_line, put_str, put_str_right};
use crate::input::{Action, Focus};
use crate::settings_page::{Fact, PageAction, PageConfirm, PageRow, Pane};
use crate::settings_spec::{Control, Section, Sections, SettingKey, changed_count};
use crate::text::{line_t, sanitize, str_width, truncate, wrap_plain};
use crate::theme::{ColorMode, mix, theme_t};
use crate::theme_set::theme_entry_t;

/// From this many columns the section list is a column of its own.
pub const SplitWidth: u16 = 80;
/// Columns of the section list column.
const ListWidth: u16 = 22;
/// The colours a swatch shows, in order.
pub const SwatchColors: [&str; 7] = ["bg", "fg", "agent", "tool", "approval", "error", "success"];

/// One drawn line of the rows pane.
struct pline_t {
    line: line_t,
    /// The row of the section this line belongs to.
    row: Option<usize>,
    /// What a click on the line does, when it is not a chip.
    action: Option<Action>,
    /// Chips on the line: (column offset, width, action).
    hits: Vec<(u16, u16, Action)>,
    /// Where a theme is named on the line, for the hover preview: (offset, width, id). A width
    /// of 0 means the whole line.
    previews: Vec<(u16, u16, String)>,
    /// A tone of its own, such as a confirmation.
    bg: Option<Color>,
    rail: Option<Color>,
}

impl pline_t {
    fn new(line: line_t, row: Option<usize>) -> Self {
        pline_t {
            line,
            row,
            action: None,
            hits: Vec::new(),
            previews: Vec::new(),
            bg: None,
            rail: None,
        }
    }
}

/// A chip of a control: text, style, what a click does and the theme it previews.
struct chip_t {
    text: String,
    style: Style,
    action: Option<Action>,
    preview: Option<String>,
}

fn chip(text: impl Into<String>, style: Style, action: Option<Action>) -> chip_t {
    chip_t {
        text: text.into(),
        style,
        action,
        preview: None,
    }
}

/// Lay chips out from the left, one column apart, onto as many lines as `width` needs.
fn flow(chips: Vec<chip_t>, width: usize, row: usize) -> Vec<pline_t> {
    let mut out: Vec<pline_t> = Vec::new();
    let mut cur = pline_t::new(line_t::blank(), Some(row));
    let mut x = 0usize;
    for c in chips {
        let w = str_width(&c.text);
        if x > 0 && x + 1 + w > width {
            out.push(cur);
            cur = pline_t::new(line_t::blank(), Some(row));
            x = 0;
        }
        if x > 0 {
            cur.line.push(" ", Style::new());
            x += 1;
        }
        let text = truncate(&c.text, width.max(1));
        let w = str_width(&text);
        cur.line.push(text, c.style);
        if let Some(a) = c.action {
            cur.hits.push((x as u16, w as u16, a));
        }
        if let Some(id) = c.preview {
            cur.previews.push((x as u16, w as u16, id));
        }
        x += w;
    }
    if !cur.line.spans.is_empty() {
        out.push(cur);
    }
    out
}

/// Put `right` at the end of `left` on one line `width` wide when both fit, else `right` on
/// the line under `left`. The chips of `right` move with it.
fn join_right(mut left: pline_t, right: pline_t, width: usize) -> Vec<pline_t> {
    let lw = left.line.width();
    let rw = right.line.width();
    if right.line.spans.is_empty() {
        return vec![left];
    }
    if lw + 2 + rw > width {
        return vec![left, right];
    }
    let pad = width - lw - rw;
    left.line.push(" ".repeat(pad), Style::new());
    let at = (lw + pad) as u16;
    for span in right.line.spans {
        left.line.push(span.text, span.style);
    }
    left.hits
        .extend(right.hits.into_iter().map(|(x, w, a)| (x + at, w, a)));
    left.previews
        .extend(right.previews.into_iter().map(|(x, w, id)| (x + at, w, id)));
    vec![left]
}

fn about_lines(text: &str, width: usize, row: usize, theme: &theme_t) -> Vec<pline_t> {
    wrap_plain(text, width, Style::new().fg(theme.fg_dim))
        .into_iter()
        .take(3)
        .map(|l| pline_t::new(l, Some(row)))
        .collect()
}

/// A swatch strip of a theme's key colours, two cells each.
fn swatch(palette: &theme_t, frame: Color) -> line_t {
    let mut line = line_t::blank();
    for name in SwatchColors {
        let color = palette.get(name).unwrap_or(frame);
        line.push("  ", Style::new().bg(color));
    }
    line
}

/// What the rows builder reads from the app.
struct rows_ctx_t<'a> {
    settings: &'a settings_t,
    theme: &'a theme_t,
    info: &'a runtime_info_t,
    entries: &'a [theme_entry_t],
    /// The command line turned animation off.
    animation_locked: bool,
    color: ColorMode,
    mouse: bool,
    contract: Option<u32>,
    prompt: Option<(&'a str, usize)>,
    confirm: Option<&'a PageConfirm>,
    preview: Option<&'a str>,
    compact: bool,
}

fn setting_lines(key: SettingKey, row: usize, width: usize, ctx: &rows_ctx_t<'_>) -> Vec<pline_t> {
    let theme = ctx.theme;
    let s = ctx.settings;
    let page = |a| Some(Action::Page(a));
    let mut head = pline_t::new(
        line_t::plain(
            key.label(),
            Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Some(row),
    );
    if !key.is_default(s) {
        let at = (head.line.width() + 2) as u16;
        head.line.push("  ", Style::new());
        head.line.push("↺ reset", Style::new().fg(theme.fg_faint));
        head.hits.push((
            at,
            str_width("↺ reset") as u16,
            Action::Page(PageAction::Reset(row)),
        ));
    }
    let chip_bg = theme.inset;
    let mut below: Vec<pline_t> = Vec::new();
    let control: Vec<chip_t> = match key.control() {
        Control::Toggle if key == SettingKey::Animation && ctx.animation_locked => vec![chip(
            "○ off · by command line",
            Style::new().fg(theme.fg_faint),
            None,
        )],
        Control::Toggle => {
            let on = key.toggle(s).unwrap_or(false);
            let (text, fg) = if on {
                (" ◉ on ", theme.success)
            } else {
                (" ○ off ", theme.fg_dim)
            };
            vec![chip(
                text,
                Style::new().fg(fg).bg(chip_bg),
                page(PageAction::Toggle(row)),
            )]
        }
        Control::Choice => {
            let current = key.choice(s);
            key.options()
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    if !key.option_enabled(i) {
                        chip(
                            format!(" {label} · command line only "),
                            Style::new().fg(theme.fg_faint).bg(chip_bg),
                            None,
                        )
                    } else if i == current {
                        chip(
                            format!(" {label} "),
                            Style::new()
                                .fg(theme.bg)
                                .bg(theme.agent)
                                .add_modifier(Modifier::BOLD),
                            page(PageAction::Pick(row, i)),
                        )
                    } else {
                        chip(
                            format!(" {label} "),
                            Style::new().fg(theme.fg_dim).bg(chip_bg),
                            page(PageAction::Pick(row, i)),
                        )
                    }
                })
                .collect()
        }
        Control::Theme => {
            // The swatch and name of the theme in use on the right, every theme as a chip below.
            let chips: Vec<chip_t> = ctx
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.palette.is_ok())
                .map(|(i, e)| {
                    let style = if e.id == s.theme {
                        Style::new()
                            .fg(theme.bg)
                            .bg(theme.agent)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().fg(theme.fg_dim).bg(chip_bg)
                    };
                    chip_t {
                        text: format!(" {} ", sanitize(&e.name, false)),
                        style,
                        action: page(PageAction::Pick(row, i)),
                        preview: Some(e.id.clone()),
                    }
                })
                .collect();
            below = flow(chips, width, row);
            Vec::new()
        }
    };
    let control_lines = if key.control() == Control::Theme {
        let active = ctx.entries.iter().find(|e| e.id == s.theme);
        let mut right = line_t::blank();
        if let Some(Ok(p)) = active.map(|e| &e.palette) {
            right = swatch(p, theme.inset);
            right.push(" ", Style::new());
        }
        right.push(
            sanitize(
                &active
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| s.theme.clone()),
                false,
            ),
            Style::new().fg(theme.fg),
        );
        vec![pline_t::new(right, Some(row))]
    } else {
        flow(control, width, row)
    };
    let mut out = Vec::new();
    match control_lines.len() {
        1 => match control_lines.into_iter().next() {
            Some(c) => out.extend(join_right(head, c, width)),
            None => out.push(head),
        },
        _ => {
            out.push(head);
            out.extend(control_lines);
        }
    }
    out.extend(below);
    out.extend(about_lines(key.about(), width, row, theme));
    out
}

/// The two buttons of a confirmation, on a tone of their own.
fn confirm_lines(
    question: &str,
    yes: &str,
    row: usize,
    width: usize,
    theme: &theme_t,
) -> Vec<pline_t> {
    let tone = mix(theme.bg, theme.error, 0.12);
    let mut out: Vec<pline_t> = wrap_plain(question, width, Style::new().fg(theme.fg))
        .into_iter()
        .take(3)
        .map(|l| pline_t::new(l, Some(row)))
        .collect();
    out.extend(flow(
        vec![
            chip(
                format!(" enter {yes} "),
                Style::new()
                    .fg(theme.bg)
                    .bg(theme.error)
                    .add_modifier(Modifier::BOLD),
                Some(Action::Page(PageAction::ConfirmYes)),
            ),
            chip(
                " n keep ",
                Style::new().fg(theme.fg).bg(theme.inset),
                Some(Action::Page(PageAction::ConfirmNo)),
            ),
        ],
        width,
        row,
    ));
    for l in out.iter_mut() {
        l.bg = Some(tone);
        l.rail = Some(theme.error);
    }
    out
}

fn theme_lines(
    entry: &theme_entry_t,
    row: usize,
    width: usize,
    ctx: &rows_ctx_t<'_>,
) -> Vec<pline_t> {
    let theme = ctx.theme;
    let name = sanitize(&entry.name, false);
    let mut head = match &entry.palette {
        Ok(p) => swatch(p, theme.inset),
        Err(_) => line_t::plain("· · · · · · · ", Style::new().fg(theme.fg_faint)),
    };
    head.push("  ", Style::new());
    head.push(
        truncate(&name, width.saturating_sub(16).max(4)),
        Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    if entry.imported {
        head.push("  imported", Style::new().fg(theme.composer_rail));
    }
    let head = pline_t::new(head, Some(row));
    let mut right = pline_t::new(line_t::blank(), Some(row));
    if ctx.preview == Some(entry.id.as_str()) && ctx.settings.theme != entry.id {
        right.line.push("◌ preview", Style::new().fg(theme.ice));
    } else if ctx.settings.theme == entry.id {
        right.line.push("● in use", Style::new().fg(theme.success));
    }
    if entry.imported {
        if !right.line.spans.is_empty() {
            right.line.push("  ", Style::new());
        }
        let at = right.line.width() as u16;
        right
            .line
            .push(" remove ", Style::new().fg(theme.fg_dim).bg(theme.inset));
        right.hits.push((
            at,
            str_width(" remove ") as u16,
            Action::Page(PageAction::RemoveTheme(entry.id.clone())),
        ));
    }
    let mut out = join_right(head, right, width);
    match &entry.palette {
        Ok(_) => {
            let detail = match &entry.extends {
                Some(base) => format!("{} · extends {}", entry.id, sanitize(base, false)),
                None => format!("{} · built-in", entry.id),
            };
            out.push(pline_t::new(
                line_t::plain(truncate(&detail, width), Style::new().fg(theme.fg_dim)),
                Some(row),
            ));
        }
        Err(err) => {
            for l in wrap_plain(&err.to_string(), width, Style::new().fg(theme.error))
                .into_iter()
                .take(3)
            {
                out.push(pline_t::new(l, Some(row)));
            }
        }
    }
    for l in out.iter_mut() {
        if entry.palette.is_ok() {
            l.action = Some(Action::Page(PageAction::ApplyTheme(entry.id.clone())));
            l.previews.push((0, 0, entry.id.clone()));
        } else {
            l.action = Some(Action::Page(PageAction::Row(row)));
        }
    }
    if ctx.confirm == Some(&PageConfirm::Remove(entry.id.clone())) {
        out.extend(confirm_lines(
            &format!("remove “{name}”? its saved copy is deleted; the file you imported stays"),
            "remove",
            row,
            width,
            theme,
        ));
    }
    out
}

fn import_lines(row: usize, width: usize, ctx: &rows_ctx_t<'_>) -> Vec<pline_t> {
    let theme = ctx.theme;
    let page = |a| Some(Action::Page(a));
    let mut out = Vec::new();
    match ctx.prompt {
        None => {
            let mut lines = flow(
                vec![chip(
                    " + import theme… ",
                    Style::new()
                        .fg(theme.fg)
                        .bg(theme.inset)
                        .add_modifier(Modifier::BOLD),
                    page(PageAction::Press(row)),
                )],
                width,
                row,
            );
            for l in lines.iter_mut() {
                l.action = page(PageAction::Press(row));
            }
            out.extend(lines);
            out.extend(about_lines(
                "a .json file with name, id, extends and colors (#rrggbb); see docs/themes/example.json",
                width,
                row,
                theme,
            ));
        }
        Some((text, cursor)) => {
            out.extend(
                wrap_plain(
                    "path to a theme file · type, paste or drop it · enter imports · esc cancels",
                    width,
                    Style::new().fg(theme.fg_dim).add_modifier(Modifier::ITALIC),
                )
                .into_iter()
                .take(2)
                .map(|l| pline_t::new(l, Some(row))),
            );
            let field = theme.inset;
            let mut input = line_t::plain("▸ ", Style::new().fg(theme.ice).bg(field));
            for span in
                crate::sidebar::prompt_line(text, cursor, width.saturating_sub(2), field, theme)
                    .spans
            {
                input.push(span.text, span.style);
            }
            let mut input = pline_t::new(input.with_fill(field), Some(row));
            input.rail = Some(theme.ice);
            out.push(input);
            out.extend(flow(
                vec![
                    chip(
                        " enter import ",
                        Style::new()
                            .fg(theme.bg)
                            .bg(theme.agent)
                            .add_modifier(Modifier::BOLD),
                        page(PageAction::PromptSubmit),
                    ),
                    chip(
                        " esc cancel ",
                        Style::new().fg(theme.fg).bg(theme.inset),
                        page(PageAction::ConfirmNo),
                    ),
                ],
                width,
                row,
            ));
        }
    }
    out
}

fn fact_lines(fact: Fact, row: usize, width: usize, ctx: &rows_ctx_t<'_>) -> Vec<pline_t> {
    let theme = ctx.theme;
    let info = ctx.info;
    let caps = &info.capabilities;
    let (label, value, tone) = match fact {
        Fact::Model => ("model", sanitize(&info.identity.id, false), theme.fg_dim),
        Fact::Backend => (
            "backend",
            format!(
                "{} backend{}",
                info.identity.kind.name(),
                if info.identity.simulated() {
                    " · simulated, not NCM output"
                } else {
                    ""
                }
            ),
            theme.fg_dim,
        ),
        Fact::Notice => match &info.notice {
            Some(n) => (
                "notice",
                format!("{} (always shown)", sanitize(n, false)),
                theme.approval,
            ),
            None => ("notice", "none".to_string(), theme.fg_faint),
        },
        Fact::State => (
            "state",
            info.state_note
                .as_ref()
                .map(|n| sanitize(n, false))
                .unwrap_or_else(|| "nothing is saved in this run".to_string()),
            theme.fg_dim,
        ),
        Fact::Contract => (
            "contract",
            ctx.contract
                .map(|v| format!("backend contract v{v}"))
                .unwrap_or_else(|| "not reported yet (no session start)".to_string()),
            theme.fg_dim,
        ),
        Fact::Shape => (
            "window",
            format!(
                "P {} · L {} · K {} · {}",
                caps.prefix_window,
                caps.reply_len,
                caps.level_count,
                if caps.supports_drafts {
                    "drafts"
                } else {
                    "no drafts"
                }
            ),
            theme.fg_dim,
        ),
        Fact::Colors => (
            "colours",
            match ctx.color {
                ColorMode::TrueColor => "24-bit (COLORTERM is truecolor)".to_string(),
                ColorMode::Indexed256 => {
                    "256-colour palette (COLORTERM is not truecolor)".to_string()
                }
            },
            theme.fg_dim,
        ),
        Fact::Mouse => (
            "mouse",
            if ctx.mouse {
                "on".to_string()
            } else {
                "off (--no-mouse)".to_string()
            },
            theme.fg_dim,
        ),
    };
    let label_w = 10usize;
    let room = width.saturating_sub(label_w).max(8);
    let mut out = Vec::new();
    for (i, l) in wrap_plain(&value, room, Style::new().fg(tone))
        .into_iter()
        .take(3)
        .enumerate()
    {
        let mut line = line_t::plain(
            if i == 0 {
                format!("{label:<label_w$}")
            } else {
                " ".repeat(label_w)
            },
            Style::new().fg(theme.fg_faint),
        );
        for span in l.spans {
            line.push(span.text, span.style);
        }
        out.push(pline_t::new(line, Some(row)));
    }
    out
}

fn reset_all_lines(row: usize, width: usize, ctx: &rows_ctx_t<'_>) -> Vec<pline_t> {
    let theme = ctx.theme;
    let n = changed_count(ctx.settings);
    let head = pline_t::new(
        line_t::plain(
            if n == 0 {
                "every setting is at its default".to_string()
            } else {
                format!("{n} setting(s) differ from the defaults")
            },
            Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Some(row),
    );
    let button = flow(
        vec![chip(
            " reset all settings… ",
            Style::new()
                .fg(if n == 0 { theme.fg_faint } else { theme.fg })
                .bg(theme.inset),
            Some(Action::Page(PageAction::Press(row))),
        )],
        width,
        row,
    );
    let mut out = Vec::new();
    match button.into_iter().next() {
        Some(b) => out.extend(join_right(head, b, width)),
        None => out.push(head),
    }
    out.extend(about_lines(
        "the theme too; imported themes and the open tabs stay",
        width,
        row,
        theme,
    ));
    if ctx.confirm == Some(&PageConfirm::ResetAll) {
        out.extend(confirm_lines(
            "reset every setting to its default?",
            "reset",
            row,
            width,
            theme,
        ));
    }
    out
}

/// Every line of the rows pane for `section`: the heading, then each row and the space after
/// it. Returns the lines and the first and last line of each row.
fn section_lines(
    section: Section,
    rows: &[PageRow],
    width: usize,
    ctx: &rows_ctx_t<'_>,
) -> (Vec<pline_t>, Vec<(usize, usize)>) {
    let theme = ctx.theme;
    let mut out = vec![pline_t::new(
        line_t::plain(
            section.label(),
            Style::new().fg(theme.ice).add_modifier(Modifier::BOLD),
        ),
        None,
    )];
    for l in wrap_plain(section.about(), width, Style::new().fg(theme.fg_faint))
        .into_iter()
        .take(2)
    {
        out.push(pline_t::new(l, None));
    }
    out.push(pline_t::new(line_t::blank(), None));
    let mut ranges = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let start = out.len();
        let lines = match row {
            PageRow::Setting(key) => setting_lines(*key, i, width, ctx),
            PageRow::Theme(id) => match ctx.entries.iter().find(|e| e.id == *id) {
                Some(entry) => theme_lines(entry, i, width, ctx),
                None => Vec::new(),
            },
            PageRow::Import => import_lines(i, width, ctx),
            PageRow::Fact(f) => fact_lines(*f, i, width, ctx),
            PageRow::ResetAll => reset_all_lines(i, width, ctx),
        };
        out.extend(lines);
        let end = out.len().saturating_sub(1).max(start);
        ranges.push((start, end));
        // Facts sit close together; other rows get a blank line unless the density is compact.
        let next_is_fact = matches!(rows.get(i + 1), Some(PageRow::Fact(_)));
        let fact = matches!(row, PageRow::Fact(_));
        if !ctx.compact && !(fact && next_is_fact) {
            out.push(pline_t::new(line_t::blank(), None));
        }
    }
    (out, ranges)
}

/// The section list: a column from `SplitWidth`, else a strip of chips. Returns the rows it
/// took (for the strip).
fn draw_sections(
    app: &mut app_t,
    buf: &mut Buffer,
    area: Rect,
    split: bool,
    theme: &theme_t,
) -> u16 {
    let Some(page) = app.page.as_ref() else {
        return 0;
    };
    let current = page.section;
    let list_focused = app.focus == Focus::Settings && page.pane == Pane::List;
    let hover = app.hover;
    let over = |r: Rect| hover.is_some_and(|(x, y)| r.contains(Position::new(x, y)));
    if split {
        let col = Rect::new(area.x, area.y, ListWidth.min(area.width), area.height);
        fill(buf, col, Style::new().bg(theme.side));
        let x0 = col.x.saturating_add(2);
        let x1 = col.right().saturating_sub(1);
        put_str(
            buf,
            x0,
            col.y.saturating_add(1),
            x1,
            "⚙ settings",
            Style::new()
                .fg(theme.fg)
                .bg(theme.side)
                .add_modifier(Modifier::BOLD),
        );
        for (i, section) in Sections.iter().enumerate() {
            let y = col.y.saturating_add(3 + i as u16);
            if y >= col.bottom() {
                break;
            }
            let rect = Rect::new(col.x, y, col.width, 1);
            let selected = *section == current;
            let bg = match (selected, list_focused, over(rect)) {
                (true, true, _) => theme.select,
                (true, false, _) => theme.hover,
                (false, _, true) => theme.hover,
                _ => theme.side,
            };
            fill(buf, rect, Style::new().bg(bg));
            if selected {
                let rail = if list_focused { theme.ice } else { theme.agent };
                put_str(buf, col.x, y, x1, "▌", Style::new().fg(rail).bg(bg));
            }
            let fg = if selected { theme.fg } else { theme.fg_dim };
            let mut st = Style::new().fg(fg).bg(bg);
            if selected {
                st = st.add_modifier(Modifier::BOLD);
            }
            put_str(buf, x0, y, x1, section.label(), st);
            put_str_right(
                buf,
                x1,
                y,
                x0,
                &format!("{}", i + 1),
                Style::new().fg(theme.fg_faint).bg(bg),
            );
            app.hits
                .add(rect, Action::Page(PageAction::Section(*section)));
        }
        let foot = col.bottom().saturating_sub(2);
        if foot > col.y.saturating_add(3 + Sections.len() as u16) {
            put_str(
                buf,
                x0,
                foot,
                x1,
                "saved as you change",
                Style::new().fg(theme.fg_faint).bg(theme.side),
            );
        }
        return 0;
    }
    // The strip: the title, then one chip per section, wrapping.
    let x0 = area.x.saturating_add(1);
    let x1 = area.right().saturating_sub(1);
    let mut y = area.y;
    let mut x = put_str(
        buf,
        x0,
        y,
        x1,
        "⚙ settings ",
        Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    for section in Sections {
        let text = format!(" {} ", section.label());
        let w = str_width(&text) as u16;
        if x.saturating_add(w) > x1 {
            y = y.saturating_add(1);
            x = x0;
        }
        if y >= area.bottom() {
            break;
        }
        let rect = Rect::new(x, y, w, 1);
        let selected = section == current;
        let style = match (selected, list_focused, over(rect)) {
            (true, true, _) => Style::new()
                .fg(theme.bg)
                .bg(theme.ice)
                .add_modifier(Modifier::BOLD),
            (true, false, _) => Style::new()
                .fg(theme.fg)
                .bg(theme.select)
                .add_modifier(Modifier::BOLD),
            (false, _, true) => Style::new().fg(theme.fg).bg(theme.hover),
            _ => Style::new().fg(theme.fg_dim).bg(theme.inset),
        };
        put_str(buf, x, y, x1, &text, style);
        app.hits
            .add(rect, Action::Page(PageAction::Section(section)));
        x = x.saturating_add(w + 1);
    }
    y.saturating_sub(area.y).saturating_add(2)
}

/// Draw the settings page into `area`, the main panel under the tab bar.
pub fn draw_settings_page(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    if area.width < 20 || area.height < 4 {
        return;
    }
    let theme = app.theme.clone();
    fill(buf, area, Style::new().bg(theme.bg).fg(theme.fg));
    app.hits.add(area, Action::SetFocus(Focus::Settings));
    let split = area.width >= SplitWidth;
    let strip = draw_sections(app, buf, area, split, &theme);
    let pane = if split {
        let x = area.x.saturating_add(ListWidth + 3);
        Rect::new(
            x,
            area.y.saturating_add(1),
            area.right().saturating_sub(x).saturating_sub(2),
            area.height.saturating_sub(1),
        )
    } else {
        Rect::new(
            area.x.saturating_add(3),
            area.y.saturating_add(strip),
            area.width.saturating_sub(5),
            area.height.saturating_sub(strip),
        )
    };
    if pane.width < 8 || pane.height == 0 {
        return;
    }
    let Some(page) = app.page.as_ref() else {
        return;
    };
    let rows = crate::settings_page::page_rows(page.section, &app.themes);
    let entries = app.themes.entries();
    let preview = app.preview_theme();
    let ctx = rows_ctx_t {
        settings: &app.settings,
        theme: &theme,
        info: &app.info,
        entries: &entries,
        animation_locked: !app.options.animate,
        color: app.options.color,
        mouse: app.options.mouse,
        contract: app
            .active()
            .and_then(|s| s.transcript.meta.as_ref())
            .map(|m| m.contract_version),
        prompt: page.prompt.as_ref().map(|p| (p.text(), p.cursor())),
        confirm: page.confirm.as_ref(),
        preview: preview.as_deref(),
        compact: app.settings.density == Density::Compact,
    };
    let width = usize::from(pane.width);
    let (lines, ranges) = section_lines(page.section, &rows, width, &ctx);
    let view = usize::from(pane.height);
    let cursor = page.cursor;
    let rows_focused = app.focus == Focus::Settings && page.pane == Pane::Rows;
    let mut scroll = page.scroll;
    if page.follow
        && let Some((lo, hi)) = ranges.get(cursor).copied()
    {
        // The heading stays in view with the first row.
        let lo = if cursor == 0 { 0 } else { lo };
        if lo < scroll {
            scroll = lo;
        } else if hi >= scroll + view {
            scroll = (hi + 1).saturating_sub(view).min(lo);
        }
    }
    scroll = scroll.min(lines.len().saturating_sub(view));
    let hover = app.hover;
    let hovered_row = hover.and_then(|(hx, hy)| {
        let inside = hx >= pane.x.saturating_sub(2)
            && hx < pane.right()
            && hy >= pane.y
            && hy < pane.bottom();
        inside
            .then(|| lines.get(scroll + usize::from(hy - pane.y)))
            .flatten()
            .and_then(|l| l.row)
    });
    let mut preview_areas = Vec::new();
    let mut hits: Vec<(Rect, Action)> = Vec::new();
    for (k, line) in lines.iter().skip(scroll).take(view).enumerate() {
        let y = pane.y.saturating_add(k as u16);
        let band = Rect::new(pane.x.saturating_sub(2), y, pane.width.saturating_add(3), 1);
        let is_cursor =
            line.row == Some(cursor) && rows.get(cursor).is_some_and(PageRow::focusable);
        let bg = match (
            line.bg,
            is_cursor && rows_focused,
            line.row.is_some() && line.row == hovered_row,
        ) {
            (Some(bg), _, _) => bg,
            (None, true, _) => theme.select,
            (None, false, true) => theme.hover,
            _ => theme.bg,
        };
        if bg != theme.bg {
            fill(buf, band, Style::new().bg(bg));
        }
        let rail = match line.rail {
            Some(c) => Some(c),
            None if is_cursor && rows_focused => Some(theme.ice),
            None if is_cursor => Some(theme.fg_faint),
            None => None,
        };
        if let Some(c) = rail {
            put_str(buf, band.x, y, band.right(), "▌", Style::new().fg(c).bg(bg));
        }
        put_line(
            buf,
            pane.x,
            y,
            pane.right(),
            &line.line,
            Style::new().bg(bg),
        );
        if let Some(row) = line.row {
            let action = line
                .action
                .clone()
                .unwrap_or(Action::Page(PageAction::Row(row)));
            hits.push((band, action));
        }
        for (x, w, id) in &line.previews {
            let rect = if *w == 0 {
                band
            } else {
                Rect::new(pane.x.saturating_add(*x), y, *w, 1)
            };
            preview_areas.push((rect.intersection(pane.union(band)), id.clone()));
        }
        for (x, w, action) in &line.hits {
            let rect = Rect::new(pane.x.saturating_add(*x), y, *w, 1).intersection(band);
            if hover.is_some_and(|(hx, hy)| rect.contains(Position::new(hx, hy))) {
                lift(buf, rect, &theme, action);
            }
            hits.push((rect, action.clone()));
        }
    }
    // A note when rows are above or below the view.
    if scroll > 0 {
        put_str_right(
            buf,
            pane.right(),
            pane.y,
            pane.x,
            " ↑ more ",
            Style::new().fg(theme.fg_faint).bg(theme.inset),
        );
    }
    if lines.len() > scroll + view {
        put_str_right(
            buf,
            pane.right(),
            pane.bottom().saturating_sub(1),
            pane.x,
            " ↓ more ",
            Style::new().fg(theme.fg_faint).bg(theme.inset),
        );
    }
    for (rect, action) in hits {
        app.hits.add(rect, action);
    }
    if let Some(page) = app.page.as_mut() {
        page.scroll = scroll;
        page.preview_areas = preview_areas;
    }
}

/// The hover lift of a chip: one tone step up, and an error tint for a destructive one.
fn lift(buf: &mut Buffer, rect: Rect, theme: &theme_t, action: &Action) {
    let destructive = matches!(action, Action::Page(PageAction::RemoveTheme(_)));
    for x in rect.left()..rect.right() {
        if let Some(cell) = buf.cell_mut((x, rect.y)) {
            if destructive {
                cell.set_fg(theme.error);
            } else if cell.fg == theme.fg_dim || cell.fg == theme.fg_faint {
                cell.set_fg(theme.fg);
            }
            if cell.bg == theme.inset {
                cell.set_bg(theme.select);
            }
            cell.modifier.insert(Modifier::UNDERLINED);
        }
    }
}
