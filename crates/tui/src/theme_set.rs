/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The themes the UI knows (PROJECT.md, D64): the built-ins, then the custom themes the runtime
//! loaded, and how a custom theme becomes a palette.
//!
//! A custom theme starts from the theme it `extends` (a built-in or another custom theme),
//! resolved first. Its `colors` are then set by name. A derived colour (see `theme::derive`) that
//! the theme does not set itself is made again from the new base colours whenever the theme
//! changes one of its sources, so a theme that sets only `bg` and `fg` still gets matching panel
//! tones and dim text. A chain of `extends` is followed to `MaxThemeDepth` themes, and a cycle,
//! an unknown base, an unknown colour name, a bad `#rrggbb` value and an id that a built-in
//! already uses are refused with a named error.

use std::collections::BTreeMap;
use std::fmt;

use harness::settings::{DefaultTheme, custom_theme_t};

use crate::builtin_themes::{BuiltinThemes, builtin};
use crate::theme::{ColorNames, derive, derived_from, parse_hex, theme_t};

/// Most themes in one `extends` chain, the theme itself included.
pub const MaxThemeDepth: usize = 8;

/// Why a theme could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeErrorKind {
    /// A key of `colors` is not a theme colour name.
    UnknownColor { name: String },
    /// A value is not `#rrggbb`.
    BadColor { name: String, value: String },
    /// `extends` names no built-in and no custom theme.
    UnknownBase { id: String },
    /// The `extends` chain comes back to a theme already in it.
    Cycle { chain: Vec<String> },
    /// The `extends` chain is longer than `MaxThemeDepth`.
    TooDeep { chain: Vec<String> },
    /// A custom theme uses the id of a built-in theme.
    BuiltinId { id: String },
    /// No theme has this id.
    NotFound { id: String },
}

/// A theme that could not be built, and which one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct theme_error_t {
    /// The theme being built.
    pub theme: String,
    pub kind: ThemeErrorKind,
}

impl fmt::Display for theme_error_t {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let theme = &self.theme;
        match &self.kind {
            ThemeErrorKind::UnknownColor { name } => write!(
                f,
                "theme {theme}: {name:?} is not a colour name; the names are {}",
                ColorNames.join(", ")
            ),
            ThemeErrorKind::BadColor { name, value } => {
                write!(f, "theme {theme}: {name} = {value:?} is not #rrggbb")
            }
            ThemeErrorKind::UnknownBase { id } => write!(
                f,
                "theme {theme} extends {id:?}, which is not a built-in or imported theme"
            ),
            ThemeErrorKind::Cycle { chain } => {
                write!(f, "theme {theme}: extends loops: {}", chain.join(" → "))
            }
            ThemeErrorKind::TooDeep { chain } => write!(
                f,
                "theme {theme}: extends goes deeper than {MaxThemeDepth} themes: {}",
                chain.join(" → ")
            ),
            ThemeErrorKind::BuiltinId { id } => write!(
                f,
                "theme {theme}: the id {id:?} belongs to a built-in theme; give the file another id"
            ),
            ThemeErrorKind::NotFound { id } => write!(f, "no theme {id:?}"),
        }
    }
}

impl std::error::Error for theme_error_t {}

fn error(theme: &str, kind: ThemeErrorKind) -> theme_error_t {
    theme_error_t {
        theme: theme.to_string(),
        kind,
    }
}

/// One theme as the settings page lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct theme_entry_t {
    pub id: String,
    pub name: String,
    pub imported: bool,
    /// The theme it builds on, for an imported theme.
    pub extends: Option<String>,
    /// The palette, or why it cannot be built.
    pub palette: Result<theme_t, theme_error_t>,
}

/// The built-in themes and the custom themes the runtime loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct theme_set_t {
    /// Custom themes by id.
    pub custom: BTreeMap<String, custom_theme_t>,
}

impl theme_set_t {
    pub fn new(custom: Vec<custom_theme_t>) -> Self {
        theme_set_t {
            custom: custom.into_iter().map(|t| (t.id.clone(), t)).collect(),
        }
    }

    pub fn is_builtin(id: &str) -> bool {
        builtin(id).is_some()
    }

    pub fn contains(&self, id: &str) -> bool {
        Self::is_builtin(id) || self.custom.contains_key(id)
    }

    /// The display name of a theme, or the id when there is none.
    pub fn name_of(&self, id: &str) -> String {
        match (builtin(id), self.custom.get(id)) {
            (Some(b), _) => b.name.to_string(),
            (None, Some(c)) => c.name.clone(),
            (None, None) => id.to_string(),
        }
    }

