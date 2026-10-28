/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Drawing the context menu and the delete dialog (PROJECT.md, D59, D61).
//!
//! Both are tone panels with a left rail and no lines. The menu opens at the pointer and stays
//! on screen: it flips left near the right edge and up near the bottom. A submenu opens as a
//! second panel to the right, or to the left when there is no room; when neither fits (a narrow
//! terminal), it opens inline under its item.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::app::app_t;
use crate::canvas::{fill, put_str, put_str_right};
use crate::conversation::delete_plan_t;
use crate::input::{Action, MenuAction, hit_registry_t};
use crate::menu::{menu_item_t, menu_t};
use crate::text::{str_width, truncate, wrap_plain};
use crate::theme::{mix, theme_t};

const MenuMinWidth: u16 = 20;
const MenuMaxWidth: u16 = 40;
const SubMinWidth: u16 = 14;
const SubMaxWidth: u16 = 30;

/// Where the menu and its submenu go on a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct menu_geometry_t {
    pub main: Rect,
    /// The submenu panel, when it opens beside the menu.
    pub sub: Option<Rect>,
    /// The submenu opens inside the menu, under its item.
    pub inline: bool,
}

/// Columns an item needs: rail, space, mark, space, label, gap, hint, arrow, space.
fn item_width(item: &menu_item_t) -> u16 {
    let hint = str_width(&item.hint);
    (4 + str_width(&item.label) + if hint > 0 { hint + 2 } else { 1 } + 2) as u16
}

fn panel_width(items: &[menu_item_t], caption: &str, min: u16, max: u16, screen: Rect) -> u16 {
    let widest = items
        .iter()
        .map(item_width)
        .max()
        .unwrap_or(0)
        .max(str_width(caption) as u16 + 4);
    widest.clamp(min, max).min(screen.width)
}

/// Place a panel of `w` by `h` at `at`, flipping left and up near the edges and clamping it
/// to the screen.
fn place(at: (u16, u16), w: u16, h: u16, screen: Rect) -> Rect {
    let h = h.min(screen.height);
    let right = screen.right();
    let bottom = screen.bottom();
    let mut x = at.0;
    if x.saturating_add(w) > right {
        x = at.0.saturating_add(1).saturating_sub(w);
    }
    let x = x.clamp(screen.x, right.saturating_sub(w).max(screen.x));
    let mut y = at.1;
    if y.saturating_add(h) > bottom {
        y = at.1.saturating_add(1).saturating_sub(h);
    }
    let y = y.clamp(screen.y, bottom.saturating_sub(h).max(screen.y));
    Rect::new(x, y, w, h)
}

/// The geometry of a menu on a screen.
pub fn menu_geometry(menu: &menu_t, screen: Rect) -> menu_geometry_t {
    let w = panel_width(
        &menu.items,
        &menu.caption,
        MenuMinWidth,
        MenuMaxWidth,
        screen,
    );
    let h = 1 + menu.items.len() as u16;
    let main = place(menu.at, w, h, screen);
    let Some(parent) = menu.sub else {
        return menu_geometry_t {
            main,
            sub: None,
            inline: false,
        };
    };
    let sub_items = menu
        .items
        .get(parent)
        .map(|i| i.sub.as_slice())
        .unwrap_or(&[]);
    let sw = panel_width(sub_items, "", SubMinWidth, SubMaxWidth, screen);
    let sh = (sub_items.len() as u16).min(screen.height);
    let row = main.y.saturating_add(1 + parent as u16);
    let sy = row.min(screen.bottom().saturating_sub(sh)).max(screen.y);
    if main.right().saturating_add(sw) <= screen.right() {
        return menu_geometry_t {
            main,
            sub: Some(Rect::new(main.right(), sy, sw, sh)),
            inline: false,
        };
    }
    if main.x >= screen.x.saturating_add(sw) {
        return menu_geometry_t {
            main,
            sub: Some(Rect::new(main.x - sw, sy, sw, sh)),
            inline: false,
        };
    }
    let main = place(menu.at, w, h.saturating_add(sub_items.len() as u16), screen);
    menu_geometry_t {
        main,
        sub: None,
        inline: true,
    }
}

/// One drawn row of a panel.
struct row_t<'a> {
    item: &'a menu_item_t,
    action: Action,
    highlighted: bool,
    /// An item of a submenu shown inline: indented, on a deeper tone.
    nested: bool,
    /// The item's submenu is open.
    open: bool,
}

/// A panel to draw: where, its caption line, and its tone.
struct panel_t<'a> {
    rect: Rect,
    caption: Option<&'a str>,
    tone: Color,
}

