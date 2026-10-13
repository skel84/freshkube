//! Remember the selected Talos file and context, or the Kubernetes-only
//! kubeconfig and context, without copying credentials.
//! This file is separate from text-size preferences so their writes cannot
//! overwrite each other. File I/O runs at startup or on a background executor.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use freshkube_prefs::{Document, Layout};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _, ser::SerializeMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) path: PathBuf,
    pub(crate) context: String,
}

/// A kubeconfig file and context chosen for Kubernetes-only mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KubeSelection {
    pub(crate) path: PathBuf,
    pub(crate) context: String,
}

/// What the last Finder launch should open: Talos on a talosconfig, or
/// Kubernetes only on a kubeconfig. Choosing one replaces the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Remembered {
    Talos(Selection),
    Kubernetes(KubeSelection),
}

fn file(preferences: &Path) -> PathBuf {
    preferences.with_file_name("connection.json")
}

/// `connection.json`: at most 64 KiB, and only the owner's to read and write
/// for a new file. It holds paths and a context name, never a credential.
fn document(path: &Path) -> Document<Option<Remembered>> {
    Document::at(path, Layout::Pretty, 0o600).limited_to(64 * 1024)
}

impl Remembered {
    /// What a file holds, or none when it isn't an absolute path with a
    /// non-empty context. `mode` picks Kubernetes-only; any other is Talos.
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let context = value.get("context")?.as_str()?.to_owned();
        if context.is_empty() {
            return None;
        }
        if value.get("mode").and_then(|mode| mode.as_str()) == Some("kubernetes") {
            let path = PathBuf::from(value.get("kubeconfig")?.as_str()?);
            return path
                .is_absolute()
                .then_some(Remembered::Kubernetes(KubeSelection { path, context }));
        }
        let path = PathBuf::from(value.get("talosconfig")?.as_str()?);
        path.is_absolute()
            .then_some(Remembered::Talos(Selection { path, context }))
    }
}

/// Talos is `{talosconfig, context}`; Kubernetes-only is
/// `{mode: "kubernetes", kubeconfig, context}`. Keys in that order.
impl Serialize for Remembered {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Remembered::Talos(selection) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("talosconfig", &selection.path)?;
                map.serialize_entry("context", &selection.context)?;
                map.end()
            }
            Remembered::Kubernetes(selection) => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("mode", "kubernetes")?;
                map.serialize_entry("kubeconfig", &selection.path)?;
                map.serialize_entry("context", &selection.context)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Remembered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::from_json(&value).ok_or_else(|| {
            D::Error::custom("not a remembered connection: an absolute path and a context")
        })
    }
}

/// What the file holds. A missing, unreadable or unusable file is nothing
/// remembered; a file that exists but isn't usable is logged.
pub(crate) fn load_remembered(preferences: &Path) -> Option<Remembered> {
    let loaded = document(&file(preferences)).load();
    if let Some(problem) = &loaded.problem {
        tracing::warn!("connection preference not used: {problem}");
    }
    loaded.value
}

/// The remembered Talos selection; a remembered Kubernetes-only choice is
/// not one.
#[cfg(test)]
pub(crate) fn load(preferences: &Path) -> Option<Selection> {
    match load_remembered(preferences)? {
        Remembered::Talos(selection) => Some(selection),
        Remembered::Kubernetes(_) => None,
    }
}

