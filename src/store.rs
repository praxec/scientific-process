//! Durable, append-only JSONL event-log storage for scientific-process sessions.
//!
//! [`FilesystemStore`] is the only I/O boundary: it appends one serialized
//! [`Event`](crate::Event) per line to `<state_dir>/<session_id>.jsonl` and
//! replays those lines back in append order. The domain fold itself stays pure
//! in [`crate::Session`].

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::Event;

/// Environment variable overriding the state directory.
pub const STATE_DIR_ENV: &str = "SCIENTIFIC_PROCESS_STATE_DIR";

/// Resolve the state directory:
///  1. `SCIENTIFIC_PROCESS_STATE_DIR`,
///  2. `$XDG_DATA_HOME/scientific-process`,
///  3. `$HOME/.local/share/scientific-process`,
///  4. `./scientific-process-state` as a last resort.
pub fn resolve_state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(STATE_DIR_ENV) {
        return PathBuf::from(dir);
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("scientific-process");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("scientific-process");
    }
    PathBuf::from("scientific-process-state")
}

/// Reject session ids that are empty or could escape the state directory or
/// collide with the `.jsonl` layout. Dots are allowed inside an id, but a
/// leading dot or any `..` segment is rejected.
pub fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty() {
        return Err("invalid session id: empty".to_string());
    }
    let has_illegal_char = session_id
        .chars()
        .any(|c| c == '/' || c == '\\' || c.is_control());
    if has_illegal_char || session_id.contains("..") || session_id.starts_with('.') {
        return Err(format!("invalid session id: {session_id}"));
    }
    Ok(())
}

/// Filesystem-backed event log: one JSONL file per session under `state_dir`.
#[derive(Debug, Clone)]
pub struct FilesystemStore {
    state_dir: PathBuf,
}

impl FilesystemStore {
    /// Create a store rooted at `state_dir`, creating the directory if needed.
    pub fn new(state_dir: impl Into<PathBuf>) -> Result<Self, String> {
        let state_dir = state_dir.into();
        fs::create_dir_all(&state_dir).map_err(|e| format!("create_dir_all {state_dir:?}: {e}"))?;
        Ok(Self { state_dir })
    }

    /// The configured state directory.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    fn path_for(&self, session_id: &str) -> Result<PathBuf, String> {
        validate_session_id(session_id)?;
        Ok(self.state_dir.join(format!("{session_id}.jsonl")))
    }

    /// Append one event to the session's log, creating it if absent. The event
    /// is flushed before returning so it is durable across a restart.
    pub fn append(&self, session_id: &str, event: &Event) -> Result<(), String> {
        let path = self.path_for(session_id)?;
        let line = serde_json::to_string(event).map_err(|e| format!("serialize event: {e}"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("open {path:?}: {e}"))?;
        writeln!(file, "{line}").map_err(|e| format!("write {path:?}: {e}"))?;
        file.flush().map_err(|e| format!("flush {path:?}: {e}"))?;
        Ok(())
    }

    /// Replay all events for a session in append order. A session that has
    /// never been written replays as an empty log.
    pub fn replay(&self, session_id: &str) -> Result<Vec<Event>, String> {
        let path = self.path_for(session_id)?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&path).map_err(|e| format!("open {path:?}: {e}"))?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (idx, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| format!("read line: {e}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let event: Event = serde_json::from_str(&line)
                .map_err(|e| format!("parse {path:?} line {}: {e}", idx + 1))?;
            events.push(event);
        }
        Ok(events)
    }
}
