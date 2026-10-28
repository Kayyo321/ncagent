/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The context menu (PROJECT.md, D61): what it offers for a conversation, a folder, a selection
//! of several rows or a tab, and what each item does.
//!
//! The menu only names commands. Each command calls the same function as its key or slash
//! command, so the menu adds no second code path. An item that cannot apply is shown dim with
//! the reason, and choosing it shows the reason instead of doing anything.
//! `menu_draw` draws it.

use harness::approval::ApprovalMode;
use harness::organize::TagColor;
use ratatui::style::Color;

use crate::app::{ToastKind, app_t};
use crate::conversation::{folder_ids, session_ids};
use crate::input::{Focus, MenuAction, Target};
use crate::layout::SidebarMode;
use crate::selection::RowId;
use crate::tabs::OpenPlace;
use crate::tags::tag_rgb;

/// What the menu acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuScope {
    /// Sidebar rows: one, or a selection of several.
    Rows(Vec<RowId>),
    /// The tab of this conversation.
    Tab(String),
}

/// What an item does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCmd {
    Open,
    OpenInTab,
    OpenInTabs,
    Rename,
    Pin(bool),
    Color(Option<TagColor>),
    Approval(Option<ApprovalMode>),
    Archive(bool),
    Move,
    CopyId,
    Delete,
    NewSubfolder,
    CloseTab,
    CloseOthers,
    CloseRight,
}

#[derive(Clone, Debug, PartialEq)]
pub struct menu_item_t {
    pub label: String,
    /// Right-aligned: the key, or the current value of a submenu.
    pub hint: String,
    /// `None` for an item that opens `sub`.
    pub cmd: Option<MenuCmd>,
    pub sub: Vec<menu_item_t>,
    /// Why the item cannot apply. Such an item is dim and skipped by the arrow keys.
    pub disabled: Option<String>,
    pub swatch: Option<Color>,
    pub checked: bool,
    /// Shown in the error colour: the delete.
    pub danger: bool,
}

impl menu_item_t {
    fn run(label: &str, hint: &str, cmd: MenuCmd) -> Self {
        menu_item_t {
            label: label.to_string(),
            hint: hint.to_string(),
            cmd: Some(cmd),
            sub: Vec::new(),
            disabled: None,
            swatch: None,
            checked: false,
            danger: false,
        }
    }

    fn submenu(label: &str, hint: &str, sub: Vec<menu_item_t>) -> Self {
        menu_item_t {
            sub,
            cmd: None,
            ..menu_item_t::run(label, hint, MenuCmd::Open)
        }
    }

    fn off(mut self, reason: &str) -> Self {
        self.disabled = Some(reason.to_string());
        self
    }

    fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    pub fn enabled(&self) -> bool {
        self.disabled.is_none()
    }
}

/// An open menu: its items, the highlighted one, and the open submenu.
#[derive(Clone, Debug, PartialEq)]
pub struct menu_t {
    pub scope: MenuScope,
    /// A dim first line naming what the menu acts on.
    pub caption: String,
    pub items: Vec<menu_item_t>,
    pub sel: usize,
    /// The item whose submenu is open.
    pub sub: Option<usize>,
    pub sub_sel: usize,
    /// Where it was opened. The drawing keeps it on screen, flipping up or left near an edge.
    pub at: (u16, u16),
}

/// The next enabled item from `from` in direction `d`, wrapping. `from` itself when no other
/// item is enabled.
fn step(items: &[menu_item_t], from: usize, d: i32) -> usize {
    let n = items.len() as i64;
    if n == 0 {
        return 0;
    }
    let mut i = from as i64;
    for _ in 0..n {
        i = (i + i64::from(d.signum())).rem_euclid(n);
        if items.get(i as usize).is_some_and(menu_item_t::enabled) {
            return i as usize;
        }
    }
    from
}

fn first_enabled(items: &[menu_item_t]) -> usize {
    items.iter().position(menu_item_t::enabled).unwrap_or(0)
}

fn color_items(current: Option<Option<TagColor>>) -> Vec<menu_item_t> {
    let mut out: Vec<menu_item_t> = TagColor::all()
        .into_iter()
        .map(|c| {
            let mut item = menu_item_t::run(c.name(), "", MenuCmd::Color(Some(c)))
                .checked(current == Some(Some(c)));
            item.swatch = Some(tag_rgb(c));
            item
        })
        .collect();
    out.push(menu_item_t::run("none", "", MenuCmd::Color(None)).checked(current == Some(None)));
    out
}