#[derive(Clone)]
pub(crate) struct ConnectionStore {
    file: PathBuf,
    // Background saves read this selection after acquiring the writer lock.
    // A delayed older task cannot restore an obsolete file or context.
    latest: Arc<Mutex<Option<Remembered>>>,
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
        *self.latest.lock().expect("connection preference mutex") =
            Some(Remembered::Talos(selection));
    }

    pub(crate) fn remember_kubernetes(&self, selection: KubeSelection) {
        *self.latest.lock().expect("connection preference mutex") =
            Some(Remembered::Kubernetes(selection));
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
        document(path).save(&Some(selection))
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

    #[test]
    fn a_kubernetes_choice_replaces_a_talos_one_and_back() {
        let directory =
            std::env::temp_dir().join(format!("freshkube-selection-mode-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        let store = ConnectionStore::new(&preferences);
        let talos = Selection {
            path: directory.join("talosconfig"),
            context: "alpha".into(),
        };
        let kube = KubeSelection {
            path: directory.join("kubeconfig"),
            context: "example".into(),
        };
        store.remember(talos.clone());
        store.save_latest().unwrap();
        assert_eq!(
            load_remembered(&preferences),
            Some(Remembered::Talos(talos.clone()))
        );
        store.remember_kubernetes(kube.clone());
        store.save_latest().unwrap();
        assert_eq!(
            load_remembered(&preferences),
            Some(Remembered::Kubernetes(kube))
        );
        // A Kubernetes-only choice is not a Talos selection.
        assert_eq!(load(&preferences), None);
        store.remember(talos.clone());
        store.save_latest().unwrap();
        assert_eq!(load(&preferences), Some(talos));
        for text in [
            r#"{"mode":"kubernetes","kubeconfig":"relative","context":"example"}"#,
            r#"{"mode":"kubernetes","kubeconfig":"/absolute","context":""}"#,
            r#"{"mode":"kubernetes","context":"example"}"#,
        ] {
            std::fs::write(file(&preferences), text).unwrap();
            assert_eq!(load_remembered(&preferences), None);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// The example files in `freshkube-prefs`, as the app writes them: what
    /// loads from one saves back byte for byte.
    // The committed examples use POSIX paths, which Windows does not treat as absolute.
    #[cfg(unix)]
    #[test]
    fn the_example_files_round_trip_byte_for_byte() {
        round_trip(&[
            include_str!("../../freshkube-prefs/src/examples/connection.json"),
            include_str!("../../freshkube-prefs/src/examples/connection-kubernetes.json"),
        ]);
    }

    /// The same round trips with Windows paths, written inline.
    #[cfg(not(unix))]
    #[test]
    fn the_example_files_round_trip_byte_for_byte_off_unix() {
        round_trip(&[
            "{\n  \"talosconfig\": \"C:\\\\acme\\\\talosconfig\",\n  \"context\": \"cluster-a\"\n}\n",
            "{\n  \"mode\": \"kubernetes\",\n  \"kubeconfig\": \"C:\\\\acme\\\\kubeconfig\",\n  \"context\": \"cluster-a\"\n}\n",
        ]);
    }

    fn round_trip(examples: &[&str]) {
        for &example in examples {
            let directory = tempfile::tempdir().unwrap();
            let preferences = directory.path().join("preferences.json");
            std::fs::write(file(&preferences), example).unwrap();
            let remembered = load_remembered(&preferences).expect("the example loads");
            let store = ConnectionStore::new(&preferences);
            match remembered {
                Remembered::Talos(selection) => store.remember(selection),
                Remembered::Kubernetes(selection) => store.remember_kubernetes(selection),
            }
            store.save_latest().unwrap();
            assert_eq!(
                std::fs::read_to_string(file(&preferences)).unwrap(),
                example
            );
        }
    }

    #[test]
    fn a_file_past_its_limit_or_not_a_file_is_nothing_remembered() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = directory.path().join("preferences.json");
        let padding = " ".repeat(64 * 1024);
        std::fs::write(
            file(&preferences),
            format!(
                "{}{padding}",
                include_str!("../../freshkube-prefs/src/examples/connection.json")
            ),
        )
        .unwrap();
        assert_eq!(load_remembered(&preferences), None);
        std::fs::remove_file(file(&preferences)).unwrap();
        std::fs::create_dir(file(&preferences)).unwrap();
        assert_eq!(load_remembered(&preferences), None);
    }
}
