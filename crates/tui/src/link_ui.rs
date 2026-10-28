/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Using a link in the transcript (PROJECT.md, D76, D77): the confirmation before it opens,
//! copying it, the keyboard's link cursor in navigate mode, and the `/links` switches.
//!
//! Nothing opens without the confirmation, which shows the whole address with its host in
//! bold. There `enter` or `y` opens it through the `link_opener_t`, `c` copies it (OSC 52) and
//! `esc` or `n` cancels. A failed open is an error toast and the address is copied instead.

use harness::settings::LinkAddresses;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};

use crate::app::{ToastKind, app_t};
use crate::canvas::{fill, put_line, put_str};
use crate::clipboard::osc52;
use crate::hyperlink::link_runs;
use crate::input::{Action, Focus};
use crate::links::{host_of, middle_truncate, safe_url};
use crate::text::{hard_wrap, line_t, str_width};
use crate::theme::mix;

/// What the link keys and clicks do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// A click on link text: the confirmation for this address. Keys: navigate mode, `[` `]`
    /// to the link, then `enter`.
    Choose(String),
    /// `]` `[` in navigate mode: the next or previous link of the selected block.
    Step(i32),
    /// `enter` in navigate mode while a link is chosen: its confirmation.
    OpenActive,
    /// `y` in navigate mode: copy the chosen link without the confirmation.
    CopyActive,
    /// The confirmation's choices: `enter` or `y`, `c`, `esc` or `n`, or their chips.
    Open,
    Copy,
    Cancel,
}

/// The `/links` switches (D77, D78). Each one changes and saves a link setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinksSwitch {
    /// Write links as terminal hyperlinks (OSC 8): `link_hyperlinks` on.
    Osc8,
    /// Do not: `link_hyperlinks` off.
    Off,
    /// Show every address after its label: `link_addresses` inline.
    Inline,
    /// Hide the address when the label is safe (the default): `link_addresses` hidden.
    Hidden,
}

impl app_t {
    /// The links of a transcript block, in order: (address, index).
    pub fn block_links(&self, block: u64) -> Vec<String> {
        let Some(live) = self.active() else {
            return Vec::new();
        };
        let Some(rows) = live.view.rows_of(block) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for run in link_runs(rows.rows.iter().map(|r| &r.line)) {
            if out.len() <= run.index {
                out.push(run.url);
            }
        }
        out
    }

    /// The address of the keyboard's chosen link, when it is in the selected block.
    pub fn active_link(&self) -> Option<String> {
        let (block, index) = self.link_cursor?;
        let selected = self.active().and_then(|s| s.selected_block)?;
        if block != selected || self.focus != Focus::Transcript {
            return None;
        }
        self.block_links(block).get(index).cloned()
    }

    /// The address under the pointer in the last frame.
    pub fn hovered_link(&self) -> Option<String> {
        let (x, y) = self.hover?;
        self.links
            .iter()
            .find(|l| l.rect.contains(Position::new(x, y)))
            .map(|l| l.url.clone())
    }

    /// The address the status line shows: the hovered one, else the chosen one.
    pub fn shown_link(&self) -> Option<String> {
        self.hovered_link().or_else(|| self.active_link())
    }

    fn copy_link(&mut self, url: &str) {
        self.outputs.push(osc52(url));
        let room = usize::from(self.size.0.saturating_sub(24)).max(16);
        self.say(
            format!("link copied: {}", middle_truncate(url, room)),
            ToastKind::Info,
        );
    }

