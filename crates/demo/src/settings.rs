/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The demo's file-backed settings and custom themes (D63, D64): `<dir>/settings.json`, and one
//! file per custom theme at `<dir>/themes/<id>.json`.
//!
//! The files are untrusted input. A settings file over 1 MiB, a theme file over 64 KiB, a file
//! that is not JSON of the right shape, and a file that fails a rule of `harness::settings` is
//! refused with a typed error, and the store never writes in `load` or `read_theme_file`. A save
//! writes a temporary file next to its target and renames it over the target, so a failed save
//! leaves the last good file in place. The store writes only under its directory, except that
//! `read_theme_file` reads the path the user names.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use harness::settings::{
    SettingsError, custom_theme_t, loaded_themes_t, settings_store_t, settings_t, theme_file_t,
    valid_theme_id,
};

/// Longest settings file the store reads or writes, in bytes (D52 applies the same way here).
pub const MaxSettingsBytes: usize = 1024 * 1024;

/// Longest theme file the store reads or writes, in bytes (D64).
pub const MaxThemeBytes: usize = 64 * 1024;

/// The file the settings are kept in, under the demo directory (D63).
const SettingsFile: &str = "settings.json";

/// The file a settings save writes before it renames it over the settings file.
const TempSettingsFile: &str = "settings.json.tmp";

/// The directory under the demo directory that holds the custom themes (D64).
const ThemesDir: &str = "themes";

/// The extension of a theme file.
const ThemeExtension: &str = "json";

/// Settings and custom themes kept under a directory.
pub struct file_settings_store_t {
    dir: PathBuf,
}

impl file_settings_store_t {
    /// A store rooted at `dir`. Nothing is created until the first save.
    pub fn new(dir: PathBuf) -> Self {
        file_settings_store_t { dir }
    }

    fn settings_path(&self) -> PathBuf {
        self.dir.join(SettingsFile)
    }

    fn themes_dir(&self) -> PathBuf {
        self.dir.join(ThemesDir)
    }

    /// The file of one theme. The id is checked first, so the path cannot leave the themes
    /// directory.
    fn theme_path(&self, id: &str) -> Result<PathBuf, SettingsError> {
        if !valid_theme_id(id) {
            return Err(SettingsError::Invalid {
                message: format!("the theme id {id:?} is not valid"),
            });
        }
        Ok(self.themes_dir().join(format!("{id}.{ThemeExtension}")))
    }
}

fn io_error(path: &Path, err: std::io::Error) -> SettingsError {
    SettingsError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    }
}

/// Reads at most `limit` bytes. `None` when the file does not exist. A file over the limit is
/// refused as malformed, so a huge file is never read whole.
fn read_limited(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, SettingsError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_error(path, err)),
    };
    // One byte more than the limit, so a file over the limit is seen as over it.
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| io_error(path, err))?;
    if bytes.len() > limit {
        return Err(SettingsError::Malformed {
            message: format!("the file is larger than {limit} bytes"),
        });
    }
    Ok(Some(bytes))
}

fn malformed(err: serde_json::Error) -> SettingsError {
    SettingsError::Malformed {
        message: err.to_string(),
    }
}

