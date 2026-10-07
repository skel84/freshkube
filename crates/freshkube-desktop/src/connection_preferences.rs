//! Remember the selected Talos file and context, without copying credentials.
//! This file is separate from text-size preferences so their writes cannot
//! overwrite each other. File I/O runs at startup or on a background executor.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) path: PathBuf,
    pub(crate) context: String,
}

fn file(preferences: &Path) -> PathBuf {
    preferences.with_file_name("connection.json")
}

pub(crate) fn load(preferences: &Path) -> Option<Selection> {
    let bytes = freshkube_core::read_bounded_regular_file(&file(preferences), 64 * 1024).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let path = PathBuf::from(value.get("talosconfig")?.as_str()?);
    let context = value.get("context")?.as_str()?.to_owned();
    (path.is_absolute() && !context.is_empty()).then_some(Selection { path, context })
}

#[derive(Clone)]
pub(crate) struct ConnectionStore {
    file: PathBuf,
    // Background saves read this selection after acquiring the writer lock.
    // A delayed older task cannot restore an obsolete file or context.
    latest: Arc<Mutex<Option<Selection>>>,
    writer: Arc<Mutex<()>>,
}

impl ConnectionStore {
    pub(crate) fn new(preferences: &Path) -> Self {
        Self {
            file: file(preferences),
            latest: Default::default(),
            writer: Default::default(),
        }
    }

    pub(crate) fn remember(&self, selection: Selection) {
        *self.latest.lock().expect("connection preference mutex") = Some(selection);
    }

    pub(crate) fn save_latest(&self) -> std::io::Result<()> {
        let path = &self.file;
        // Serialize disk writes without holding the selection lock during I/O.
        // UI updates take only that short-lived selection lock.
        let _writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("connection writer mutex poisoned"))?;
        let latest = self
            .latest
            .lock()
            .map_err(|_| std::io::Error::other("connection preference mutex poisoned"))?
            .clone();
        let Some(selection) = latest else {
            return Ok(());
        };
        let value = serde_json::json!({
            "talosconfig": selection.path,
            "context": selection.context,
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
        let temporary = path.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut output = options.open(&temporary)?;
            output.write_all(format!("{value:#}\n").as_bytes())?;
            output.sync_all()?;
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_only_selection_and_late_tasks_write_the_latest_choice() {
        let directory =
            std::env::temp_dir().join(format!("freshkube-selection-store-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        std::fs::write(&preferences, "{\"text_size\":18}").unwrap();
        let store = ConnectionStore::new(&preferences);
        store.remember(Selection {
            path: directory.join("first"),
            context: "alpha".into(),
        });
        let delayed = store.clone();
        let selected = Selection {
            path: directory.join("second"),
            context: "beta".into(),
        };
        store.remember(selected.clone());
        store.save_latest().unwrap();
        delayed.save_latest().unwrap();
        assert_eq!(load(&preferences), Some(selected));
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file(&preferences)).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert_eq!(
            std::fs::read_to_string(preferences).unwrap(),
            "{\"text_size\":18}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_or_relative_selections_are_ignored_and_missing_configs_stay_selected() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-selection-invalid-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        for text in [
            "not json",
            "[]",
            r#"{"talosconfig":"relative", "context":"beta"}"#,
            r#"{"talosconfig":"/absolute", "context":""}"#,
        ] {
            std::fs::write(file(&preferences), text).unwrap();
            assert_eq!(load(&preferences), None);
        }
        let selection = Selection {
            path: directory.join("missing"),
            context: "beta".into(),
        };
        let store = ConnectionStore::new(&preferences);
        store.remember(selection.clone());
        store.save_latest().unwrap();
        assert_eq!(load(&preferences), Some(selection));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
