/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Settings and custom themes (PROJECT.md, D63 to D65).
//!
//! The harness holds the types, the rules and the store trait. The settings are the user's
//! preferences, not records, so they carry no demo label. A theme is a JSON file the user can
//! import; the demo's file store reads it as untrusted input. Whether a colour name is a field of
//! the UI's theme type, and whether an id is a built-in theme, is checked by the UI, because this
//! crate does not know the UI's themes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::approval::ApprovalMode;
use crate::store::valid_session_id;

/// The theme a fresh install uses, and the `extends` a custom theme gets by default (D64).
pub const DefaultTheme: &str = "ncagent-dark";

/// Most tabs that can be saved for the next start (D62, D63).
pub const MaxSavedTabs: usize = 32;

/// Most colours a custom theme can set (D64).
pub const MaxThemeColors: usize = 64;

/// Longest theme id, in bytes (D64).
const ThemeIdLimit: usize = 40;

/// Longest theme name, in characters (D64).
const ThemeNameLimit: usize = 40;

/// Longest colour name, in bytes: a lowercase letter, then up to 31 more (D64).
const ColorNameLimit: usize = 32;

/// When the sidebar is shown (D63).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarVisibility {
    Auto,
    Pinned,
    Hidden,
}

/// How wide the sidebar is drawn (D63).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarWidth {
    Compact,
    Normal,
    Wide,
}

/// How much space each row takes (D63).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Comfortable,
    Compact,
}

/// Whether a tool result is folded when it arrives (D63).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultView {
    Folded,
    Expanded,
}

/// Where a conversation opens when it is chosen (D63).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenIn {
    NewTab,
    CurrentTab,
}

/// How the conversation list is ordered (D63).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationOrder {
    Recent,
    Title,
}

/// The user's settings (D63). Unknown fields in a file are ignored, and a missing field takes its
/// default, so a file from a later or earlier build still loads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct settings_t {
    /// A built-in theme id, or the id of a custom theme.
    pub theme: String,
    pub animation: bool,
    pub sidebar: SidebarVisibility,
    pub sidebar_width: SidebarWidth,
    pub density: Density,
    pub show_context: bool,
    pub show_turn: bool,
    pub show_generation: bool,
    pub show_model: bool,
    pub show_row_details: bool,
    pub tool_results: ToolResultView,
    /// The approval mode of a new conversation. `full` is refused (D15).
    pub default_approval: ApprovalMode,
    pub confirm_delete: bool,
    pub open_in: OpenIn,
    pub show_archived: bool,
    pub conversation_order: ConversationOrder,
    pub restore_tabs: bool,
    /// The conversations whose tabs were open, in tab order (D62).
    pub open_tabs: Vec<String>,
    /// The tab that was active. Must be one of `open_tabs` when set.
    pub active_tab: Option<String>,
}

impl Default for settings_t {
    fn default() -> Self {
        settings_t {
            theme: DefaultTheme.to_string(),
            animation: true,
            sidebar: SidebarVisibility::Auto,
            sidebar_width: SidebarWidth::Normal,
            density: Density::Comfortable,
            show_context: true,
            show_turn: true,
            show_generation: true,
            show_model: true,
            show_row_details: true,
            tool_results: ToolResultView::Folded,
            default_approval: ApprovalMode::Ask,
            confirm_delete: true,
            open_in: OpenIn::NewTab,
            show_archived: false,
            conversation_order: ConversationOrder::Recent,
            restore_tabs: true,
            open_tabs: Vec::new(),
            active_tab: None,
        }
    }
}

impl settings_t {
    /// Checks every rule a saved value must hold. The store calls this before it writes, and
    /// after it reads.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !valid_theme_id(&self.theme) {
            return Err(invalid(format!(
                "the theme id {:?} is not valid",
                self.theme
            )));
        }
        if self.default_approval == ApprovalMode::Full {
            return Err(invalid(
                "the default approval mode cannot be full, which is selected only on the command line"
                    .to_string(),
            ));
        }
        if self.open_tabs.len() > MaxSavedTabs {
            return Err(invalid(format!("at most {MaxSavedTabs} tabs can be saved")));
        }
        let mut seen = BTreeSet::new();
        for tab in &self.open_tabs {
            if !valid_session_id(tab) {
                return Err(invalid(format!(
                    "the saved tab {tab:?} is not a session id"
                )));
            }
            if !seen.insert(tab.as_str()) {
                return Err(invalid(format!("the tab {tab} is saved twice")));
            }
        }
        if let Some(active) = &self.active_tab
            && !seen.contains(active.as_str())
        {
            return Err(invalid(format!(
                "the active tab {active} is not one of the saved tabs"
            )));
        }
        Ok(())
    }
}

