/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Conversation folders in the app: the name prompt, the delete confirmation, and moving a row
//! by drag or by pick-up (PROJECT.md, D50 to D55). The rules live in `harness::organize`. This
//! module checks a destination with them to show it allowed or refused, asks the runtime to
//! apply each change, and shows what the runtime answered. It never edits the organisation
//! itself.

use harness::api::SessionError;
use harness::organize::{OrganizeError, OrganizeOp};

use crate::app::{ToastKind, app_t};
use crate::composer::composer_t;
use crate::input::{Focus, Modal};
use crate::selection::{ClickKind, RowId};
use crate::sidebar::{MoveMark, OverlayKind, SidebarItemKind, overlay_t, sidebar_item_t};

/// What a move carries: a conversation, a folder or a selection of several (D60), with the
/// title it shows while moving.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveSource {
    Session {
        id: String,
        title: String,
    },
    Folder {
        id: String,
        name: String,
    },
    /// Several rows, moved as one batch. Each one is checked, and one refusal refuses all.
    Many {
        items: Vec<MoveSource>,
        title: String,
    },
}

impl MoveSource {
    pub fn title(&self) -> &str {
        match self {
            MoveSource::Session { title, .. } => title,
            MoveSource::Folder { name, .. } => name,
            MoveSource::Many { title, .. } => title,
        }
    }

    fn is_item(&self, kind: &SidebarItemKind) -> bool {
        match (self, kind) {
            (MoveSource::Session { id, .. }, SidebarItemKind::Session { id: other }) => id == other,
            (MoveSource::Folder { id, .. }, SidebarItemKind::Folder { id: other, .. }) => {
                id == other
            }
            (MoveSource::Many { items, .. }, _) => items.iter().any(|m| m.is_item(kind)),
            _ => false,
        }
    }

    /// The single rows this source moves.
    fn parts(&self) -> Vec<&MoveSource> {
        match self {
            MoveSource::Many { items, .. } => items.iter().collect(),
            other => vec![other],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// A new folder under `parent`, or at the top level.
    Create {
        parent: Option<String>,
    },
    Rename {
        id: String,
    },
    /// A conversation's title (D57). An empty title clears it.
    RenameSession {
        id: String,
    },
}

/// The folder-name prompt: a one-line editor in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct prompt_t {
    pub kind: PromptKind,
    pub editor: composer_t,
}

/// A sidebar row held by the mouse. `moved` turns true once the pointer leaves the row; until
/// then a release on the row is a plain click (D55).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct grab_t {
    pub index: usize,
    /// What a drag of this row would move. `None` for a row that does not move, and while
    /// pick-up mode is on (a press then only chooses a destination).
    pub source: Option<MoveSource>,
    pub moved: bool,
    /// The row under the pointer at the last drag event, read from the hit registry of the
    /// frame the user saw.
    pub over: Option<usize>,
    /// What a release on the same row does: open it, or change the selection (D60).
    pub click: ClickKind,
}

/// A few words for a refused destination, short enough for a sidebar row. The toast carries
/// the full text of the error.
pub fn short_reason(err: &OrganizeError) -> &'static str {
    match err {
        OrganizeError::WouldNest { .. } => "inside itself",
        OrganizeError::TooDeep { .. } => "too deep",
        OrganizeError::NameTaken { .. } => "name taken",
        OrganizeError::FolderNotFound { .. } => "gone",
        _ => "not allowed",
    }
}

/// Where a move ends: a folder id, or `None` for the top level.
type destination_t = Option<String>;

impl app_t {
    /// Read the folders once at start. On an error the list stays flat, the error is shown
    /// once, and every change reports the same error when tried (D52).
    pub fn load_organization(&mut self) {
        match self.runtime.organization() {
            Ok(org) => self.organization = org,
            Err(err) => self.say(format!("could not read folders: {err}"), ToastKind::Warning),
        }
    }

    /// The sidebar widget that owns the keys, if any.
    pub fn sidebar_modal(&self) -> Option<Modal> {
        if self.prompt.is_some() {
            Some(Modal::Prompt)
        } else if self.confirm.is_some() {
            Some(Modal::Confirm)
        } else if self.pickup.is_some() {
            Some(Modal::Move)
        } else {
            None
        }
    }

