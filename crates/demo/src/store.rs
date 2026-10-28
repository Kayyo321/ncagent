/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The demo's file-backed session store (D40): one JSON-lines file per session, under
//! `<dir>/sessions/<id>.jsonl`.
//!
//! Ids are checked with `harness::store::valid_session_id` before any path is built, so an id
//! cannot name a file outside the store. The store reads what is on disk as untrusted input. A
//! line that is not an event, or is longer than 1 MiB, is skipped and counted. It never panics.
//! The only writes are the store's own files under its directory.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use harness::event::stamped_event_t;
use harness::store::{
    StoreError, loaded_session_t, session_store_t, session_summary_t, summarize_events,
    valid_session_id,
};

/// Longest line the store reads back, in bytes. Longer lines are skipped (D40).
pub const MaxLineBytes: usize = 1024 * 1024;

/// The directory under the store's root that holds the session files.
const SessionsDir: &str = "sessions";

/// A session store that keeps each session in its own JSON-lines file.
pub struct file_store_t {
    dir: PathBuf,
}

impl file_store_t {
    /// A store rooted at `dir`. Nothing is created until a session is appended.
    pub fn new(dir: PathBuf) -> Self {
        file_store_t { dir }
    }

    /// The root directory of the store.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn sessions_dir(&self) -> PathBuf {
        self.dir.join(SessionsDir)
    }

    fn session_path(&self, id: &str) -> Result<PathBuf, StoreError> {
        if !valid_session_id(id) {
            return Err(StoreError::InvalidId { id: id.to_string() });
        }
        Ok(self.sessions_dir().join(format!("{id}.jsonl")))
    }
}

fn io_error(path: &Path, err: std::io::Error) -> StoreError {
    StoreError::Io {
        path: path.display().to_string(),
        message: err.to_string(),
    }
}

impl session_store_t for file_store_t {
    fn list(&self) -> Result<Vec<session_summary_t>, StoreError> {
        let sessions = self.sessions_dir();
        let entries = match fs::read_dir(&sessions) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_error(&sessions, err)),
        };
        let mut summaries = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !valid_session_id(id) {
                continue;
            }
            let Ok(loaded) = self.load(id) else {
                continue;
            };
            if let Some(summary) = summarize_events(id, &loaded.events) {
                summaries.push(summary);
            }
        }
        summaries.sort_by(|a, b| {
            b.updated_ms
                .cmp(&a.updated_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(summaries)
    }

    fn load(&self, id: &str) -> Result<loaded_session_t, StoreError> {
        let path = self.session_path(id)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Err(StoreError::NotFound { id: id.to_string() });
            }
            Err(err) => return Err(io_error(&path, err)),
        };
        Ok(parse_lines(&bytes))
    }

    fn append(&self, id: &str, event: &stamped_event_t) -> Result<(), StoreError> {
        let path = self.session_path(id)?;
        let line = serde_json::to_string(event).map_err(|err| StoreError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        if line.len() > MaxLineBytes {
            return Err(StoreError::Io {
                path: path.display().to_string(),
                message: "the event is over 1 MiB and could not be read back".to_string(),
            });
        }
        let sessions = self.sessions_dir();
        fs::create_dir_all(&sessions).map_err(|err| io_error(&sessions, err))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|err| io_error(&path, err))?;
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        file.write_all(&bytes).map_err(|err| io_error(&path, err))?;
        file.flush().map_err(|err| io_error(&path, err))?;
        Ok(())
    }

    /// Removes only the session's own file. The id is checked first, so the path is always
    /// `<dir>/sessions/<id>.jsonl`.
    fn delete(&self, id: &str) -> Result<(), StoreError> {
        let path = self.session_path(id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                Err(StoreError::NotFound { id: id.to_string() })
            }
            Err(err) => Err(io_error(&path, err)),
        }
    }
}

