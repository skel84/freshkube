use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Sample {
    name: String,
    count: u32,
}

fn sample(directory: &tempfile::TempDir) -> Document<Sample> {
    Dir::beside(&directory.path().join("preferences.json")).document("sample.json", Layout::Compact)
}

#[test]
fn a_saved_value_reads_back() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    let value = Sample {
        name: "shop-api".into(),
        count: 3,
    };
    document.save(&value).unwrap();
    let loaded = document.load();
    assert_eq!(loaded.value, value);
    assert!(loaded.problem.is_none());
    assert_eq!(
        std::fs::read_to_string(document.path()).unwrap(),
        "{\"name\":\"shop-api\",\"count\":3}\n"
    );
}

#[test]
fn a_save_creates_the_directory_and_leaves_no_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("nested").join("preferences.json");
    let document = Dir::beside(&nested).document::<Sample>("sample.json", Layout::Compact);
    document.save(&Sample::default()).unwrap();
    let names: Vec<_> = std::fs::read_dir(nested.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["sample.json"]);
}

#[test]
fn a_missing_file_is_the_default_without_a_problem() {
    let directory = tempfile::tempdir().unwrap();
    let loaded = sample(&directory).load();
    assert_eq!(loaded.value, Sample::default());
    assert!(loaded.problem.is_none());
}

#[test]
fn a_corrupt_file_is_the_default_and_reported() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{\"name\": ").unwrap();
    let loaded = document.load();
    assert_eq!(loaded.value, Sample::default());
    let problem = loaded.problem.expect("a problem");
    assert!(problem.contains("sample.json"), "{problem}");
    assert!(problem.contains("ends early"), "{problem}");
}

#[test]
fn a_problem_never_quotes_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    for text in [
        "\"/home/acme/.kube/shop-api-config\"",
        "{\"name\": \"shop-api\", \"count\": \"/home/acme/secret\"}",
        "{\"name\": 42}",
        "shop-api",
    ] {
        std::fs::write(document.path(), text).unwrap();
        let problem = document.load().problem.expect("a problem");
        for word in ["acme", "shop-api", "secret"] {
            assert!(!problem.contains(word.trim()), "{problem} quotes {word}");
        }
    }
}

#[test]
fn a_file_of_the_wrong_shape_is_reported() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "[1, 2]").unwrap();
    assert!(document.load().problem.is_some());
}

// Unix only: on Windows a directory's read-only attribute doesn't stop
// creating files in it.
#[cfg(unix)]
#[test]
fn a_save_into_a_read_only_directory_keeps_the_old_file() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    let old = Sample {
        name: "acme".into(),
        count: 1,
    };
    document.save(&old).unwrap();
    let before = std::fs::read(document.path()).unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    // A process that ignores directory permissions (root) can still write;
    // there is nothing to prove there.
    if std::fs::File::create(directory.path().join("probe")).is_ok() {
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: read-only directories do not stop this process");
        return;
    }
    let result = document.save(&Sample {
        name: "new".into(),
        count: 2,
    });
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(document.path()).unwrap(), before);
    assert_eq!(document.load().value, old);
}

/// The app's own files, as the example files committed here show them: what
/// loads from an example saves back byte for byte.
fn round_trip(name: &str, layout: Layout, example: &str) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join(name), example).unwrap();
    let document = Dir::beside(&directory.path().join("preferences.json"))
        .document::<Map<String, Value>>(name, layout);
    let loaded = document.load();
    assert!(loaded.problem.is_none(), "{:?}", loaded.problem);
    assert!(!loaded.value.is_empty());
    document.save(&loaded.value).unwrap();
    assert_eq!(
        std::fs::read_to_string(document.path()).unwrap(),
        example,
        "{name} changed"
    );
}

#[test]
fn preferences_json_round_trips_byte_for_byte() {
    round_trip(
        "preferences.json",
        Layout::Pretty,
        include_str!("examples/preferences.json"),
    );
}

#[test]
fn navigation_json_round_trips_byte_for_byte() {
    round_trip(
        "navigation.json",
        Layout::Compact,
        include_str!("examples/navigation.json"),
    );
}

#[test]
fn a_failed_rename_leaves_the_target_and_no_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    // A non-empty directory where the file should go: the rename can't
    // replace it.
    std::fs::create_dir(document.path()).unwrap();
    std::fs::write(document.path().join("keep"), "kept").unwrap();
    assert!(document.save(&Sample::default()).is_err());
    assert_eq!(
        std::fs::read_to_string(document.path().join("keep")).unwrap(),
        "kept"
    );
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["sample.json"]);
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// What the process's umask leaves of `wanted`.
    fn after_umask(directory: &std::path::Path, wanted: u32) -> u32 {
        let probe = directory.join("umask-probe");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o777)
            .open(&probe)
            .unwrap();
        let allowed = mode(&probe);
        std::fs::remove_file(probe).unwrap();
        wanted & allowed
    }

    #[test]
    fn a_new_file_takes_the_mode_its_document_names() {
        let directory = tempfile::tempdir().unwrap();
        let plain = sample(&directory);
        plain.save(&Sample::default()).unwrap();
        assert_eq!(mode(plain.path()), after_umask(directory.path(), 0o644));
        let private = Dir::beside(&directory.path().join("preferences.json"))
            .document::<Sample>("private.json", Layout::Compact)
            .new_file_mode(0o600);
        private.save(&Sample::default()).unwrap();
        assert_eq!(mode(private.path()), 0o600);
    }

    #[test]
    fn a_file_that_exists_keeps_its_mode() {
        let directory = tempfile::tempdir().unwrap();
        let document = sample(&directory);
        document.save(&Sample::default()).unwrap();
        std::fs::set_permissions(document.path(), std::fs::Permissions::from_mode(0o640)).unwrap();
        document
            .save(&Sample {
                name: "acme".into(),
                count: 1,
            })
            .unwrap();
        assert_eq!(mode(document.path()), 0o640);
    }

    #[test]
    fn a_symlinked_file_is_written_through_and_stays_a_link() {
        let directory = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let real = elsewhere.path().join("dotfiles.json");
        std::fs::write(&real, "{\"name\":\"old\",\"count\":0}\n").unwrap();
        let document = sample(&directory);
        std::os::unix::fs::symlink(&real, document.path()).unwrap();
        document
            .save(&Sample {
                name: "shop-api".into(),
                count: 2,
            })
            .unwrap();
        assert!(
            std::fs::symlink_metadata(document.path())
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "{\"name\":\"shop-api\",\"count\":2}\n"
        );
        let in_link_directory: Vec<_> = std::fs::read_dir(directory.path()).unwrap().collect();
        assert_eq!(in_link_directory.len(), 1);
        let in_target_directory: Vec<_> = std::fs::read_dir(elsewhere.path()).unwrap().collect();
        assert_eq!(in_target_directory.len(), 1);
    }
}