/// True for an id of 1 to 40 bytes: a lowercase ASCII letter or digit first, then lowercase
/// letters, digits, `-` or `_` (D64).
pub fn valid_theme_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    match bytes.split_first() {
        Some((first, rest)) => {
            (1..=ThemeIdLimit).contains(&bytes.len())
                && (first.is_ascii_lowercase() || first.is_ascii_digit())
                && rest.iter().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(*byte, b'-' | b'_')
                })
        }
        None => false,
    }
}

/// A theme id made from a name (D64). Each ASCII letter or digit is kept in lowercase. Each run
/// of any other character becomes one `-`, and no id starts or ends with `-`. The result is cut
/// to 40 bytes and is `theme` when nothing is left. A character that is not ASCII counts as
/// "other".
pub fn slugify_theme_name(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.truncate(ThemeIdLimit);
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "theme".to_string()
    } else {
        slug
    }
}

/// The trimmed name, when it is 1 to 40 characters with no control character (D64).
fn checked_theme_name(name: &str) -> Result<String, SettingsError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(invalid("the theme name is empty".to_string()));
    }
    if trimmed.chars().count() > ThemeNameLimit {
        return Err(invalid(format!(
            "the theme name is longer than {ThemeNameLimit} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(invalid(
            "the theme name holds a control character".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

/// True for `[a-z][a-z0-9_]{0,31}`.
fn valid_color_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    match bytes.split_first() {
        Some((first, rest)) => {
            first.is_ascii_lowercase()
                && bytes.len() <= ColorNameLimit
                && rest
                    .iter()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        }
        None => false,
    }
}

/// The colour in lowercase, when it is `#` and exactly six hex digits (D64).
fn checked_color(value: &str) -> Result<String, SettingsError> {
    match value.strip_prefix('#') {
        Some(hex) if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            Ok(value.to_ascii_lowercase())
        }
        _ => Err(invalid(format!("the colour {value:?} is not #rrggbb"))),
    }
}

/// A custom theme as the file holds it (D64). The fields are public so a store can read and
/// write it; `custom_theme_t::from_file` checks it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct theme_file_t {
    pub name: String,
    /// Defaults to the slug of the name.
    #[serde(default)]
    pub id: Option<String>,
    /// Defaults to `ncagent-dark`.
    #[serde(default)]
    pub extends: Option<String>,
    /// A map from a theme colour name to `#rrggbb`.
    pub colors: BTreeMap<String, String>,
}

/// A custom theme that passed every rule of D64.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct custom_theme_t {
    pub id: String,
    pub name: String,
    /// The built-in or custom theme this one builds on.
    pub extends: String,
    /// Lowercase `#rrggbb` values, keyed by colour name.
    pub colors: BTreeMap<String, String>,
}

impl custom_theme_t {
    /// Checks a theme file against every rule of D64 and builds the theme from it.
    pub fn from_file(file: &theme_file_t) -> Result<custom_theme_t, SettingsError> {
        let name = checked_theme_name(&file.name)?;
        let id = match &file.id {
            Some(id) => id.clone(),
            None => slugify_theme_name(&name),
        };
        if !valid_theme_id(&id) {
            return Err(invalid(format!("the theme id {id:?} is not valid")));
        }
        let extends = file
            .extends
            .clone()
            .unwrap_or_else(|| DefaultTheme.to_string());
        if !valid_theme_id(&extends) {
            return Err(invalid(format!(
                "the theme to extend, {extends:?}, is not a valid id"
            )));
        }
        if file.colors.len() > MaxThemeColors {
            return Err(invalid(format!(
                "a theme sets at most {MaxThemeColors} colours"
            )));
        }
        let mut colors = BTreeMap::new();
        for (key, value) in &file.colors {
            if !valid_color_name(key) {
                return Err(invalid(format!("the colour name {key:?} is not valid")));
            }
            colors.insert(key.clone(), checked_color(value)?);
        }
        Ok(custom_theme_t {
            id,
            name,
            extends,
            colors,
        })
    }

    /// The file form of this theme. The id is always written, so the file does not depend on the
    /// name's slug.
    pub fn to_file(&self) -> theme_file_t {
        theme_file_t {
            name: self.name.clone(),
            id: Some(self.id.clone()),
            extends: Some(self.extends.clone()),
            colors: self.colors.clone(),
        }
    }
}

