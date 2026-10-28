/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Painting a range of sidebar rows with the mouse and no modifier key (D60).
//!
//! A press on a row's `☐` chip starts the paint, with that row as the anchor. Dragging up or
//! down selects every row from the anchor to the row under the pointer, live, on top of what
//! was selected before; the release keeps it. A paint that starts on a selected row removes the
//! range instead. A press and release on the same chip is a toggle of that row. Near the top or
//! bottom of the list it scrolls, and `esc` puts the selection back as it was.
//!
//! This is the mouse path for terminals that keep shift+click for their own text selection.

use ratatui::layout::Position;

use crate::app::app_t;
use crate::input::{Action, Focus, SelectAction, Target};
use crate::selection::{RowId, selection_t};

/// A paint in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct paint_t {
    /// The pressed row: the anchor of the range.
    pub from: RowId,
    /// The row under the pointer.
    pub to: RowId,
    /// The paint removes the range: it started on a selected row.
    pub off: bool,
    /// The selection before the press.
    pub base: selection_t,
    /// The pointer moved with the button held. A release without a move is a toggle of the
    /// pressed row, wherever the list moved to under the pointer since the press.
    pub moved: bool,
}

impl app_t {
    /// The sidebar row under a point: a row, or the selection chip on a row.
    fn paint_row_at(&self, x: u16, y: u16) -> Option<usize> {
        match self.hits.at(x, y).map(|h| &h.target) {
            Some(Target::Row(i)) => Some(*i),
            Some(Target::Action(Action::Select(SelectAction::ToggleAt(i)))) => Some(*i),
            // Over another chip of a row: the row under it.
            _ => self.hits.hits.iter().rev().find_map(|h| match h.target {
                Target::Row(i) if h.rect.contains(Position::new(x, y)) => Some(i),
                _ => None,
            }),
        }
    }

    /// A press on the selection chip of row `index`.
    pub fn start_paint(&mut self, index: usize) {
        let Some(id) = self.row_id(index) else {
            return;
        };
        self.cancel_sidebar_modal();
        self.sidebar_sel = index;
        self.focus = Focus::Sidebar;
        let off = self.selection.shown() && self.selection.contains(&id);
        let base = if self.selection.shown() {
            self.selection.clone()
        } else {
            // A plain click's row is no selection of its own.
            selection_t::default()
        };
        self.paint = Some(paint_t {
            from: id.clone(),
            to: id,
            off,
            base,
            moved: false,
        });
        self.apply_paint();
    }

    /// The pointer moved with the button held during a paint. Near the list's edges it scrolls.
    pub fn paint_to(&mut self, x: u16, y: u16) {
        self.hover = Some((x, y));
        let list = self.sidebar_list;
        if list.height > 0 {
            if y <= list.y {
                self.sidebar_scroll = self.sidebar_scroll.saturating_sub(1);
            } else if y >= list.bottom().saturating_sub(1) {
                self.sidebar_scroll = self.sidebar_scroll.saturating_add(1);
            }
        }
        let Some(index) = self.paint_row_at(x, y) else {
            return;
        };
        let Some(id) = self.row_id(index) else {
            return;
        };
        if let Some(paint) = self.paint.as_mut() {
            paint.to = id;
            paint.moved = true;
        }
        self.sidebar_sel = index;
        self.apply_paint();
    }

    fn apply_paint(&mut self) {
        let Some(paint) = self.paint.clone() else {
            return;
        };
        let order = self.visible_rows();
        self.selection
            .paint(&paint.base, &order, &paint.from, &paint.to, paint.off);
        self.rebuild_sidebar_items();
    }

    /// The release: the painted range stays.
    pub fn end_paint(&mut self) {
        self.paint = None;
        self.rebuild_sidebar_items();
    }

    /// `esc` during a paint: the selection as it was before the press.
    pub fn cancel_paint(&mut self) {
        if let Some(paint) = self.paint.take() {
            self.selection = paint.base;
            self.rebuild_sidebar_items();
        }
    }
}
