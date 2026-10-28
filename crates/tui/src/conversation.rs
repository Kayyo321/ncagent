/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! A conversation's own settings and deleting conversations (PROJECT.md, D57 to D59).
//!
//! Every change goes through `runtime_t::organize`, and a change to several conversations is one
//! `OrganizeOp::Batch`, so it applies to all of them or to none (D58). The menu, the keys and
//! the slash commands all call the functions here, so each change has one code path.
//!
//! The approval mode of a conversation is applied to its session whenever it opens in a tab,
//! and at once when it changes. `full` is never applied from here: it is set on the command
//! line only (D15).

use harness::api::SessionError;
use harness::approval::ApprovalMode;
use harness::organize::{MetaChange, OrganizeOp, TagColor};

use crate::app::{ToastKind, app_t, live_session_t};
use crate::clipboard::osc52;
use crate::folders::short_reason;
use crate::selection::RowId;
use crate::sidebar::SidebarItemKind;

/// The conversation ids among `rows`, in order.
pub fn session_ids(rows: &[RowId]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| r.session().map(str::to_string))
        .collect()
}

/// The folder ids among `rows`, in order.
pub fn folder_ids(rows: &[RowId]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| r.folder().map(str::to_string))
        .collect()
}

/// A delete that waits for the user (D59): the conversations to delete, the ones skipped
/// because they still run, and the folders to delete (their contents move up, D51).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct delete_plan_t {
    pub sessions: Vec<String>,
    pub busy: Vec<String>,
    pub folders: Vec<String>,
    /// The question and the lines under it, written when the plan is made.
    pub question: String,
    pub details: Vec<String>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

impl app_t {
    /// The approval mode a conversation should run in: its own, else the default for new
    /// conversations. `None` leaves the session as it is: a stored `full` (never written), or
    /// no setting while the run itself was started in `full`.
    pub(crate) fn approval_for(&self, id: &str) -> Option<ApprovalMode> {
        match self.organization.meta(id).approval {
            Some(ApprovalMode::Full) => None,
            Some(mode) => Some(mode),
            None if self.info.approval_mode == ApprovalMode::Full => None,
            None => Some(self.settings.default_approval).filter(|m| *m != ApprovalMode::Full),
        }
    }

    /// Put a session that is about to get a tab in the approval mode of its conversation,
    /// when it differs. A refusal is shown and leaves the session's mode as it was.
    pub(crate) fn apply_approval(&mut self, live: &mut live_session_t) {
        let Some(mode) = self.approval_for(live.session.id()) else {
            return;
        };
        if live.session.approval_mode() != mode
            && let Err(err) = live.session.set_approval_mode(mode)
        {
            self.say(err.to_string(), ToastKind::Error);
        }
    }

    /// Apply one setting change to each conversation as one batch. Returns true on success;
    /// a refusal is shown and changes nothing.
    fn change_meta(&mut self, ids: &[String], change: impl Fn(&str) -> MetaChange) -> bool {
        if ids.is_empty() {
            self.say("select a conversation first", ToastKind::Info);
            return false;
        }
        let ops = ids
            .iter()
            .map(|id| OrganizeOp::SetSessionMeta {
                session_id: id.clone(),
                change: change(id),
            })
            .collect();
        match self.runtime.organize(OrganizeOp::Batch(ops)) {
            Ok(org) => {
                self.organization = org;
                self.rebuild_sidebar_items();
                true
            }
            Err(err) => {
                self.say(err.to_string(), ToastKind::Error);
                false
            }
        }
    }

    /// "“title”" for one conversation, "3 conversations" for several.
    fn named(&self, ids: &[String]) -> String {
        match ids {
            [one] => format!("“{}”", self.conversation_title(one)),
            _ => plural(ids.len(), "conversation", "conversations"),
        }
    }

    pub fn set_pinned(&mut self, ids: &[String], pinned: bool) {
        if self.change_meta(ids, |_| MetaChange::Pinned(pinned)) {
            let verb = if pinned { "pinned" } else { "unpinned" };
            let who = self.named(ids);
            self.say(format!("{verb} {who}"), ToastKind::Info);
        }
    }

    /// Pin them all, or unpin them all when every one is pinned already.
    pub fn toggle_pin(&mut self, ids: &[String]) {
        let all = !ids.is_empty() && ids.iter().all(|id| self.organization.meta(id).pinned);
        self.set_pinned(ids, !all);
    }

    pub fn set_color(&mut self, ids: &[String], color: Option<TagColor>) {
        if self.change_meta(ids, |_| MetaChange::Color(color)) {
            let who = self.named(ids);
            let text = match color {
                Some(c) => format!("tagged {who} {}", c.name()),
                None => format!("cleared the tag of {who}"),
            };
            self.say(text, ToastKind::Info);
        }
    }