    pub(crate) fn apply_link(&mut self, action: LinkAction) {
        match action {
            LinkAction::Choose(url) => match safe_url(&url) {
                Some(url) => {
                    self.menu = None;
                    self.link_dialog = Some(url);
                }
                None => self.say("that address is not an http(s) link", ToastKind::Warning),
            },
            LinkAction::Step(d) => self.step_link(d),
            LinkAction::OpenActive => {
                if let Some(url) = self.active_link() {
                    self.link_dialog = Some(url);
                }
            }
            LinkAction::CopyActive => match self.active_link() {
                Some(url) => self.copy_link(&url),
                None => self.say("no link chosen: ] picks the next one", ToastKind::Info),
            },
            LinkAction::Open => {
                if let Some(url) = self.link_dialog.take() {
                    match self.opener.open(&url) {
                        Ok(()) => self.say(
                            format!("opening {} in your browser", host_of(&url)),
                            ToastKind::Info,
                        ),
                        Err(err) => {
                            self.outputs.push(osc52(&url));
                            self.say(
                                format!("could not open the link: {err}; copied it instead"),
                                ToastKind::Error,
                            );
                        }
                    }
                }
            }
            LinkAction::Copy => {
                if let Some(url) = self.link_dialog.take() {
                    self.copy_link(&url);
                }
            }
            LinkAction::Cancel => self.link_dialog = None,
        }
    }

    /// Move the link cursor of the selected block. Past either end the cursor leaves the
    /// links, so `enter` toggles the block again.
    fn step_link(&mut self, d: i32) {
        let Some(block) = self.active().and_then(|s| s.selected_block) else {
            return;
        };
        let n = self.block_links(block).len();
        if n == 0 {
            self.link_cursor = None;
            self.say("this block has no link", ToastKind::Info);
            return;
        }
        let at = match self.link_cursor {
            Some((b, i)) if b == block => Some(i),
            _ => None,
        };
        let next = match (at, d > 0) {
            (None, true) => Some(0),
            (None, false) => Some(n - 1),
            (Some(i), true) if i + 1 < n => Some(i + 1),
            (Some(i), false) if i > 0 => Some(i - 1),
            _ => None,
        };
        self.link_cursor = next.map(|i| (block, i));
    }

    /// `/links` with no argument reports both link settings in one toast. With a switch it saves
    /// that setting through `change_setting`, as the settings page does, so a refused save is
    /// the page's toast and the value stays as it was (D78).
    pub(crate) fn links_command(&mut self, switch: Option<LinksSwitch>) {
        let Some(switch) = switch else {
            let hyperlinks = if self.settings.link_hyperlinks {
                "on"
            } else {
                "off"
            };
            let addresses = match self.settings.link_addresses {
                LinkAddresses::Hidden => "hidden",
                LinkAddresses::Inline => "inline",
            };
            self.say(
                format!("links: terminal hyperlinks {hyperlinks}, addresses {addresses}"),
                ToastKind::Info,
            );
            return;
        };
        let (saved, text) = match switch {
            LinksSwitch::Osc8 => (
                self.change_setting(|s| s.link_hyperlinks = true),
                "links are terminal hyperlinks (OSC 8)",
            ),
            LinksSwitch::Off => (
                self.change_setting(|s| s.link_hyperlinks = false),
                "links are plain text",
            ),
            LinksSwitch::Inline => (
                self.change_setting(|s| s.link_addresses = LinkAddresses::Inline),
                "every link shows its address",
            ),
            LinksSwitch::Hidden => (
                self.change_setting(|s| s.link_addresses = LinkAddresses::Hidden),
                "addresses show only when a label looks like another address",
            ),
        };
        if saved {
            self.say(format!("{text} (saved)"), ToastKind::Info);
        }
    }

    /// Brings the hyperlink overlay up to date after a change of the settings, given whether
    /// hyperlinks were on before it (D77). Hyperlinks coming back on are written again over the
    /// next frame. Hyperlinks going off get one full redraw, so no cell keeps a link from before.
    pub(crate) fn links_changed(&mut self, were_on: bool) {
        match (were_on, self.settings.link_hyperlinks) {
            (true, false) => self.redraw = true,
            (false, true) => self.link_overlay.reset(),
            _ => {}
        }
    }