    /// Close the prompt, the confirmation or pick-up mode. True when one was open.
    pub fn cancel_sidebar_modal(&mut self) -> bool {
        let open = self.sidebar_modal().is_some();
        self.prompt = None;
        self.confirm = None;
        self.pickup = None;
        if open {
            self.rebuild_sidebar_items();
        }
        open
    }

    /// Bring the sidebar into view with the keys on it: docked when wide, the drawer when
    /// narrow.
    fn show_sidebar(&mut self) {
        if self.can_dock() {
            self.sidebar_pinned = true;
        } else {
            self.drawer_open = true;
        }
        self.focus = Focus::Sidebar;
    }

    pub fn select_row(&mut self, index: usize) {
        self.sidebar_sel = index.min(self.sidebar_items.len().saturating_sub(1));
        self.focus = Focus::Sidebar;
    }

    fn item(&self, index: usize) -> Option<&sidebar_item_t> {
        self.sidebar_items.get(index)
    }

    fn folder_index(&self, id: &str) -> Option<usize> {
        self.sidebar_items.iter().position(
            |i| matches!(&i.kind, SidebarItemKind::Folder { id: other, .. } if other == id),
        )
    }

    fn folder_name(&self, id: &str) -> String {
        self.organization
            .folder(id)
            .map(|f| crate::text::sanitize(&f.name, false))
            .unwrap_or_else(|| id.to_string())
    }

    /// The folder of the selected row, when it is a folder.
    pub fn selected_folder(&self) -> Option<String> {
        match self.item(self.sidebar_sel).map(|i| &i.kind) {
            Some(SidebarItemKind::Folder { id, .. }) => Some(id.clone()),
            _ => None,
        }
    }

    fn source_at(&self, index: usize) -> Option<MoveSource> {
        let item = self.item(index)?;
        match &item.kind {
            SidebarItemKind::Session { id } => Some(MoveSource::Session {
                id: id.clone(),
                title: item.title.clone(),
            }),
            SidebarItemKind::Folder { id, .. } => Some(MoveSource::Folder {
                id: id.clone(),
                name: item.title.clone(),
            }),
            _ => None,
        }
    }

    /// Apply one change through the runtime and keep what it returns.
    fn organize(&mut self, op: OrganizeOp) -> Result<(), SessionError> {
        let org = self.runtime.organize(op)?;
        self.organization = org;
        self.rebuild_sidebar_items();
        Ok(())
    }

    /// Unfold a folder and every folder above it, so a row inside it is visible.
    fn expand_to(&mut self, folder: Option<&str>) {
        if let Some(id) = folder {
            let path: Vec<String> = self
                .organization
                .path(id)
                .iter()
                .map(|f| f.id.clone())
                .collect();
            for id in path {
                self.folded.remove(&id);
            }
        }
    }

    fn select_where(&mut self, pick: impl Fn(&SidebarItemKind) -> bool) {
        if let Some(i) = self.sidebar_items.iter().position(|i| pick(&i.kind)) {
            self.sidebar_sel = i;
        }
    }

    // Creating, renaming and deleting.

    /// Open the name prompt for a new folder under `parent`. A parent at the depth limit is
    /// refused at once, before anything is typed.
    pub fn start_create(&mut self, parent: Option<String>) {
        if self.prompt.is_some() {
            return;
        }
        self.cancel_sidebar_modal();
        if let Err(err) = self.organization.can_create_under(parent.as_deref()) {
            self.say(err.to_string(), ToastKind::Warning);
            return;
        }
        self.expand_to(parent.as_deref());
        self.prompt = Some(prompt_t {
            kind: PromptKind::Create { parent },
            editor: composer_t::new(),
        });
        self.show_sidebar();
        self.rebuild_sidebar_items();
    }