fn draw_panel(
    buf: &mut Buffer,
    panel: &panel_t<'_>,
    rows: &[row_t<'_>],
    hover: Option<(u16, u16)>,
    theme: &theme_t,
    hits: &mut hit_registry_t,
) {
    let (rect, caption, tone) = (panel.rect, panel.caption, panel.tone);
    fill(buf, rect, Style::new().bg(tone));
    let mut y = rect.y;
    let right = rect.right();
    if let Some(caption) = caption {
        let cap_bg = mix(tone, theme.bg, 0.3);
        fill(
            buf,
            Rect::new(rect.x, y, rect.width, 1),
            Style::new().bg(cap_bg),
        );
        put_str(
            buf,
            rect.x,
            y,
            right,
            "▌",
            Style::new().fg(mix(cap_bg, theme.ice, 0.35)).bg(cap_bg),
        );
        put_str(
            buf,
            rect.x.saturating_add(2),
            y,
            right.saturating_sub(1),
            &truncate(caption, usize::from(rect.width.saturating_sub(3))),
            Style::new()
                .fg(theme.fg_dim)
                .bg(cap_bg)
                .add_modifier(Modifier::ITALIC),
        );
        y = y.saturating_add(1);
    }
    for row in rows {
        if y >= rect.bottom() {
            break;
        }
        let line = Rect::new(rect.x, y, rect.width, 1);
        let over = hover.is_some_and(|(hx, hy)| line.contains(Position::new(hx, hy)));
        let base = if row.nested {
            mix(tone, theme.select, 0.45)
        } else {
            tone
        };
        let bg = if row.highlighted {
            theme.select
        } else if over && row.item.enabled() {
            mix(base, theme.fg, 0.07)
        } else {
            base
        };
        let st = Style::new().bg(bg);
        fill(buf, line, st);
        let rail = if row.highlighted {
            theme.ice
        } else {
            mix(bg, theme.ice, 0.35)
        };
        put_str(buf, rect.x, y, right, "▌", st.fg(rail));
        let indent: u16 = if row.nested { 2 } else { 0 };
        let mark_x = rect.x.saturating_add(2 + indent);
        if let Some(color) = row.item.swatch {
            put_str(buf, mark_x, y, right, "●", st.fg(color));
        } else if row.item.checked {
            put_str(buf, mark_x, y, right, "✓", st.fg(theme.ice));
        }
        let fg = if !row.item.enabled() {
            theme.fg_faint
        } else if row.item.danger {
            theme.error
        } else {
            theme.fg
        };
        let end = right.saturating_sub(2);
        if row.item.checked && row.item.swatch.is_some() {
            // A colour swatch takes the mark column, so the current colour is ticked at the end.
            put_str(buf, end, y, right, "✓", st.fg(theme.ice));
        }
        let hint_x = if row.item.hint.is_empty() {
            end
        } else {
            put_str_right(
                buf,
                end.saturating_sub(1),
                y,
                mark_x.saturating_add(4),
                &row.item.hint,
                st.fg(theme.fg_faint),
            )
        };
        let label_x = mark_x.saturating_add(2);
        let label_room = usize::from(hint_x.saturating_sub(label_x).saturating_sub(1));
        let label_style = if row.highlighted {
            st.fg(fg).add_modifier(Modifier::BOLD)
        } else {
            st.fg(fg)
        };
        put_str(
            buf,
            label_x,
            y,
            hint_x,
            &truncate(&row.item.label, label_room),
            label_style,
        );
        if !row.item.sub.is_empty() {
            let arrow = match (row.open, rows.iter().any(|r| r.nested)) {
                (true, true) => "▾",
                (false, _) | (true, false) => "›",
            };
            put_str(buf, end, y, right, arrow, st.fg(theme.fg_dim));
        }
        hits.add(line, row.action.clone());
        y = y.saturating_add(1);
    }
}

/// Draw the open menu and record where it is.
pub fn draw_menu(app: &mut app_t, buf: &mut Buffer, screen: Rect) {
    let Some(menu) = app.menu.clone() else {
        return;
    };
    let theme = app.theme.clone();
    let geo = menu_geometry(&menu, screen);
    let tone = theme.inset;
    let mut rows: Vec<row_t<'_>> = Vec::new();
    for (i, item) in menu.items.iter().enumerate() {
        let open = menu.sub == Some(i);
        rows.push(row_t {
            item,
            action: Action::Menu(MenuAction::Pick(i)),
            highlighted: i == menu.sel && menu.sub.is_none(),
            nested: false,
            open,
        });
        if open && geo.inline {
            for (j, sub) in item.sub.iter().enumerate() {
                rows.push(row_t {
                    item: sub,
                    action: Action::Menu(MenuAction::PickSub(j)),
                    highlighted: j == menu.sub_sel,
                    nested: true,
                    open: false,
                });
            }
        }
    }
    let main = panel_t {
        rect: geo.main,
        caption: Some(&menu.caption),
        tone,
    };
    draw_panel(buf, &main, &rows, app.hover, &theme, &mut app.hits);
    app.overlay_areas.push(geo.main);
    if let (Some(rect), Some(parent)) = (geo.sub, menu.sub.and_then(|p| menu.items.get(p))) {
        let rows: Vec<row_t<'_>> = parent
            .sub
            .iter()
            .enumerate()
            .map(|(j, item)| row_t {
                item,
                action: Action::Menu(MenuAction::PickSub(j)),
                highlighted: j == menu.sub_sel,
                nested: false,
                open: false,
            })
            .collect();
        let sub = panel_t {
            rect,
            caption: None,
            tone: mix(tone, theme.select, 0.45),
        };
        draw_panel(buf, &sub, &rows, app.hover, &theme, &mut app.hits);
        app.overlay_areas.push(rect);
    }
}

/// Blend the text of `area` toward `toward`, keeping its tones.
fn fade(buf: &mut Buffer, area: Rect, toward: Color, t: f32) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.fg = mix(cell.fg, toward, t);
            }
        }
    }
}

