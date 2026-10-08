//! `navigation.json` beside the preferences: whether the sidebar is
//! collapsed, the size of every split the user dragged, in dp
//! (`freshkube_ui::split_size`: `inspector.<page>`, `drawer.resources`,
//! `dock.height`), and the dock's state and log tabs
//! (`desktop/dock/saved.rs`). The shell opens it once and makes it a
//! global, so every writer saves the same snapshot and none drops another's
//! key.
//!
//! An older build reads the file too: it reads `collapsed` and ignores the
//! keys it doesn't know, and a file it wrote without a split's size gives
//! that split its default here. Keys this build doesn't know are kept.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use freshkube_ui::split_size::{SizeKey, SizeStore};
use gpui_kit::{App, Global};
use serde_json::{Map, Value};

const COLLAPSED: &str = "collapsed";
const DOCK: &str = "dock";

/// Resources' drawer width: one for the page, whatever kind it shows.
pub(crate) const DRAWER_WIDTH: SizeKey = SizeKey::new("drawer", "resources");
/// The dock's height, one for the app, inside the dock's object.
pub(crate) const DOCK_HEIGHT: SizeKey = SizeKey::new(DOCK, "height");

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

    /// Makes this the shell's file and the store every split's size is
    /// read from and saved to.
    pub(crate) fn install(self, cx: &mut App) {
        freshkube_ui::split_size::set_store(std::rc::Rc::new(self.clone()), cx);
        cx.set_global(self);
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

    /// The dock as last saved, if it reads as one.
    pub(crate) fn dock<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        self.read(|map| serde_json::from_value(map.get(DOCK)?.clone()).ok())
    }

    /// Writes the dock's fields into its object, keeping the others there:
    /// its height is a split's size, saved under `dock.height`.
    pub(crate) fn set_dock<T: serde::Serialize>(&self, dock: &T, cx: &App) {
        let Ok(Value::Object(fields)) = serde_json::to_value(dock) else {
            return;
        };
        self.change(cx, |map| {
            let dock = object(map, DOCK);
            for (field, value) in fields {
                dock.insert(field, value);
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

/// Every split's size, under `group.name`, in whole dp.
impl SizeStore for NavigationFile {
    fn size(&self, key: SizeKey) -> Option<f32> {
        self.read(|map| {
            let size = map.get(key.group)?.get(key.name)?.as_f64()? as f32;
            (size.is_finite() && size > 0.).then_some(size)
        })
    }

    fn save(&self, key: SizeKey, size: f32, cx: &App) {
        self.change(cx, |map| {
            object(map, key.group).insert(key.name.into(), size.round().into());
        });
    }
}

/// The object under `key`, made one if it is missing or isn't one.
fn object<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let value = map.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    match value {
        Value::Object(object) => object,
        _ => unreachable!("made an object above"),
    }
}

#[cfg(test)]
mod tests {
    use super::{DOCK_HEIGHT, DRAWER_WIDTH, NavigationFile};
    use freshkube_ui::inspector::width_key;
    use freshkube_ui::split_size::{SizeStore as _, SplitSize};
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

    fn read(path: PathBuf) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
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
        assert_eq!(file.size(width_key("incidents")), None);

        cx.update(|cx| file.save(width_key("incidents"), 512.4, cx));
        cx.run_until_parked();
        // An older build reads `collapsed` alone, as here, and ignores the rest.
        let value = read(directory.join("navigation.json"));
        assert_eq!(value.get("collapsed").and_then(|v| v.as_bool()), Some(true));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[gpui_kit::test]
    fn widths_and_the_sidebar_survive_each_others_saves(cx: &mut TestAppContext) {
        let directory = directory("navigation-both");
        let preferences = directory.join("preferences.json");
        let file = NavigationFile::open(Some(&preferences));
        cx.update(|cx| {
            file.save(width_key("incidents"), 512.4, cx);
            file.set_collapsed(true, cx);
            file.save(width_key("traces"), 400., cx);
            file.set_collapsed(false, cx);
        });
        cx.run_until_parked();
        let reopened = NavigationFile::open(Some(&preferences));
        assert_eq!(reopened.collapsed(), Some(false));
        assert_eq!(reopened.size(width_key("incidents")), Some(512.));
        assert_eq!(reopened.size(width_key("traces")), Some(400.));
        assert_eq!(reopened.size(width_key("resources")), None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Every split saves through the installed file, and the next run's
    /// splits start where the user left them.
    #[gpui_kit::test]
    fn a_split_size_survives_a_restart(cx: &mut TestAppContext) {
        let directory = directory("navigation-restart");
        let preferences = directory.join("preferences.json");
        let splits = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                NavigationFile::open(Some(&preferences)).install(cx);
                [
                    SplitSize::new(width_key("map"), 320., 320., cx),
                    SplitSize::new(DRAWER_WIDTH, 725., 300., cx),
                    SplitSize::new(DOCK_HEIGHT, 300., 100., cx),
                ]
            })
        };
        let [map, drawer, dock] = splits(cx);
        cx.update(|cx| {
            map.release(400.4, cx);
            drawer.drag(560., cx);
            dock.drag(420., cx);
        });
        cx.executor()
            .advance_clock(freshkube_ui::split_size::SAVE_DELAY * 2);
        cx.run_until_parked();

        let [map, drawer, dock] = splits(cx);
        assert_eq!(map.size(), 400.);
        assert_eq!(drawer.size(), 560.);
        assert_eq!(dock.size(), 420.);
        let value = read(directory.join("navigation.json"));
        assert_eq!(value["inspector"]["map"], serde_json::json!(400.0));
        assert_eq!(value["drawer"]["resources"], serde_json::json!(560.0));
        assert_eq!(value["dock"]["height"], serde_json::json!(420.0));
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// A file as the build before the shared helper wrote it gives every
    /// split its size, and the dock's own saves keep its height, where
    /// that build reads it.
    #[gpui_kit::test]
    fn the_keys_an_earlier_build_wrote_still_read(cx: &mut TestAppContext) {
        let directory = directory("navigation-earlier");
        std::fs::create_dir_all(&directory).unwrap();
        let preferences = directory.join("preferences.json");
        std::fs::write(
            directory.join("navigation.json"),
            r#"{"collapsed":false,"inspector":{"nodes":540,"health":500},"drawer":{"resources":880},"dock":{"height":360,"open":true,"maximized":false,"selected":null,"tabs":[]}}"#,
        )
        .unwrap();
        let file = NavigationFile::open(Some(&preferences));
        cx.update(|cx| file.clone().install(cx));
        cx.update(|cx| {
            assert_eq!(
                SplitSize::new(width_key("nodes"), 460., 320., cx).size(),
                540.
            );
            assert_eq!(
                SplitSize::new(width_key("health"), 460., 320., cx).size(),
                500.
            );
            assert_eq!(SplitSize::new(DRAWER_WIDTH, 725., 300., cx).size(), 880.);
            assert_eq!(SplitSize::new(DOCK_HEIGHT, 300., 100., cx).size(), 360.);
        });

        // The dock saves its state without its height.
        #[derive(serde::Serialize)]
        struct State {
            open: bool,
            tabs: Vec<u8>,
        }
        cx.update(|cx| {
            file.set_dock(
                &State {
                    open: false,
                    tabs: vec![],
                },
                cx,
            )
        });
        cx.run_until_parked();
        let value = read(directory.join("navigation.json"));
        assert_eq!(value["dock"]["height"], serde_json::json!(360));
        assert_eq!(value["dock"]["open"], serde_json::json!(false));
        assert_eq!(value["inspector"]["nodes"], serde_json::json!(540));
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
        assert_eq!(file.size(width_key("incidents")), None);
        assert_eq!(file.size(width_key("traces")), None);
        cx.update(|cx| file.set_collapsed(true, cx));
        cx.run_until_parked();
        let value = read(directory.join("navigation.json"));
        assert_eq!(value["later"], serde_json::json!([1, 2]));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn without_preferences_nothing_is_kept() {
        let file = NavigationFile::open(None);
        assert_eq!(file.collapsed(), None);
        assert_eq!(file.size(width_key("incidents")), None);
    }
}
