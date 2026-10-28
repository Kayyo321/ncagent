/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The sidebar list, rebuilt from the session summaries, the open tabs, the folders and each
//! conversation's own settings (D54, D57).
//!
//! At every level of the tree the pinned conversations come first, then the folders sorted by
//! name, then the other conversations in the chosen order (`conversation_order`: newest first,
//! or by title). Archived conversations are left out unless `show_archived` is on; then they
//! show dim, with an "archived" tag. A conversation's title override replaces its first message.
//! With `show_row_details` off a conversation row shows its title only (and the archived tag).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use harness::organize::{MaxFolderDepth, organization_t};
use harness::settings::ConversationOrder;

use crate::app::{app_t, live_session_t};
use crate::selection::RowId;
use crate::sidebar::{Activity, SidebarItemKind, sidebar_item_t};
use crate::text::{format_age, sanitize, truncate};

/// One conversation before it is placed in the tree.
struct entry_t {
    id: String,
    title: String,
    updated: u64,
    turns: u32,
    demo: bool,
    project: String,
}

/// The conversations of one level, split so the pinned ones lead.
#[derive(Default)]
struct level_t {
    pinned: Vec<sidebar_item_t>,
    rest: Vec<sidebar_item_t>,
}

impl level_t {
    fn push(&mut self, item: sidebar_item_t) {
        if item.pinned {
            self.pinned.push(item);
        } else {
            self.rest.push(item);
        }
    }
}

/// The aggregate activity of the conversations inside a folder: waiting outranks working.
fn folder_activity<'a>(inside: impl Iterator<Item = &'a sidebar_item_t>) -> Activity {
    let mut out = Activity::Idle;
    for item in inside {
        match item.activity {
            Activity::Waiting => return Activity::Waiting,
            Activity::Generating | Activity::Tool => out = Activity::Generating,
            Activity::Idle => {}
        }
    }
    out
}

fn walk(
    org: &organization_t,
    parent: Option<&str>,
    depth: usize,
    folded: &BTreeSet<String>,
    levels: &BTreeMap<String, level_t>,
    out: &mut Vec<sidebar_item_t>,
) {
    if depth > MaxFolderDepth {
        return;
    }
    for folder in org.children(parent) {
        let collapsed = folded.contains(&folder.id);
        let inside = levels
            .iter()
            .filter(|(f, _)| org.is_within(f, &folder.id))
            .flat_map(|(_, level)| level.pinned.iter().chain(level.rest.iter()));
        let mut item = sidebar_item_t::label(
            SidebarItemKind::Folder {
                id: folder.id.clone(),
                collapsed,
            },
            sanitize(&folder.name, false),
        );
        item.activity = folder_activity(inside);
        item.detail = org.session_count(&folder.id).to_string();
        item.depth = depth;
        out.push(item);
        if collapsed {
            continue;
        }
        let level = levels.get(&folder.id);
        for item in level.into_iter().flat_map(|l| l.pinned.iter()) {
            let mut item = item.clone();
            item.depth = depth + 1;
            out.push(item);
        }
        walk(org, Some(&folder.id), depth + 1, folded, levels, out);
        for item in level.into_iter().flat_map(|l| l.rest.iter()) {
            let mut item = item.clone();
            item.depth = depth + 1;
            out.push(item);
        }
    }
}

impl app_t {
    /// The title the UI shows for a conversation: its title override, else its first message,
    /// else "New chat".
    pub fn conversation_title(&self, id: &str) -> String {
        if let Some(title) = self.organization.meta(id).title {
            return sanitize(&title, false);
        }
        if let Some(live) = self.sessions.iter().find(|s| s.session.id() == id)
            && live.has_messages()
        {
            return live.title();
        }
        match self.summaries.iter().find(|s| s.id == id) {
            Some(s) if !s.title.trim().is_empty() => sanitize(&s.title, false),
            _ => "New chat".to_string(),
        }
    }

