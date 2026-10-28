/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Selecting several sidebar rows, like a file explorer (PROJECT.md, D60).
//!
//! A selection is a set of row ids (conversations and folders) and an anchor, so it survives
//! every rebuild of the sidebar list. A plain click selects one row and opens it, `ctrl`+click
//! toggles a row, `shift`+click selects the visible range from the anchor, and `ctrl`+`shift`
//! adds that range. The keys do the same from the keyboard cursor. The keyboard cursor
//! (`sidebar_sel`) is apart from the selection: it moves with `j` and `k` and leaves the
//! selected rows as they are.
//!
//! The anchor of a range is, in order: the anchor set by the last plain click, ctrl+click,
//! chip, cursor move or opened conversation; else the cursor row; else the row of the open
//! conversation; else the clicked row itself. An anchor that left the list (deleted, or hidden
//! in a folded folder) falls through to the next one.
//!
//! Many terminals keep shift+click (and some ctrl+click) for their own text selection while
//! the mouse is reported, so the app never sees it. A range also needs no modifier: press on a
//! row's `☐` chip and drag over the other rows to paint the range (see `select_paint`).

use std::collections::BTreeSet;

use crossterm::event::KeyModifiers;

use crate::app::{ToastKind, app_t};
use crate::input::{Focus, SelectAction};
use crate::sidebar::SidebarItemKind;

/// A row that can be selected: a conversation or a folder, by id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RowId {
    Folder(String),
    Session(String),
}

impl RowId {
    /// The id of a tree row. `None` for the new-chat button and the "conversations" label.
    pub fn of(kind: &SidebarItemKind) -> Option<RowId> {
        match kind {
            SidebarItemKind::Folder { id, .. } => Some(RowId::Folder(id.clone())),
            SidebarItemKind::Session { id } => Some(RowId::Session(id.clone())),
            SidebarItemKind::NewChat | SidebarItemKind::Root => None,
        }
    }

    pub fn session(&self) -> Option<&str> {
        match self {
            RowId::Session(id) => Some(id),
            RowId::Folder(_) => None,
        }
    }

    pub fn folder(&self) -> Option<&str> {
        match self {
            RowId::Folder(id) => Some(id),
            RowId::Session(_) => None,
        }
    }
}

/// The selected rows and the anchor of a range.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct selection_t {
    ids: BTreeSet<RowId>,
    anchor: Option<RowId>,
    /// True when the selection was made on purpose (a modifier, a key, a chip or a right
    /// click). A plain click selects its row too, but that row shows as the open conversation,
    /// not as a selection.
    explicit: bool,
}