    /// Create a folder. On success the new folder is selected; on a refusal the error is shown
    /// and false returned.
    pub fn create_folder_named(&mut self, name: String, parent: Option<String>) -> bool {
        let op = OrganizeOp::CreateFolder {
            name: name.clone(),
            parent: parent.clone(),
        };
        match self.organize(op) {
            Ok(()) => {
                self.expand_to(parent.as_deref());
                self.rebuild_sidebar_items();
                let id = self
                    .organization
                    .find_child(parent.as_deref(), &name)
                    .map(|f| f.id.clone());
                if let Some(id) = id {
                    self.select_where(
                        |k| matches!(k, SidebarItemKind::Folder { id: other, .. } if *other == id),
                    );
                    let shown = self.folder_name(&id);
                    self.say(format!("created folder “{shown}”"), ToastKind::Info);
                }
                true
            }
            Err(err) => {
                self.say(err.to_string(), ToastKind::Error);
                false
            }
        }
    }

    /// `r`: rename the cursor row, a folder or a conversation.
    pub fn start_rename(&mut self) {
        if let Some(RowId::Session(id)) = self.row_id(self.sidebar_sel) {
            self.start_rename_session(&id);
            return;
        }
        self.cancel_sidebar_modal();
        let Some(id) = self.selected_folder() else {
            self.say(
                "select a folder or a conversation to rename it",
                ToastKind::Info,
            );
            return;
        };
        let mut editor = composer_t::new();
        editor.set_text(&self.folder_name(&id));
        self.prompt = Some(prompt_t {
            kind: PromptKind::Rename { id },
            editor,
        });
        self.show_sidebar();
        self.rebuild_sidebar_items();
    }

    /// The title prompt of a conversation, filled with the title it shows now (D57).
    pub fn start_rename_session(&mut self, id: &str) {
        self.cancel_sidebar_modal();
        self.menu = None;
        let mut editor = composer_t::new();
        editor.set_text(&self.conversation_title(id));
        self.prompt = Some(prompt_t {
            kind: PromptKind::RenameSession { id: id.to_string() },
            editor,
        });
        self.show_sidebar();
        self.rebuild_sidebar_items();
        if let Some(i) = self.row_index(&RowId::Session(id.to_string())) {
            self.sidebar_sel = i;
        }
    }

    pub fn start_delete(&mut self) {
        self.cancel_sidebar_modal();
        let Some(id) = self.selected_folder() else {
            self.say(
                "select a folder to delete it; conversations are never deleted here",
                ToastKind::Info,
            );
            return;
        };
        self.confirm = Some(id);
        self.show_sidebar();
        self.rebuild_sidebar_items();
    }

    /// Enter in the prompt. A refused name keeps the prompt open with the error shown.
    pub fn prompt_submit(&mut self) {
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        let name = prompt.editor.text().to_string();
        match prompt.kind {
            PromptKind::Create { parent } => {
                if self.create_folder_named(name, parent) {
                    self.prompt = None;
                    self.rebuild_sidebar_items();
                }
            }
            PromptKind::RenameSession { id } => {
                if self.rename_conversation(&id, name) {
                    self.prompt = None;
                    self.rebuild_sidebar_items();
                }
            }
            PromptKind::Rename { id } => {
                match self.organize(OrganizeOp::RenameFolder {
                    id: id.clone(),
                    name,
                }) {
                    Ok(()) => {
                        self.prompt = None;
                        self.rebuild_sidebar_items();
                        self.select_where(
                            |k| matches!(k, SidebarItemKind::Folder { id: other, .. } if *other == id),
                        );
                        let shown = self.folder_name(&id);
                        self.say(format!("renamed to “{shown}”"), ToastKind::Info);
                    }
                    Err(err) => self.say(err.to_string(), ToastKind::Error),
                }
            }
        }
    }

