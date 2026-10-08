//! `navigation.json` beside the preferences: whether the sidebar is
//! collapsed, how wide each page's inspector and drawer are, in dp, and
//! the dock's state and log tabs (`desktop/dock/saved.rs`), and which
//! workspace entry was active (`workspace.active`, an entry id from
//! `workspace.json`, never a session key). The shell opens
//! it once and makes it a global, so every writer saves the same snapshot
//! and none drops another's key.
//!
//! An older build reads the file too: it reads `collapsed` and ignores the
//! keys it doesn't know, and a file it wrote without `inspector` gives each
//! inspector its default width here. Keys this build doesn't know are kept.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::{App, Global};
use serde_json::{Map, Value};

const COLLAPSED: &str = "collapsed";
const INSPECTOR: &str = "inspector";
const DRAWER: &str = "drawer";
const DOCK: &str = "dock";
const WORKSPACE: &str = "workspace";
const ACTIVE: &str = "active";

#[derive(Clone, Default)]
pub(crate) struct NavigationFile(Option<Arc<File>>);

struct File {
    path: PathBuf,
    /// The file's object as last changed; a save writes all of it.
    value: Mutex<Map<String, Value>>,
    /// Keeps two saves from writing at once.
    writer: Mutex<()>,
}

impl Global for NavigationFile {}

