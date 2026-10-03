//! Where dashboards come from besides the built-ins: a folder of the user's
//! Grafana JSON, read once and kept. Also the time settings a dashboard
//! carries that grafaui's model doesn't read: its quick ranges, its saved
//! range and its auto-refresh.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use super::model::{Dashboard, time};

/// Files read from one folder at most; more are listed as skipped.
pub const MAX_FILES: usize = 200;
/// The largest dashboard file read.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Ranges offered when a dashboard names none: 5 minutes to 30 days.
pub const DEFAULT_RANGES: [u64; 11] = [
    300,
    900,
    1800,
    3600,
    3 * 3600,
    6 * 3600,
    12 * 3600,
    86_400,
    2 * 86_400,
    7 * 86_400,
    30 * 86_400,
];

/// The auto-refresh choices, in seconds; `None` is off.
pub const REFRESH_CHOICES: [Option<u64>; 4] = [None, Some(30), Some(60), Some(300)];

/// A dashboard's time settings, as its picker and refresh start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeSettings {
    /// Spans the picker offers, shortest first; always holds `start`.
    pub ranges: Vec<u64>,
    /// The span the dashboard was saved with, or six hours.
    pub start: u64,
    /// The saved auto-refresh when it is one of [`REFRESH_CHOICES`].
    pub refresh: Option<u64>,
}

