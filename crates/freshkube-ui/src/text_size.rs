//! The text size the user chooses, in Settings or with Command-= and
//! Command-- (Command-0 restores the default), remembered between launches.
//!
//! Kit sizes its controls from the theme's font size, which `Root` makes the
//! window's rem size. The app's own layout is in `ui::dp` lengths, which are
//! rems too, so text, rows, padding and widths all scale together.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use freshkube_prefs::{Document, Layout};
use gpui_kit::component::Theme;
use gpui_kit::{App, Global, KeyBinding};

use crate::ui::BASE_TEXT;

/// The sizes offered, in pixels of the theme's base text.
pub const STEPS: [f32; 6] = [12., 13., 14., 16., 18., 20.];
/// Monospace text keeps its proportion to the base text, as the theme sets
/// them at the default size.
const MONO_RATIO: f32 = 12.5 / BASE_TEXT;

gpui_kit::actions!(text_size, [LargerText, SmallerText, DefaultText]);

struct TextSize {
    size: f32,
    /// Where the choice is saved; `None` keeps it for this session only.
    preferences: Option<PathBuf>,
    /// Whether the last save failed, so a folder that stays unwritable is
    /// logged once, not on every step.
    save_failed: Arc<AtomicBool>,
    /// The size last chosen, as `f32` bits. A save writes this, not the size
    /// it was spawned for, so whichever save takes the file's lock last
    /// still writes the newest choice.
    latest: Arc<AtomicU32>,
}

impl Global for TextSize {}

/// Applies the size saved in `preferences`, or the default, and binds the
/// shortcuts. Call it once, after `theme::install`.
pub fn install(preferences: Option<PathBuf>, cx: &mut App) {
    install_with(preferences, None, cx)
}

/// The step `FRESHKUBE_TEXT_SIZE` names: one of `STEPS`, else `None`.
pub fn parse(value: &str) -> Option<f32> {
    value
        .parse::<f32>()
        .ok()
        .filter(|size| STEPS.contains(size))
}

/// [`install`], starting at `start` instead of the saved size when given,
/// for visual checks: the choice is not saved. The caller parses it from
/// the debug knobs (`parse`); the filter here is defensive.
pub fn install_with(preferences: Option<PathBuf>, start: Option<f32>, cx: &mut App) {
    let size = start
        .filter(|size| STEPS.contains(size))
        .or_else(|| preferences.as_deref().and_then(saved))
        .unwrap_or(BASE_TEXT);
    cx.set_global(TextSize {
        size,
        preferences,
        save_failed: Arc::default(),
        latest: Arc::new(AtomicU32::new(size.to_bits())),
    });
    cx.bind_keys([
        // Command-Shift-= arrives as `+`. macOS's View menu shows the first
        // binding, so `⌘+` comes before `⌘=`.
        KeyBinding::new("secondary-+", LargerText, None),
        KeyBinding::new("secondary-=", LargerText, None),
        KeyBinding::new("secondary--", SmallerText, None),
        KeyBinding::new("secondary-0", DefaultText, None),
    ]);
    cx.on_action(|_: &LargerText, cx| step(1, cx));
    cx.on_action(|_: &SmallerText, cx| step(-1, cx));
    cx.on_action(|_: &DefaultText, cx| set(BASE_TEXT, cx));
    apply(cx);
}

/// The chosen size, in pixels of base text.
pub fn current(cx: &App) -> f32 {
    cx.try_global::<TextSize>()
        .map_or(BASE_TEXT, |text| text.size)
}

/// Chooses one of `STEPS`, applies it to every window and saves it.
pub fn set(size: f32, cx: &mut App) {
    if !STEPS.contains(&size) || size == current(cx) {
        return;
    }
    if !cx.has_global::<TextSize>() {
        return;
    }
    let text = cx.global_mut::<TextSize>();
    text.size = size;
    let preferences = text.preferences.clone();
    let save_failed = text.save_failed.clone();
    let latest = text.latest.clone();
    latest.store(size.to_bits(), Ordering::Relaxed);
    apply(cx);
    if let Some(path) = preferences {
        cx.background_executor()
            .spawn(async move {
                // A failed save only means the next launch starts at the
                // default; say so once.
                match save(&path, &latest) {
                    Ok(()) => save_failed.store(false, Ordering::Relaxed),
                    Err(error) => {
                        if !save_failed.swap(true, Ordering::Relaxed) {
                            tracing::warn!("{} not saved: {error}", path.display());
                        }
                    }
                }
            })
            .detach();
    }
}

fn step(by: isize, cx: &mut App) {
    let at = STEPS
        .iter()
        .position(|size| *size == current(cx))
        .unwrap_or(1);
    let next = at.saturating_add_signed(by).min(STEPS.len() - 1);
    set(STEPS[next], cx);
}

/// Writes the size into both registered themes and reloads the current one.
/// Kit reloads a theme's font sizes with it, so an appearance change keeps
/// the size.
fn apply(cx: &mut App) {
    let size = current(cx);
    let theme = Theme::global_mut(cx);
    for config in [&mut theme.light_theme, &mut theme.dark_theme] {
        let mut sized = (**config).clone();
        sized.font_size = Some(size);
        sized.mono_font_size = Some(size * MONO_RATIO);
        *config = Rc::new(sized);
    }
    let mode = theme.mode;
    Theme::change(mode, None, cx);
}