impl app_t {
    /// The colour every one of these conversations has, when they agree.
    fn common_color(&self, ids: &[String]) -> Option<Option<TagColor>> {
        let first = self.organization.meta(ids.first()?).color;
        ids.iter()
            .all(|id| self.organization.meta(id).color == first)
            .then_some(first)
    }

    fn conversation_items(&self, id: &str) -> Vec<menu_item_t> {
        let meta = self.organization.meta(id);
        let open = self.tab_of(id).is_some();
        let mut open_in_tab = menu_item_t::run("Open in new tab", "", MenuCmd::OpenInTab);
        if open {
            open_in_tab = open_in_tab.off("it is open in a tab");
        }
        let mode_hint = match meta.approval {
            Some(m) => m.name().to_string(),
            None => "default".to_string(),
        };
        let approval = vec![
            menu_item_t::run("ask", "", MenuCmd::Approval(Some(ApprovalMode::Ask)))
                .checked(meta.approval == Some(ApprovalMode::Ask)),
            menu_item_t::run("edit", "", MenuCmd::Approval(Some(ApprovalMode::Edit)))
                .checked(meta.approval == Some(ApprovalMode::Edit)),
            menu_item_t::run(
                "default",
                self.settings.default_approval.name(),
                MenuCmd::Approval(None),
            )
            .checked(meta.approval.is_none()),
            menu_item_t::run("full", "command line", MenuCmd::Approval(None)).off(
                "approval mode full can be selected only on the command line (ncagent demo --approval full)",
            ),
        ];
        let mut delete = menu_item_t::run("Delete…", "d", MenuCmd::Delete);
        delete.danger = true;
        if self.is_running(id) {
            delete = delete.off("it is still running; stop it first");
        }
        vec![
            menu_item_t::run("Open", "enter", MenuCmd::Open),
            open_in_tab,
            menu_item_t::run("Rename…", "r", MenuCmd::Rename),
            menu_item_t::run(
                if meta.pinned { "Unpin" } else { "Pin" },
                "p",
                MenuCmd::Pin(!meta.pinned),
            ),
            menu_item_t::submenu(
                "Colour tag",
                meta.color.map(TagColor::name).unwrap_or("none"),
                color_items(Some(meta.color)),
            ),
            menu_item_t::submenu("Approval mode", &mode_hint, approval),
            menu_item_t::run(
                if meta.archived {
                    "Unarchive"
                } else {
                    "Archive"
                },
                "",
                MenuCmd::Archive(!meta.archived),
            ),
            menu_item_t::run("Move to folder…", "m", MenuCmd::Move),
            menu_item_t::run("Copy id", "", MenuCmd::CopyId),
            delete,
        ]
    }

    fn folder_items(&self, id: &str) -> Vec<menu_item_t> {
        let mut sub = menu_item_t::run("New sub-folder", "f", MenuCmd::NewSubfolder);
        if let Err(err) = self.organization.can_create_under(Some(id)) {
            sub = sub.off(&err.to_string());
        }
        let mut delete = menu_item_t::run("Delete…", "d", MenuCmd::Delete);
        delete.danger = true;
        vec![
            sub,
            menu_item_t::run("Rename…", "r", MenuCmd::Rename),
            menu_item_t::run("Move to folder…", "m", MenuCmd::Move),
            delete,
        ]
    }

