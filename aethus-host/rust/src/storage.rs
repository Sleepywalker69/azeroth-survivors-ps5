//! The page's localStorage, kept in a JSON file in the player's folder. Changes are written a
//! moment after they are made, and when the host closes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::log::warn;

pub struct Storage {
    path: PathBuf,
    entries: BTreeMap<String, String>,
    /// When the unsaved changes were first made (the page's clock), if there are any.
    dirty_since: Option<f64>,
}

/// How long changes wait before they are written, so a burst is one write.
const DELAY_MS: f64 = 1000.0;

impl Storage {
    pub fn open(path: PathBuf) -> Storage {
        let mut entries = BTreeMap::new();
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(Value::Object(map)) => {
                    for (k, v) in map {
                        if let Value::String(s) = v {
                            entries.insert(k, s);
                        }
                    }
                }
                _ => warn!("storage: {} is not a JSON object; starting empty", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("storage: {} cannot be read: {e}", path.display()),
        }
        Storage { path, entries, dirty_since: None }
    }

    /// Every entry, as the JSON object `storage_load` returns.
    pub fn load(&self) -> String {
        serde_json::to_string(&self.entries).unwrap_or_else(|_| "{}".into())
    }

    /// Sets an entry, or removes it when `value` is None.
    pub fn set(&mut self, key: &str, value: Option<&str>, now: f64) {
        let changed = match value {
            Some(v) => self.entries.insert(key.to_owned(), v.to_owned()).as_deref() != Some(v),
            None => self.entries.remove(key).is_some(),
        };
        if changed && self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
    }

    /// Writes the changes once they have waited long enough, or at once with `force`.
    pub fn flush(&mut self, now: f64, force: bool) {
        let Some(since) = self.dirty_since else { return };
        if !force && now - since < DELAY_MS {
            return;
        }
        self.dirty_since = None;
        let text = match serde_json::to_string_pretty(&self.entries) {
            Ok(text) => text,
            Err(e) => {
                warn!("storage: {e}");
                return;
            }
        };
        // Written beside, then moved over, so a crash mid-write leaves the old file.
        let temp = self.path.with_extension("json.new");
        let result = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, &self.path));
        if let Err(e) = result {
            warn!("storage: {} was not written: {e}", self.path.display());
        }
    }
}
