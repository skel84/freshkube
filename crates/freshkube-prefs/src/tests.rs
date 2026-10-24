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
    let text = std::fs::read_to_string(document.path()).unwrap();
    assert!(text.ends_with("}\n") && !text[..text.len() - 1].contains('\n'));
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        serde_json::json!({"name": "shop-api", "count": 3, "version": 1})
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
        serde_json::from_str::<Value>(&std::fs::read_to_string(&target).unwrap()).unwrap(),
        serde_json::json!({"name": "new", "count": 5, "version": 1})
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
            serde_json::from_str::<Value>(&std::fs::read_to_string(document.path()).unwrap())
                .unwrap(),
            serde_json::json!({"text_size": 18.0, "version": 1})
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

#[test]
fn a_limited_document_refuses_a_large_file_and_a_directory() {
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory).limited_to(16);
    std::fs::write(document.path(), "{\"a\": \"0123456789abcdef\"}").unwrap();
    let loaded = document.load();
    assert!(loaded.value.is_empty());
    assert!(loaded.problem.unwrap().contains("larger than 16 bytes"));
    std::fs::write(document.path(), "{\"a\": 1}").unwrap();
    assert_eq!(document.load().value["a"], 1);
    std::fs::remove_file(document.path()).unwrap();
    assert!(document.load().problem.is_none());
    std::fs::create_dir(document.path()).unwrap();
    assert!(document.load().problem.unwrap().contains("regular file"));
}

#[test]
fn an_update_of_a_file_a_limited_document_refuses_returns_the_error_and_saves_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory).limited_to(16);
    let big = "{\"keep\": \"0123456789abcdef\"}\n";
    std::fs::write(document.path(), big).unwrap();
    let mut called = false;
    let result = document.update(|_| called = true);
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert!(!called, "change must not run on a refused file");
    assert_eq!(std::fs::read_to_string(document.path()).unwrap(), big);

    std::fs::remove_file(document.path()).unwrap();
    std::fs::create_dir(document.path()).unwrap();
    assert!(document.update(|_| called = true).is_err());
    assert!(!called);
    assert!(document.path().is_dir());
}

/// A value that fails to serialize, to fail a save before it writes anything,
/// as a permission error does when the process is allowed to ignore them.
struct Unserializable;

impl Serialize for Unserializable {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("cannot be written"))
    }
}

impl<'de> Deserialize<'de> for Unserializable {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Ok(Self)
    }
}

impl Default for Unserializable {
    fn default() -> Self {
        Self
    }
}

#[test]
fn a_save_that_fails_before_writing_keeps_the_old_file_and_leaves_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let old = Sample {
        name: "acme".into(),
        count: 1,
    };
    sample(&directory).save(&old).unwrap();
    let before = std::fs::read(sample(&directory).path()).unwrap();
    let failing = Dir::beside(&directory.path().join("preferences.json"))
        .document::<Unserializable>("sample.json", Layout::Compact, 0o644);
    assert!(failing.save(&Unserializable).is_err());
    assert_eq!(std::fs::read(failing.path()).unwrap(), before);
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["sample.json"]);
}

fn copies(directory: &tempfile::TempDir) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("sample.corrupt-"))
        .collect();
    names.sort();
    names
}

#[test]
fn a_corrupt_file_is_kept_and_then_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{\"name\": \"shop-api\", oops").unwrap();
    let loaded = document.load();
    assert_eq!(loaded.value, Sample::default());
    let kept = copies(&directory);
    assert_eq!(kept.len(), 1, "{kept:?}");
    let problem = loaded.problem.expect("a problem");
    assert!(problem.contains(&kept[0]), "{problem}");
    assert_eq!(
        std::fs::read_to_string(directory.path().join(&kept[0])).unwrap(),
        "{\"name\": \"shop-api\", oops"
    );
    // The original is still there until something saves.
    assert!(
        std::fs::read_to_string(document.path())
            .unwrap()
            .contains("oops")
    );
    document.update(|sample| sample.count = 7).unwrap();
    assert_eq!(document.load().value.count, 7);
    assert_eq!(copies(&directory), kept, "no second copy of the same bytes");
}

#[test]
fn a_save_over_a_corrupt_file_keeps_it_first() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "not json at all").unwrap();
    // No load first: the save itself must not lose the file.
    document.save(&Sample::default()).unwrap();
    let kept = copies(&directory);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        std::fs::read_to_string(directory.path().join(&kept[0])).unwrap(),
        "not json at all"
    );
    assert!(document.load().problem.is_none());
}

#[test]
fn a_truncated_file_is_corrupt_and_kept() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    document
        .save(&Sample {
            name: "shop-api".into(),
            count: 3,
        })
        .unwrap();
    let whole = std::fs::read(document.path()).unwrap();
    std::fs::write(document.path(), &whole[..whole.len() / 2]).unwrap();
    let loaded = document.load();
    assert_eq!(loaded.value, Sample::default());
    assert!(loaded.problem.unwrap().contains("ends too early"));
    let kept = copies(&directory);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        std::fs::read(directory.path().join(&kept[0])).unwrap(),
        &whole[..whole.len() / 2]
    );
}

#[test]
fn only_the_newest_three_copies_are_kept_and_none_is_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    for n in 0..5 {
        std::fs::write(document.path(), format!("broken {n}")).unwrap();
        document.load();
    }
    let kept = copies(&directory);
    assert_eq!(kept.len(), KEEP, "{kept:?}");
    let contents: Vec<String> = kept
        .iter()
        .map(|name| std::fs::read_to_string(directory.path().join(name)).unwrap())
        .collect();
    assert_eq!(contents, ["broken 2", "broken 3", "broken 4"]);
}