    fn many_items(&self, rows: &[RowId]) -> Vec<menu_item_t> {
        let ids = session_ids(rows);
        let mut delete = menu_item_t::run("Delete…", "d", MenuCmd::Delete);
        delete.danger = true;
        let mov = menu_item_t::run("Move to folder…", "m", MenuCmd::Move);
        if !folder_ids(rows).is_empty() {
            return vec![mov, delete];
        }
        let pinned = ids.iter().all(|id| self.organization.meta(id).pinned);
        let archived = ids.iter().all(|id| self.organization.meta(id).archived);
        let common = self.common_color(&ids);
        let closed = ids.iter().filter(|id| self.tab_of(id).is_none()).count();
        let mut open = menu_item_t::run("Open in tabs", "", MenuCmd::OpenInTabs);
        if closed == 0 {
            open = open.off("they are all open in tabs");
        }
        vec![
            open,
            menu_item_t::run(
                if pinned { "Unpin all" } else { "Pin all" },
                "p",
                MenuCmd::Pin(!pinned),
            ),
            menu_item_t::submenu(
                "Colour tag",
                match common {
                    Some(Some(c)) => c.name(),
                    Some(None) => "none",
                    None => "mixed",
                },
                color_items(common),
            ),
            menu_item_t::run(
                if archived {
                    "Unarchive all"
                } else {
                    "Archive all"
                },
                "",
                MenuCmd::Archive(!archived),
            ),
            mov,
            delete,
        ]
    }

    fn tab_items(&self, id: &str) -> Vec<menu_item_t> {
        let meta = self.organization.meta(id);
        let n = self.sessions.len();
        let at = self.tab_of(id).unwrap_or(0);
        let mut others = menu_item_t::run("Close others", "", MenuCmd::CloseOthers);
        if n < 2 {
            others = others.off("there is no other tab");
        }
        let mut right = menu_item_t::run("Close tabs to the right", "", MenuCmd::CloseRight);
        if at + 1 >= n {
            right = right.off("there is no tab to the right");
        }
        vec![
            menu_item_t::run("Close", "alt+w", MenuCmd::CloseTab),
            others,
            right,
            menu_item_t::run("Rename…", "", MenuCmd::Rename),
            menu_item_t::run(
                if meta.pinned { "Unpin" } else { "Pin" },
                "",
                MenuCmd::Pin(!meta.pinned),
            ),
            menu_item_t::submenu(
                "Colour tag",
                meta.color.map(TagColor::name).unwrap_or("none"),
                color_items(Some(meta.color)),
            ),
        ]
    }

    /// Open the menu for these rows at a screen position.
    pub fn open_row_menu(&mut self, rows: Vec<RowId>, at: (u16, u16)) {
        let (caption, items) = match rows.as_slice() {
            [] => {
                self.say(
                    "select a conversation or a folder for its menu",
                    ToastKind::Info,
                );
                return;
            }
            [RowId::Session(id)] => (
                format!("“{}”", self.conversation_title(id)),
                self.conversation_items(id),
            ),
            [RowId::Folder(id)] => (
                format!(
                    "folder “{}”",
                    self.organization
                        .folder(id)
                        .map(|f| crate::text::sanitize(&f.name, false))
                        .unwrap_or_else(|| id.clone())
                ),
                self.folder_items(id),
            ),
            many => (format!("{} selected", many.len()), self.many_items(many)),
        };
        self.open_menu(MenuScope::Rows(rows), caption, items, at);
    }

    /// Open the menu of the tab that shows this conversation.
    pub fn open_tab_menu(&mut self, id: &str, at: (u16, u16)) {
        let caption = format!("tab “{}”", self.conversation_title(id));
        let items = self.tab_items(id);
        self.open_menu(MenuScope::Tab(id.to_string()), caption, items, at);
    }

    fn open_menu(
        &mut self,
        scope: MenuScope,
        caption: String,
        items: Vec<menu_item_t>,
        at: (u16, u16),
    ) {
        self.cancel_sidebar_modal();
        self.delete_plan = None;
        let sel = first_enabled(&items);
        self.menu = Some(menu_t {
            scope,
            caption,
            items,
            sel,
            sub: None,
            sub_sel: 0,
            at,
        });
        self.rebuild_sidebar_items();
    }

    /// A right press at a screen position: the menu of the row or the tab under it. A row
    /// outside the selection becomes the only selected row first, like a file explorer.
    pub fn context_at(&mut self, x: u16, y: u16) {
        let target = self.hits.at(x, y).map(|h| h.target.clone());
        match target {
            Some(Target::Row(index)) => {
                let Some(id) = self.row_id(index) else {
                    return;
                };
                if !self.selection.contains(&id) {
                    self.selection.only(id, true);
                }
                self.sidebar_sel = index;
                self.focus = Focus::Sidebar;
                let rows = self.selection.in_order(&self.visible_rows());
                self.open_row_menu(rows, (x, y));
            }
            Some(Target::Tab(index)) => {
                // Under the bar, so the tab it belongs to stays in sight.
                if let Some(id) = self.sessions.get(index).map(|s| s.session.id().to_string()) {
                    self.open_tab_menu(&id, (x, y.saturating_add(1)));
                }
            }
            _ => {}
        }
    }