    /// The bytes that turn this frame's visible links into terminal hyperlinks. Empty while the
    /// `link_hyperlinks` setting is off (D77).
    pub fn link_bytes(&mut self, buf: &Buffer) -> Vec<u8> {
        self.link_overlay.emit(
            buf,
            &self.links,
            &self.covers,
            self.settings.link_hyperlinks,
        )
    }
}

/// The confirmation's lines: the question, the host in bold, the whole address wrapped.
fn dialog_lines(
    url: &str,
    width: usize,
    max_rows: usize,
    theme: &crate::theme::theme_t,
) -> Vec<line_t> {
    let mut lines = vec![
        line_t::plain(
            "open this link in your browser?",
            Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        line_t::blank(),
    ];
    let mut host = line_t::plain("host  ", Style::new().fg(theme.fg_faint));
    host.push(
        middle_truncate(&host_of(url), width.saturating_sub(6).max(4)),
        Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    lines.push(host);
    let rows = hard_wrap(url, width.max(8));
    let shown = max_rows.max(1);
    if rows.len() <= shown {
        for r in rows {
            lines.push(line_t::plain(r, Style::new().fg(theme.link)));
        }
    } else {
        // Too long to show whole: the middle goes, both ends stay.
        for r in hard_wrap(&middle_truncate(url, width * shown), width.max(8)) {
            lines.push(line_t::plain(r, Style::new().fg(theme.link)));
        }
    }
    lines
}

/// Draw the confirmation over `area` (the main panel), like the delete dialog.
pub(crate) fn draw_link_dialog(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    let Some(url) = app.link_dialog.clone() else {
        return;
    };
    let theme = app.theme.clone();
    let w = area.width.saturating_sub(4).min(72);
    if w < 20 || area.height < 8 {
        return;
    }
    let inner = usize::from(w.saturating_sub(5));
    let max_rows = usize::from(area.height.saturating_sub(9)).clamp(1, 8);
    let lines = dialog_lines(&url, inner, max_rows, &theme);
    let h = (lines.len() as u16 + 4).min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect::new(x, y, w, h);
    let tone = mix(theme.dock, theme.agent, 0.08);
    fill(buf, rect, Style::new().bg(tone));
    for row in rect.top()..rect.bottom() {
        put_str(
            buf,
            rect.x,
            row,
            rect.right(),
            "▌",
            Style::new().fg(theme.agent).bg(tone),
        );
    }
    let x0 = rect.x.saturating_add(3);
    let right = rect.right().saturating_sub(2);
    let mut ty = rect.y.saturating_add(1);
    for line in lines.iter().take(usize::from(h.saturating_sub(4))) {
        put_line(buf, x0, ty, right, line, Style::new().bg(tone));
        ty = ty.saturating_add(1);
    }
    let by = rect.bottom().saturating_sub(2);
    let mut bx = x0;
    let hover = app.hover;
    for (label, action, primary) in [
        (" enter open ", LinkAction::Open, true),
        (" c copy ", LinkAction::Copy, false),
        (" esc cancel ", LinkAction::Cancel, false),
    ] {
        let bw = str_width(label) as u16;
        if bx.saturating_add(bw) > right {
            break;
        }
        let r = Rect::new(bx, by, bw, 1);
        let over = hover.is_some_and(|(hx, hy)| r.contains(Position::new(hx, hy)));
        let mut style = if primary {
            Style::new()
                .fg(theme.bg)
                .bg(theme.agent)
                .add_modifier(Modifier::BOLD)
        } else if over {
            Style::new().fg(theme.fg).bg(theme.select)
        } else {
            Style::new().fg(theme.fg).bg(theme.inset)
        };
        if primary && over {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        put_str(buf, bx, by, right, label, style);
        app.hits.add(r, Action::Link(action));
        bx = bx.saturating_add(bw + 1);
    }
    app.overlay_areas.push(rect);
}