#[test]
fn a_copy_is_named_for_its_utc_time_and_does_not_replace_one() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("sample.json");
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let first = keep::copy_at(&file, b"one", at).unwrap();
    let second = keep::copy_at(&file, b"two", at).unwrap();
    assert_eq!(
        first.file_name().unwrap(),
        "sample.corrupt-20231114T221320Z.json"
    );
    assert_eq!(
        second.file_name().unwrap(),
        "sample.corrupt-20231114T221320Z.1.json"
    );
    assert_eq!(std::fs::read(&first).unwrap(), b"one");
    assert_eq!(std::fs::read(&second).unwrap(), b"two");
}

#[test]
fn a_notice_names_the_file_and_the_copy_once() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{").unwrap();
    document.load();
    document.load();
    let mine: Vec<_> = take_notices()
        .into_iter()
        .filter(|notice| notice.file == document.path())
        .collect();
    assert_eq!(mine.len(), 1, "{mine:?}");
    let line = mine[0].to_string();
    assert!(line.starts_with("sample.json could not be read"), "{line}");
    assert!(line.contains("sample.corrupt-"), "{line}");
    assert!(
        !line.contains(&*directory.path().to_string_lossy()),
        "{line}"
    );
}

#[test]
fn a_file_without_a_version_loads_and_a_save_writes_one() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{\"name\":\"a\",\"count\":2}").unwrap();
    let loaded = document.load();
    assert!(loaded.problem.is_none());
    assert_eq!(loaded.value.count, 2);
    document.save(&loaded.value).unwrap();
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(document.path()).unwrap()).unwrap();
    assert_eq!(written["version"], VERSION);
    assert!(copies(&directory).is_empty());
}

#[test]
fn the_version_key_never_reaches_the_value() {
    let directory = tempfile::tempdir().unwrap();
    let document = shared(&directory);
    document
        .update(|map| drop(map.insert("a".into(), 1.into())))
        .unwrap();
    let loaded = document.load();
    assert!(!loaded.value.contains_key("version"));
    assert_eq!(loaded.value.len(), 1);
}

#[test]
fn a_newer_version_is_not_used_and_not_written() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    let newer = "{\"version\":2,\"name\":\"later\",\"count\":9,\"added\":true}";
    std::fs::write(document.path(), newer).unwrap();
    let loaded = document.load();
    assert_eq!(loaded.value, Sample::default());
    assert!(
        loaded
            .problem
            .unwrap()
            .contains("newer Freshkube (version 2)")
    );
    assert!(document.save(&Sample::default()).is_err());
    assert!(document.update(|sample| sample.count = 1).is_err());
    assert_eq!(std::fs::read_to_string(document.path()).unwrap(), newer);
    assert!(copies(&directory).is_empty(), "a newer file is not corrupt");
    assert!(
        directory.path().read_dir().unwrap().count() == 1,
        "no temporary file"
    );
}

#[test]
fn a_version_that_is_not_a_number_is_corrupt() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{\"version\":\"one\"}").unwrap();
    assert!(document.load().problem.is_some());
    assert_eq!(copies(&directory).len(), 1);
}

#[test]
fn a_save_over_a_newer_file_that_was_never_loaded_still_tells_the_person() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{\"version\":3}").unwrap();
    assert!(document.save(&Sample::default()).is_err());
    let mine: Vec<_> = take_notices()
        .into_iter()
        .filter(|notice| notice.file == document.path())
        .collect();
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(mine[0].why, Why::Newer { version: 3 });
    let line = mine[0].to_string();
    assert!(line.contains("version 3"), "{line}");
    assert!(line.contains("changes to it are not saved"), "{line}");
}

#[test]
fn a_watcher_is_woken_when_a_notice_is_queued() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static WOKEN: AtomicUsize = AtomicUsize::new(0);
    watch(|| {
        WOKEN.fetch_add(1, Ordering::SeqCst);
    });
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{").unwrap();
    document.load();
    assert!(WOKEN.load(Ordering::SeqCst) >= 1);
    watch(|| {});
}

#[test]
fn a_typed_file_keeps_its_field_order_and_ends_with_the_version() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    document
        .save(&Sample {
            name: "shop-api".into(),
            count: 3,
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(document.path()).unwrap(),
        "{\"name\":\"shop-api\",\"count\":3,\"version\":1}\n"
    );
}

#[test]
fn a_file_too_large_to_read_is_not_overwritten_by_a_save() {
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory).limited_to(16);
    let big = "x".repeat(64);
    std::fs::write(document.path(), &big).unwrap();
    assert!(document.save(&Sample::default()).is_err());
    assert_eq!(std::fs::read_to_string(document.path()).unwrap(), big);
}

/// A directory where the copy can't be made: the load still gives the
/// default and says so, and a save is refused and leaves the file.
#[cfg(unix)]
#[test]
fn a_copy_that_cannot_be_made_refuses_the_save_and_keeps_the_file() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let document = sample(&directory);
    std::fs::write(document.path(), "{ broken").unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(directory.path().join("probe")).is_ok() {
        // Running as a user who ignores permissions (root): nothing to prove.
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let loaded = document.load();
    let saved = document.save(&Sample::default());
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(loaded.value, Sample::default());
    let problem = loaded.problem.expect("a problem");
    assert!(problem.contains("a copy could not be kept"), "{problem}");
    assert!(saved.is_err());
    assert_eq!(
        std::fs::read_to_string(document.path()).unwrap(),
        "{ broken"
    );
    assert!(copies(&directory).is_empty());
}