    /// Enter on the delete confirmation. The folder's contents move up to its parent; no
    /// conversation is deleted (D51).
    pub fn confirm_delete(&mut self) {
        let Some(id) = self.confirm.take() else {
            return;
        };
        let name = self.folder_name(&id);
        let parent = self.organization.folder(&id).and_then(|f| f.parent.clone());
        match self.organize(OrganizeOp::DeleteFolder { id: id.clone() }) {
            Ok(()) => {
                self.folded.remove(&id);
                self.rebuild_sidebar_items();
                match parent {
                    Some(p) => self.select_where(
                        |k| matches!(k, SidebarItemKind::Folder { id: other, .. } if *other == p),
                    ),
                    None => self.select_where(|k| *k == SidebarItemKind::NewChat),
                }
                self.say(
                    format!("deleted folder “{name}”; its contents moved up"),
                    ToastKind::Info,
                );
            }
            Err(err) => {
                self.rebuild_sidebar_items();
                self.say(err.to_string(), ToastKind::Error);
            }
        }
    }

    // Navigation and folding.

    /// `j` and `k` in the sidebar. The "conversations" label is a heading, so the selection
    /// steps over it.
    pub fn sidebar_step(&mut self, d: i32) {
        let n = self.sidebar_items.len() as i64;
        if n == 0 || d == 0 {
            return;
        }
        let step = i64::from(d.signum());
        let mut next = (self.sidebar_sel as i64 + i64::from(d)).clamp(0, n - 1);
        while (0..n).contains(&next)
            && self
                .sidebar_items
                .get(next as usize)
                .is_some_and(|i| i.kind == SidebarItemKind::Root)
        {
            next += step;
        }
        if (0..n).contains(&next) {
            self.sidebar_sel = next as usize;
        }
    }

    /// `←` folds the selected folder, or goes to the folder above; `→` unfolds it.
    pub fn fold_selected(&mut self, fold: bool) {
        let Some(item) = self.item(self.sidebar_sel).cloned() else {
            return;
        };
        let parent = match &item.kind {
            SidebarItemKind::Folder { id, collapsed } => {
                if fold && !collapsed {
                    self.folded.insert(id.clone());
                    self.rebuild_sidebar_items();
                    return;
                }
                if !fold {
                    self.folded.remove(id);
                    self.rebuild_sidebar_items();
                    return;
                }
                self.organization.folder(id).and_then(|f| f.parent.clone())
            }
            SidebarItemKind::Session { id } if fold => {
                self.organization.folder_of(id).map(str::to_string)
            }
            _ => None,
        };
        if let Some(p) = parent {
            self.select_where(
                |k| matches!(k, SidebarItemKind::Folder { id: other, .. } if *other == p),
            );
        }
    }

    // Moving.

    /// What is being moved now: the pick-up, or a row dragged off its place.
    pub fn moving(&self) -> Option<&MoveSource> {
        self.pickup.as_ref().or_else(|| {
            self.grab
                .as_ref()
                .filter(|g| g.moved)
                .and_then(|g| g.source.as_ref())
        })
    }

    pub fn moving_title(&self) -> Option<String> {
        self.moving().map(|m| m.title().to_string())
    }

    /// Mark each row allowed or refused for the move in progress, from the rules of
    /// `harness::organize` (D51).
    pub fn mark_moves(&self, items: &mut [sidebar_item_t]) {
        let Some(source) = self.moving() else {
            return;
        };
        for item in items.iter_mut() {
            item.mark = if source.is_item(&item.kind) {
                MoveMark::Source
            } else {
                match &item.kind {
                    SidebarItemKind::NewChat => MoveMark::None,
                    SidebarItemKind::Session { .. } => MoveMark::Inert,
                    SidebarItemKind::Root => self.mark_for(source, None),
                    SidebarItemKind::Folder { id, .. } => self.mark_for(source, Some(id)),
                }
            };
        }
    }

    fn mark_for(&self, source: &MoveSource, dest: Option<&str>) -> MoveMark {
        match self.check_move(source, dest) {
            Ok(()) => MoveMark::Allowed,
            Err(err) => MoveMark::Refused {
                reason: short_reason(&err).to_string(),
            },
        }
    }

    fn check_move(&self, source: &MoveSource, dest: Option<&str>) -> Result<(), OrganizeError> {
        match source {
            MoveSource::Many { items, .. } => items
                .iter()
                .filter(|m| !self.rides_along(m, source))
                .try_for_each(|m| self.check_move(m, dest)),
            MoveSource::Session { .. } => match dest {
                Some(f) if self.organization.folder(f).is_none() => {
                    Err(OrganizeError::FolderNotFound { id: f.to_string() })
                }
                _ => Ok(()),
            },
            MoveSource::Folder { id, .. } => self.organization.can_move_folder(id, dest),
        }
    }