    /// Where a menu opened by a key appears: under the cursor row, or under the tab.
    fn key_anchor(&self, target: Target) -> (u16, u16) {
        match self.hits.find_target(&target) {
            Some(r) => (r.x.saturating_add(4), r.y.saturating_add(1)),
            None => (
                self.layout.main.x.saturating_add(2),
                self.layout.transcript.y,
            ),
        }
    }

    pub(crate) fn apply_menu(&mut self, action: MenuAction) {
        match action {
            MenuAction::OpenKeys => {
                let sidebar = self.layout.sidebar_mode() != SidebarMode::Hidden;
                if self.focus == Focus::Sidebar && sidebar {
                    let at = self.key_anchor(Target::Row(self.sidebar_sel));
                    let rows = self.targets();
                    self.open_row_menu(rows, at);
                } else if let Some(id) = self.active().map(|s| s.session.id().to_string()) {
                    let at = self.key_anchor(Target::Tab(self.active));
                    self.open_tab_menu(&id, at);
                }
            }
            MenuAction::OpenSelection => {
                let at = self
                    .hits
                    .find(&crate::input::Action::Menu(MenuAction::OpenSelection))
                    .map(|r| (r.x, r.y.saturating_add(1)))
                    .unwrap_or_else(|| self.key_anchor(Target::Row(self.sidebar_sel)));
                let rows = self.selection.in_order(&self.visible_rows());
                self.open_row_menu(rows, at);
            }
            MenuAction::Move(d) => {
                if let Some(menu) = self.menu.as_mut() {
                    match menu.sub {
                        Some(parent) => {
                            if let Some(item) = menu.items.get(parent) {
                                menu.sub_sel = step(&item.sub, menu.sub_sel, d);
                            }
                        }
                        None => menu.sel = step(&menu.items, menu.sel, d),
                    }
                }
            }
            MenuAction::Enter => {
                let Some(menu) = self.menu.as_ref() else {
                    return;
                };
                match menu.sub {
                    Some(_) => self.menu_pick_sub(menu.sub_sel),
                    None => self.menu_pick(menu.sel),
                }
            }
            MenuAction::Right => {
                if let Some(menu) = self.menu.as_mut()
                    && menu.sub.is_none()
                    && let Some(item) = menu.items.get(menu.sel)
                    && !item.sub.is_empty()
                {
                    menu.sub_sel = first_enabled(&item.sub);
                    menu.sub = Some(menu.sel);
                }
            }
            MenuAction::Back => self.menu_back(),
            MenuAction::Pick(i) => {
                // A click on an item of the menu itself, whatever submenu is open. A click on
                // the item whose submenu is open closes that submenu.
                let Some(menu) = self.menu.as_mut() else {
                    return;
                };
                if menu.sub.take() != Some(i) {
                    self.menu_pick(i);
                }
            }
            MenuAction::PickSub(i) => self.menu_pick_sub(i),
            MenuAction::Digit(i) => match self.menu.as_ref().and_then(|m| m.sub) {
                Some(_) => self.menu_pick_sub(i),
                None => self.menu_pick(i),
            },
        }
    }

    /// `←` or `esc`: close the submenu, or the menu.
    pub fn menu_back(&mut self) {
        match self.menu.as_mut() {
            Some(menu) if menu.sub.is_some() => menu.sub = None,
            _ => {
                self.menu = None;
                self.rebuild_sidebar_items();
            }
        }
    }

    fn menu_pick(&mut self, i: usize) {
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let Some(item) = menu.items.get(i).cloned() else {
            return;
        };
        menu.sel = i;
        if let Some(reason) = item.disabled {
            self.say(reason, ToastKind::Info);
            return;
        }
        if !item.sub.is_empty() {
            if menu.sub == Some(i) {
                menu.sub = None;
            } else {
                menu.sub = Some(i);
                menu.sub_sel = first_enabled(&item.sub);
            }
            return;
        }
        if let Some(cmd) = item.cmd {
            self.run_menu_cmd(cmd);
        }
    }