/// Reads the time settings of a dashboard export. Only ranges relative to
/// now are offered: an absolute range or one ending before now falls back.
pub fn time_settings(json: &str) -> TimeSettings {
    let value: Value = serde_json::from_str(json).unwrap_or_default();
    let value = value
        .get("dashboard")
        .filter(|inner| inner.is_object())
        .unwrap_or(&value);
    let relative = |from: Option<&str>, to: Option<&str>| {
        (to.is_none_or(|to| to.trim() == "now"))
            .then(|| from.and_then(time::parse_relative))
            .flatten()
    };
    let picker = value.get("timepicker");
    let mut ranges: Vec<u64> = picker
        .and_then(|picker| picker.get("quick_ranges"))
        .and_then(Value::as_array)
        .map(|ranges| {
            ranges
                .iter()
                .filter_map(|range| {
                    relative(
                        range.get("from").and_then(Value::as_str),
                        range.get("to").and_then(Value::as_str),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if ranges.is_empty() {
        // Older exports list spans such as "5m" and "1h".
        ranges = picker
            .and_then(|picker| picker.get("time_options"))
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(|span| time::parse_relative(&format!("now-{span}")))
                    .collect()
            })
            .unwrap_or_default();
    }
    if ranges.is_empty() {
        ranges = DEFAULT_RANGES.to_vec();
    }
    let saved = value.get("time");
    let start = relative(
        saved
            .and_then(|time| time.get("from"))
            .and_then(Value::as_str),
        saved
            .and_then(|time| time.get("to"))
            .and_then(Value::as_str),
    )
    .unwrap_or(6 * 3600);
    ranges.push(start);
    ranges.sort_unstable();
    ranges.dedup();
    let refresh = value
        .get("refresh")
        .and_then(Value::as_str)
        .and_then(|text| time::parse_relative(&format!("now-{text}")))
        .filter(|seconds| REFRESH_CHOICES.contains(&Some(*seconds)));
    TimeSettings {
        ranges,
        start,
        refresh,
    }
}

/// A short label for an auto-refresh choice: "Off", "30s", "1m", "5m".
pub fn refresh_label(every: Option<u64>) -> String {
    match every {
        None => "Off".into(),
        Some(seconds) if seconds % 60 == 0 => format!("{}m", seconds / 60),
        Some(seconds) => format!("{seconds}s"),
    }
}

/// One file of the user's folder: a dashboard to open, or why it can't be.
#[derive(Clone, Debug)]
pub struct FolderDashboard {
    pub path: PathBuf,
    /// The dashboard's title, or the file's name when it doesn't read.
    pub title: String,
    pub outcome: Result<Arc<str>, String>,
}

/// What a folder held.
#[derive(Clone, Debug, Default)]
pub struct Folder {
    /// Dashboards by title, then file name.
    pub dashboards: Vec<FolderDashboard>,
    /// JSON files past [`MAX_FILES`], not read.
    pub skipped: usize,
}

/// Reads every `.json` file directly in `path` (not its subfolders) as a
/// dashboard. A file that isn't one is listed with the reason, so the
/// column never hides it.
pub fn read_folder(path: &Path) -> Result<Folder, String> {
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("Couldn't read {}: {error}", path.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                && path.is_file()
        })
        .collect();
    files.sort();
    let skipped = files.len().saturating_sub(MAX_FILES);
    files.truncate(MAX_FILES);
    let mut dashboards: Vec<FolderDashboard> = files.into_iter().map(read_file).collect();
    dashboards.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(Folder {
        dashboards,
        skipped,
    })
}

fn read_file(path: PathBuf) -> FolderDashboard {
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let outcome = (|| {
        let size = std::fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len();
        if size > MAX_FILE_BYTES {
            return Err(format!("Larger than {} MiB", MAX_FILE_BYTES / 1024 / 1024));
        }
        let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let dashboard = Dashboard::parse_unexpanded(&text)
            .map_err(|error| format!("Not a dashboard: {error}"))?;
        Ok((dashboard.title, Arc::<str>::from(text)))
    })();
    match outcome {
        Ok((title, json)) => FolderDashboard {
            path,
            title,
            outcome: Ok(json),
        },
        Err(error) => FolderDashboard {
            path,
            title: name,
            outcome: Err(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_settings_read_quick_ranges_the_saved_range_and_refresh() {
        let json = r#"{"time":{"from":"now-3h","to":"now"},"refresh":"1m",
            "timepicker":{"quick_ranges":[
                {"display":"Last 2 days","from":"now-2d","to":"now"},
                {"display":"Yesterday","from":"now-1d/d","to":"now-1d/d"},
                {"display":"Last 30 minutes","from":"now-30m","to":"now"}]}}"#;
        let settings = time_settings(json);
        assert_eq!(settings.ranges, vec![1800, 3 * 3600, 2 * 86_400]);
        assert_eq!(settings.start, 3 * 3600);
        assert_eq!(settings.refresh, Some(60));
    }

    #[test]
    fn time_settings_fall_back_on_old_options_defaults_and_absolute_ranges() {
        let old = r#"{"timepicker":{"time_options":["5m","1h","7d"]},"refresh":"10s"}"#;
        let settings = time_settings(old);
        assert_eq!(settings.ranges, vec![300, 3600, 6 * 3600, 7 * 86_400]);
        assert_eq!(settings.refresh, None);
        let absolute =
            r#"{"dashboard":{"time":{"from":"2026-01-01T00:00:00Z","to":"2026-01-02T00:00:00Z"}}}"#;
        let settings = time_settings(absolute);
        assert_eq!(settings.start, 6 * 3600);
        assert_eq!(settings.ranges, DEFAULT_RANGES.to_vec());
        assert_eq!(time_settings("not json").start, 6 * 3600);
    }

    #[test]
    fn refresh_labels() {
        let labels: Vec<String> = REFRESH_CHOICES.into_iter().map(refresh_label).collect();
        assert_eq!(labels, ["Off", "30s", "1m", "5m"]);
    }

    #[test]
    fn a_folder_lists_dashboards_by_title_and_explains_the_rest() {
        let directory = std::env::temp_dir().join(format!(
            "freshkube-dashboards-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("nested")).unwrap();
        let write = |name: &str, text: &str| std::fs::write(directory.join(name), text).unwrap();
        write("b.json", r#"{"title":"Zeta","panels":[]}"#);
        write("a.json", r#"{"title":"alpha","panels":[]}"#);
        write("broken.json", "{");
        write("notes.txt", "not a dashboard");
        std::fs::write(directory.join("nested/c.json"), r#"{"title":"Nested"}"#).unwrap();
        let folder = read_folder(&directory).unwrap();
        let titles: Vec<&str> = folder
            .dashboards
            .iter()
            .map(|dashboard| dashboard.title.as_str())
            .collect();
        assert_eq!(titles, ["alpha", "broken", "Zeta"]);
        assert!(folder.dashboards[0].outcome.is_ok());
        assert!(
            folder.dashboards[1]
                .outcome
                .as_ref()
                .unwrap_err()
                .starts_with("Not a dashboard")
        );
        assert_eq!(folder.skipped, 0);
        assert!(read_folder(&directory.join("missing")).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