/// The rows of the delete dialog for a width: the question, the details, and the buttons.
fn dialog_lines(plan: &delete_plan_t, width: usize, theme: &theme_t) -> Vec<crate::text::line_t> {
    let mut lines = wrap_plain(
        &plan.question,
        width,
        Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    for detail in &plan.details {
        lines.extend(wrap_plain(detail, width, Style::new().fg(theme.fg_dim)));
    }
    lines
}

/// Draw the delete dialog in the middle of `area` and record where it is (D59).
pub fn draw_delete_dialog(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    let Some(plan) = app.delete_plan.clone() else {
        return;
    };
    let theme = app.theme.clone();
    let w = area.width.saturating_sub(4).min(56);
    if w < 16 || area.height < 6 {
        return;
    }
    let inner = usize::from(w.saturating_sub(5));
    let lines = dialog_lines(&plan, inner, &theme);
    let h = (lines.len() as u16 + 4).min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect::new(x, y, w, h);
    // The panel behind the dialog fades, so the question stands alone without a frame.
    fade(buf, area, theme.bg, 0.6);
    let tone = mix(theme.dock, theme.error, 0.10);
    fill(buf, rect, Style::new().bg(tone));
    for row in rect.top()..rect.bottom() {
        put_str(
            buf,
            rect.x,
            row,
            rect.right(),
            "▌",
            Style::new().fg(theme.error).bg(tone),
        );
    }
    let x0 = rect.x.saturating_add(3);
    let right = rect.right().saturating_sub(2);
    let mut ty = rect.y.saturating_add(1);
    for line in lines.iter().take(usize::from(h.saturating_sub(4))) {
        crate::canvas::put_line(buf, x0, ty, right, line, Style::new().bg(tone));
        ty = ty.saturating_add(1);
    }
    let by = rect.bottom().saturating_sub(2);
    let yes = " enter delete ";
    let no = " esc cancel ";
    let yes_rect = Rect::new(x0, by, str_width(yes) as u16, 1);
    let no_rect = Rect::new(
        yes_rect.right().saturating_add(1),
        by,
        str_width(no) as u16,
        1,
    );
    let hover = |r: Rect| {
        app.hover
            .is_some_and(|(hx, hy)| r.contains(Position::new(hx, hy)))
    };
    let yes_style = Style::new()
        .fg(theme.bg)
        .bg(theme.error)
        .add_modifier(Modifier::BOLD);
    put_str(
        buf,
        yes_rect.x,
        by,
        right,
        yes,
        if hover(yes_rect) {
            yes_style.add_modifier(Modifier::UNDERLINED)
        } else {
            yes_style
        },
    );
    let no_bg = if hover(no_rect) {
        theme.select
    } else {
        theme.inset
    };
    put_str(
        buf,
        no_rect.x,
        by,
        right,
        no,
        Style::new().fg(theme.fg).bg(no_bg),
    );
    app.hits.add(yes_rect, Action::ConfirmYes);
    app.hits.add(no_rect, Action::SidebarCancel);
    app.overlay_areas.push(rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_flips_left_and_up_near_the_edges_and_stays_on_screen() {
        let screen = Rect::new(0, 0, 80, 24);
        assert_eq!(place((10, 5), 20, 6, screen), Rect::new(10, 5, 20, 6));
        // Near the right edge it ends at the pointer.
        assert_eq!(place((75, 5), 20, 6, screen), Rect::new(56, 5, 20, 6));
        // Near the bottom it ends on the pointer row.
        assert_eq!(place((10, 22), 20, 6, screen), Rect::new(10, 17, 20, 6));
        // Bigger than the screen: clamped to it.
        let huge = place((3, 3), 200, 100, screen);
        assert_eq!((huge.x, huge.y, huge.height), (0, 0, 24));
        for at in [(0, 0), (79, 23), (40, 12), (u16::MAX, u16::MAX)] {
            let r = place(at, 30, 10, screen);
            assert!(r.right() <= 80 && r.bottom() <= 24, "{at:?} -> {r:?}");
        }
    }
}