/// Why a settings or theme operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsError {
    /// A value breaks a rule, such as a bad theme id or `full` as the default approval mode.
    Invalid {
        message: String,
    },
    /// A file is not JSON of the right shape, or is over its size limit.
    Malformed {
        message: String,
    },
    /// No custom theme has this id.
    NotFound {
        id: String,
    },
    Io {
        path: String,
        message: String,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Invalid { message } => write!(f, "invalid setting: {message}"),
            SettingsError::Malformed { message } => write!(f, "the file is not valid: {message}"),
            SettingsError::NotFound { id } => write!(f, "no custom theme {id}"),
            SettingsError::Io { path, message } => {
                write!(f, "settings i/o error at {path}: {message}")
            }
        }
    }
}

impl std::error::Error for SettingsError {}

fn invalid(message: String) -> SettingsError {
    SettingsError::Invalid { message }
}

/// The custom themes that loaded, and one `file name: reason` line for each file that was skipped
/// (D64). A bad file never stops the others from loading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct loaded_themes_t {
    pub themes: Vec<custom_theme_t>,
    pub skipped: Vec<String>,
}

/// Where settings and custom themes are kept.
pub trait settings_store_t: Send + Sync {
    /// The saved settings. The defaults when nothing was saved.
    fn load(&self) -> Result<settings_t, SettingsError>;

    /// Checks the settings, then saves them.
    fn save(&self, settings: &settings_t) -> Result<(), SettingsError>;

    /// Every custom theme that loads, and the files that were skipped.
    fn list_themes(&self) -> Result<loaded_themes_t, SettingsError>;

    /// Reads, parses and checks a theme file at any path the caller names. Saves nothing.
    fn read_theme_file(&self, path: &Path) -> Result<custom_theme_t, SettingsError>;

    /// Checks a theme, then saves it, replacing a theme with the same id.
    fn save_theme(&self, theme: &custom_theme_t) -> Result<(), SettingsError>;

    /// Removes a custom theme. `NotFound` when there is none with that id.
    fn remove_theme(&self, id: &str) -> Result<(), SettingsError>;
}

#[derive(Debug, Default)]
struct held_t {
    settings: settings_t,
    themes: BTreeMap<String, custom_theme_t>,
}

/// Settings and themes kept in memory until the process ends.
#[derive(Debug, Default)]
pub struct memory_settings_store_t {
    held: Mutex<held_t>,
}

impl memory_settings_store_t {
    pub fn new() -> Self {
        Self::default()
    }

    /// The held values, even when a panic poisoned the lock. Each value is replaced whole, so it
    /// is never half-written.
    fn locked(&self) -> MutexGuard<'_, held_t> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl settings_store_t for memory_settings_store_t {
    fn load(&self) -> Result<settings_t, SettingsError> {
        Ok(self.locked().settings.clone())
    }

    fn save(&self, settings: &settings_t) -> Result<(), SettingsError> {
        settings.validate()?;
        self.locked().settings = settings.clone();
        Ok(())
    }

    fn list_themes(&self) -> Result<loaded_themes_t, SettingsError> {
        Ok(loaded_themes_t {
            themes: self.locked().themes.values().cloned().collect(),
            skipped: Vec::new(),
        })
    }

    fn read_theme_file(&self, _path: &Path) -> Result<custom_theme_t, SettingsError> {
        Err(invalid(
            "the in-memory settings store does not read files".to_string(),
        ))
    }

    fn save_theme(&self, theme: &custom_theme_t) -> Result<(), SettingsError> {
        let checked = custom_theme_t::from_file(&theme.to_file())?;
        self.locked().themes.insert(checked.id.clone(), checked);
        Ok(())
    }