/// The JSON text of a value, refused when it is over `limit` bytes, because the store would not
/// read it back.
fn encode<T: serde::Serialize>(
    path: &Path,
    value: &T,
    limit: usize,
) -> Result<Vec<u8>, SettingsError> {
    let text = serde_json::to_string_pretty(value).map_err(|err| SettingsError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    if text.len() > limit {
        return Err(SettingsError::Invalid {
            message: format!(
                "the value takes more than {limit} bytes, so it could not be read back"
            ),
        });
    }
    Ok(text.into_bytes())
}

/// Writes `bytes` to `temp` and renames it over `target`. A stale temporary file is removed
/// first, so the create below never follows a link that someone left at that name.
fn write_then_rename(temp: &Path, target: &Path, bytes: &[u8]) -> Result<(), SettingsError> {
    let _ = fs::remove_file(temp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(|err| io_error(temp, err))?;
    file.write_all(bytes).map_err(|err| io_error(temp, err))?;
    file.sync_all().map_err(|err| io_error(temp, err))?;
    drop(file);
    fs::rename(temp, target).map_err(|err| io_error(target, err))
}

/// Writes `bytes` to `target` through a temporary file in the same directory. The temporary file
/// is removed when the write fails.
fn save_file(dir: &Path, temp: &Path, target: &Path, bytes: &[u8]) -> Result<(), SettingsError> {
    fs::create_dir_all(dir).map_err(|err| io_error(dir, err))?;
    write_then_rename(temp, target, bytes).inspect_err(|_| {
        // Best effort: the target is still the last good file.
        let _ = fs::remove_file(temp);
    })
}

/// Parses one theme file and checks it against D64.
fn parse_theme(bytes: &[u8]) -> Result<custom_theme_t, SettingsError> {
    let file: theme_file_t = serde_json::from_slice(bytes).map_err(malformed)?;
    custom_theme_t::from_file(&file)
}

impl settings_store_t for file_settings_store_t {
    fn load(&self) -> Result<settings_t, SettingsError> {
        let path = self.settings_path();
        let Some(bytes) = read_limited(&path, MaxSettingsBytes)? else {
            return Ok(settings_t::default());
        };
        let settings: settings_t = serde_json::from_slice(&bytes).map_err(malformed)?;
        settings.validate()?;
        Ok(settings)
    }

    fn save(&self, settings: &settings_t) -> Result<(), SettingsError> {
        settings.validate()?;
        let target = self.settings_path();
        let bytes = encode(&target, settings, MaxSettingsBytes)?;
        let temp = self.dir.join(TempSettingsFile);
        save_file(&self.dir, &temp, &target, &bytes)
    }

    fn list_themes(&self) -> Result<loaded_themes_t, SettingsError> {
        let dir = self.themes_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Ok(loaded_themes_t::default());
            }
            Err(err) => return Err(io_error(&dir, err)),
        };
        let mut loaded = loaded_themes_t::default();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    loaded.skipped.push(format!("{}: {err}", dir.display()));
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some(ThemeExtension) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            match self.read_theme_file(&path) {
                Ok(theme)
                    if path.file_stem().and_then(|stem| stem.to_str())
                        == Some(theme.id.as_str()) =>
                {
                    loaded.themes.push(theme);
                }
                Ok(theme) => loaded.skipped.push(format!(
                    "{name}: the file name does not match the theme id {}",
                    theme.id
                )),
                Err(err) => loaded.skipped.push(format!("{name}: {err}")),
            }
        }
        loaded.themes.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(loaded)
    }

    /// The path is any the user names, so it is not confined to the themes directory. The file
    /// must be a regular file of at most 64 KiB.
    fn read_theme_file(&self, path: &Path) -> Result<custom_theme_t, SettingsError> {
        let metadata = fs::metadata(path).map_err(|err| match err.kind() {
            ErrorKind::NotFound => SettingsError::Io {
                path: path.display().to_string(),
                message: "the file does not exist".to_string(),
            },
            _ => io_error(path, err),
        })?;
        if !metadata.is_file() {
            return Err(SettingsError::Io {
                path: path.display().to_string(),
                message: "the path is not a regular file".to_string(),
            });
        }
        let bytes = read_limited(path, MaxThemeBytes)?.ok_or_else(|| SettingsError::Io {
            path: path.display().to_string(),
            message: "the file does not exist".to_string(),
        })?;
        parse_theme(&bytes)
    }

    fn save_theme(&self, theme: &custom_theme_t) -> Result<(), SettingsError> {
        let checked = custom_theme_t::from_file(&theme.to_file())?;
        let target = self.theme_path(&checked.id)?;
        let bytes = encode(&target, &checked.to_file(), MaxThemeBytes)?;
        let temp = self.themes_dir().join(format!("{}.tmp", checked.id));
        save_file(&self.themes_dir(), &temp, &target, &bytes)
    }

    fn remove_theme(&self, id: &str) -> Result<(), SettingsError> {
        let path = self.theme_path(id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                Err(SettingsError::NotFound { id: id.to_string() })
            }
            Err(err) => Err(io_error(&path, err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use harness::approval::ApprovalMode;
    use harness::settings::LinkAddresses;

    use super::*;

    static ScratchCounter: AtomicUsize = AtomicUsize::new(0);

    /// A directory under the system temp directory, removed when the test ends.
    struct scratch_dir_t {
        path: PathBuf,
    }

    impl scratch_dir_t {
        fn new(name: &str) -> Self {
            let counter = ScratchCounter.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ncagent-demo-settings-{}-{counter}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            scratch_dir_t { path }
        }

        fn settings_file(&self) -> PathBuf {
            self.path.join(SettingsFile)
        }

        fn themes_dir(&self) -> PathBuf {
            self.path.join(ThemesDir)
        }
    }

    impl Drop for scratch_dir_t {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn ok<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(err) => panic!("expected Ok, found {err:?}"),
        }
    }

    fn theme(id: &str) -> custom_theme_t {
        custom_theme_t {
            id: id.to_string(),
            name: format!("Theme {id}"),
            extends: "ncagent-dark".to_string(),
            colors: BTreeMap::from([("accent".to_string(), "#112233".to_string())]),
        }
    }

    /// Writes `text` as the settings file, then loads it.
    fn load_settings_text(
        scratch: &scratch_dir_t,
        text: &str,
    ) -> Result<settings_t, SettingsError> {
        ok(fs::create_dir_all(&scratch.path));
        ok(fs::write(scratch.settings_file(), text));
        file_settings_store_t::new(scratch.path.clone()).load()
    }

    #[test]
    fn saved_settings_load_back_unchanged() {
        let scratch = scratch_dir_t::new("round-trip");
        let store = file_settings_store_t::new(scratch.path.clone());
        let changed = settings_t {
            animation: false,
            open_tabs: vec!["s_1".to_string(), "s_2".to_string()],
            active_tab: Some("s_2".to_string()),
            ..settings_t::default()
        };
        ok(store.save(&changed));
        assert_eq!(ok(store.load()), changed);
        assert!(!scratch.path.join(TempSettingsFile).exists());
    }

    #[test]
    fn missing_settings_load_as_the_defaults_and_create_nothing() {
        let scratch = scratch_dir_t::new("missing");
        let store = file_settings_store_t::new(scratch.path.clone());
        assert_eq!(ok(store.load()), settings_t::default());
        assert!(!scratch.path.exists(), "loading must not create the store");
    }

    #[test]
    fn malformed_settings_are_refused_and_the_file_is_left_alone() {
        let scratch = scratch_dir_t::new("malformed");
        let text = "{ \"theme\": ";
        assert!(matches!(
            load_settings_text(&scratch, text),
            Err(SettingsError::Malformed { .. })
        ));
        assert_eq!(ok(fs::read_to_string(scratch.settings_file())), text);
    }

    #[test]
    fn a_wrong_value_type_is_malformed_and_an_unknown_enum_value_is_too() {
        let scratch = scratch_dir_t::new("wrong-type");
        assert!(matches!(
            load_settings_text(&scratch, r#"{"animation":"yes"}"#),
            Err(SettingsError::Malformed { .. })
        ));
        assert!(matches!(
            load_settings_text(&scratch, r#"{"sidebar":"sideways"}"#),
            Err(SettingsError::Malformed { .. })
        ));
    }

    #[test]
    fn a_settings_file_over_one_mebibyte_is_refused_even_when_it_is_valid_json() {
        let scratch = scratch_dir_t::new("oversized");
        // Valid JSON. Unknown fields are ignored, so only the size check refuses it.
        let pad = "x".repeat(MaxSettingsBytes);
        let text = format!("{{\"theme\":\"ncagent-dark\",\"pad\":\"{pad}\"}}");
        assert!(text.len() > MaxSettingsBytes);
        assert!(matches!(
            load_settings_text(&scratch, &text),
            Err(SettingsError::Malformed { .. })
        ));
    }

    #[test]
    fn an_unknown_settings_field_is_ignored_and_missing_fields_take_defaults() {
        let scratch = scratch_dir_t::new("unknown-field");
        let loaded = ok(load_settings_text(
            &scratch,
            r#"{"theme":"ncagent-dark","from_a_later_build":[1,2],"animation":false}"#,
        ));
        assert!(!loaded.animation);
        assert_eq!(loaded.sidebar, settings_t::default().sidebar);
        assert_eq!(loaded.default_approval, ApprovalMode::Ask);
    }

    #[test]
    fn the_link_settings_round_trip_through_the_file() {
        let scratch = scratch_dir_t::new("link-round-trip");
        let store = file_settings_store_t::new(scratch.path.clone());
        let changed = settings_t {
            link_hyperlinks: false,
            link_addresses: LinkAddresses::Inline,
            ..settings_t::default()
        };
        ok(store.save(&changed));
        let text = ok(fs::read_to_string(scratch.settings_file()));
        assert!(text.contains("\"link_hyperlinks\": false"), "{text}");
        assert!(text.contains("\"link_addresses\": \"inline\""), "{text}");
        assert_eq!(ok(store.load()), changed);
    }

    #[test]
    fn a_settings_file_without_the_link_fields_loads_the_link_defaults() {
        let scratch = scratch_dir_t::new("link-old-file");
        let loaded = ok(load_settings_text(
            &scratch,
            r#"{"theme":"ncagent-dark","animation":false,"tool_results":"expanded"}"#,
        ));
        assert!(loaded.link_hyperlinks, "hyperlinks stay on");
        assert_eq!(loaded.link_addresses, LinkAddresses::Hidden);
        assert!(!loaded.animation, "the fields that are there still load");
    }

    #[test]
    fn an_unknown_link_address_mode_is_refused_as_malformed() {
        let scratch = scratch_dir_t::new("link-unknown");
        assert!(matches!(
            load_settings_text(&scratch, r#"{"link_addresses":"sometimes"}"#),
            Err(SettingsError::Malformed { .. })
        ));
        assert!(matches!(
            load_settings_text(&scratch, r#"{"link_hyperlinks":"yes"}"#),
            Err(SettingsError::Malformed { .. })
        ));
    }

    #[test]
    fn invalid_settings_values_are_refused_on_load_and_on_save() {
        let scratch = scratch_dir_t::new("invalid-values");
        assert!(matches!(
            load_settings_text(&scratch, r#"{"default_approval":"full"}"#),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(matches!(
            load_settings_text(&scratch, r#"{"theme":"Not Valid"}"#),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(matches!(
            load_settings_text(&scratch, r#"{"open_tabs":["s_1","s_1"]}"#),
            Err(SettingsError::Invalid { .. })
        ));

        let fresh = scratch_dir_t::new("invalid-save");
        let store = file_settings_store_t::new(fresh.path.clone());
        let bad = settings_t {
            default_approval: ApprovalMode::Full,
            ..settings_t::default()
        };
        assert!(matches!(
            store.save(&bad),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(
            !fresh.settings_file().exists(),
            "a refused save writes nothing"
        );
    }

    #[test]
    fn a_failed_settings_save_leaves_the_last_good_file() {
        let scratch = scratch_dir_t::new("blocked");
        ok(fs::create_dir_all(&scratch.path));
        ok(fs::write(
            scratch.path.join("blocker"),
            "a file, not a folder",
        ));
        let store = file_settings_store_t::new(scratch.path.join("blocker").join("demo"));
        assert!(matches!(
            store.save(&settings_t::default()),
            Err(SettingsError::Io { .. })
        ));
        assert!(scratch.path.join("blocker").is_file());
    }

    #[test]
    fn a_stale_temporary_settings_file_is_replaced_and_none_is_left() {
        let scratch = scratch_dir_t::new("stale-temp");
        ok(fs::create_dir_all(&scratch.path));
        ok(fs::write(scratch.path.join(TempSettingsFile), "left over"));
        let store = file_settings_store_t::new(scratch.path.clone());
        let changed = settings_t {
            confirm_delete: false,
            ..settings_t::default()
        };
        ok(store.save(&changed));
        assert!(!scratch.path.join(TempSettingsFile).exists());
        assert_eq!(ok(store.load()), changed);
    }

    #[test]
    fn a_saved_theme_is_listed_and_a_missing_themes_directory_lists_as_empty() {
        let scratch = scratch_dir_t::new("list-empty");
        let store = file_settings_store_t::new(scratch.path.clone());
        assert_eq!(ok(store.list_themes()), loaded_themes_t::default());
        assert!(!scratch.path.exists(), "listing must not create the store");

        ok(store.save_theme(&theme("night")));
        let listed = ok(store.list_themes());
        assert_eq!(listed.themes, vec![theme("night")]);
        assert!(listed.skipped.is_empty());
        assert!(!scratch.themes_dir().join("night.tmp").exists());
    }

    #[test]
    fn list_skips_a_bad_theme_file_and_says_why_and_keeps_the_good_ones() {
        let scratch = scratch_dir_t::new("list-skips");
        let store = file_settings_store_t::new(scratch.path.clone());
        ok(store.save_theme(&theme("good")));
        let dir = scratch.themes_dir();
        ok(fs::write(dir.join("broken.json"), "{ not json"));
        ok(fs::write(
            dir.join("wrong-name.json"),
            r#"{"name":"Other","id":"other","colors":{}}"#,
        ));
        ok(fs::write(
            dir.join("bad-colour.json"),
            r#"{"name":"Bad","id":"bad-colour","colors":{"accent":"red"}}"#,
        ));
        ok(fs::write(dir.join("notes.txt"), "not a theme"));
        ok(fs::create_dir_all(dir.join("folder.json")));

        let listed = ok(store.list_themes());
        assert_eq!(listed.themes, vec![theme("good")]);
        assert_eq!(listed.skipped.len(), 4, "saw {:?}", listed.skipped);
        assert!(
            listed
                .skipped
                .iter()
                .any(|line| line.starts_with("broken.json: "))
        );
        assert!(
            listed
                .skipped
                .iter()
                .any(|line| line.starts_with("wrong-name.json: "))
        );
        assert!(
            listed
                .skipped
                .iter()
                .any(|line| line.starts_with("bad-colour.json: "))
        );
        assert!(
            listed
                .skipped
                .iter()
                .any(|line| line.starts_with("folder.json: "))
        );
    }

    #[test]
    fn read_theme_file_reads_a_file_the_user_names_and_saves_nothing() {
        let scratch = scratch_dir_t::new("import");
        ok(fs::create_dir_all(&scratch.path));
        let source = scratch.path.join("anywhere.json");
        ok(fs::write(
            &source,
            r##"{"name":"Import Me","extends":"ncagent-dark","colors":{"accent":"#ABCDEF"}}"##,
        ));
        let store = file_settings_store_t::new(scratch.path.join("state"));
        let imported = ok(store.read_theme_file(&source));
        assert_eq!(imported.id, "import-me");
        assert_eq!(
            imported.colors.get("accent").map(String::as_str),
            Some("#abcdef")
        );
        assert!(!scratch.path.join("state").exists(), "import saves nothing");
    }

    #[test]
    fn read_theme_file_refuses_a_directory_a_missing_path_and_bad_json() {
        let scratch = scratch_dir_t::new("import-bad");
        ok(fs::create_dir_all(&scratch.path));
        let store = file_settings_store_t::new(scratch.path.clone());
        assert!(matches!(
            store.read_theme_file(&scratch.path),
            Err(SettingsError::Io { .. })
        ));
        assert!(matches!(
            store.read_theme_file(&scratch.path.join("missing.json")),
            Err(SettingsError::Io { .. })
        ));
        let bad = scratch.path.join("bad.json");
        ok(fs::write(&bad, "[1, 2"));
        assert!(matches!(
            store.read_theme_file(&bad),
            Err(SettingsError::Malformed { .. })
        ));
        let invalid = scratch.path.join("invalid.json");
        ok(fs::write(&invalid, r#"{"name":"  ","colors":{}}"#));
        assert!(matches!(
            store.read_theme_file(&invalid),
            Err(SettingsError::Invalid { .. })
        ));
    }

    #[test]
    fn read_theme_file_refuses_a_file_of_64_kib_or_more() {
        let scratch = scratch_dir_t::new("import-big");
        ok(fs::create_dir_all(&scratch.path));
        let big = scratch.path.join("big.json");
        let pad = " ".repeat(MaxThemeBytes);
        ok(fs::write(
            &big,
            format!("{{\"name\":\"Big\",\"colors\":{{}}}}{pad}"),
        ));
        let store = file_settings_store_t::new(scratch.path.clone());
        assert!(matches!(
            store.read_theme_file(&big),
            Err(SettingsError::Malformed { .. })
        ));
    }

    #[test]
    fn save_theme_refuses_an_id_that_is_not_a_theme_id_and_writes_nothing() {
        let scratch = scratch_dir_t::new("bad-id");
        let store = file_settings_store_t::new(scratch.path.clone());
        let mut bad = theme("ok");
        bad.id = "../escape".to_string();
        assert!(matches!(
            store.save_theme(&bad),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(!scratch.path.exists(), "a refused theme writes nothing");
    }

    #[test]
    fn save_theme_replaces_a_theme_with_the_same_id_and_leaves_no_temporary_file() {
        let scratch = scratch_dir_t::new("replace");
        let store = file_settings_store_t::new(scratch.path.clone());
        ok(store.save_theme(&theme("same")));
        let mut newer = theme("same");
        newer.name = "Newer".to_string();
        ok(store.save_theme(&newer));
        let listed = ok(store.list_themes());
        assert_eq!(listed.themes, vec![newer]);
        assert!(!scratch.themes_dir().join("same.tmp").exists());
    }

    #[test]
    fn remove_theme_deletes_its_file_then_reports_not_found() {
        let scratch = scratch_dir_t::new("remove");
        let store = file_settings_store_t::new(scratch.path.clone());
        ok(store.save_theme(&theme("gone")));
        ok(store.save_theme(&theme("kept")));
        ok(store.remove_theme("gone"));
        assert!(!scratch.themes_dir().join("gone.json").exists());
        assert_eq!(ok(store.list_themes()).themes, vec![theme("kept")]);
        assert!(matches!(
            store.remove_theme("gone"),
            Err(SettingsError::NotFound { .. })
        ));
        assert!(matches!(
            store.remove_theme("../kept"),
            Err(SettingsError::Invalid { .. })
        ));
        assert!(scratch.themes_dir().join("kept.json").exists());
    }
}