/// `preferences.json`, whose other keys belong to no one here: the size is
/// the only key this module sets, and a save keeps every other key. New
/// files are created `0o666` less the umask, as `std::fs::write` did. The
/// app's file is always `preferences.json`; the path comes from
/// `GpuiOptions.preferences`.
fn document(path: &Path) -> Document<serde_json::Map<String, serde_json::Value>> {
    Document::at(path, Layout::Pretty, 0o666)
}

/// The size saved in a preferences file, if it holds one of `STEPS`.
fn saved(path: &Path) -> Option<f32> {
    let loaded = document(path).load();
    if let Some(problem) = &loaded.problem {
        tracing::warn!("{problem}");
    }
    let size = loaded.value.get("text_size")?.as_f64()? as f32;
    STEPS.contains(&size).then_some(size)
}

/// Sets `text_size` in the preferences file to the size last chosen
/// (`latest`, read under the file's lock), keeping anything else in it.
fn save(path: &Path, latest: &AtomicU32) -> std::io::Result<()> {
    document(path).update(|map| {
        let size = f32::from_bits(latest.load(Ordering::Relaxed));
        map.insert("text_size".into(), serde_json::json!(size));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save_size(path: &Path, size: f32) -> std::io::Result<()> {
        save(path, &AtomicU32::new(size.to_bits()))
    }

    fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("freshkube-text-size-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_saved_size_reads_back_and_keeps_other_preferences() {
        let directory = directory("round-trip");
        let path = directory.join("nested").join("preferences.json");
        assert_eq!(saved(&path), None);
        save_size(&path, 18.).unwrap();
        assert_eq!(saved(&path), Some(18.));
        std::fs::write(&path, r#"{"other": true, "text_size": 16}"#).unwrap();
        save_size(&path, 20.).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["other"], true);
        assert_eq!(saved(&path), Some(20.));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[gpui_kit::test]
    fn a_start_beats_the_saved_size_and_is_not_saved(cx: &mut gpui_kit::TestAppContext) {
        let directory = directory("start");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        save_size(&path, 14.).unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            install_with(Some(path.clone()), Some(18.), cx);
            assert_eq!(current(cx), 18.);
            // A start outside `STEPS` falls back to the saved size.
            install_with(Some(path.clone()), Some(15.), cx);
            assert_eq!(current(cx), 14.);
            install_with(Some(path.clone()), None, cx);
            assert_eq!(current(cx), 14.);
        });
        assert_eq!(saved(&path), Some(14.));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn parse_keeps_one_of_the_steps() {
        assert_eq!(parse("16"), Some(16.));
        assert_eq!(parse("16.0"), Some(16.));
        for bad in ["15", "", "big", "-16"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_unreadable_or_unknown_size_is_ignored() {
        let directory = directory("unknown");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        for text in [
            "not json",
            "[]",
            r#"{"text_size": 15}"#,
            r#"{"text_size": "big"}"#,
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(saved(&path), None, "{text}");
        }
        // A file that isn't an object is replaced, not merged into.
        std::fs::write(&path, "[]").unwrap();
        save_size(&path, 12.).unwrap();
        assert_eq!(saved(&path), Some(12.));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_save_writes_the_example_file_byte_for_byte() {
        let scratch = tempfile::tempdir().unwrap();
        let directory = scratch.path().to_path_buf();
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        let example = include_str!("../../freshkube-prefs/src/examples/preferences.json");
        // From the older compact form, an unknown key and another size.
        std::fs::write(&path, r#"{"other":true,"text_size":12}"#).unwrap();
        save_size(&path, 16.).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), example);
        // A first save into a missing folder writes the same layout.
        let fresh = directory.join("nested").join("preferences.json");
        save_size(&fresh, 16.).unwrap();
        assert_eq!(
            std::fs::read_to_string(&fresh).unwrap(),
            "{\n  \"text_size\": 16.0\n}\n"
        );
    }

    #[test]
    fn a_save_keeps_every_other_key_unknown_ones_included() {
        let scratch = tempfile::tempdir().unwrap();
        let directory = scratch.path().to_path_buf();
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("preferences.json");
        std::fs::write(
            &path,
            "{\n  \"a_future_key\": {\"nested\": [1, \"two\", null]},\n  \"other\": true,\n  \"text_size\": 12.0\n}\n",
        )
        .unwrap();
        save_size(&path, 20.).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"a_future_key\": {\n    \"nested\": [\n      1,\n      \"two\",\n      null\n    ]\n  },\n  \"other\": true,\n  \"text_size\": 20.0\n}\n"
        );
    }

    #[test]
    fn a_missing_file_is_the_default_and_saves_leave_no_temporary_file() {
        let scratch = tempfile::tempdir().unwrap();
        let directory = scratch.path().to_path_buf();
        let path = directory.join("preferences.json");
        assert_eq!(saved(&path), None);
        save_size(&path, 14.).unwrap();
        let names: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["preferences.json"]);
    }

    #[test]
    fn a_save_that_runs_late_still_writes_the_newest_size() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("preferences.json");
        let latest = AtomicU32::new(16f32.to_bits());
        // 18 is chosen after a save for 16 was spawned; 18's save runs
        // first and the stale one for 16 runs last.
        latest.store(18f32.to_bits(), Ordering::Relaxed);
        save(&path, &latest).unwrap();
        save(&path, &latest).unwrap();
        assert_eq!(saved(&path), Some(18.));
    }
}