    /// True when `part` of a selection lies inside a folder that the same selection moves, so
    /// it travels with that folder and is not moved on its own.
    fn rides_along(&self, part: &MoveSource, all: &MoveSource) -> bool {
        let inside = |f: &str| {
            all.parts().iter().any(|m| match m {
                MoveSource::Folder { id, .. } => self.organization.is_within(f, id),
                _ => false,
            })
        };
        match part {
            MoveSource::Session { id, .. } => self.organization.folder_of(id).is_some_and(inside),
            MoveSource::Folder { id, .. } => self
                .organization
                .folder(id)
                .and_then(|f| f.parent.as_deref())
                .is_some_and(inside),
            MoveSource::Many { .. } => false,
        }
    }

    fn already_there(&self, source: &MoveSource, dest: Option<&str>) -> bool {
        match source {
            MoveSource::Many { items, .. } => items
                .iter()
                .filter(|m| !self.rides_along(m, source))
                .all(|m| self.already_there(m, dest)),
            MoveSource::Session { id, .. } => self.organization.folder_of(id) == dest,
            MoveSource::Folder { id, .. } => self
                .organization
                .folder(id)
                .is_some_and(|f| f.parent.as_deref() == dest),
        }
    }

    /// The destination a row stands for: a folder, or the top level for the label.
    fn destination_of(&self, index: usize) -> Option<destination_t> {
        match &self.item(index)?.kind {
            SidebarItemKind::Root => Some(None),
            SidebarItemKind::Folder { id, .. } => Some(Some(id.clone())),
            _ => None,
        }
    }

    fn allowed(&self, index: usize) -> bool {
        self.item(index)
            .is_some_and(|i| i.mark == MoveMark::Allowed)
    }

    /// The row a move would drop on now: the pick-up choice, or the destination under the
    /// dragging pointer.
    pub fn move_hot(&self) -> Option<usize> {
        if self.pickup.is_some() {
            return Some(self.sidebar_sel);
        }
        let grab = self.grab.as_ref().filter(|g| g.moved)?;
        let row = grab.over?;
        self.destination_of(row).map(|_| row)
    }

    /// What moving these rows carries: one row, or the selection as one source.
    pub fn source_of(&self, rows: &[RowId]) -> Option<MoveSource> {
        let mut parts: Vec<MoveSource> = rows
            .iter()
            .filter_map(|r| self.row_index(r))
            .filter_map(|i| self.source_at(i))
            .collect();
        match parts.len() {
            0 => None,
            1 => parts.pop(),
            n => Some(MoveSource::Many {
                items: parts,
                title: format!("{n} items"),
            }),
        }
    }

    /// `m`: pick the selection, or the cursor row, up. `j` `k` then choose among the allowed
    /// destinations.
    pub fn start_pickup(&mut self) {
        let rows = self.targets();
        self.start_pickup_of(rows);
    }

    /// Pick these rows up (`m`, the move chip, "Move to folder…" in the menu).
    pub fn start_pickup_of(&mut self, rows: Vec<RowId>) {
        self.cancel_sidebar_modal();
        self.menu = None;
        let origin = rows
            .first()
            .and_then(|r| self.row_index(r))
            .unwrap_or(self.sidebar_sel);
        let Some(source) = self.source_of(&rows) else {
            self.say(
                "select a conversation or a folder to move it",
                ToastKind::Info,
            );
            return;
        };
        if self.organization.folders().is_empty() {
            self.say("no folders yet; ctrl+f makes one", ToastKind::Info);
            return;
        }
        self.pickup = Some(source);
        self.show_sidebar();
        self.rebuild_sidebar_items();
        let n = self.sidebar_items.len();
        let pick = (0..=origin.min(n.saturating_sub(1)))
            .rev()
            .find(|i| self.allowed(*i))
            .or_else(|| (origin..n).find(|i| self.allowed(*i)));
        match pick {
            Some(i) => self.sidebar_sel = i,
            None => {
                self.pickup = None;
                self.rebuild_sidebar_items();
                self.say("there is nowhere to move it", ToastKind::Info);
            }
        }
    }