    fn menu_pick_sub(&mut self, i: usize) {
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let Some(item) = menu
            .sub
            .and_then(|p| menu.items.get(p))
            .and_then(|p| p.sub.get(i))
            .cloned()
        else {
            return;
        };
        menu.sub_sel = i;
        if let Some(reason) = item.disabled {
            self.say(reason, ToastKind::Info);
            return;
        }
        if let Some(cmd) = item.cmd {
            self.run_menu_cmd(cmd);
        }
    }

    /// Close the menu and run one of its commands on what it was opened for.
    fn run_menu_cmd(&mut self, cmd: MenuCmd) {
        let Some(menu) = self.menu.take() else {
            return;
        };
        let rows = match &menu.scope {
            MenuScope::Rows(rows) => rows.clone(),
            MenuScope::Tab(id) => vec![RowId::Session(id.clone())],
        };
        let ids = session_ids(&rows);
        let first = ids.first().cloned();
        match cmd {
            MenuCmd::Open => {
                if let Some(id) = first {
                    self.open_conversation(&id, OpenPlace::Setting);
                }
            }
            MenuCmd::OpenInTab => {
                if let Some(id) = first {
                    self.open_conversation(&id, OpenPlace::NewTab);
                }
            }
            MenuCmd::OpenInTabs => {
                for id in &ids {
                    self.open_conversation(id, OpenPlace::NewTab);
                }
                if let Some(i) = first.and_then(|id| self.tab_of(&id)) {
                    self.switch_tab(i);
                }
            }
            MenuCmd::Rename => match rows.first() {
                Some(RowId::Session(id)) => self.start_rename_session(id),
                Some(row @ RowId::Folder(_)) => {
                    if let Some(i) = self.row_index(row) {
                        self.select_row(i);
                    }
                    self.start_rename();
                }
                None => {}
            },
            MenuCmd::Pin(on) => self.set_pinned(&ids, on),
            MenuCmd::Color(color) => self.set_color(&ids, color),
            MenuCmd::Approval(mode) => self.set_approval(&ids, mode),
            MenuCmd::Archive(on) => self.set_archived(&ids, on),
            MenuCmd::Move => self.start_pickup_of(rows),
            MenuCmd::CopyId => {
                if let Some(id) = first {
                    self.copy_id(&id);
                }
            }
            MenuCmd::Delete => self.start_delete_rows(rows),
            MenuCmd::NewSubfolder => {
                if let Some(RowId::Folder(id)) = rows.first() {
                    self.start_create(Some(id.clone()));
                }
            }
            MenuCmd::CloseTab => {
                if let Some(i) = first.and_then(|id| self.tab_of(&id)) {
                    self.close_tab(i);
                }
            }
            MenuCmd::CloseOthers => {
                if let Some(keep) = first {
                    let others: Vec<String> = self
                        .sessions
                        .iter()
                        .map(|s| s.session.id().to_string())
                        .filter(|id| *id != keep)
                        .collect();
                    self.close_tabs_of(&others);
                }
            }
            MenuCmd::CloseRight => {
                if let Some(at) = first.and_then(|id| self.tab_of(&id)) {
                    let right: Vec<String> = self
                        .sessions
                        .iter()
                        .skip(at + 1)
                        .map(|s| s.session.id().to_string())
                        .collect();
                    self.close_tabs_of(&right);
                }
            }
        }
        self.rebuild_sidebar_items();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_skip_dim_items_and_wrap() {
        let items = vec![
            menu_item_t::run("a", "", MenuCmd::Open),
            menu_item_t::run("b", "", MenuCmd::Open).off("no"),
            menu_item_t::run("c", "", MenuCmd::Open),
        ];
        assert_eq!(step(&items, 0, 1), 2);
        assert_eq!(step(&items, 2, 1), 0);
        assert_eq!(step(&items, 0, -1), 2);
        let none = vec![menu_item_t::run("x", "", MenuCmd::Open).off("no")];
        assert_eq!(step(&none, 0, 1), 0);
        assert_eq!(first_enabled(&items), 0);
        assert_eq!(step(&[], 0, 1), 0);
    }
}
