use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Sample {
    name: String,
    count: u32,
}

fn sample(directory: &tempfile::TempDir) -> Document<Sample> {
    Dir::beside(&directory.path().join("preferences.json")).document(
        "sample.json",
        Layout::Compact,
        0o644,
    )
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
    let document = Dir::beside(&nested).document::<Sample>("sample.json", Layout::Compact, 0o644);
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
fn a_corrupt_file_is_the_default_with_a_problem() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{\"name\": ").unwrap();
    let loaded = document.load();
    assert_eq!(loaded.value, Sample::default());
    let problem = loaded.problem.expect("a problem");
    assert!(problem.contains("sample.json"), "{problem}");
    assert!(problem.contains("could not be parsed"), "{problem}");
}

#[test]
fn a_file_of_the_wrong_shape_is_reported() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "[1, 2]").unwrap();
    assert!(document.load().problem.is_some());
}

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
    // `cfg(unix)`: on Windows a directory's read-only attribute doesn't stop
    // files being created in it.
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
        .document::<Map<String, Value>>(name, layout, 0o644);
    let loaded = document.load();
    assert!(loaded.problem.is_none(), "{:?}", loaded.problem);
    assert!(!loaded.value.is_empty());
    document.save(&loaded.value).unwrap();
    // Key order follows serde_json's `preserve_order`, which another crate
    // in the build may turn on, so this compares the values and the size
    // and trailing newline; desktop's test of `navigation.json` compares the
    // bytes the app writes.
    let written = std::fs::read_to_string(document.path()).unwrap();
    let parsed = |text: &str| serde_json::from_str::<Value>(text).unwrap();
    assert_eq!(parsed(&written), parsed(example), "{name} changed");
    assert_eq!(written.len(), example.len(), "{name} changed layout");
    assert!(written.ends_with("}\n"), "{name} lost its newline");
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
fn a_problem_never_carries_the_files_contents() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    // serde_json's own message would say `invalid type: string "…"`.
    std::fs::write(document.path(), "\"/home/someone/kube/config-acme\"").unwrap();
    let problem = document.load().problem.expect("a problem");
    assert!(!problem.contains("config-acme"), "{problem}");
    assert!(!problem.contains("someone"), "{problem}");
    assert!(problem.contains("wrong type or shape"), "{problem}");
    assert!(problem.contains("line 1, column"), "{problem}");
    // A wrong field type in an object, too.
    std::fs::write(
        document.path(),
        "{\"name\": \"a\", \"count\": \"ten-acme\"}",
    )
    .unwrap();
    let problem = document.load().problem.expect("a problem");
    assert!(!problem.contains("ten-acme"), "{problem}");
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

/// The mode a plain file created with `mode` gets here: the umask applied.
#[cfg(unix)]
fn created_with(mode: u32) -> u32 {
    use std::os::unix::fs::OpenOptionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&path)
        .unwrap();
    mode_of(&path)
}

#[cfg(unix)]
#[test]
fn a_new_file_takes_the_documents_mode_less_the_umask() {
    for mode in [0o666, 0o644, 0o600] {
        let directory = tempfile::tempdir().unwrap();
        let document = Dir::beside(&directory.path().join("preferences.json")).document::<Sample>(
            "sample.json",
            Layout::Compact,
            mode,
        );
        document.save(&Sample::default()).unwrap();
        assert_eq!(mode_of(document.path()), created_with(mode), "{mode:o}");
    }
}

#[cfg(unix)]
#[test]
fn an_existing_file_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    document.save(&Sample::default()).unwrap();
    std::fs::set_permissions(document.path(), std::fs::Permissions::from_mode(0o640)).unwrap();
    document
        .save(&Sample {
            name: "changed".into(),
            count: 1,
        })
        .unwrap();
    assert_eq!(mode_of(document.path()), 0o640);
    assert_eq!(document.load().value.name, "changed");
}

