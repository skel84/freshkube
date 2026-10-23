use super::*;

#[test]
fn a_file_that_is_not_yaml_says_where() {
    // The second entry is indented under the first one's value.
    let text = "version: 1\nclusters:\n  - id: cluster-a\n   role: core\n";
    let invalid = refused(text);
    assert_eq!(invalid, Invalid::NotYaml(at(4, 4)));
    assert_eq!(invalid.to_string(), "invalid YAML at line 4, column 4");
}

#[test]
fn a_duplicate_key_is_invalid_yaml_at_its_line() {
    let invalid = refused("version: 1\nkubeconfig: /a\nkubeconfig: /b\n");
    assert!(matches!(invalid, Invalid::NotYaml(_)), "{invalid:?}");
    assert!(!invalid.to_string().contains("/b"), "{invalid}");
}

#[test]
fn a_file_that_is_not_text_is_invalid_yaml() {
    let invalid = parse(&[0x76, 0xff, 0xfe, 0x3a]).unwrap_err();
    assert!(matches!(invalid, Invalid::NotYaml(_)), "{invalid:?}");
}

#[test]
fn words_without_a_place_say_which_pass_failed() {
    assert_eq!(Invalid::NotYaml(None).to_string(), "invalid YAML");
    assert_eq!(
        Invalid::WrongShape(None).to_string(),
        "a value of the wrong type or shape"
    );
}

#[test]
fn a_value_of_the_wrong_shape_says_where() {
    let text = with_cluster("    talosconfig: [not, a, path]\n");
    let invalid = refused(&text);
    assert_eq!(invalid, Invalid::WrongShape(at(6, 18)));
    assert_eq!(
        invalid.to_string(),
        "a value of the wrong type or shape at line 6, column 18"
    );
}

#[test]
fn a_wrong_role_and_a_wrong_mode_are_found_at_their_own_lines() {
    let text = "version: 1\nclusters:\n  - id: cluster-a\n    role: wizard\n    context: x\n";
    assert_eq!(refused(text), Invalid::WrongShape(at(4, 11)));
    let text = "version: 1\nmetrics:\n  mode: sideways\n";
    assert_eq!(refused(text), Invalid::WrongShape(at(3, 9)));
    let text = "version: 1\ncoroot:\n  server: https://coroot.example.test\n";
    let Invalid::WrongShape(Some(place)) = refused(text) else {
        panic!("a missing sign_in is a wrong shape");
    };
    // A missing key is found at the mapping that lacks it.
    assert_eq!(place.line, 3);
}

#[test]
fn a_refusal_never_repeats_what_was_read() {
    let secret = "s3cr3t-value-7d41";
    for text in [
        format!(
            "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: api-key\n  key: {secret}\n"
        ),
        format!("version: 1\nkubeconfig: [{secret}]\n"),
        format!("version: 1\nmetrics:\n  mode: {secret}\n"),
        format!("version: 1\nclusters:\n  - id: a\n    role: {secret}\n    context: c\n"),
        format!("version: 1\nclusters: [\n  {secret}\n"),
        format!("version: 1\ncoroot:\n  server: {secret}\n  sign_in: session\n"),
    ] {
        let words = refused(&text).to_string();
        assert!(!words.contains(secret), "{words}");
    }
}

#[test]
fn version_is_required_and_is_one() {
    assert_eq!(refused(""), Invalid::MissingVersion);
    assert_eq!(refused("clusters: []\n"), Invalid::MissingVersion);
    assert_eq!(refused("- a\n- b\n"), Invalid::MissingVersion);
    assert_eq!(refused("version: \"1\"\n"), Invalid::UnreadableVersion);
    assert_eq!(refused("version: 1.5\n"), Invalid::UnreadableVersion);
    assert_eq!(refused("version: -1\n"), Invalid::UnreadableVersion);
    assert_eq!(refused("version: 0\n"), Invalid::UnknownVersion(0));
    // A later version is named as such even when the rest is not ours.
    assert_eq!(
        refused("version: 2\nclusters: nonsense\n"),
        Invalid::UnknownVersion(2)
    );
    assert_eq!(
        refused("version: 2\n").to_string(),
        "it is version 2, and this app reads 1"
    );
    assert_eq!(refused("clusters: []\n").to_string(), "it has no version");
}

#[test]
fn the_version_is_not_an_unknown_key_and_is_written_fresh() {
    let setup = ok("version: 1\n");
    assert!(setup.unknown_keys().is_empty());
    assert!(to_text(&setup).contains("\nversion: 1\n"));
}

#[test]
fn a_file_over_the_limit_is_refused() {
    let mut text = String::from("version: 1\n# ");
    text.push_str(&"x".repeat(MAX_BYTES as usize));
    assert_eq!(refused(&text), Invalid::TooLarge);
}

#[test]
fn off_is_a_word_in_this_file_and_not_a_switch() {
    // YAML 1.1 readers take a bare `off` for false; there is no `results:
    // off` for that reason, and here it is refused where it stands.
    for value in ["off", "false", "no", "\"off\""] {
        let text = with_cluster(&format!("    results: {value}\n"));
        let invalid = refused(&text);
        assert_eq!(invalid, Invalid::WrongShape(at(6, 14)), "{value}");
        assert!(!invalid.to_string().contains("off"), "{invalid}");
    }
    let text = "version: 1\nmetrics:\n  mode: off\n";
    assert_eq!(refused(text), Invalid::WrongShape(at(3, 9)));
    // Words a YAML 1.1 reader would take for booleans stay strings here:
    // this reader takes only true and false, and the file reads back as
    // written.
    let text = "version: 1\nnote_a: off\nnote_b: 'yes'\nnote_c: \"n\"\n";
    let setup = ok(text);
    let written = to_text(&setup);
    assert_eq!(ok(&written), setup);
    for (key, word) in [("note_a", "off"), ("note_b", "yes"), ("note_c", "n")] {
        assert_eq!(setup.extra[key], serde_json::Value::String(word.into()));
    }
}