impl NavigationFile {
    /// The file beside `preferences`, read once. Without preferences, as in
    /// tests, nothing is read or written.
    pub(crate) fn open(preferences: Option<&Path>) -> Self {
        let Some(path) = preferences.map(|p| p.with_file_name("navigation.json")) else {
            return Self(None);
        };
        let value = std::fs::read(&path)
            .ok()
            .and_then(|s| serde_json::from_slice::<Value>(&s).ok())
            .and_then(|v| match v {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default();
        Self(Some(Arc::new(File {
            path,
            value: Mutex::new(value),
            writer: Mutex::new(()),
        })))
    }

    /// The shell's file, or none when the shell hasn't opened one.
    pub(crate) fn global(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    pub(crate) fn collapsed(&self) -> Option<bool> {
        self.read(|map| map.get(COLLAPSED).and_then(Value::as_bool))
    }

    pub(crate) fn set_collapsed(&self, collapsed: bool, cx: &App) {
        self.change(cx, |map| {
            map.insert(COLLAPSED.into(), collapsed.into());
        });
    }

    /// The width `page`'s inspector was left at, in dp.
    pub(crate) fn inspector_width(&self, page: &str) -> Option<f32> {
        self.width(INSPECTOR, page)
    }

    pub(crate) fn set_inspector_width(&self, page: &str, width: f32, cx: &App) {
        self.set_width(INSPECTOR, page, width, cx);
    }

    /// The width `page`'s drawer was left at, in dp.
    pub(crate) fn drawer_width(&self, page: &str) -> Option<f32> {
        self.width(DRAWER, page)
    }

    pub(crate) fn set_drawer_width(&self, page: &str, width: f32, cx: &App) {
        self.set_width(DRAWER, page, width, cx);
    }

    fn width(&self, group: &str, page: &str) -> Option<f32> {
        self.read(|map| {
            let width = map.get(group)?.get(page)?.as_f64()? as f32;
            (width.is_finite() && width > 0.).then_some(width)
        })
    }

    fn set_width(&self, group: &str, page: &str, width: f32, cx: &App) {
        self.change(cx, |map| {
            let widths = map
                .entry(group)
                .or_insert_with(|| Value::Object(Map::new()));
            if !widths.is_object() {
                *widths = Value::Object(Map::new());
            }
            if let Value::Object(widths) = widths {
                widths.insert(page.into(), width.round().into());
            }
        });
    }

    /// The dock as last saved, if it reads as one.
    pub(crate) fn dock<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        self.read(|map| serde_json::from_value(map.get(DOCK)?.clone()).ok())
    }

    pub(crate) fn set_dock<T: serde::Serialize>(&self, dock: &T, cx: &App) {
        let Ok(value) = serde_json::to_value(dock) else {
            return;
        };
        self.change(cx, |map| {
            map.insert(DOCK.into(), value);
        });
    }

    /// The workspace entry that was active when the app last switched.
    pub(crate) fn active_cluster(&self) -> Option<String> {
        self.read(|map| {
            let id = map.get(WORKSPACE)?.get(ACTIVE)?.as_str()?;
            (!id.is_empty()).then(|| id.to_owned())
        })
    }

    pub(crate) fn set_active_cluster(&self, id: &str, cx: &App) {
        self.change(cx, |map| {
            let group = map
                .entry(WORKSPACE)
                .or_insert_with(|| Value::Object(Map::new()));
            if !group.is_object() {
                *group = Value::Object(Map::new());
            }
            if let Value::Object(group) = group {
                group.insert(ACTIVE.into(), id.into());
            }
        });
    }

    /// Forgets the remembered entry: the window left it by hand.
    pub(crate) fn clear_active_cluster(&self, cx: &App) {
        if self.active_cluster().is_none() {
            return;
        }
        self.change(cx, |map| {
            if let Some(Value::Object(group)) = map.get_mut(WORKSPACE) {
                group.remove(ACTIVE);
            }
        });
    }

    fn read<T>(&self, read: impl FnOnce(&Map<String, Value>) -> Option<T>) -> Option<T> {
        let file = self.0.as_ref()?;
        read(&*file.value.lock().ok()?)
    }

    /// Changes the snapshot, then writes it in the background, replacing
    /// the file whole.
    fn change(&self, cx: &App, change: impl FnOnce(&mut Map<String, Value>)) {
        let Some(file) = self.0.clone() else {
            return;
        };
        let Ok(mut value) = file.value.lock() else {
            return;
        };
        change(&mut value);
        drop(value);
        cx.background_executor()
            .spawn(async move {
                let Ok(_guard) = file.writer.lock() else {
                    return;
                };
                // The latest snapshot, which a later change may have moved on.
                let Ok(text) = file
                    .value
                    .lock()
                    .map(|value| Value::Object(value.clone()).to_string())
                else {
                    return;
                };
                if let Some(parent) = file.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let temporary = file
                    .path
                    .with_extension(format!("{}.tmp", std::process::id()));
                if std::fs::write(&temporary, format!("{text}\n")).is_ok() {
                    let _ = std::fs::rename(&temporary, &file.path);
                }
            })
            .detach();
    }
}

/// The inspectors' widths, under `inspector`, for pages outside desktop.
impl freshkube_ui::inspector::SavedWidths for NavigationFile {
    fn width(&self, page: &str) -> Option<f32> {
        self.inspector_width(page)
    }

    fn save(&self, page: &str, width: f32, cx: &App) {
        self.set_inspector_width(page, width, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::NavigationFile;
    use gpui_kit::TestAppContext;
    use std::path::PathBuf;

    /// A fresh directory for one test's preferences.
    fn directory(test: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "freshkube-{test}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[gpui_kit::test]
    fn an_older_file_without_widths_keeps_its_choice_and_the_default_widths(
        cx: &mut TestAppContext,
    ) {
        let directory = directory("navigation-old");
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        // What a build before the inspector writes.
        std::fs::write(directory.join("navigation.json"), "{\"collapsed\":true}\n").unwrap();
        let file = NavigationFile::open(Some(&preferences));
        assert_eq!(file.collapsed(), Some(true));
        assert_eq!(file.inspector_width("incidents"), None);

        cx.update(|cx| file.set_inspector_width("incidents", 512.4, cx));
        cx.run_until_parked();
        let text = std::fs::read_to_string(directory.join("navigation.json")).unwrap();
        // An older build reads `collapsed` alone, as here, and ignores the rest.
        let value = serde_json::from_str::<serde_json::Value>(&text).unwrap();
        assert_eq!(value.get("collapsed").and_then(|v| v.as_bool()), Some(true));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[gpui_kit::test]
    fn widths_and_the_sidebar_survive_each_others_saves(cx: &mut TestAppContext) {
        let directory = directory("navigation-both");
        let preferences = directory.join("preferences.json");
        let file = NavigationFile::open(Some(&preferences));
        cx.update(|cx| {
            file.set_inspector_width("incidents", 512.4, cx);
            file.set_collapsed(true, cx);
            file.set_inspector_width("traces", 400., cx);
            file.set_collapsed(false, cx);
        });
        cx.run_until_parked();
        let reopened = NavigationFile::open(Some(&preferences));
        assert_eq!(reopened.collapsed(), Some(false));
        assert_eq!(reopened.inspector_width("incidents"), Some(512.));
        assert_eq!(reopened.inspector_width("traces"), Some(400.));
        assert_eq!(reopened.inspector_width("resources"), None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Pages outside desktop read and save their widths through the store
    /// the shell sets, which is this file's `inspector` key.
    #[gpui_kit::test]
    fn the_saved_widths_store_is_the_inspector_key(cx: &mut TestAppContext) {
        use freshkube_ui::inspector::{saved_widths, set_saved_widths};
        let directory = directory("navigation-store");
        let preferences = directory.join("preferences.json");
        let file = NavigationFile::open(Some(&preferences));
        cx.update(|cx| {
            set_saved_widths(std::rc::Rc::new(file.clone()), cx);
            saved_widths(cx).save("map", 400.4, cx);
        });
        cx.run_until_parked();
        let reopened = NavigationFile::open(Some(&preferences));
        assert_eq!(reopened.inspector_width("map"), Some(400.));
        cx.update(|cx| {
            set_saved_widths(std::rc::Rc::new(reopened), cx);
            assert_eq!(saved_widths(cx).width("map"), Some(400.));
            assert_eq!(saved_widths(cx).width("incidents"), None);
        });
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[gpui_kit::test]
    fn unknown_keys_survive_and_bad_widths_are_ignored(cx: &mut TestAppContext) {
        let directory = directory("navigation-unknown");
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        std::fs::write(
            directory.join("navigation.json"),
            r#"{"later":[1,2],"inspector":{"incidents":"wide","traces":-4}}"#,
        )
        .unwrap();
        let file = NavigationFile::open(Some(&preferences));
        assert_eq!(file.collapsed(), None);
        assert_eq!(file.inspector_width("incidents"), None);
        assert_eq!(file.inspector_width("traces"), None);
        cx.update(|cx| file.set_collapsed(true, cx));
        cx.run_until_parked();
        let text = std::fs::read_to_string(directory.join("navigation.json")).unwrap();
        let value = serde_json::from_str::<serde_json::Value>(&text).unwrap();
        assert_eq!(value["later"], serde_json::json!([1, 2]));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn without_preferences_nothing_is_kept() {
        let file = NavigationFile::open(None);
        assert_eq!(file.collapsed(), None);
        assert_eq!(file.inspector_width("incidents"), None);
    }

    #[gpui_kit::test]
    fn the_active_cluster_is_an_entry_id_that_keeps_the_other_keys(cx: &mut TestAppContext) {
        let directory = directory("navigation-active");
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        std::fs::write(
            directory.join("navigation.json"),
            "{\"collapsed\":true,\"workspace\":\"not an object\",\"future\":1}\n",
        )
        .unwrap();
        let file = NavigationFile::open(Some(&preferences));
        assert_eq!(file.active_cluster(), None);
        cx.update(|cx| file.set_active_cluster("acme-core", cx));
        cx.run_until_parked();
        let reopened = NavigationFile::open(Some(&preferences));
        assert_eq!(reopened.active_cluster().as_deref(), Some("acme-core"));
        assert_eq!(reopened.collapsed(), Some(true));
        let text = std::fs::read_to_string(directory.join("navigation.json")).unwrap();
        assert!(text.contains("\"future\":1"), "{text}");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