impl selection_t {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &RowId) -> bool {
        self.ids.contains(id)
    }

    pub fn anchor(&self) -> Option<&RowId> {
        self.anchor.as_ref()
    }

    /// True when the selected rows show their selection tone.
    pub fn shown(&self) -> bool {
        !self.ids.is_empty() && (self.explicit || self.ids.len() > 1)
    }

    /// The selected ids in `order`, the visible order of the tree.
    pub fn in_order(&self, order: &[RowId]) -> Vec<RowId> {
        order
            .iter()
            .filter(|id| self.ids.contains(id))
            .cloned()
            .collect()
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
        self.explicit = false;
    }

    /// Select only `id` and make it the anchor.
    pub fn only(&mut self, id: RowId, explicit: bool) {
        self.ids.clear();
        self.ids.insert(id.clone());
        self.anchor = Some(id);
        self.explicit = explicit;
    }

    /// Add or remove `id`, and make it the anchor.
    pub fn toggle(&mut self, id: RowId) {
        if !self.ids.remove(&id) {
            self.ids.insert(id.clone());
        }
        self.anchor = Some(id);
        self.explicit = true;
    }

    /// Select the rows of `order` from the anchor to `to`, inclusive. `add` keeps the rows
    /// already selected. Without an anchor in `order`, `to` becomes the anchor. The anchor is
    /// kept, so a second range starts where the first did.
    pub fn range(&mut self, order: &[RowId], to: &RowId, add: bool) {
        let Some(end) = order.iter().position(|id| id == to) else {
            return;
        };
        let start = self
            .anchor
            .as_ref()
            .and_then(|a| order.iter().position(|id| id == a))
            .unwrap_or(end);
        if !add {
            self.ids.clear();
        }
        let (lo, hi) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        for id in order.iter().take(hi + 1).skip(lo) {
            self.ids.insert(id.clone());
        }
        if self.anchor.as_ref().is_none_or(|a| !order.contains(a)) {
            self.anchor = Some(to.clone());
        }
        self.explicit = true;
    }

    /// Make `id` the anchor of the next range, keeping the selected rows.
    pub fn set_anchor(&mut self, id: RowId) {
        self.anchor = Some(id);
    }

    /// Select (or, with `off`, unselect) the rows of `order` from `from` to `to`, inclusive,
    /// on top of `base`, and make `from` the anchor. A row id not in `order` changes nothing.
    pub fn paint(
        &mut self,
        base: &selection_t,
        order: &[RowId],
        from: &RowId,
        to: &RowId,
        off: bool,
    ) {
        let (Some(a), Some(b)) = (
            order.iter().position(|id| id == from),
            order.iter().position(|id| id == to),
        ) else {
            return;
        };
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let mut ids = base.ids.clone();
        for id in order.iter().take(hi + 1).skip(lo) {
            if off {
                ids.remove(id);
            } else {
                ids.insert(id.clone());
            }
        }
        self.ids = ids;
        self.anchor = Some(from.clone());
        self.explicit = !self.ids.is_empty();
    }

    /// Select every row of `order`. The anchor stays.
    pub fn all(&mut self, order: &[RowId]) {
        self.ids = order.iter().cloned().collect();
        self.explicit = true;
    }

    /// Drop the rows that are no longer in `order`: deleted, or hidden in a folded folder.
    pub fn prune(&mut self, order: &[RowId]) {
        self.ids.retain(|id| order.contains(id));
        if self.anchor.as_ref().is_some_and(|a| !order.contains(a)) {
            self.anchor = None;
        }
        if self.ids.is_empty() {
            self.explicit = false;
        }
    }
}

/// What a click on a sidebar row does to the selection, from its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickKind {
    /// No modifier: select the row and open it.
    Plain,
    /// `ctrl` (or `alt`, where a terminal keeps `ctrl`+click for itself): toggle the row.
    Toggle,
    /// `shift`: select the range from the anchor.
    Range,
    /// `ctrl`+`shift`: add the range from the anchor.
    AddRange,
}

impl ClickKind {
    pub fn of(mods: KeyModifiers) -> ClickKind {
        let toggle = mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let range = mods.contains(KeyModifiers::SHIFT);
        match (toggle, range) {
            (false, false) => ClickKind::Plain,
            (true, false) => ClickKind::Toggle,
            (false, true) => ClickKind::Range,
            (true, true) => ClickKind::AddRange,
        }
    }
}

impl app_t {
    /// The tree rows in the order they are drawn.
    pub fn visible_rows(&self) -> Vec<RowId> {
        self.sidebar_items
            .iter()
            .filter_map(|i| RowId::of(&i.kind))
            .collect()
    }

    /// The id of sidebar row `index`, when it is a tree row.
    pub fn row_id(&self, index: usize) -> Option<RowId> {
        self.sidebar_items
            .get(index)
            .and_then(|i| RowId::of(&i.kind))
    }

    /// The index of the row with this id in the current list.
    pub fn row_index(&self, id: &RowId) -> Option<usize> {
        self.sidebar_items
            .iter()
            .position(|i| RowId::of(&i.kind).as_ref() == Some(id))
    }

    /// The rows the keys and the menu act on: the selection when it holds two or more rows,
    /// else the cursor row.
    pub fn targets(&self) -> Vec<RowId> {
        if self.selection.len() > 1 {
            return self.selection.in_order(&self.visible_rows());
        }
        self.row_id(self.sidebar_sel).into_iter().collect()
    }

