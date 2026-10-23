use super::*;

#[test]
fn commit_writes_a_file_that_reads_back_and_refuses_a_changed_one() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("config").join("freshkube.yaml");
    let setup = example().expect("the bundled example is valid");
    let seen = commit(&file, &Seen::Missing, &setup).unwrap();
    assert_eq!(seen, Seen::Bytes(std::fs::read(&file).unwrap()));
    assert_eq!(ok(&std::fs::read_to_string(&file).unwrap()), setup);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // No temporary file is left beside it.
    let names: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["freshkube.yaml"]);

    // The next save knows what the last one wrote.
    let mut next = setup.clone();
    next.metrics.mode = Mode::Automatic;
    let seen = commit(&file, &seen, &next).unwrap();
    assert_eq!(ok(&std::fs::read_to_string(&file).unwrap()), next);

    // A hand edit in between is not written over.
    std::fs::write(&file, "version: 1\n# edited\n").unwrap();
    let result = commit(&file, &seen, &setup);
    assert!(matches!(result, Err(SaveError::Changed)), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "version: 1\n# edited\n"
    );
    // Nor is a file that appeared when none was expected.
    let result = commit(&file, &Seen::Missing, &setup);
    assert!(matches!(result, Err(SaveError::Changed)), "{result:?}");
}

#[test]
fn commit_writes_nothing_for_an_invalid_setup() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    let mut setup = example().expect("the bundled example is valid");
    setup.coroot.as_mut().unwrap().server = "nonsense".into();
    let result = commit(&file, &Seen::Missing, &setup);
    assert!(
        matches!(
            result,
            Err(SaveError::Invalid(Invalid::Rule(Fault::NotUrl(_))))
        ),
        "{result:?}"
    );
    assert!(!file.exists());
}

#[test]
fn a_setup_too_large_to_read_back_is_not_written() {
    let mut setup = example().expect("the bundled example is valid");
    setup.extra.insert(
        "padding".into(),
        serde_json::Value::String("x".repeat(MAX_BYTES as usize)),
    );
    assert_eq!(setup.check_saveable(), Err(Invalid::TooLarge));
}

#[test]
fn the_first_settings_setup_keeps_legacy_workspace_unknown_keys() {
    let workspace = workspace::parse(br#"{"version":1,"future":{"enabled":true}}"#).unwrap();
    let setup = Setup::new(workspace);
    setup.check_saveable().unwrap();
    assert_eq!(setup.unknown_keys(), ["future"]);
    assert_eq!(parse(to_text(&setup).as_bytes()).unwrap(), setup);
}

#[test]
fn a_legacy_key_reserved_by_setup_is_refused_before_a_save() {
    let workspace = workspace::parse(br#"{"version":1,"metrics":{"future":true}}"#).unwrap();
    assert!(Setup::new(workspace).check_saveable().is_err());
}