    fn entries(&self) -> Vec<entry_t> {
        let current_project = sanitize(&self.info.project, false);
        let mut entries: Vec<entry_t> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for s in &self.summaries {
            if !seen.insert(s.id.clone()) {
                continue;
            }
            entries.push(entry_t {
                id: s.id.clone(),
                title: self.conversation_title(&s.id),
                updated: s.updated_ms,
                turns: s.turns,
                demo: s.simulated,
                project: sanitize(&s.project, false),
            });
        }
        for live in &self.sessions {
            let id = live.session.id().to_string();
            if !seen.insert(id.clone()) {
                continue;
            }
            let project = live
                .transcript
                .meta
                .as_ref()
                .map(|m| sanitize(&m.project, false))
                .unwrap_or_else(|| current_project.clone());
            entries.push(entry_t {
                title: self.conversation_title(&id),
                id,
                updated: if live.transcript.last_at_ms > 0 {
                    live.transcript.last_at_ms
                } else {
                    self.now_ms
                },
                turns: live.transcript.turn.turn,
                demo: self.info.identity.simulated(),
                project,
            });
        }
        match self.settings.conversation_order {
            ConversationOrder::Recent => entries.sort_by_key(|e| std::cmp::Reverse(e.updated)),
            ConversationOrder::Title => entries.sort_by(|a, b| {
                a.title
                    .to_lowercase()
                    .cmp(&b.title.to_lowercase())
                    .then_with(|| b.updated.cmp(&a.updated))
                    .then_with(|| a.id.cmp(&b.id))
            }),
        }
        entries
    }

    /// The sidebar list: new chat, the "conversations" label, then the tree (see the module
    /// note). Rows carry their move marks while something moves (D55), and the selection is
    /// trimmed to the rows that are still listed (D60).
    pub fn rebuild_sidebar_items(&mut self) {
        let current_project = sanitize(&self.info.project, false);
        let active_id = self.active().map(|s| s.session.id().to_string());
        let activity_of = |id: &str| {
            self.sessions
                .iter()
                .find(|s| s.session.id() == id)
                .map(live_session_t::activity)
                .unwrap_or(Activity::Idle)
        };
        let org = &self.organization;
        let mut levels: BTreeMap<String, level_t> = BTreeMap::new();
        let mut top = level_t::default();
        for e in self.entries() {
            let meta = org.meta(&e.id);
            if meta.archived && !self.settings.show_archived {
                continue;
            }
            // The age, turns and project are the row's details; `show_row_details` hides them
            // and keeps the archived tag (D63).
            let mut detail = if self.settings.show_row_details {
                let mut detail = format!("{} · {}t", format_age(self.now_ms, e.updated), e.turns);
                if !e.project.is_empty() && e.project != current_project {
                    detail = format!("{} · {detail}", truncate(&e.project, 12));
                }
                detail
            } else {
                String::new()
            };
            if meta.archived {
                detail = if detail.is_empty() {
                    "archived".to_string()
                } else {
                    format!("archived · {detail}")
                };
            }
            let mut item =
                sidebar_item_t::label(SidebarItemKind::Session { id: e.id.clone() }, e.title);
            item.active = active_id.as_deref() == Some(e.id.as_str());
            item.detail = detail;
            item.activity = activity_of(&e.id);
            item.demo = e.demo;
            item.pinned = meta.pinned;
            item.color = meta.color;
            item.archived = meta.archived;
            // A conversation filed in a folder that does not exist is listed as unfiled (the
            // organisation's rules make that impossible, D51).
            match org.folder_of(&e.id).filter(|f| org.folder(f).is_some()) {
                Some(folder) => levels.entry(folder.to_string()).or_default().push(item),
                None => top.push(item),
            }
        }
        let mut items = vec![
            sidebar_item_t::label(SidebarItemKind::NewChat, "New chat".to_string()),
            sidebar_item_t::label(SidebarItemKind::Root, "conversations".to_string()),
        ];
        items.extend(top.pinned);
        walk(org, None, 0, &self.folded, &levels, &mut items);
        items.extend(top.rest);
        let order: Vec<RowId> = items.iter().filter_map(|i| RowId::of(&i.kind)).collect();
        self.selection.prune(&order);
        if self.selection.shown() {
            for item in items.iter_mut() {
                item.selected =
                    RowId::of(&item.kind).is_some_and(|id| self.selection.contains(&id));
            }
        }
        self.mark_moves(&mut items);
        // The cursor stays on its row when the rows around it move, such as when a
        // conversation's time changes its place in the order.
        let same_row = |a: &SidebarItemKind, b: &SidebarItemKind| match (RowId::of(a), RowId::of(b))
        {
            (Some(x), Some(y)) => x == y,
            (None, None) => a == b,
            _ => false,
        };
        let cursor = self
            .sidebar_items
            .get(self.sidebar_sel)
            .map(|i| i.kind.clone());
        self.sidebar_sel = cursor
            .and_then(|k| items.iter().position(|i| same_row(&i.kind, &k)))
            .unwrap_or(self.sidebar_sel)
            .min(items.len().saturating_sub(1));
        self.sidebar_items = items;
    }
}