    /// Every theme id: the built-ins in their order, then the custom themes by id.
    pub fn ids(&self) -> Vec<String> {
        BuiltinThemes
            .iter()
            .map(|b| b.id.to_string())
            .chain(self.custom.keys().cloned())
            .collect()
    }

    /// A theme id from what a user typed: an id, or a name ignoring case.
    pub fn find(&self, typed: &str) -> Option<String> {
        let typed = typed.trim();
        if self.contains(typed) {
            return Some(typed.to_string());
        }
        let lower = typed.to_lowercase();
        self.ids()
            .into_iter()
            .find(|id| self.name_of(id).to_lowercase() == lower)
    }

    /// The palette of the theme with this id.
    pub fn resolve(&self, id: &str) -> Result<theme_t, theme_error_t> {
        if let Some(b) = builtin(id) {
            return Ok((b.palette)());
        }
        match self.custom.get(id) {
            Some(custom) => theme_t::from_custom(self, custom),
            None => Err(error(id, ThemeErrorKind::NotFound { id: id.to_string() })),
        }
    }

    /// Every theme, built-ins first, as the settings page lists them.
    pub fn entries(&self) -> Vec<theme_entry_t> {
        let builtins = BuiltinThemes.iter().map(|b| theme_entry_t {
            id: b.id.to_string(),
            name: b.name.to_string(),
            imported: false,
            extends: None,
            palette: Ok((b.palette)()),
        });
        let customs = self.custom.values().map(|c| theme_entry_t {
            id: c.id.clone(),
            name: c.name.clone(),
            imported: true,
            extends: Some(c.extends.clone()),
            palette: theme_t::from_custom(self, c),
        });
        builtins.chain(customs).collect()
    }
}

impl theme_t {
    /// Build a custom theme on top of `set` (see the module note). `custom` may be a theme that
    /// is not in `set` yet, such as a file being imported; inside the chain its own id means
    /// `custom` itself.
    pub fn from_custom(
        set: &theme_set_t,
        custom: &custom_theme_t,
    ) -> Result<theme_t, theme_error_t> {
        let mut chain = vec![custom.id.clone()];
        resolve_chain(set, custom, custom, &mut chain)
    }
}

/// Resolve `theme`, the newest link of `chain`, whose first link is `root`. Errors name `root`.
fn resolve_chain(
    set: &theme_set_t,
    root: &custom_theme_t,
    theme: &custom_theme_t,
    chain: &mut Vec<String>,
) -> Result<theme_t, theme_error_t> {
    let named = |kind| error(&root.id, kind);
    if theme_set_t::is_builtin(&theme.id) {
        return Err(named(ThemeErrorKind::BuiltinId {
            id: theme.id.clone(),
        }));
    }
    // Check every colour before following the chain, so the error names this theme's mistake.
    let mut colors = Vec::with_capacity(theme.colors.len());
    for (name, value) in &theme.colors {
        if !ColorNames.contains(&name.as_str()) {
            return Err(named(ThemeErrorKind::UnknownColor { name: name.clone() }));
        }
        let Some(color) = parse_hex(value) else {
            return Err(named(ThemeErrorKind::BadColor {
                name: name.clone(),
                value: value.clone(),
            }));
        };
        colors.push((name.as_str(), color));
    }
    let base_id = if theme.extends.is_empty() {
        DefaultTheme
    } else {
        theme.extends.as_str()
    };
    if chain.iter().any(|link| link == base_id) {
        let mut looped = chain.clone();
        looped.push(base_id.to_string());
        return Err(named(ThemeErrorKind::Cycle { chain: looped }));
    }
    chain.push(base_id.to_string());
    if chain.len() > MaxThemeDepth {
        return Err(named(ThemeErrorKind::TooDeep {
            chain: chain.clone(),
        }));
    }
    let mut out = match builtin(base_id) {
        Some(b) => (b.palette)(),
        // The root's own id is in the chain, so a saved copy of it is never used as a base.
        None => {
            let next = set.custom.get(base_id).ok_or_else(|| {
                named(ThemeErrorKind::UnknownBase {
                    id: base_id.to_string(),
                })
            })?;
            resolve_chain(set, root, next, chain)?
        }
    };
    for (name, color) in &colors {
        out.set(name, *color);
    }
    // Derived colours follow the base colours this theme changed, unless it set them too.
    for name in ColorNames {
        let set_here = colors.iter().any(|(n, _)| n == name);
        let source_changed = derived_from(name)
            .iter()
            .any(|source| colors.iter().any(|(n, _)| n == source));
        if !set_here
            && source_changed
            && let Some(color) = derive(name, &out)
        {
            out.set(name, color);
        }
    }
    Ok(out)
}