    /// Pick-up mode: step to the next allowed destination.
    pub fn move_step(&mut self, d: i32) {
        let n = self.sidebar_items.len() as i64;
        let step = i64::from(d.signum());
        if step == 0 {
            return;
        }
        let mut i = self.sidebar_sel as i64 + step;
        while (0..n).contains(&i) {
            if self.allowed(i as usize) {
                self.sidebar_sel = i as usize;
                return;
            }
            i += step;
        }
    }

    /// Pick-up mode: drop on row `index` (enter, or a click). A refused destination keeps the
    /// mode open.
    pub fn drop_at(&mut self, index: usize) {
        let Some(source) = self.pickup.clone() else {
            return;
        };
        if self.finish_move(source, index) {
            self.pickup = None;
            self.rebuild_sidebar_items();
        }
    }

    /// Move `source` to the destination of row `index`. True when the move is over (done, a
    /// no-op, or failed in the runtime); false when the row is not a destination or is refused.
    fn finish_move(&mut self, source: MoveSource, index: usize) -> bool {
        let Some(dest) = self.destination_of(index) else {
            self.say(
                "drop on a folder, or on “conversations” for the top level",
                ToastKind::Info,
            );
            return false;
        };
        if let Err(err) = self.check_move(&source, dest.as_deref()) {
            self.say(
                format!("cannot move “{}” there: {err}", source.title()),
                ToastKind::Warning,
            );
            return false;
        }
        if self.already_there(&source, dest.as_deref()) {
            self.say(
                format!("“{}” is already there", source.title()),
                ToastKind::Info,
            );
            return true;
        }
        let op_of = |m: &MoveSource| match m {
            MoveSource::Session { id, .. } => Some(OrganizeOp::MoveSession {
                session_id: id.clone(),
                folder: dest.clone(),
            }),
            MoveSource::Folder { id, .. } => Some(OrganizeOp::MoveFolder {
                id: id.clone(),
                parent: dest.clone(),
            }),
            MoveSource::Many { .. } => None,
        };
        let op = match &source {
            MoveSource::Many { items, .. } => OrganizeOp::Batch(
                items
                    .iter()
                    .filter(|m| {
                        !self.rides_along(m, &source) && !self.already_there(m, dest.as_deref())
                    })
                    .filter_map(op_of)
                    .collect(),
            ),
            single => match op_of(single) {
                Some(op) => op,
                None => return true,
            },
        };
        match self.organize(op) {
            Ok(()) => {
                self.expand_to(dest.as_deref());
                self.grab = None;
                self.pickup = None;
                self.rebuild_sidebar_items();
                self.select_where(|k| source.is_item(k));
                self.focus = Focus::Sidebar;
                let place = match &dest {
                    Some(id) => format!("into “{}”", self.folder_name(id)),
                    None => "to the top level".to_string(),
                };
                self.say(
                    format!("moved “{}” {place}", source.title()),
                    ToastKind::Info,
                );
            }
            Err(err) => self.say(err.to_string(), ToastKind::Error),
        }
        true
    }

    // Dragging.

    /// A press on a sidebar row. Pressing a row of a selection of several grabs the whole
    /// selection, so a drag moves all of it (D60).
    pub fn grab_row(&mut self, index: usize, click: ClickKind) {
        if self.prompt.is_some() || self.confirm.is_some() {
            self.cancel_sidebar_modal();
        }
        // Only a plain press can start a move: a modified press changes the selection.
        let source = if self.pickup.is_some() || click != ClickKind::Plain {
            None
        } else {
            match self.row_id(index) {
                Some(id) if self.selection.len() > 1 && self.selection.contains(&id) => {
                    let rows = self.selection.in_order(&self.visible_rows());
                    self.source_of(&rows)
                }
                _ => self.source_at(index),
            }
        };
        self.grab = Some(grab_t {
            index,
            source,
            moved: false,
            over: Some(index),
            click,
        });
    }

