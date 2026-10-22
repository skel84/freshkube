//! `follows.json` beside the preferences: the follows, the notice settings
//! and the next id. Follows are the user's preference, so they live here and
//! never in freshkube.yaml, which a shared setup file can carry.
//!
//! A missing or unreadable file is no follows and the default settings; the
//! load says why once for the caller to log.

use std::path::Path;

use freshkube_prefs::{Dir, Document, Layout};
use serde::{Deserialize, Serialize};

use crate::model::{Follow, MAX_FOLLOWS};
use crate::notice::Settings;

const NAME: &str = "follows.json";
/// The file is read to at most this many bytes.
const LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub follows: Vec<Follow>,
    pub next_id: u64,
    pub settings: Settings,
}

/// The document beside `preferences`.
pub fn document(preferences: &Path) -> Document<Saved> {
    Dir::beside(preferences)
        .document(NAME, Layout::Pretty, 0o600)
        .limited_to(LIMIT)
}

/// What the file holds, never more than [`MAX_FOLLOWS`] follows, with ids
/// that no later follow reuses.
pub fn load(document: &Document<Saved>) -> Saved {
    let loaded = document.load();
    if let Some(problem) = &loaded.problem {
        tracing::warn!("{problem}; following nothing");
    }
    let mut saved = loaded.value;
    saved.follows.truncate(MAX_FOLLOWS);
    let highest = saved.follows.iter().map(|f| f.id).max().unwrap_or(0);
    saved.next_id = saved.next_id.max(highest + 1);
    saved
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::model::Subject;
    use crate::notice::Quiet;

    fn follow(id: u64) -> Follow {
        Follow {
            id,
            subject: Subject::CrashLoop {
                namespace: "shop".into(),
                pod: format!("api-{id}"),
            },
            context: "dev-fra".into(),
            sensitive: true,
            since: Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap(),
        }
    }

    #[test]
    fn follows_and_settings_round_trip_through_the_file() {
        let directory =
            std::env::temp_dir().join(format!("freshkube-follows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let document = document(&directory.join("preferences.json"));
        let saved = Saved {
            follows: vec![follow(1), follow(2)],
            next_id: 3,
            settings: Settings {
                grouping: false,
                quiet: Quiet {
                    enabled: true,
                    from: 60,
                    until: 120,
                },
                hide_details: Some(true),
            },
        };
        document.save(&saved).expect("saved");
        assert_eq!(load(&document), saved);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_file_follows_nothing() {
        let directory =
            std::env::temp_dir().join(format!("freshkube-follows-none-{}", std::process::id()));
        let document = document(&directory.join("preferences.json"));
        let saved = load(&document);
        assert!(saved.follows.is_empty());
        assert_eq!(saved.next_id, 1);
        assert_eq!(saved.settings, Settings::default());
    }

    #[test]
    fn a_file_with_more_than_the_cap_keeps_the_first_twenty() {
        let directory =
            std::env::temp_dir().join(format!("freshkube-follows-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let document = document(&directory.join("preferences.json"));
        let saved = Saved {
            follows: (1..=25).map(follow).collect(),
            ..Saved::default()
        };
        document.save(&saved).expect("saved");
        let loaded = load(&document);
        assert_eq!(loaded.follows.len(), MAX_FOLLOWS);
        assert_eq!(loaded.next_id, 21);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_unreadable_file_follows_nothing() {
        let directory =
            std::env::temp_dir().join(format!("freshkube-follows-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let document = document(&directory.join("preferences.json"));
        std::fs::write(document.path(), "{ not json").unwrap();
        assert!(load(&document).follows.is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