    fn remove_theme(&self, id: &str) -> Result<(), SettingsError> {
        if !valid_theme_id(id) {
            return Err(invalid(format!("the theme id {id:?} is not valid")));
        }
        match self.locked().themes.remove(id) {
            Some(_) => Ok(()),
            None => Err(SettingsError::NotFound { id: id.to_string() }),
        }
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

    fn file(name: &str, colors: &[(&str, &str)]) -> theme_file_t {
        theme_file_t {
            name: name.to_string(),
            id: None,
            extends: None,
            colors: colors
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    fn theme(id: &str) -> custom_theme_t {
        custom_theme_t {
            id: id.to_string(),
            name: format!("Theme {id}"),
            extends: DefaultTheme.to_string(),
            colors: BTreeMap::from([("accent".to_string(), "#112233".to_string())]),
        }
    }

    #[test]
    fn the_defaults_are_the_documented_ones_and_they_validate() {
        let settings = settings_t::default();
        assert_eq!(settings.theme, "ncagent-dark");
        assert!(settings.animation && settings.confirm_delete && settings.restore_tabs);
        assert!(!settings.show_archived);
        assert_eq!(settings.sidebar, SidebarVisibility::Auto);
        assert_eq!(settings.default_approval, ApprovalMode::Ask);
        assert_eq!(settings.tool_results, ToolResultView::Folded);
        assert_eq!(settings.open_in, OpenIn::NewTab);
        assert_eq!(settings.conversation_order, ConversationOrder::Recent);
        assert!(settings.open_tabs.is_empty());
        assert_eq!(settings.active_tab, None);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn each_settings_rule_refuses_its_own_bad_value() {
        let theme = settings_t {
            theme: "Not Valid".to_string(),
            ..settings_t::default()
        };
        assert!(matches!(
            theme.validate(),
            Err(SettingsError::Invalid { .. })
        ));

        let approval = settings_t {
            default_approval: ApprovalMode::Full,
            ..settings_t::default()
        };
        assert!(matches!(
            approval.validate(),
            Err(SettingsError::Invalid { .. })
        ));

        let mut settings = settings_t {
            open_tabs: (0..=MaxSavedTabs).map(|i| format!("s_{i}")).collect(),
            ..settings_t::default()
        };
        assert!(matches!(
            settings.validate(),
            Err(SettingsError::Invalid { .. })
        ));
        settings.open_tabs.truncate(MaxSavedTabs);
        assert!(settings.validate().is_ok(), "exactly the limit is allowed");

        let traversal = settings_t {
            open_tabs: vec!["s_1".to_string(), "../x".to_string()],
            ..settings_t::default()
        };
        assert!(matches!(
            traversal.validate(),
            Err(SettingsError::Invalid { .. })
        ));

        let duplicate = settings_t {
            open_tabs: vec!["s_1".to_string(), "s_1".to_string()],
            ..settings_t::default()
        };
        assert!(matches!(
            duplicate.validate(),
            Err(SettingsError::Invalid { .. })
        ));

        let mut active = settings_t {
            open_tabs: vec!["s_1".to_string()],
            active_tab: Some("s_2".to_string()),
            ..settings_t::default()
        };
        assert!(matches!(
            active.validate(),
            Err(SettingsError::Invalid { .. })
        ));
        active.active_tab = Some("s_1".to_string());
        assert!(active.validate().is_ok());
    }

    #[test]
    fn theme_ids_are_lowercase_with_dashes_or_underscores_and_at_most_forty_bytes() {
        assert!(valid_theme_id("ncagent-dark"));
        assert!(valid_theme_id("9lives"));
        assert!(valid_theme_id("a-"));
        assert!(valid_theme_id("high_contrast"));
        assert!(valid_theme_id(&"a".repeat(40)));
        let too_long = "a".repeat(41);
        for bad in ["", "-a", "_a", "A", "a b", "é", "a/b", too_long.as_str()] {
            assert!(!valid_theme_id(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn slugs_are_valid_ids_with_a_fallback() {
        assert_eq!(slugify_theme_name("My Night Theme!"), "my-night-theme");
        assert_eq!(slugify_theme_name("  --A__b--  "), "a-b");
        assert_eq!(slugify_theme_name("Café"), "caf");
        assert_eq!(slugify_theme_name("!!!"), "theme");
        assert_eq!(slugify_theme_name(""), "theme");
        let long = slugify_theme_name(&"word ".repeat(30));
        assert!(long.len() <= ThemeIdLimit);
        assert!(!long.ends_with('-'));
        assert!(valid_theme_id(&long));
    }

    #[test]
    fn a_theme_file_is_checked_and_its_colours_lowercased() {
        let theme = ok(custom_theme_t::from_file(&file(
            "  Night Owl ",
            &[("accent", "#AABBCC")],
        )));
        assert_eq!(theme.name, "Night Owl");
        assert_eq!(theme.id, "night-owl");
        assert_eq!(theme.extends, "ncagent-dark");
        assert_eq!(
            theme.colors.get("accent").map(String::as_str),
            Some("#aabbcc")
        );
    }

    #[test]
    fn an_explicit_id_and_extends_are_used_when_valid() {
        let mut given = file("Anything", &[]);
        given.id = Some("my-id".to_string());
        given.extends = Some("other-base".to_string());
        let theme = ok(custom_theme_t::from_file(&given));
        assert_eq!(theme.id, "my-id");
        assert_eq!(theme.extends, "other-base");

        let mut upper = file("Anything", &[]);
        upper.id = Some("My-Id".to_string());
        assert!(matches!(
            custom_theme_t::from_file(&upper),
            Err(SettingsError::Invalid { .. })
        ));
        let mut bad_extends = file("Anything", &[]);
        bad_extends.extends = Some("bad base".to_string());
        assert!(matches!(
            custom_theme_t::from_file(&bad_extends),
            Err(SettingsError::Invalid { .. })
        ));
    }

    #[test]
    fn a_theme_name_is_one_to_forty_characters_without_control_characters() {
        assert!(custom_theme_t::from_file(&file(&"n".repeat(40), &[])).is_ok());
        let too_long = "n".repeat(41);
        for bad in ["   ", too_long.as_str(), "a\u{7}b"] {
            assert!(
                matches!(
                    custom_theme_t::from_file(&file(bad, &[])),
                    Err(SettingsError::Invalid { .. })
                ),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn colours_are_checked_by_name_and_value() {
        for name in ["accent", "panel_2", "a"] {
            assert!(
                custom_theme_t::from_file(&file("T", &[(name, "#000000")])).is_ok(),
                "{name} is a valid colour name"
            );
        }
        let too_long = "a".repeat(33);
        for bad_name in ["Accent", "2fast", "a-b", "", too_long.as_str()] {
            assert!(
                matches!(
                    custom_theme_t::from_file(&file("T", &[(bad_name, "#000000")])),
                    Err(SettingsError::Invalid { .. })
                ),
                "{bad_name:?} must be refused"
            );
        }
        for bad_value in [
            "000000", "#00000", "#0000000", "#00000g", "red", " #000000", "#ggg000",
        ] {
            assert!(
                matches!(
                    custom_theme_t::from_file(&file("T", &[("accent", bad_value)])),
                    Err(SettingsError::Invalid { .. })
                ),
                "{bad_value:?} must be refused"
            );
        }
    }

    #[test]
    fn a_theme_sets_at_most_sixty_four_colours() {
        let names: Vec<String> = (0..MaxThemeColors).map(|i| format!("c{i}")).collect();
        let at_limit: Vec<(&str, &str)> = names
            .iter()
            .map(|name| (name.as_str(), "#000000"))
            .collect();
        assert!(custom_theme_t::from_file(&file("T", &at_limit)).is_ok());

        let mut over = at_limit;
        over.push(("extra", "#000000"));
        assert!(matches!(
            custom_theme_t::from_file(&file("T", &over)),
            Err(SettingsError::Invalid { .. })
        ));
    }

    #[test]
    fn a_theme_round_trips_through_its_file_form() {
        let original = theme("round");
        let back = ok(custom_theme_t::from_file(&original.to_file()));
        assert_eq!(back, original);
    }

    #[test]
    fn the_memory_store_saves_and_loads_settings() {
        let store = memory_settings_store_t::new();
        assert_eq!(ok(store.load()), settings_t::default());
        let changed = settings_t {
            animation: false,
            ..settings_t::default()
        };
        ok(store.save(&changed));
        assert_eq!(ok(store.load()), changed);

        let bad = settings_t {
            default_approval: ApprovalMode::Full,
            ..settings_t::default()
        };
        assert!(matches!(
            store.save(&bad),
            Err(SettingsError::Invalid { .. })
        ));
        assert_eq!(ok(store.load()), changed, "a refused save changes nothing");
    }

    #[test]
    fn the_memory_store_saves_lists_and_removes_themes() {
        let store = memory_settings_store_t::new();
        ok(store.save_theme(&theme("beta")));
        ok(store.save_theme(&theme("alpha")));
        let listed = ok(store.list_themes());
        let ids: Vec<&str> = listed.themes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["alpha", "beta"]);
        assert!(listed.skipped.is_empty());

        ok(store.remove_theme("alpha"));
        assert!(matches!(
            store.remove_theme("alpha"),
            Err(SettingsError::NotFound { .. })
        ));
        assert!(matches!(
            store.remove_theme("../x"),
            Err(SettingsError::Invalid { .. })
        ));
        assert_eq!(ok(store.list_themes()).themes.len(), 1);
    }

    #[test]
    fn the_memory_store_refuses_a_bad_theme_and_reads_no_files() {
        let store = memory_settings_store_t::new();
        let mut bad = theme("ok");
        bad.id = "Bad Id".to_string();
        assert!(matches!(
            store.save_theme(&bad),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(ok(store.list_themes()).themes.is_empty());
        assert!(matches!(
            store.read_theme_file(Path::new("/nowhere/theme.json")),
            Err(SettingsError::Invalid { .. })
        ));
    }
}