    /// The pointer moved with the button held. Leaving the grabbed row starts the drag; near
    /// the top or bottom of the tree the list scrolls.
    pub fn drag_to(&mut self, x: u16, y: u16) {
        self.hover = Some((x, y));
        let row = self.hits.row_at(x, y);
        let list = self.sidebar_list;
        let Some(grab) = self.grab.as_mut() else {
            return;
        };
        grab.over = row;
        if grab.source.is_none() {
            return;
        }
        let started = !grab.moved && row != Some(grab.index);
        if started {
            grab.moved = true;
        }
        if grab.moved && list.height > 0 {
            if y <= list.y {
                self.sidebar_scroll = self.sidebar_scroll.saturating_sub(1);
            } else if y >= list.bottom().saturating_sub(1) {
                self.sidebar_scroll = self.sidebar_scroll.saturating_add(1);
            }
        }
        if started {
            self.rebuild_sidebar_items();
        }
    }

    /// The button went up. Without a drag this is a click on the row; after a drag it drops.
    pub fn release_at(&mut self, x: u16, y: u16) {
        let Some(grab) = self.grab.take() else {
            return;
        };
        self.hover = Some((x, y));
        let row = self.hits.row_at(x, y);
        if self.pickup.is_some() {
            if row == Some(grab.index) {
                self.select_row(grab.index);
                self.drop_at(grab.index);
            }
            return;
        }
        if !grab.moved {
            if row == Some(grab.index) {
                self.click_row(grab.index, grab.click);
                self.rebuild_sidebar_items();
            }
            return;
        }
        let Some(source) = grab.source else {
            return;
        };
        match row {
            Some(r) if r == grab.index => self.say("move cancelled", ToastKind::Info),
            Some(r) => {
                self.finish_move(source, r);
            }
            None => self.say(
                "move cancelled: release on a folder, or on “conversations” for the top level",
                ToastKind::Info,
            ),
        }
        self.rebuild_sidebar_items();
    }

    /// Escape during a drag.
    pub fn cancel_drag(&mut self) {
        if self.grab.take().is_some_and(|g| g.moved) {
            self.say("move cancelled", ToastKind::Info);
        }
        self.rebuild_sidebar_items();
    }

    /// The prompt or the confirmation as rows for the sidebar to place in the tree.
    pub fn overlay(&self) -> Option<overlay_t> {
        let depth_of = |index: Option<usize>| index.and_then(|i| self.item(i)).map(|i| i.depth);
        if let Some(prompt) = &self.prompt {
            let (caption, anchor, replace, depth) = match &prompt.kind {
                PromptKind::Create { parent: Some(p) } => {
                    let anchor = self.folder_index(p);
                    (
                        format!("new folder in {}", self.folder_name(p)),
                        anchor,
                        false,
                        depth_of(anchor).map(|d| d + 1).unwrap_or(0),
                    )
                }
                PromptKind::Create { parent: None } => {
                    ("new folder at top level".to_string(), None, false, 0)
                }
                PromptKind::Rename { id } => {
                    let anchor = self.folder_index(id);
                    (
                        "rename folder".to_string(),
                        anchor,
                        anchor.is_some(),
                        depth_of(anchor).unwrap_or(0),
                    )
                }
                PromptKind::RenameSession { id } => {
                    let anchor = self.row_index(&RowId::Session(id.clone()));
                    (
                        "rename conversation · empty clears".to_string(),
                        anchor,
                        anchor.is_some(),
                        depth_of(anchor).unwrap_or(0),
                    )
                }
            };
            return Some(overlay_t {
                kind: OverlayKind::Prompt {
                    caption,
                    text: prompt.editor.text().to_string(),
                    cursor: prompt.editor.cursor(),
                },
                anchor,
                replace,
                depth,
            });
        }
        let id = self.confirm.as_ref()?;
        let anchor = self.folder_index(id);
        Some(overlay_t {
            kind: OverlayKind::Confirm {
                name: self.folder_name(id),
            },
            anchor,
            replace: false,
            depth: depth_of(anchor).unwrap_or(0),
        })
    }
}
