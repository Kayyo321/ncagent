/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The demo's file-backed folder store (D50): the folders and the filed conversations live in
//! `<dir>/folders.json`, next to the demo sessions.
//!
//! The file is untrusted input (D52). A file over 1 MiB, a file that is not JSON, and a file
//! that fails any rule of `harness::organize` is refused with a typed error, and the store
//! never writes in `load`. A save writes a temporary file next to the folders file and renames
//! it over the folders file, so a failed save leaves the last good file in place. The store
//! writes nothing outside its directory.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use harness::organize::{OrganizeError, organization_file_t, organization_store_t, organization_t};

use crate::Label;

/// Longest folders file the store reads or writes, in bytes (D52). A larger file is refused, and
/// a save that would produce one is refused too, so the store never writes a file it cannot read.
pub const MaxFileBytes: usize = 1024 * 1024;

/// The file the folders are kept in, under the demo directory (D50).
const FoldersFile: &str = "folders.json";

/// The file a save writes before it renames it over the folders file.
const TempFile: &str = "folders.json.tmp";

/// Folders kept in `<dir>/folders.json`.
pub struct file_organization_store_t {
    dir: PathBuf,
}

impl file_organization_store_t {
    /// A store rooted at `dir`. Nothing is created until the first save.
    pub fn new(dir: PathBuf) -> Self {
        file_organization_store_t { dir }
    }

    fn folders_path(&self) -> PathBuf {
        self.dir.join(FoldersFile)
    }

    fn temp_path(&self) -> PathBuf {
        self.dir.join(TempFile)
    }
}

fn io_error(path: &Path, err: std::io::Error) -> OrganizeError {
    OrganizeError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    }
}

impl organization_store_t for file_organization_store_t {
    fn load(&self) -> Result<organization_t, OrganizeError> {
        let path = self.folders_path();
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(organization_t::new()),
            Err(err) => return Err(io_error(&path, err)),
        };
        // One byte more than the limit, so a file over the limit is seen as over it.
        let mut bytes = Vec::new();
        file.take(MaxFileBytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| io_error(&path, err))?;
        if bytes.len() > MaxFileBytes {
            return Err(OrganizeError::Malformed {
                message: format!("the file is larger than {MaxFileBytes} bytes"),
            });
        }
        let parsed: organization_file_t =
            serde_json::from_slice(&bytes).map_err(|err| OrganizeError::Malformed {
                message: err.to_string(),
            })?;
        organization_t::from_file(&parsed)
    }

    fn save(&self, organization: &organization_t) -> Result<(), OrganizeError> {
        let folders = self.folders_path();
        let temp = self.temp_path();
        let text =
            serde_json::to_string_pretty(&organization.to_file(Some(Label))).map_err(|err| {
                OrganizeError::Io {
                    path: folders.display().to_string(),
                    message: err.to_string(),
                }
            })?;
        // A file over the limit would be refused by `load`, so it is never written.
        if text.len() > MaxFileBytes {
            return Err(OrganizeError::Io {
                path: folders.display().to_string(),
                message: format!(
                    "the folders take more than {MaxFileBytes} bytes, so they could not be read back"
                ),
            });
        }
        fs::create_dir_all(&self.dir).map_err(|err| io_error(&self.dir, err))?;
        write_then_rename(&temp, &folders, text.as_bytes()).inspect_err(|_| {
            // Best effort: the folders file is still the last good one.
            let _ = fs::remove_file(&temp);
        })
    }
}

