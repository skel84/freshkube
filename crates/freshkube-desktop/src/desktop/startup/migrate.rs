//! The one-time move of `workspace.json` and `coroot.json` into
//! `freshkube.yaml` (`freshkube_core::setup::migrate`), run once at startup,
//! before anything reads the workspace.
use crate::GpuiOptions;
use freshkube_core::setup::{self, Outcome};

/// Runs the migration beside the preferences and returns the status line to
/// show once. Example and maintenance launches touch no file, and neither
/// does a window that keeps no preferences.
pub(in crate::desktop) fn migrate_setup(options: &GpuiOptions) -> Option<String> {
    if options.starts_with_examples() || options.maintenance_endpoint().is_some() {
        return None;
    }
    let outcome = setup::migrate(options.preferences.as_deref()?, chrono::Utc::now());
    if let Outcome::Kept(why) = &outcome {
        tracing::info!("workspace.json and coroot.json stay in use: {why:?}");
    }
    outcome.notice()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACE: &str = r#"{"version": 1, "clusters": [{"id": "cluster-a", "role": "core", "context": "cluster-a"}]}"#;

    fn home() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("workspace.json"), WORKSPACE).unwrap();
        let preferences = directory.path().join("preferences.json");
        (directory, preferences)
    }

    #[test]
    fn a_launch_moves_the_workspace_and_says_so_once() {
        let (directory, preferences) = home();
        let options = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences));
        assert_eq!(
            migrate_setup(&options).as_deref(),
            Some("Moved workspace.json into freshkube.yaml; the old file is .bak")
        );
        assert!(directory.path().join("freshkube.yaml").is_file());
        assert!(directory.path().join("workspace.json.bak").is_file());
        assert_eq!(
            migrate_setup(&options),
            None,
            "the next launch has nothing to say"
        );
    }

    #[test]
    fn example_mode_touches_no_file() {
        let (directory, preferences) = home();
        let options = GpuiOptions::fixture().with_preferences(Some(preferences));
        assert_eq!(migrate_setup(&options), None);
        assert!(directory.path().join("workspace.json").is_file());
        assert!(!directory.path().join("freshkube.yaml").exists());
        assert!(!directory.path().join("workspace.json.bak").exists());
    }

    #[test]
    fn a_window_without_preferences_touches_nothing() {
        assert_eq!(migrate_setup(&GpuiOptions::new(None, None, 100)), None);
    }

    #[test]
    fn a_workspace_file_that_is_not_used_stays_where_it_is() {
        let (directory, preferences) = home();
        std::fs::write(directory.path().join("workspace.json"), "not json").unwrap();
        let options = GpuiOptions::new(None, None, 100).with_preferences(Some(preferences));
        assert_eq!(migrate_setup(&options), None);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("workspace.json")).unwrap(),
            "not json"
        );
        assert!(!directory.path().join("freshkube.yaml").exists());
    }
}
