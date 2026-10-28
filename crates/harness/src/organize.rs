/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Conversation folders (PROJECT.md, D50 to D55).
//!
//! A folder holds sub-folders and conversations. The organisation is kept apart from the session
//! logs, which are append-only (D24), so filing a conversation never rewrites a log. Every
//! operation checks all of its rules before it changes anything, so a refused operation leaves
//! the organisation as it was.
//!
//! The harness holds the types, the rules and the store trait. The demo's file store lives in
//! `crates/demo` and reads its file as untrusted input (D52).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::approval::ApprovalMode;
use crate::store::{TitleLimit, valid_session_id};

/// Folders nest to this many levels. A top-level folder is level 1 (D51).
pub const MaxFolderDepth: usize = 5;

/// Longest folder name, in characters (D51).
pub const FolderNameLimit: usize = 40;

/// The version a file of folders carries.
pub const OrganizationVersion: u32 = 1;

/// Most operations one batch holds, counting the operations nested in it (D58).
pub const MaxBatchOps: usize = 256;

/// Longest numeric part of a folder id, so an id always fits a `u32`.
const FolderIdDigits: usize = 9;

/// One folder. `parent` is `None` for a top-level folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct folder_t {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
}

/// What a store reads and writes: the folders and the filed sessions, with a version and an
/// optional label. A store checks this with [`organization_t::from_file`] before it uses it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct organization_file_t {
    pub version: u32,
    /// Set by the demo store to the demo label (O1). Ignored when reading.
    #[serde(default)]
    pub label: Option<String>,
    pub folders: Vec<folder_t>,
    /// Session id to folder id. A session not listed here is unfiled.
    pub filed: BTreeMap<String, String>,
    /// Session id to its settings (D57). A session not listed here has the defaults. Optional,
    /// so a file written before D57 still loads.
    #[serde(default)]
    pub meta: BTreeMap<String, session_meta_t>,
}

/// A colour tag a conversation can carry (D57).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagColor {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
}

impl TagColor {
    /// Every colour, in the order a menu shows them.
    pub fn all() -> [TagColor; 8] {
        [
            TagColor::Red,
            TagColor::Orange,
            TagColor::Yellow,
            TagColor::Green,
            TagColor::Teal,
            TagColor::Blue,
            TagColor::Purple,
            TagColor::Pink,
        ]
    }

    /// The lowercase name the file stores.
    pub fn name(self) -> &'static str {
        match self {
            TagColor::Red => "red",
            TagColor::Orange => "orange",
            TagColor::Yellow => "yellow",
            TagColor::Green => "green",
            TagColor::Teal => "teal",
            TagColor::Blue => "blue",
            TagColor::Purple => "purple",
            TagColor::Pink => "pink",
        }
    }

    /// The colour with exactly this lowercase name.
    pub fn from_name(name: &str) -> Option<TagColor> {
        TagColor::all()
            .into_iter()
            .find(|color| color.name() == name)
    }
}

/// What the user set on one conversation (D57). A conversation with only default values has no
/// entry in the organisation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct session_meta_t {
    /// A title that replaces the conversation's first message, trimmed (D57).
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub color: Option<TagColor>,
    #[serde(default)]
    pub archived: bool,
    /// The approval mode of this conversation. `full` is never stored (D15, D57).
    #[serde(default)]
    pub approval: Option<ApprovalMode>,
}

impl session_meta_t {
    /// True when every field has its default value.
    pub fn is_default(&self) -> bool {
        *self == session_meta_t::default()
    }
}

/// One change to a conversation's settings (D57).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetaChange {
    /// `None`, or a title that is empty after trimming, clears the title.
    Title(Option<String>),
    Pinned(bool),
    Color(Option<TagColor>),
    Archived(bool),
    /// `None` clears the mode, so the conversation uses the default for new conversations.
    Approval(Option<ApprovalMode>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrganizeError {
    /// The name is empty after trimming, too long, or holds a control character.
    InvalidName {
        reason: String,
    },
    /// A sibling already has this name, ignoring case.
    NameTaken {
        name: String,
    },
    FolderNotFound {
        id: String,
    },
    /// A folder id is not `f` and 1 to 9 digits, or a session id is not a plain session id.
    InvalidId {
        id: String,
    },
    /// The change would nest folders deeper than `MaxFolderDepth`.
    TooDeep {
        limit: usize,
    },
    /// A folder cannot move into itself or into one of its own sub-folders.
    WouldNest {
        id: String,
    },
    /// The file does not hold a valid organisation.
    Malformed {
        message: String,
    },
    UnsupportedVersion {
        version: u32,
    },
    /// The title is longer than `TitleLimit` characters, or holds a control character (D57).
    InvalidTitle {
        reason: String,
    },
    /// A batch holds more than `limit` operations, counting nested ones (D58).
    TooManyOps {
        limit: usize,
    },
    /// The change is not allowed, such as `full` as a conversation's approval mode (D15, D57).
    NotAllowed {
        reason: String,
    },
    Io {
        path: String,
        message: String,
    },
}

impl fmt::Display for OrganizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrganizeError::InvalidName { reason } => write!(f, "invalid folder name: {reason}"),
            OrganizeError::NameTaken { name } => {
                write!(f, "a folder named {name:?} is already there")
            }
            OrganizeError::FolderNotFound { id } => write!(f, "no folder {id}"),
            OrganizeError::InvalidId { id } => write!(f, "invalid id {id:?}"),
            OrganizeError::TooDeep { limit } => {
                write!(f, "folders nest at most {limit} levels deep")
            }
            OrganizeError::WouldNest { id } => {
                write!(f, "folder {id} cannot go inside itself")
            }
            OrganizeError::Malformed { message } => {
                write!(f, "folders file is not valid: {message}")
            }
            OrganizeError::UnsupportedVersion { version } => {
                write!(f, "folders file has unsupported version {version}")
            }
            OrganizeError::InvalidTitle { reason } => write!(f, "invalid title: {reason}"),
            OrganizeError::TooManyOps { limit } => {
                write!(f, "a batch holds at most {limit} operations")
            }
            OrganizeError::NotAllowed { reason } => write!(f, "not allowed: {reason}"),
            OrganizeError::Io { path, message } => {
                write!(f, "folders file i/o error at {path}: {message}")
            }
        }
    }
}