    /// The row a range starts from (see the module note), given the clicked row.
    pub fn range_anchor(&self, clicked: &RowId) -> RowId {
        let order = self.visible_rows();
        let listed = |id: &RowId| order.contains(id);
        self.selection
            .anchor()
            .filter(|a| listed(a))
            .cloned()
            .or_else(|| self.row_id(self.sidebar_sel).filter(listed))
            .or_else(|| {
                self.active()
                    .map(|s| RowId::Session(s.session.id().to_string()))
                    .filter(listed)
            })
            .unwrap_or_else(|| clicked.clone())
    }

    /// While no selection shows, the anchor follows the open conversation, so a later
    /// shift+click ranges from it. A plain click's row stays selected (a ctrl+click adds to it)
    /// only while it is the open conversation.
    pub(crate) fn anchor_on_shown(&mut self) {
        if self.selection.shown() {
            return;
        }
        let Some(id) = self
            .active()
            .map(|s| RowId::Session(s.session.id().to_string()))
        else {
            return;
        };
        if !self.selection.contains(&id) {
            self.selection.clear();
        }
        self.selection.set_anchor(id);
    }

    /// A plain cursor move (`j`, `k`) while no selection shows: the anchor follows the cursor.
    pub(crate) fn anchor_on_cursor(&mut self) {
        if !self.selection.shown()
            && let Some(id) = self.row_id(self.sidebar_sel)
        {
            self.selection.set_anchor(id);
        }
    }

    /// A click on row `index` with `kind`. A plain click selects the row and opens it (or folds
    /// a folder); the others change the selection and open nothing.
    pub fn click_row(&mut self, index: usize, kind: ClickKind) {
        let Some(id) = self.row_id(index) else {
            if kind == ClickKind::Plain {
                self.apply(crate::input::Action::SidebarActivateAt(index));
            }
            return;
        };
        if matches!(kind, ClickKind::Range | ClickKind::AddRange) {
            // Before the cursor moves to the clicked row, which is a fallback anchor itself.
            let anchor = self.range_anchor(&id);
            self.selection.set_anchor(anchor);
        }
        self.sidebar_sel = index;
        self.focus = Focus::Sidebar;
        let order = self.visible_rows();
        match kind {
            ClickKind::Plain => {
                self.selection.only(id, false);
                self.apply(crate::input::Action::SidebarActivateAt(index));
            }
            ClickKind::Toggle => self.selection.toggle(id),
            ClickKind::Range => self.selection.range(&order, &id, false),
            ClickKind::AddRange => self.selection.range(&order, &id, true),
        }
    }

    /// `shift`+`↑`/`↓`, `J`, `K`: move the cursor and select from the anchor to it.
    pub fn select_extend(&mut self, d: i32) {
        if self.selection.is_empty() || !self.selection.shown() {
            // The keys start at the cursor row: for them the cursor is where the user is. A
            // cursor move already made it the anchor (`anchor_on_cursor`).
            let start = self.row_id(self.sidebar_sel);
            match start {
                Some(id) => self.selection.only(id, true),
                None => self.selection.clear(),
            }
        }
        self.sidebar_step(d);
        if let Some(id) = self.row_id(self.sidebar_sel) {
            let order = self.visible_rows();
            self.selection.range(&order, &id, false);
        }
    }

    /// `ctrl`+`space`: add or remove the cursor row.
    pub fn select_toggle_cursor(&mut self) {
        self.select_toggle_at(self.sidebar_sel);
    }

    /// The selection chip of a row, or `ctrl`+`space` on it.
    pub fn select_toggle_at(&mut self, index: usize) {
        if let Some(id) = self.row_id(index) {
            self.sidebar_sel = index;
            self.focus = Focus::Sidebar;
            self.selection.toggle(id);
        }
    }

    /// `ctrl`+`a` in the sidebar: every visible row.
    pub fn select_all(&mut self) {
        let order = self.visible_rows();
        if order.is_empty() {
            self.say("nothing to select", ToastKind::Info);
            return;
        }
        self.selection.all(&order);
    }

    pub(crate) fn apply_select(&mut self, action: SelectAction) {
        match action {
            SelectAction::Extend(d) => self.select_extend(d),
            SelectAction::ToggleCursor => self.select_toggle_cursor(),
            SelectAction::ToggleAt(i) => self.select_toggle_at(i),
            SelectAction::All => self.select_all(),
            SelectAction::Clear => {
                self.clear_selection();
            }
        }
    }