    pub fn set_archived(&mut self, ids: &[String], archived: bool) {
        let who = self.named(ids);
        if self.change_meta(ids, |_| MetaChange::Archived(archived)) {
            let text = match (archived, self.settings.show_archived) {
                (true, false) => format!("archived {who}; it is hidden from the list"),
                (true, true) => format!("archived {who}"),
                (false, _) => format!("unarchived {who}"),
            };
            self.say(text, ToastKind::Info);
        }
    }

    /// Archive them all, or unarchive them all when every one is archived already.
    pub fn toggle_archived(&mut self, ids: &[String]) {
        let all = !ids.is_empty() && ids.iter().all(|id| self.organization.meta(id).archived);
        self.set_archived(ids, !all);
    }

    /// Save the approval mode of each conversation and apply it at once to the open ones.
    /// `None` returns them to the default for new conversations.
    pub fn set_approval(&mut self, ids: &[String], mode: Option<ApprovalMode>) {
        if mode == Some(ApprovalMode::Full) {
            self.say(
                "approval mode full can be selected only on the command line",
                ToastKind::Warning,
            );
            return;
        }
        if !self.change_meta(ids, |_| MetaChange::Approval(mode)) {
            return;
        }
        for id in ids {
            let want = self.approval_for(id);
            let Some(i) = self.tab_of(id) else {
                continue;
            };
            let Some(want) = want else {
                continue;
            };
            let result = match self.sessions.get_mut(i) {
                Some(live) if live.session.approval_mode() != want => {
                    live.session.set_approval_mode(want)
                }
                _ => Ok(()),
            };
            if let Err(err) = result {
                self.say(err.to_string(), ToastKind::Error);
                return;
            }
        }
        let who = self.named(ids);
        let text = match mode {
            Some(m) => format!("approval mode for {who}: {}", m.name()),
            None => format!(
                "approval mode for {who}: the default ({})",
                self.settings.default_approval.name()
            ),
        };
        self.say(text, ToastKind::Info);
    }

    /// Set a conversation's title. An empty title clears the override. True on success.
    pub fn rename_conversation(&mut self, id: &str, title: String) -> bool {
        let cleared = title.trim().is_empty();
        let ids = [id.to_string()];
        if !self.change_meta(&ids, |_| MetaChange::Title(Some(title.clone()))) {
            return false;
        }
        let text = if cleared {
            format!(
                "cleared the title; it shows “{}”",
                self.conversation_title(id)
            )
        } else {
            format!("renamed to “{}”", self.conversation_title(id))
        };
        self.say(text, ToastKind::Info);
        true
    }

    /// Copy a conversation's id through OSC 52.
    pub fn copy_id(&mut self, id: &str) {
        self.outputs.push(osc52(id));
        self.say(format!("copied the id {id} (OSC 52)"), ToastKind::Info);
    }

    /// Delete these rows (D59). One folder alone uses the folder's inline confirmation in the
    /// sidebar. Anything else asks in the delete dialog, unless `confirm_delete` is off.
    /// Conversations that still run are skipped and named.
    pub fn start_delete_rows(&mut self, rows: Vec<RowId>) {
        self.menu = None;
        if rows.is_empty() {
            self.say(
                "select a conversation or a folder to delete it",
                ToastKind::Info,
            );
            return;
        }
        if let [only @ RowId::Folder(_)] = rows.as_slice()
            && self.settings.confirm_delete
        {
            if let Some(i) = self.row_index(only) {
                self.sidebar_sel = i;
            }
            self.start_delete();
            return;
        }
        let (busy, sessions): (Vec<String>, Vec<String>) = session_ids(&rows)
            .into_iter()
            .partition(|id| self.is_running(id));
        let folders = folder_ids(&rows);
        if sessions.is_empty() && folders.is_empty() {
            self.say(
                format!("cannot delete {}: still running", self.named(&busy)),
                ToastKind::Warning,
            );
            return;
        }
        let mut parts = Vec::new();
        if !sessions.is_empty() {
            parts.push(match sessions.as_slice() {
                [one] => format!("“{}”", self.conversation_title(one)),
                _ => plural(sessions.len(), "conversation", "conversations"),
            });
        }
        if !folders.is_empty() {
            parts.push(match folders.as_slice() {
                [one] => format!(
                    "the folder “{}”",
                    self.organization
                        .folder(one)
                        .map(|f| crate::text::sanitize(&f.name, false))
                        .unwrap_or_else(|| one.clone())
                ),
                _ => plural(folders.len(), "folder", "folders"),
            });
        }
        let mut details = Vec::new();
        match sessions.len() {
            0 => {}
            1 => details.push("this removes its saved log permanently".to_string()),
            _ => details.push("this removes their saved logs permanently".to_string()),
        }
        if !folders.is_empty() {
            details.push("folder contents move up; no conversation in them is deleted".into());
        }
        if !busy.is_empty() {
            details.push(format!(
                "{} still running and will be skipped",
                plural(busy.len(), "conversation is", "conversations are")
            ));
        }
        let plan = delete_plan_t {
            question: format!("delete {}?", parts.join(" and ")),
            details,
            sessions,
            busy,
            folders,
        };
        if self.settings.confirm_delete {
            self.delete_plan = Some(plan);
        } else {
            self.run_delete(plan);
        }
    }