/// Writes `bytes` to `temp` and renames it over `target`. A stale temporary file is removed
/// first, so the create below never follows a link that someone left at that name.
fn write_then_rename(temp: &Path, target: &Path, bytes: &[u8]) -> Result<(), OrganizeError> {
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

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
                "ncagent-demo-organize-{}-{counter}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            scratch_dir_t { path }
        }

        fn folders_file(&self) -> PathBuf {
            self.path.join(FoldersFile)
        }

        fn temp_file(&self) -> PathBuf {
            self.path.join(TempFile)
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

    fn sample() -> organization_t {
        let mut org = organization_t::new();
        let research = ok(org.create_folder("Research", None));
        let papers = ok(org.create_folder("Papers", Some(&research)));
        ok(org.create_folder("Ideas", None));
        ok(org.move_session("s_1", Some(&papers)));
        ok(org.move_session("s_2", Some(&research)));
        org
    }

    /// Writes `text` as the folders file, then loads it.
    fn load_text(scratch: &scratch_dir_t, text: &str) -> Result<organization_t, OrganizeError> {
        ok(fs::create_dir_all(&scratch.path));
        ok(fs::write(scratch.folders_file(), text));
        file_organization_store_t::new(scratch.path.clone()).load()
    }

    #[test]
    fn a_saved_organization_loads_back_unchanged() {
        let scratch = scratch_dir_t::new("round-trip");
        let store = file_organization_store_t::new(scratch.path.clone());
        let org = sample();
        ok(store.save(&org));
        assert_eq!(ok(store.load()), org);
        assert_eq!(org.folder_of("s_1"), Some("f2"));
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_organization() {
        let scratch = scratch_dir_t::new("missing");
        let store = file_organization_store_t::new(scratch.path.clone());
        assert_eq!(ok(store.load()), organization_t::new());
        assert!(!scratch.path.exists(), "loading must not create the store");
    }

    #[test]
    fn malformed_json_is_refused_and_the_file_is_left_alone() {
        let scratch = scratch_dir_t::new("malformed");
        let text = "{ \"version\": 1, \"folders\": [";
        ok(fs::create_dir_all(&scratch.path));
        ok(fs::write(scratch.folders_file(), text));
        let result = file_organization_store_t::new(scratch.path.clone()).load();
        assert!(matches!(result, Err(OrganizeError::Malformed { .. })));
        assert_eq!(ok(fs::read_to_string(scratch.folders_file())), text);
    }

    #[test]
    fn a_file_over_one_mebibyte_is_refused_even_when_it_is_valid_json() {
        let scratch = scratch_dir_t::new("oversized");
        // Valid JSON. Unknown fields are ignored, so only the size check refuses it.
        let pad = "x".repeat(MaxFileBytes);
        let text = format!("{{\"version\":1,\"folders\":[],\"filed\":{{}},\"pad\":\"{pad}\"}}");
        assert!(text.len() > MaxFileBytes);
        let result = load_text(&scratch, &text);
        assert!(matches!(result, Err(OrganizeError::Malformed { .. })));
    }

    #[test]
    fn a_save_that_would_produce_an_unreadable_file_is_refused_and_writes_nothing() {
        let scratch = scratch_dir_t::new("too-big");
        let store = file_organization_store_t::new(scratch.path.clone());
        let mut org = organization_t::new();
        let big = ok(org.create_folder("Big", None));
        // Each entry is about 32 bytes in the pretty form, so 40,000 entries pass 1 MiB.
        for index in 0..40_000u64 {
            ok(org.move_session(&format!("s_{index:016x}"), Some(&big)));
        }
        assert!(matches!(store.save(&org), Err(OrganizeError::Io { .. })));
        assert!(!scratch.folders_file().exists());
        assert!(!scratch.temp_file().exists());
    }

    #[test]
    fn a_folders_path_that_is_a_directory_is_an_io_error() {
        let scratch = scratch_dir_t::new("is-a-dir");
        ok(fs::create_dir_all(scratch.folders_file()));
        let result = file_organization_store_t::new(scratch.path.clone()).load();
        assert!(matches!(result, Err(OrganizeError::Io { .. })));
    }

    #[test]
    fn a_save_under_a_file_is_an_io_error() {
        let scratch = scratch_dir_t::new("blocked");
        ok(fs::create_dir_all(&scratch.path));
        ok(fs::write(
            scratch.path.join("blocker"),
            "a file, not a folder",
        ));
        let store = file_organization_store_t::new(scratch.path.join("blocker").join("demo"));
        assert!(matches!(
            store.save(&sample()),
            Err(OrganizeError::Io { .. })
        ));
        assert!(scratch.path.join("blocker").is_file());
    }

    #[test]
    fn a_file_with_a_cycle_is_refused() {
        let scratch = scratch_dir_t::new("cycle");
        let text = r#"{"version":1,"folders":[
            {"id":"f1","name":"A","parent":"f2"},
            {"id":"f2","name":"B","parent":"f1"}],"filed":{}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::Malformed { .. })
        ));
    }

    #[test]
    fn a_file_with_a_duplicate_folder_id_is_refused() {
        let scratch = scratch_dir_t::new("duplicate");
        let text = r#"{"version":1,"folders":[
            {"id":"f1","name":"A","parent":null},
            {"id":"f1","name":"B","parent":null}],"filed":{}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::Malformed { .. })
        ));
    }

    #[test]
    fn a_file_with_an_unknown_version_is_refused() {
        let scratch = scratch_dir_t::new("version");
        let text = r#"{"version":2,"folders":[],"filed":{}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::UnsupportedVersion { version: 2 })
        ));
    }

    #[test]
    fn a_file_with_a_missing_parent_is_refused() {
        let scratch = scratch_dir_t::new("missing-parent");
        let text = r#"{"version":1,"folders":[{"id":"f1","name":"A","parent":"f9"}],"filed":{}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::Malformed { .. })
        ));
    }

    #[test]
    fn a_file_with_a_name_taken_among_siblings_is_refused() {
        let scratch = scratch_dir_t::new("name-clash");
        let text = r#"{"version":1,"folders":[
            {"id":"f1","name":"Work","parent":null},
            {"id":"f2","name":"work","parent":null}],"filed":{}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::NameTaken { .. })
        ));
    }

    #[test]
    fn a_file_with_a_bad_folder_id_is_refused() {
        let scratch = scratch_dir_t::new("bad-folder-id");
        let text = r#"{"version":1,"folders":[{"id":"../f","name":"A","parent":null}],"filed":{}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::InvalidId { .. })
        ));
    }

    #[test]
    fn a_file_with_a_bad_session_id_is_refused() {
        let scratch = scratch_dir_t::new("bad-session-id");
        let text = r#"{"version":1,"folders":[{"id":"f1","name":"A","parent":null}],
            "filed":{"../x":"f1"}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::InvalidId { .. })
        ));
    }

    #[test]
    fn a_file_filing_a_session_in_a_missing_folder_is_refused() {
        let scratch = scratch_dir_t::new("filed-missing");
        let text = r#"{"version":1,"folders":[],"filed":{"s_1":"f1"}}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::Malformed { .. })
        ));
    }

    #[test]
    fn a_file_missing_a_required_field_is_refused() {
        let scratch = scratch_dir_t::new("no-filed");
        let text = r#"{"version":1,"folders":[]}"#;
        assert!(matches!(
            load_text(&scratch, text),
            Err(OrganizeError::Malformed { .. })
        ));
    }

    #[test]
    fn unknown_fields_are_ignored_as_serde_does_by_default() {
        let scratch = scratch_dir_t::new("extra-fields");
        let text = r#"{"version":1,"note":"from a later build","folders":[
            {"id":"f1","name":"A","parent":null,"colour":"red"}],
            "filed":{"s_1":"f1"}}"#;
        let org = ok(load_text(&scratch, text));
        assert_eq!(org.folder("f1").map(|f| f.name.as_str()), Some("A"));
        assert_eq!(org.folder_of("s_1"), Some("f1"));
    }

    #[test]
    fn the_file_carries_the_demo_label_and_a_loaded_label_is_ignored() {
        let scratch = scratch_dir_t::new("label");
        let store = file_organization_store_t::new(scratch.path.clone());
        ok(store.save(&sample()));
        let saved: serde_json::Value = ok(serde_json::from_str(&ok(fs::read_to_string(
            scratch.folders_file(),
        ))));
        assert_eq!(saved["label"], serde_json::json!(Label));

        let text = r#"{"version":1,"label":"someone else","folders":[],"filed":{}}"#;
        assert_eq!(ok(load_text(&scratch, text)), organization_t::new());
    }

    #[test]
    fn no_temporary_file_is_left_after_a_save() {
        let scratch = scratch_dir_t::new("no-temp");
        let store = file_organization_store_t::new(scratch.path.clone());
        ok(store.save(&sample()));
        assert!(scratch.folders_file().exists());
        assert!(!scratch.temp_file().exists());
    }

    #[test]
    fn a_stale_temporary_file_is_replaced_by_a_save() {
        let scratch = scratch_dir_t::new("stale-temp");
        ok(fs::create_dir_all(&scratch.path));
        ok(fs::write(scratch.temp_file(), "left over from a crash"));
        let store = file_organization_store_t::new(scratch.path.clone());
        ok(store.save(&sample()));
        assert!(!scratch.temp_file().exists());
        assert_eq!(ok(store.load()), sample());
    }

    #[test]
    fn saving_twice_keeps_the_latest() {
        let scratch = scratch_dir_t::new("twice");
        let store = file_organization_store_t::new(scratch.path.clone());
        let mut org = sample();
        ok(store.save(&org));
        ok(org.rename_folder("f1", "Reading"));
        ok(org.move_session("s_9", Some("f3")));
        ok(store.save(&org));
        let loaded = ok(store.load());
        assert_eq!(loaded, org);
        assert_eq!(
            loaded.folder("f1").map(|f| f.name.as_str()),
            Some("Reading")
        );
        assert_eq!(loaded.folder_of("s_9"), Some("f3"));
    }
}