impl std::error::Error for OrganizeError {}

/// One change to the organisation (D53).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrganizeOp {
    CreateFolder {
        name: String,
        parent: Option<String>,
    },
    RenameFolder {
        id: String,
        name: String,
    },
    /// Moves the folder's sub-folders and conversations up to its parent (D51).
    DeleteFolder {
        id: String,
    },
    MoveFolder {
        id: String,
        parent: Option<String>,
    },
    MoveSession {
        session_id: String,
        folder: Option<String>,
    },
    /// Sets one of a conversation's settings (D57).
    SetSessionMeta {
        session_id: String,
        change: MetaChange,
    },
    /// Removes a conversation's filing and settings. The caller deletes its log (D59).
    ForgetSession {
        session_id: String,
    },
    /// Applies the operations in order, or none of them (D58).
    Batch(Vec<OrganizeOp>),
}

/// True for `f` followed by 1 to 9 ASCII digits.
pub fn valid_folder_id(id: &str) -> bool {
    match id.strip_prefix('f') {
        Some(digits) => {
            (1..=FolderIdDigits).contains(&digits.len())
                && digits.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

/// The trimmed name, or the reason it is not allowed (D51).
pub fn validate_folder_name(name: &str) -> Result<String, OrganizeError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(OrganizeError::InvalidName {
            reason: "the name is empty".to_string(),
        });
    }
    if trimmed.chars().count() > FolderNameLimit {
        return Err(OrganizeError::InvalidName {
            reason: format!("the name is longer than {FolderNameLimit} characters"),
        });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(OrganizeError::InvalidName {
            reason: "the name holds a control character".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

/// The trimmed title, or `None` when it is empty after trimming, which clears the title. A title
/// is refused when it is longer than `TitleLimit` characters or holds a control character (D57).
pub fn validate_title(title: &str) -> Result<Option<String>, OrganizeError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > TitleLimit {
        return Err(OrganizeError::InvalidTitle {
            reason: format!("the title is longer than {TitleLimit} characters"),
        });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(OrganizeError::InvalidTitle {
            reason: "the title holds a control character".to_string(),
        });
    }
    Ok(Some(trimmed.to_string()))
}

/// A stored title must already be in the form `validate_title` returns, so a file is never
/// silently rewritten on load.
fn checked_stored_title(title: &str) -> Result<String, OrganizeError> {
    match validate_title(title)? {
        Some(clean) if clean == title => Ok(clean),
        _ => Err(OrganizeError::Malformed {
            message: "a stored title is empty or not trimmed".to_string(),
        }),
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// True when `ops`, with every operation nested in a batch counted, holds no more operations than
/// `budget`. Each operation takes one unit, so the recursion is never deeper than the budget.
fn within_batch_budget(ops: &[OrganizeOp], budget: &mut usize) -> bool {
    for op in ops {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        if let OrganizeOp::Batch(inner) = op
            && !within_batch_budget(inner, budget)
        {
            return false;
        }
    }
    true
}

/// The folders and which conversation is in which. The fields are private, so every value of
/// this type satisfies the rules of D51.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct organization_t {
    folders: Vec<folder_t>,
    filed: BTreeMap<String, String>,
    meta: BTreeMap<String, session_meta_t>,
}

impl organization_t {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn folders(&self) -> &[folder_t] {
        &self.folders
    }

    pub fn folder(&self, id: &str) -> Option<&folder_t> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// The folders directly under `parent` (`None` is the top level), sorted by name ignoring
    /// case, then by id.
    pub fn children(&self, parent: Option<&str>) -> Vec<&folder_t> {
        let mut found: Vec<&folder_t> = self
            .folders
            .iter()
            .filter(|folder| folder.parent.as_deref() == parent)
            .collect();
        found.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        found
    }

    /// The sibling of that name under `parent`, ignoring case.
    pub fn find_child(&self, parent: Option<&str>, name: &str) -> Option<&folder_t> {
        let name = name.trim();
        self.folders
            .iter()
            .find(|folder| folder.parent.as_deref() == parent && same_name(&folder.name, name))
    }

    /// The folder a session is in. `None` for an unfiled session.
    pub fn folder_of(&self, session_id: &str) -> Option<&str> {
        self.filed.get(session_id).map(String::as_str)
    }

    /// The sessions filed directly in a folder, in id order.
    pub fn sessions_in(&self, folder_id: &str) -> Vec<&str> {
        self.filed
            .iter()
            .filter(|(_, folder)| folder.as_str() == folder_id)
            .map(|(session, _)| session.as_str())
            .collect()
    }

    /// Every filed session with its folder.
    pub fn filed(&self) -> impl Iterator<Item = (&str, &str)> {
        self.filed
            .iter()
            .map(|(session, folder)| (session.as_str(), folder.as_str()))
    }

    /// The level of a folder: 1 for a top-level folder.
    pub fn depth(&self, id: &str) -> Option<usize> {
        let mut depth = 0;
        let mut current = Some(id);
        while let Some(at) = current {
            let folder = self.folder(at)?;
            depth += 1;
            if depth > MaxFolderDepth + 1 {
                break;
            }
            current = folder.parent.as_deref();
        }
        Some(depth)
    }

    /// The folders from the top level down to `id`, inclusive. Empty when `id` is unknown.
    pub fn path(&self, id: &str) -> Vec<&folder_t> {
        let mut path = Vec::new();
        let mut current = Some(id);
        while let Some(at) = current {
            let Some(folder) = self.folder(at) else {
                return Vec::new();
            };
            path.push(folder);
            if path.len() > MaxFolderDepth + 1 {
                break;
            }
            current = folder.parent.as_deref();
        }
        path.reverse();
        path
    }

    /// True when `id` is `ancestor` or lies below it.
    pub fn is_within(&self, id: &str, ancestor: &str) -> bool {
        self.path(id).iter().any(|folder| folder.id == ancestor)
    }

    /// The levels a folder and everything below it take: 1 for a folder with no sub-folders.
    pub fn height(&self, id: &str) -> usize {
        let below = self
            .folders
            .iter()
            .filter(|folder| folder.parent.as_deref() == Some(id))
            .map(|folder| self.height(&folder.id))
            .max()
            .unwrap_or(0);
        below + 1
    }

    /// The conversations filed in a folder and in every folder below it.
    pub fn session_count(&self, id: &str) -> usize {
        self.filed
            .values()
            .filter(|folder| self.is_within(folder, id))
            .count()
    }

    /// Whether a folder may be created under `parent`.
    pub fn can_create_under(&self, parent: Option<&str>) -> Result<(), OrganizeError> {
        if let Some(parent) = parent {
            let depth = self
                .depth(parent)
                .ok_or_else(|| OrganizeError::FolderNotFound {
                    id: parent.to_string(),
                })?;
            if depth + 1 > MaxFolderDepth {
                return Err(OrganizeError::TooDeep {
                    limit: MaxFolderDepth,
                });
            }
        }
        Ok(())
    }

    /// Whether a folder may move under `parent` (`None` is the top level). The UI uses this to
    /// show which destinations are allowed.
    pub fn can_move_folder(&self, id: &str, parent: Option<&str>) -> Result<(), OrganizeError> {
        let folder = self
            .folder(id)
            .ok_or_else(|| OrganizeError::FolderNotFound { id: id.to_string() })?;
        let new_depth = match parent {
            Some(parent) => {
                let depth = self
                    .depth(parent)
                    .ok_or_else(|| OrganizeError::FolderNotFound {
                        id: parent.to_string(),
                    })?;
                if self.is_within(parent, id) {
                    return Err(OrganizeError::WouldNest { id: id.to_string() });
                }
                depth + 1
            }
            None => 1,
        };
        if new_depth + self.height(id) - 1 > MaxFolderDepth {
            return Err(OrganizeError::TooDeep {
                limit: MaxFolderDepth,
            });
        }
        match self.find_child(parent, &folder.name) {
            Some(other) if other.id != id => Err(OrganizeError::NameTaken {
                name: folder.name.clone(),
            }),
            _ => Ok(()),
        }
    }

    fn next_folder_id(&self) -> String {
        let largest = self
            .folders
            .iter()
            .filter_map(|folder| folder.id.strip_prefix('f')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("f{}", largest + 1)
    }

    /// Creates a folder and returns its id.
    pub fn create_folder(
        &mut self,
        name: &str,
        parent: Option<&str>,
    ) -> Result<String, OrganizeError> {
        let name = validate_folder_name(name)?;
        self.can_create_under(parent)?;
        if self.find_child(parent, &name).is_some() {
            return Err(OrganizeError::NameTaken { name });
        }
        let id = self.next_folder_id();
        self.folders.push(folder_t {
            id: id.clone(),
            name,
            parent: parent.map(str::to_string),
        });
        Ok(id)
    }

    pub fn rename_folder(&mut self, id: &str, name: &str) -> Result<(), OrganizeError> {
        let name = validate_folder_name(name)?;
        let parent = self
            .folder(id)
            .ok_or_else(|| OrganizeError::FolderNotFound { id: id.to_string() })?
            .parent
            .clone();
        if let Some(other) = self.find_child(parent.as_deref(), &name)
            && other.id != id
        {
            return Err(OrganizeError::NameTaken { name });
        }
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) {
            folder.name = name;
        }
        Ok(())
    }

    /// Deletes a folder. Its sub-folders and conversations move up to its parent (D51). The
    /// delete is refused when a sub-folder would clash by name with a folder at that level.
    pub fn delete_folder(&mut self, id: &str) -> Result<(), OrganizeError> {
        let parent = self
            .folder(id)
            .ok_or_else(|| OrganizeError::FolderNotFound { id: id.to_string() })?
            .parent
            .clone();
        for child in self.children(Some(id)) {
            if let Some(other) = self.find_child(parent.as_deref(), &child.name)
                && other.id != id
            {
                return Err(OrganizeError::NameTaken {
                    name: child.name.clone(),
                });
            }
        }
        for folder in self.folders.iter_mut() {
            if folder.parent.as_deref() == Some(id) {
                folder.parent = parent.clone();
            }
        }
        match &parent {
            Some(parent) => {
                for folder in self.filed.values_mut() {
                    if folder == id {
                        *folder = parent.clone();
                    }
                }
            }
            None => self.filed.retain(|_, folder| folder != id),
        }
        self.folders.retain(|folder| folder.id != id);
        Ok(())
    }

    pub fn move_folder(&mut self, id: &str, parent: Option<&str>) -> Result<(), OrganizeError> {
        self.can_move_folder(id, parent)?;
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) {
            folder.parent = parent.map(str::to_string);
        }
        Ok(())
    }

    /// Files a session in a folder, or unfiles it when `folder` is `None`.
    pub fn move_session(
        &mut self,
        session_id: &str,
        folder: Option<&str>,
    ) -> Result<(), OrganizeError> {
        if !valid_session_id(session_id) {
            return Err(OrganizeError::InvalidId {
                id: session_id.to_string(),
            });
        }
        match folder {
            Some(folder) => {
                if self.folder(folder).is_none() {
                    return Err(OrganizeError::FolderNotFound {
                        id: folder.to_string(),
                    });
                }
                self.filed
                    .insert(session_id.to_string(), folder.to_string());
            }
            None => {
                self.filed.remove(session_id);
            }
        }
        Ok(())
    }

    /// The settings of a conversation. The defaults when none were set.
    pub fn meta(&self, session_id: &str) -> session_meta_t {
        self.meta.get(session_id).cloned().unwrap_or_default()
    }

    /// Every conversation that has a setting other than the default, in id order.
    pub fn metas(&self) -> impl Iterator<Item = (&str, &session_meta_t)> {
        self.meta
            .iter()
            .map(|(session, meta)| (session.as_str(), meta))
    }

    /// Changes one setting of a conversation (D57). A setting that ends up at its default is
    /// removed, so the file holds only what differs from the default.
    pub fn set_meta(&mut self, session_id: &str, change: &MetaChange) -> Result<(), OrganizeError> {
        if !valid_session_id(session_id) {
            return Err(OrganizeError::InvalidId {
                id: session_id.to_string(),
            });
        }
        let mut meta = self.meta(session_id);
        match change {
            MetaChange::Title(title) => {
                meta.title = match title {
                    Some(title) => validate_title(title)?,
                    None => None,
                };
            }
            MetaChange::Pinned(pinned) => meta.pinned = *pinned,
            MetaChange::Color(color) => meta.color = *color,
            MetaChange::Archived(archived) => meta.archived = *archived,
            MetaChange::Approval(Some(ApprovalMode::Full)) => {
                return Err(OrganizeError::NotAllowed {
                    reason: "approval mode full can be selected only on the command line"
                        .to_string(),
                });
            }
            MetaChange::Approval(mode) => meta.approval = *mode,
        }
        if meta.is_default() {
            self.meta.remove(session_id);
        } else {
            self.meta.insert(session_id.to_string(), meta);
        }
        Ok(())
    }

    /// Removes a conversation's filing and its settings. The session log is not touched here
    /// (D59).
    pub fn forget_session(&mut self, session_id: &str) {
        self.filed.remove(session_id);
        self.meta.remove(session_id);
    }

    /// Applies the operations in order to a copy, and keeps the copy only when all of them
    /// succeed (D58).
    fn apply_batch(&mut self, ops: &[OrganizeOp]) -> Result<(), OrganizeError> {
        let mut budget = MaxBatchOps;
        if !within_batch_budget(ops, &mut budget) {
            return Err(OrganizeError::TooManyOps { limit: MaxBatchOps });
        }
        let mut next = self.clone();
        for op in ops {
            next.apply(op)?;
        }
        *self = next;
        Ok(())
    }

    pub fn apply(&mut self, op: &OrganizeOp) -> Result<(), OrganizeError> {
        match op {
            OrganizeOp::CreateFolder { name, parent } => {
                self.create_folder(name, parent.as_deref()).map(|_| ())
            }
            OrganizeOp::RenameFolder { id, name } => self.rename_folder(id, name),
            OrganizeOp::DeleteFolder { id } => self.delete_folder(id),
            OrganizeOp::MoveFolder { id, parent } => self.move_folder(id, parent.as_deref()),
            OrganizeOp::MoveSession { session_id, folder } => {
                self.move_session(session_id, folder.as_deref())
            }
            OrganizeOp::SetSessionMeta { session_id, change } => self.set_meta(session_id, change),
            OrganizeOp::ForgetSession { session_id } => {
                if !valid_session_id(session_id) {
                    return Err(OrganizeError::InvalidId {
                        id: session_id.clone(),
                    });
                }
                self.forget_session(session_id);
                Ok(())
            }
            OrganizeOp::Batch(ops) => self.apply_batch(ops),
        }
    }

    /// Checks a file against every rule of D51 and D52 and builds the organisation from it.
    pub fn from_file(file: &organization_file_t) -> Result<Self, OrganizeError> {
        let malformed = |message: String| OrganizeError::Malformed { message };
        if file.version != OrganizationVersion {
            return Err(OrganizeError::UnsupportedVersion {
                version: file.version,
            });
        }
        let mut ids = BTreeSet::new();
        let mut folders = Vec::with_capacity(file.folders.len());
        for folder in &file.folders {
            if !valid_folder_id(&folder.id) {
                return Err(OrganizeError::InvalidId {
                    id: folder.id.clone(),
                });
            }
            if !ids.insert(folder.id.as_str()) {
                return Err(malformed(format!("folder id {} appears twice", folder.id)));
            }
            folders.push(folder_t {
                id: folder.id.clone(),
                name: validate_folder_name(&folder.name)?,
                parent: folder.parent.clone(),
            });
        }
        for folder in &folders {
            if let Some(parent) = &folder.parent
                && !ids.contains(parent.as_str())
            {
                return Err(malformed(format!(
                    "folder {} has the missing parent {parent}",
                    folder.id
                )));
            }
        }
        let org = organization_t {
            folders,
            filed: BTreeMap::new(),
            meta: BTreeMap::new(),
        };
        for folder in &org.folders {
            let mut seen = BTreeSet::new();
            let mut current = Some(folder.id.as_str());
            while let Some(at) = current {
                if !seen.insert(at) {
                    return Err(malformed(format!(
                        "folder {} is its own ancestor",
                        folder.id
                    )));
                }
                current = org.folder(at).and_then(|f| f.parent.as_deref());
            }
            if seen.len() > MaxFolderDepth {
                return Err(OrganizeError::TooDeep {
                    limit: MaxFolderDepth,
                });
            }
            let clash = org.folders.iter().any(|other| {
                other.id != folder.id
                    && other.parent == folder.parent
                    && same_name(&other.name, &folder.name)
            });
            if clash {
                return Err(OrganizeError::NameTaken {
                    name: folder.name.clone(),
                });
            }
        }
        let mut filed = BTreeMap::new();
        for (session, folder) in &file.filed {
            if !valid_session_id(session) {
                return Err(OrganizeError::InvalidId {
                    id: session.clone(),
                });
            }
            if org.folder(folder).is_none() {
                return Err(malformed(format!(
                    "session {session} is filed in the missing folder {folder}"
                )));
            }
            filed.insert(session.clone(), folder.clone());
        }
        let mut meta = BTreeMap::new();
        for (session, stored) in &file.meta {
            if !valid_session_id(session) {
                return Err(OrganizeError::InvalidId {
                    id: session.clone(),
                });
            }
            if stored.approval == Some(ApprovalMode::Full) {
                return Err(OrganizeError::NotAllowed {
                    reason: "approval mode full can be selected only on the command line"
                        .to_string(),
                });
            }
            let entry = session_meta_t {
                title: match &stored.title {
                    Some(title) => Some(checked_stored_title(title)?),
                    None => None,
                },
                pinned: stored.pinned,
                color: stored.color,
                archived: stored.archived,
                approval: stored.approval,
            };
            if !entry.is_default() {
                meta.insert(session.clone(), entry);
            }
        }
        Ok(organization_t {
            folders: org.folders,
            filed,
            meta,
        })
    }

    /// The file form of this organisation.
    pub fn to_file(&self, label: Option<&str>) -> organization_file_t {
        organization_file_t {
            version: OrganizationVersion,
            label: label.map(str::to_string),
            folders: self.folders.clone(),
            filed: self.filed.clone(),
            meta: self.meta.clone(),
        }
    }
}

/// Where the organisation is kept. A change is a load, an operation and a save, so a store
/// whose file does not load is never overwritten (D52).
pub trait organization_store_t: Send + Sync {
    /// The saved organisation. An empty one when nothing was saved yet.
    fn load(&self) -> Result<organization_t, OrganizeError>;

    fn save(&self, organization: &organization_t) -> Result<(), OrganizeError>;
}

/// An organisation kept in memory until the process ends.
#[derive(Debug, Default)]
pub struct memory_organization_store_t {
    held: Mutex<organization_t>,
}

impl memory_organization_store_t {
    pub fn new() -> Self {
        Self::default()
    }

    /// The held value, even when a panic poisoned the lock. The value is replaced whole, so it
    /// is never half-written.
    fn locked(&self) -> MutexGuard<'_, organization_t> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl organization_store_t for memory_organization_store_t {
    fn load(&self) -> Result<organization_t, OrganizeError> {
        Ok(self.locked().clone())
    }

    fn save(&self, organization: &organization_t) -> Result<(), OrganizeError> {
        *self.locked() = organization.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok<T, E: fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(err) => panic!("expected Ok, found {err:?}"),
        }
    }

    fn sample() -> organization_t {
        let mut org = organization_t::new();
        let research = ok(org.create_folder("Research", None));
        let papers = ok(org.create_folder("Papers", Some(&research)));
        ok(org.create_folder("Ideas", None));
        ok(org.move_session("s_1", Some(&papers)));
        ok(org.move_session("s_2", Some(&research)));
        org
    }

    #[test]
    fn folders_nest_and_report_their_depth_path_and_counts() {
        let org = sample();
        assert_eq!(org.depth("f2"), Some(2));
        let names: Vec<&str> = org.path("f2").iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Research", "Papers"]);
        assert_eq!(org.session_count("f1"), 2);
        assert_eq!(org.session_count("f2"), 1);
        assert_eq!(org.height("f1"), 2);
        let top: Vec<&str> = org.children(None).iter().map(|f| f.name.as_str()).collect();
        assert_eq!(top, ["Ideas", "Research"]);
    }

    #[test]
    fn names_are_trimmed_checked_and_unique_among_siblings_ignoring_case() {
        let mut org = organization_t::new();
        assert!(matches!(
            org.create_folder("  ", None),
            Err(OrganizeError::InvalidName { .. })
        ));
        assert!(matches!(
            org.create_folder("a\u{7}b", None),
            Err(OrganizeError::InvalidName { .. })
        ));
        assert!(matches!(
            org.create_folder(&"x".repeat(FolderNameLimit + 1), None),
            Err(OrganizeError::InvalidName { .. })
        ));
        let id = ok(org.create_folder("  Work ", None));
        assert_eq!(org.folder(&id).map(|f| f.name.as_str()), Some("Work"));
        assert!(matches!(
            org.create_folder("work", None),
            Err(OrganizeError::NameTaken { .. })
        ));
        ok(org.create_folder("work", Some(&id)));
    }

    #[test]
    fn nesting_stops_at_the_depth_limit() {
        let mut org = organization_t::new();
        let mut parent: Option<String> = None;
        for level in 0..MaxFolderDepth {
            parent = Some(ok(
                org.create_folder(&format!("l{level}"), parent.as_deref())
            ));
        }
        assert!(matches!(
            org.create_folder("one more", parent.as_deref()),
            Err(OrganizeError::TooDeep { .. })
        ));
    }

    #[test]
    fn a_folder_cannot_move_into_itself_or_below_the_depth_limit_or_onto_a_name() {
        let mut org = sample();
        assert!(matches!(
            org.move_folder("f1", Some("f2")),
            Err(OrganizeError::WouldNest { .. })
        ));
        assert!(matches!(
            org.move_folder("f1", Some("f1")),
            Err(OrganizeError::WouldNest { .. })
        ));
        ok(org.create_folder("Papers", None));
        assert!(matches!(
            org.move_folder("f2", None),
            Err(OrganizeError::NameTaken { .. })
        ));
        ok(org.move_folder("f3", Some("f1")));
        assert_eq!(
            org.folder("f3").and_then(|f| f.parent.as_deref()),
            Some("f1")
        );
    }

    #[test]
    fn a_refused_operation_changes_nothing() {
        let mut org = sample();
        let before = org.clone();
        assert!(org.move_folder("f1", Some("f2")).is_err());
        assert!(org.rename_folder("f2", "Ideas").is_ok());
        assert!(org.rename_folder("f1", "ideas").is_err());
        assert!(org.move_session("../x", Some("f1")).is_err());
        assert!(org.move_session("s_9", Some("f99")).is_err());
        assert_eq!(org.folder_of("s_9"), None);
        let mut renamed = before;
        ok(renamed.rename_folder("f2", "Ideas"));
        assert_eq!(org, renamed);
    }

    #[test]
    fn deleting_a_folder_moves_its_contents_up() {
        let mut org = sample();
        ok(org.delete_folder("f1"));
        assert!(org.folder("f1").is_none());
        assert_eq!(org.folder("f2").and_then(|f| f.parent.as_deref()), None);
        assert_eq!(org.folder_of("s_1"), Some("f2"));
        assert_eq!(org.folder_of("s_2"), None);
    }

    #[test]
    fn a_delete_that_would_clash_by_name_is_refused() {
        let mut org = sample();
        ok(org.create_folder("Papers", None));
        assert!(matches!(
            org.delete_folder("f1"),
            Err(OrganizeError::NameTaken { .. })
        ));
        assert!(org.folder("f1").is_some());
    }

    #[test]
    fn moving_a_session_files_and_unfiles_it() {
        let mut org = sample();
        ok(org.move_session("s_1", None));
        assert_eq!(org.folder_of("s_1"), None);
        ok(org.move_session("s_1", Some("f3")));
        assert_eq!(org.sessions_in("f3"), ["s_1"]);
    }

    #[test]
    fn a_file_round_trips_and_a_bad_file_is_refused() {
        let org = sample();
        let file = org.to_file(Some("DEMO"));
        assert_eq!(ok(organization_t::from_file(&file)), org);

        let mut bad = file.clone();
        bad.version = 2;
        assert!(matches!(
            organization_t::from_file(&bad),
            Err(OrganizeError::UnsupportedVersion { version: 2 })
        ));

        let mut cycle = file.clone();
        cycle.folders[0].parent = Some("f2".to_string());
        assert!(matches!(
            organization_t::from_file(&cycle),
            Err(OrganizeError::Malformed { .. })
        ));

        let mut missing = file.clone();
        missing.filed.insert("s_3".to_string(), "f77".to_string());
        assert!(matches!(
            organization_t::from_file(&missing),
            Err(OrganizeError::Malformed { .. })
        ));

        let mut twice = file.clone();
        let copy = twice.folders[0].clone();
        twice.folders.push(copy);
        assert!(matches!(
            organization_t::from_file(&twice),
            Err(OrganizeError::Malformed { .. })
        ));

        let mut odd_id = file;
        odd_id.folders[0].id = "../f".to_string();
        assert!(matches!(
            organization_t::from_file(&odd_id),
            Err(OrganizeError::InvalidId { .. })
        ));
    }

    #[test]
    fn the_memory_store_saves_and_loads() {
        let store = memory_organization_store_t::new();
        assert_eq!(ok(store.load()), organization_t::new());
        let org = sample();
        ok(store.save(&org));
        assert_eq!(ok(store.load()), org);
    }

    fn meta_sample() -> organization_t {
        let mut org = sample();
        ok(org.set_meta("s_1", &MetaChange::Pinned(true)));
        org
    }

    #[test]
    fn tag_colors_have_fixed_lowercase_names() {
        assert_eq!(TagColor::all().len(), 8);
        for color in TagColor::all() {
            assert_eq!(TagColor::from_name(color.name()), Some(color));
        }
        assert_eq!(TagColor::from_name("Red"), None);
        assert_eq!(TagColor::from_name("magenta"), None);
        assert_eq!(ok(serde_json::to_string(&TagColor::Teal)), "\"teal\"");
    }

    #[test]
    fn session_settings_are_kept_and_a_default_setting_leaves_no_entry() {
        let mut org = sample();
        assert_eq!(org.meta("s_1"), session_meta_t::default());
        ok(org.set_meta("s_1", &MetaChange::Pinned(true)));
        ok(org.set_meta("s_1", &MetaChange::Color(Some(TagColor::Blue))));
        ok(org.set_meta("s_1", &MetaChange::Archived(true)));
        let meta = org.meta("s_1");
        assert!(meta.pinned && meta.archived);
        assert_eq!(meta.color, Some(TagColor::Blue));
        assert_eq!(org.metas().count(), 1);

        ok(org.set_meta("s_1", &MetaChange::Pinned(false)));
        ok(org.set_meta("s_1", &MetaChange::Color(None)));
        ok(org.set_meta("s_1", &MetaChange::Archived(false)));
        assert!(org.meta("s_1").is_default());
        assert_eq!(org.metas().count(), 0);
    }

    #[test]
    fn titles_are_trimmed_cleared_when_empty_and_refused_when_bad() {
        let mut org = sample();
        ok(org.set_meta("s_1", &MetaChange::Title(Some("  Plan  ".to_string()))));
        assert_eq!(org.meta("s_1").title.as_deref(), Some("Plan"));
        ok(org.set_meta("s_1", &MetaChange::Title(Some("   ".to_string()))));
        assert_eq!(org.meta("s_1").title, None);
        assert_eq!(org.metas().count(), 0);

        let before = org.clone();
        assert!(matches!(
            org.set_meta("s_1", &MetaChange::Title(Some("t".repeat(TitleLimit + 1)))),
            Err(OrganizeError::InvalidTitle { .. })
        ));
        assert!(matches!(
            org.set_meta("s_1", &MetaChange::Title(Some("a\u{7}b".to_string()))),
            Err(OrganizeError::InvalidTitle { .. })
        ));
        assert_eq!(org, before, "a refused title changes nothing");

        ok(org.set_meta("s_1", &MetaChange::Title(Some("t".repeat(TitleLimit)))));
        assert_eq!(
            org.meta("s_1").title.map(|title| title.chars().count()),
            Some(TitleLimit)
        );
    }

    #[test]
    fn full_is_refused_and_the_other_approval_modes_are_kept() {
        let mut org = sample();
        assert!(matches!(
            org.set_meta("s_1", &MetaChange::Approval(Some(ApprovalMode::Full))),
            Err(OrganizeError::NotAllowed { .. })
        ));
        assert_eq!(org.metas().count(), 0);
        ok(org.set_meta("s_1", &MetaChange::Approval(Some(ApprovalMode::Edit))));
        assert_eq!(org.meta("s_1").approval, Some(ApprovalMode::Edit));
        ok(org.set_meta("s_1", &MetaChange::Approval(None)));
        assert!(org.meta("s_1").is_default());
    }

    #[test]
    fn a_setting_needs_a_plain_session_id() {
        let mut org = sample();
        assert!(matches!(
            org.set_meta("../x", &MetaChange::Pinned(true)),
            Err(OrganizeError::InvalidId { .. })
        ));
        assert!(matches!(
            org.apply(&OrganizeOp::ForgetSession {
                session_id: "a/b".to_string()
            }),
            Err(OrganizeError::InvalidId { .. })
        ));
    }

    #[test]
    fn forgetting_a_session_removes_its_filing_and_settings_only() {
        let mut org = meta_sample();
        ok(org.apply(&OrganizeOp::ForgetSession {
            session_id: "s_1".to_string(),
        }));
        assert_eq!(org.folder_of("s_1"), None);
        assert!(org.meta("s_1").is_default());
        assert_eq!(org.folder_of("s_2"), Some("f1"));
        ok(org.apply(&OrganizeOp::ForgetSession {
            session_id: "never-seen".to_string(),
        }));
    }

    #[test]
    fn a_batch_applies_every_operation_or_none() {
        let mut org = sample();
        let batch = OrganizeOp::Batch(vec![
            OrganizeOp::CreateFolder {
                name: "Batch".to_string(),
                parent: None,
            },
            OrganizeOp::SetSessionMeta {
                session_id: "s_1".to_string(),
                change: MetaChange::Pinned(true),
            },
            OrganizeOp::MoveSession {
                session_id: "s_1".to_string(),
                folder: Some("f4".to_string()),
            },
        ]);
        ok(org.apply(&batch));
        assert_eq!(org.folder("f4").map(|f| f.name.as_str()), Some("Batch"));
        assert_eq!(org.folder_of("s_1"), Some("f4"));
        assert!(org.meta("s_1").pinned);

        let before = org.clone();
        let failing = OrganizeOp::Batch(vec![
            OrganizeOp::CreateFolder {
                name: "Never".to_string(),
                parent: None,
            },
            OrganizeOp::MoveSession {
                session_id: "s_2".to_string(),
                folder: Some("f4".to_string()),
            },
            OrganizeOp::RenameFolder {
                id: "f99".to_string(),
                name: "Missing".to_string(),
            },
        ]);
        assert!(matches!(
            org.apply(&failing),
            Err(OrganizeError::FolderNotFound { .. })
        ));
        assert_eq!(org, before, "a failing batch changes nothing");
    }

    #[test]
    fn a_batch_may_nest_and_runs_in_order() {
        let mut org = sample();
        let nested = OrganizeOp::Batch(vec![
            OrganizeOp::Batch(vec![OrganizeOp::CreateFolder {
                name: "Inner".to_string(),
                parent: None,
            }]),
            OrganizeOp::MoveSession {
                session_id: "s_9".to_string(),
                folder: Some("f4".to_string()),
            },
        ]);
        ok(org.apply(&nested));
        assert_eq!(org.folder_of("s_9"), Some("f4"));
        assert_eq!(org.folder("f4").map(|f| f.name.as_str()), Some("Inner"));
        ok(org.apply(&OrganizeOp::Batch(Vec::new())));
    }

    #[test]
    fn a_batch_is_limited_to_max_batch_ops_counting_nested_ones() {
        let forget = || OrganizeOp::ForgetSession {
            session_id: "s_9".to_string(),
        };
        let mut org = organization_t::new();
        let full: Vec<OrganizeOp> = (0..MaxBatchOps).map(|_| forget()).collect();
        ok(org.apply(&OrganizeOp::Batch(full)));

        let too_many: Vec<OrganizeOp> = (0..=MaxBatchOps).map(|_| forget()).collect();
        assert_eq!(
            org.apply(&OrganizeOp::Batch(too_many)),
            Err(OrganizeError::TooManyOps { limit: MaxBatchOps })
        );

        // The nested batch takes one unit itself: 255 operations inside it make 256 in all.
        let inside_limit = vec![OrganizeOp::Batch(
            (0..MaxBatchOps - 1).map(|_| forget()).collect(),
        )];
        ok(org.apply(&OrganizeOp::Batch(inside_limit)));
        let over_limit = vec![OrganizeOp::Batch(
            (0..MaxBatchOps).map(|_| forget()).collect(),
        )];
        assert!(matches!(
            org.apply(&OrganizeOp::Batch(over_limit)),
            Err(OrganizeError::TooManyOps { .. })
        ));
    }

    #[test]
    fn settings_round_trip_through_the_file_and_defaults_are_not_stored() {
        let mut org = sample();
        ok(org.set_meta("s_1", &MetaChange::Title(Some("Plan".to_string()))));
        ok(org.set_meta("s_2", &MetaChange::Approval(Some(ApprovalMode::Edit))));
        let file = org.to_file(None);
        assert_eq!(file.meta.len(), 2);
        assert_eq!(ok(organization_t::from_file(&file)), org);
        let text = ok(serde_json::to_string(&file));
        assert!(text.contains("\"title\":\"Plan\""));

        let mut stored = file;
        stored
            .meta
            .insert("s_3".to_string(), session_meta_t::default());
        let loaded = ok(organization_t::from_file(&stored));
        assert_eq!(loaded.metas().count(), 2, "a default entry is dropped");
    }

    #[test]
    fn a_file_without_settings_still_loads() {
        let text = r#"{"version":1,"folders":[{"id":"f1","name":"A","parent":null}],
            "filed":{"s_1":"f1"}}"#;
        let file: organization_file_t = ok(serde_json::from_str(text));
        assert!(file.meta.is_empty());
        let org = ok(organization_t::from_file(&file));
        assert_eq!(org.folder_of("s_1"), Some("f1"));
        assert_eq!(org.metas().count(), 0);
    }

    #[test]
    fn a_file_with_bad_settings_is_refused() {
        let file = meta_sample().to_file(None);

        let mut bad_id = file.clone();
        bad_id.meta.insert(
            "../x".to_string(),
            session_meta_t {
                pinned: true,
                ..Default::default()
            },
        );
        assert!(matches!(
            organization_t::from_file(&bad_id),
            Err(OrganizeError::InvalidId { .. })
        ));

        let mut full = file.clone();
        full.meta.insert(
            "s_2".to_string(),
            session_meta_t {
                approval: Some(ApprovalMode::Full),
                ..Default::default()
            },
        );
        assert!(matches!(
            organization_t::from_file(&full),
            Err(OrganizeError::NotAllowed { .. })
        ));

        let untrimmed = session_meta_t {
            title: Some(" Plan".to_string()),
            ..Default::default()
        };
        let mut untrimmed_file = file.clone();
        untrimmed_file.meta.insert("s_2".to_string(), untrimmed);
        assert!(matches!(
            organization_t::from_file(&untrimmed_file),
            Err(OrganizeError::Malformed { .. })
        ));

        let mut long = file.clone();
        long.meta.insert(
            "s_2".to_string(),
            session_meta_t {
                title: Some("t".repeat(TitleLimit + 1)),
                ..Default::default()
            },
        );
        assert!(matches!(
            organization_t::from_file(&long),
            Err(OrganizeError::InvalidTitle { .. })
        ));

        let mut empty = file;
        empty.meta.insert(
            "s_2".to_string(),
            session_meta_t {
                title: Some(String::new()),
                ..Default::default()
            },
        );
        assert!(matches!(
            organization_t::from_file(&empty),
            Err(OrganizeError::Malformed { .. })
        ));
    }
}