/// Parses a session file. Blank lines are ignored. Any other line that is too long, is not
/// UTF-8, or is not a stamped event is skipped and counted.
fn parse_lines(bytes: &[u8]) -> loaded_session_t {
    let mut events = Vec::new();
    let mut skipped_lines = 0usize;
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        if raw.len() > MaxLineBytes {
            skipped_lines += 1;
            continue;
        }
        let parsed = std::str::from_utf8(raw)
            .ok()
            .and_then(|text| serde_json::from_str::<stamped_event_t>(text).ok());
        match parsed {
            Some(event) => events.push(event),
            None => skipped_lines += 1,
        }
    }
    loaded_session_t {
        events,
        skipped_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::approval::ApprovalMode;
    use harness::event::SessionEvent;
    use ncm::backend::BackendKind;

    /// A directory under the system temp directory, removed when the test ends.
    struct scratch_dir_t {
        path: PathBuf,
    }

    impl scratch_dir_t {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ncagent-demo-store-{}-{}",
                std::process::id(),
                name
            ));
            let _ = fs::remove_dir_all(&path);
            scratch_dir_t { path }
        }
    }

    impl Drop for scratch_dir_t {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn start(id: &str, at_ms: u64) -> stamped_event_t {
        stamped_event_t {
            seq: 0,
            at_ms,
            event: SessionEvent::SessionStart {
                session_id: id.to_string(),
                simulated: true,
                label: Some("DEMO: simulated model, not NCM output".to_string()),
                model_id: "demo-standard".to_string(),
                backend_kind: BackendKind::Demo,
                contract_version: 1,
                approval_mode: ApprovalMode::Ask,
                cwd: "/work/project".to_string(),
                project: "project".to_string(),
                prefix_window: 4096,
                reply_len: 1024,
                level_count: 5,
                level_lens: vec![8, 32, 128, 512, 1024],
                session_seed: 4242,
            },
        }
    }

    fn message(seq: u64, at_ms: u64, text: &str) -> stamped_event_t {
        stamped_event_t {
            seq,
            at_ms,
            event: SessionEvent::UserMessage {
                turn: 1,
                text: text.to_string(),
                from_queue: false,
            },
        }
    }

    fn ok<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(err) => panic!("expected Ok, found {err:?}"),
        }
    }

    #[test]
    fn an_appended_session_loads_back_unchanged() {
        let scratch = scratch_dir_t::new("round-trip");
        let store = file_store_t::new(scratch.path.clone());
        ok(store.append("s1", &start("s1", 1)));
        ok(store.append("s1", &message(1, 2, "hello")));

        let loaded = ok(store.load("s1"));
        assert_eq!(loaded.skipped_lines, 0);
        assert_eq!(loaded.events, vec![start("s1", 1), message(1, 2, "hello")]);

        let text = ok(fs::read_to_string(
            scratch.path.join("sessions").join("s1.jsonl"),
        ));
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn malformed_lines_are_skipped_and_counted() {
        let scratch = scratch_dir_t::new("malformed");
        let store = file_store_t::new(scratch.path.clone());
        let dir = scratch.path.join("sessions");
        ok(fs::create_dir_all(&dir));
        let good = serde_json::to_string(&start("bad", 1)).unwrap_or_default();
        let content =
            format!("{good}\nnot json at all\n{{\"seq\":\n\n   \n[1,2,3]\n{good}\r\n\u{FFFD}\n");
        ok(fs::write(dir.join("bad.jsonl"), content));

        let loaded = ok(store.load("bad"));
        assert_eq!(loaded.events.len(), 2);
        // "not json", the truncated object, the array, and a lone U+FFFD, which is not JSON.
        assert_eq!(loaded.skipped_lines, 4);
    }

    #[test]
    fn a_line_over_one_mebibyte_is_skipped() {
        let scratch = scratch_dir_t::new("huge");
        let store = file_store_t::new(scratch.path.clone());
        let dir = scratch.path.join("sessions");
        ok(fs::create_dir_all(&dir));
        let good = serde_json::to_string(&start("huge", 1)).unwrap_or_default();
        let mut content = Vec::new();
        content.extend_from_slice(good.as_bytes());
        content.push(b'\n');
        content.extend(std::iter::repeat_n(b'x', MaxLineBytes + 1));
        content.push(b'\n');
        content.extend_from_slice(good.as_bytes());
        content.push(b'\n');
        ok(fs::write(dir.join("huge.jsonl"), content));

        let loaded = ok(store.load("huge"));
        assert_eq!(loaded.events.len(), 2);
        assert_eq!(loaded.skipped_lines, 1);
    }

    #[test]
    fn path_like_ids_are_refused_and_write_nothing() {
        let scratch = scratch_dir_t::new("traversal");
        let store = file_store_t::new(scratch.path.clone());
        for bad in ["../x", "a/b", "..", "", "a.b"] {
            assert!(
                matches!(store.load(bad), Err(StoreError::InvalidId { .. })),
                "load {bad:?}"
            );
            assert!(
                matches!(
                    store.append(bad, &start("x", 1)),
                    Err(StoreError::InvalidId { .. })
                ),
                "append {bad:?}"
            );
        }
        assert!(
            !scratch.path.exists(),
            "a refused id must not create anything"
        );
    }

    #[test]
    fn a_missing_session_is_not_found() {
        let scratch = scratch_dir_t::new("missing");
        let store = file_store_t::new(scratch.path.clone());
        assert!(matches!(
            store.load("nothing-here"),
            Err(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn a_missing_sessions_directory_lists_as_empty() {
        let scratch = scratch_dir_t::new("no-dir");
        let store = file_store_t::new(scratch.path.clone());
        assert!(ok(store.list()).is_empty());
        assert!(!scratch.path.exists(), "listing must not create the store");
    }

    #[test]
    fn list_sorts_newest_first_and_skips_junk() {
        let scratch = scratch_dir_t::new("list");
        let store = file_store_t::new(scratch.path.clone());
        ok(store.append("older", &start("older", 100)));
        ok(store.append("newer", &start("newer", 200)));
        ok(store.append("newer", &message(1, 250, "hi")));

        let dir = scratch.path.join("sessions");
        ok(fs::write(dir.join("notes.txt"), "not a session"));
        ok(fs::write(dir.join("junk.jsonl"), "garbage only\n"));
        ok(fs::write(dir.join("bad name.jsonl"), "{}"));
        ok(fs::create_dir_all(dir.join("folder.jsonl")));

        let summaries = ok(store.list());
        let ids: Vec<&str> = summaries
            .iter()
            .map(|summary| summary.id.as_str())
            .collect();
        assert_eq!(ids, vec!["newer", "older"]);
        assert_eq!(summaries[0].updated_ms, 250);
        assert_eq!(summaries[0].title, "hi");
        assert!(summaries[0].simulated);
    }

    #[test]
    fn a_deleted_session_is_not_found_and_leaves_the_list() {
        let scratch = scratch_dir_t::new("delete");
        let store = file_store_t::new(scratch.path.clone());
        ok(store.append("keep", &start("keep", 1)));
        ok(store.append("gone", &start("gone", 2)));
        ok(store.delete("gone"));

        assert!(matches!(
            store.load("gone"),
            Err(StoreError::NotFound { .. })
        ));
        let ids: Vec<String> = ok(store.list()).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["keep"]);
        assert!(!scratch.path.join("sessions").join("gone.jsonl").exists());
        assert!(matches!(
            store.delete("gone"),
            Err(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn delete_refuses_path_like_ids_and_touches_nothing_outside_the_sessions_dir() {
        let scratch = scratch_dir_t::new("delete-traversal");
        let store = file_store_t::new(scratch.path.clone());
        ok(store.append("keep", &start("keep", 1)));
        // A file next to the sessions directory must survive a delete aimed at it.
        let outside = scratch.path.join("outside.jsonl");
        ok(fs::write(&outside, "not a session"));

        for bad in ["../outside", "a/b", "..", ""] {
            assert!(
                matches!(store.delete(bad), Err(StoreError::InvalidId { .. })),
                "delete {bad:?}"
            );
        }
        assert!(outside.exists());
        assert_eq!(ok(store.load("keep")).events.len(), 1);
    }

    #[test]
    fn delete_of_a_session_whose_file_is_a_directory_is_an_io_error() {
        let scratch = scratch_dir_t::new("delete-dir");
        let store = file_store_t::new(scratch.path.clone());
        let dir = scratch.path.join("sessions");
        ok(fs::create_dir_all(dir.join("odd.jsonl")));
        assert!(matches!(store.delete("odd"), Err(StoreError::Io { .. })));
        assert!(dir.join("odd.jsonl").is_dir());
    }

    #[test]
    fn append_refuses_an_event_too_large_to_read_back() {
        let scratch = scratch_dir_t::new("too-large");
        let store = file_store_t::new(scratch.path.clone());
        let huge = "y".repeat(MaxLineBytes);
        let result = store.append("big", &message(1, 1, &huge));
        assert!(matches!(result, Err(StoreError::Io { .. })));
    }
}