#[cfg(unix)]
#[test]
fn a_symlinked_file_stays_a_link_and_its_target_is_rewritten() {
    let directory = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    let target = elsewhere.path().join("kept-elsewhere.json");
    std::fs::write(&target, "{\"name\":\"old\",\"count\":0}\n").unwrap();
    std::os::unix::fs::symlink(&target, document.path()).unwrap();
    document
        .save(&Sample {
            name: "new".into(),
            count: 5,
        })
        .unwrap();
    assert!(
        std::fs::symlink_metadata(document.path())
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "{\"name\":\"new\",\"count\":5}\n"
    );
    // No temporary file beside the link or its target.
    for dir in [directory.path(), elsewhere.path()] {
        let names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_to_a_missing_file_is_written_where_it_points() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    let target = directory.path().join("later.json");
    std::os::unix::fs::symlink("later.json", document.path()).unwrap();
    document.save(&Sample::default()).unwrap();
    assert!(target.is_file());
    assert!(
        std::fs::symlink_metadata(document.path())
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_failed_rename_leaves_no_temporary_file_and_the_target_alone() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    // A non-empty directory where the file should go: the rename fails.
    std::fs::create_dir(document.path()).unwrap();
    std::fs::write(document.path().join("inside"), "kept").unwrap();
    assert!(document.save(&Sample::default()).is_err());
    assert_eq!(
        std::fs::read_to_string(document.path().join("inside")).unwrap(),
        "kept"
    );
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["sample.json"]);
}

fn shared(directory: &tempfile::TempDir) -> Document<Map<String, Value>> {
    Dir::beside(&directory.path().join("preferences.json")).document(
        "shared.json",
        Layout::Pretty,
        0o666,
    )
}

#[test]
fn an_update_changes_its_keys_and_keeps_the_rest() {
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory);
    std::fs::write(
        document.path(),
        "{\n  \"future\": {\"kept\": [1, 2]},\n  \"other\": true,\n  \"text_size\": 16.0\n}\n",
    )
    .unwrap();
    document
        .update(|map| {
            map.insert("text_size".into(), serde_json::json!(20.0f32));
        })
        .unwrap();
    let value = document.load().value;
    assert_eq!(value["text_size"], 20.0);
    assert_eq!(value["other"], true);
    assert_eq!(value["future"]["kept"], serde_json::json!([1, 2]));
}

#[test]
fn an_update_of_a_missing_or_unusable_file_starts_from_the_default() {
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory);
    for before in [None, Some("not json"), Some("[]")] {
        if let Some(text) = before {
            std::fs::write(document.path(), text).unwrap();
        }
        document
            .update(|map| {
                map.insert("text_size".into(), serde_json::json!(18.0f32));
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(document.path()).unwrap(),
            "{\n  \"text_size\": 18.0\n}\n"
        );
        std::fs::remove_file(document.path()).unwrap();
    }
}

#[test]
fn updates_from_several_threads_lose_none() {
    let directory = tempfile::tempdir().unwrap();
    let document = std::sync::Arc::new(shared(&directory));
    // Behind a barrier, so the updates overlap: without the lock, some of
    // them would load the same file and lose each other's key.
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|n| {
            let document = document.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                document
                    .update(|map| {
                        map.insert(format!("key{n}"), n.into());
                    })
                    .unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(document.load().value.len(), 8);
}

#[test]
fn an_update_of_a_file_that_cannot_be_read_returns_the_error_and_saves_nothing() {
    // A directory where the file should be: reading it fails with something
    // other than "not found", on every platform and for any user.
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory);
    std::fs::create_dir(document.path()).unwrap();
    let mut called = false;
    let result = document.update(|_| called = true);
    assert!(result.is_err());
    assert!(!called, "change must not run on a file that wasn't read");
    assert!(document.path().is_dir());
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["shared.json"]);
}

#[cfg(unix)]
#[test]
fn an_update_of_a_file_without_read_permission_keeps_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory);
    std::fs::write(document.path(), "{\"keep\": 1}\n").unwrap();
    std::fs::set_permissions(document.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
    // A process that ignores file permissions (root) can still read it.
    if std::fs::read(document.path()).is_ok() {
        std::fs::set_permissions(document.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        eprintln!("skipped: unreadable files do not stop this process");
        return;
    }
    let result = document.update(|map| {
        map.insert("text_size".into(), serde_json::json!(18.0f32));
    });
    std::fs::set_permissions(document.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(document.path()).unwrap(),
        "{\"keep\": 1}\n"
    );
}

#[test]
fn a_load_of_a_file_that_cannot_be_read_is_the_default_with_a_problem() {
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory);
    std::fs::create_dir(document.path()).unwrap();
    let loaded = document.load();
    assert!(loaded.value.is_empty());
    assert!(loaded.problem.unwrap().contains("could not be read"));
}