    /// Drop the selection. True when one was shown.
    pub fn clear_selection(&mut self) -> bool {
        let shown = self.selection.shown();
        self.selection.clear();
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str) -> RowId {
        RowId::Session(id.to_string())
    }

    fn order() -> Vec<RowId> {
        vec![RowId::Folder("f1".into()), s("a"), s("b"), s("c"), s("d")]
    }

    #[test]
    fn toggles_and_ranges_chain_like_a_file_explorer() {
        let order = order();
        let mut sel = selection_t::default();
        sel.only(s("b"), false);
        assert!(!sel.shown(), "a plain click is not a shown selection");
        sel.toggle(s("d"));
        assert_eq!(sel.in_order(&order), vec![s("b"), s("d")]);
        assert!(sel.shown());
        // shift from the anchor d back to a replaces the selection.
        sel.range(&order, &s("a"), false);
        assert_eq!(sel.in_order(&order), vec![s("a"), s("b"), s("c"), s("d")]);
        // ctrl toggles one out, then ctrl+shift adds a second range from the new anchor.
        sel.toggle(s("c"));
        assert_eq!(sel.in_order(&order), vec![s("a"), s("b"), s("d")]);
        sel.range(&order, &RowId::Folder("f1".into()), true);
        assert_eq!(
            sel.in_order(&order),
            vec![RowId::Folder("f1".into()), s("a"), s("b"), s("c"), s("d")]
        );
    }

    #[test]
    fn a_range_without_an_anchor_starts_at_its_own_row() {
        let mut sel = selection_t::default();
        sel.range(&order(), &s("c"), false);
        assert_eq!(sel.in_order(&order()), vec![s("c")]);
        assert_eq!(sel.anchor(), Some(&s("c")));
        sel.range(&order(), &s("zzz"), false);
        assert_eq!(sel.len(), 1, "an id not in the order changes nothing");
    }

    #[test]
    fn painting_a_range_adds_or_removes_it_on_top_of_the_base() {
        let order = order();
        let mut base = selection_t::default();
        base.toggle(s("a"));
        let mut sel = base.clone();
        sel.paint(&base, &order, &s("b"), &s("d"), false);
        assert_eq!(sel.in_order(&order), vec![s("a"), s("b"), s("c"), s("d")]);
        assert_eq!(sel.anchor(), Some(&s("b")));
        // Up from b, over the folder.
        sel.paint(&base, &order, &s("b"), &RowId::Folder("f1".into()), false);
        assert_eq!(
            sel.in_order(&order),
            vec![RowId::Folder("f1".into()), s("a"), s("b")]
        );
        // Off: from a selected row the range is removed.
        let mut all = selection_t::default();
        all.all(&order);
        sel.paint(&all, &order, &s("c"), &s("a"), true);
        assert_eq!(
            sel.in_order(&order),
            vec![RowId::Folder("f1".into()), s("d")]
        );
        sel.paint(&all, &order, &s("c"), &s("zz"), true);
        assert_eq!(sel.len(), 2, "a row not listed changes nothing");
    }

    #[test]
    fn prune_drops_rows_that_left_the_tree() {
        let mut sel = selection_t::default();
        sel.all(&order());
        sel.toggle(s("a"));
        sel.prune(&[s("b"), s("c")]);
        assert_eq!(sel.in_order(&order()), vec![s("b"), s("c")]);
        assert_eq!(sel.anchor(), None);
        sel.prune(&[]);
        assert!(sel.is_empty() && !sel.shown());
    }

    #[test]
    fn modifiers_choose_the_click() {
        assert_eq!(ClickKind::of(KeyModifiers::NONE), ClickKind::Plain);
        assert_eq!(ClickKind::of(KeyModifiers::CONTROL), ClickKind::Toggle);
        assert_eq!(ClickKind::of(KeyModifiers::ALT), ClickKind::Toggle);
        assert_eq!(ClickKind::of(KeyModifiers::SHIFT), ClickKind::Range);
        assert_eq!(
            ClickKind::of(KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            ClickKind::AddRange
        );
    }
}