    /// Enter on the delete dialog.
    pub fn confirm_delete_plan(&mut self) {
        if let Some(plan) = self.delete_plan.take() {
            self.run_delete(plan);
        }
    }

    /// Delete each conversation (its tab closes first) and each folder, deepest first, then
    /// put the cursor on the row next to where the first one was.
    fn run_delete(&mut self, plan: delete_plan_t) {
        let gone: Vec<RowId> = plan
            .sessions
            .iter()
            .map(|id| RowId::Session(id.clone()))
            .chain(plan.folders.iter().map(|id| RowId::Folder(id.clone())))
            .collect();
        let neighbour = self.neighbour_of(&gone);
        let mut deleted = 0usize;
        let mut skipped = plan.busy.len();
        let mut errors: Vec<String> = Vec::new();
        for id in &plan.sessions {
            if self.is_running(id) {
                skipped += 1;
                continue;
            }
            if let Some(i) = self.tab_of(id) {
                self.close_tab(i);
            }
            match self.runtime.delete_session(id) {
                Ok(org) => {
                    self.organization = org;
                    self.summaries.retain(|s| s.id != *id);
                    deleted += 1;
                }
                Err(err) => errors.push(err.to_string()),
            }
        }
        let mut folders = plan.folders.clone();
        folders.sort_by_key(|id| std::cmp::Reverse(self.organization.depth(id).unwrap_or(0)));
        let mut folders_done = 0usize;
        let mut refused: Vec<&'static str> = Vec::new();
        for id in &folders {
            match self
                .runtime
                .organize(OrganizeOp::DeleteFolder { id: id.clone() })
            {
                Ok(org) => {
                    self.organization = org;
                    self.folded.remove(id);
                    folders_done += 1;
                }
                Err(SessionError::Organize(err)) => refused.push(short_reason(&err)),
                Err(err) => errors.push(err.to_string()),
            }
        }
        self.selection.clear();
        self.refresh_summaries();
        self.rebuild_sidebar_items();
        self.select_near(neighbour);
        let mut parts = Vec::new();
        if deleted > 0 || folders_done == 0 {
            parts.push(format!("{deleted} deleted"));
        }
        if folders_done > 0 {
            parts.push(format!(
                "{} deleted, contents moved up",
                plural(folders_done, "folder", "folders")
            ));
        }
        if skipped > 0 {
            parts.push(format!("{skipped} skipped: still running"));
        }
        if !refused.is_empty() {
            parts.push(format!(
                "{} refused: {}",
                plural(refused.len(), "folder", "folders"),
                refused.join(", ")
            ));
        }
        if let Some(err) = errors.first() {
            parts.push(format!("failed: {err}"));
        }
        let kind = if errors.is_empty() && refused.is_empty() && skipped == 0 {
            ToastKind::Info
        } else {
            ToastKind::Warning
        };
        self.say(parts.join(" · "), kind);
    }

    /// The row that stays next to rows about to go: the first one after the last of them,
    /// else the last one before them.
    fn neighbour_of(&self, gone: &[RowId]) -> Option<RowId> {
        let order = self.visible_rows();
        let last = order.iter().rposition(|r| gone.contains(r))?;
        let first = order.iter().position(|r| gone.contains(r))?;
        order
            .iter()
            .skip(last + 1)
            .find(|r| !gone.contains(r))
            .or_else(|| order.iter().take(first).rev().find(|r| !gone.contains(r)))
            .cloned()
    }

    /// Put the cursor on this row, or on the new-chat button when it is gone.
    fn select_near(&mut self, row: Option<RowId>) {
        self.sidebar_sel = row
            .and_then(|r| self.row_index(&r))
            .or_else(|| {
                self.sidebar_items
                    .iter()
                    .position(|i| i.kind == SidebarItemKind::NewChat)
            })
            .unwrap_or(0);
    }
}
