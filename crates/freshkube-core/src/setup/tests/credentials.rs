use super::*;

#[test]
fn a_key_or_token_is_keys_json_or_it_is_refused() {
    let coroot = |key: &str| {
        format!(
            "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: api-key\n  key: {key}\n"
        )
    };
    assert_eq!(ok(&coroot("keys.json")).coroot.unwrap().key, Some(KeysJson));
    for refused_value in ["abc123", "\"\"", "Keys.json", "keys.json.bak", "42", "true"] {
        let invalid = refused(&coroot(refused_value));
        assert_eq!(invalid, Invalid::KeyValue(at(5, 8)), "{refused_value}");
        assert!(!invalid.to_string().contains(refused_value), "{invalid}");
    }
    // A list or a mapping is no value at all: a wrong shape, as anywhere.
    for shape in ["[keys.json]", "{a: b}"] {
        let invalid = refused(&coroot(shape));
        assert!(
            matches!(invalid, Invalid::WrongShape(Some(_))),
            "{invalid:?}"
        );
    }
    let text = "version: 1\nmetrics:\n  mode: central\n  store:\n    url: https://metrics.example.test\n    token: hunter2\n";
    assert_eq!(refused(text), Invalid::KeyValue(at(6, 12)));
    let text = text.replace("hunter2", "keys.json");
    assert_eq!(ok(&text).metrics.store.unwrap().token, Some(KeysJson));
    let text = with_cluster(
        "    metrics:\n      source:\n        url: https://m.example.test\n        token: hunter2\n",
    );
    assert_eq!(refused(&text), Invalid::KeyValue(at(9, 16)));
}

#[test]
fn a_key_file_stands_instead_of_the_key_and_is_a_path() {
    let head = "version: 1\ncoroot:\n  server: https://coroot.example.test\n  sign_in: api-key\n";
    let file = ok(&format!("{head}  api_key_file: /home/acme/coroot.key\n"));
    let coroot = file.coroot.as_ref().unwrap();
    assert_eq!(coroot.api_key_file, Some(home("/home/acme/coroot.key")));
    assert_eq!(coroot.key, None);
    assert_eq!(ok(&to_text(&file)), file);
    assert_eq!(
        refused(&format!("{head}  api_key_file: coroot.key\n")),
        Invalid::Rule(Fault::RelativePath("coroot.api_key_file".into()))
    );
    assert_eq!(
        refused(&format!(
            "{head}  key: keys.json\n  api_key_file: /home/acme/coroot.key\n"
        )),
        Invalid::Rule(Fault::KeyTwice)
    );
}

#[test]
fn the_github_token_is_gh_or_keys_json_or_nothing_and_round_trips() {
    let text = |value: &str| {
        format!("version: 1\ndelivery:\n  github:\n    token: {value}\n    future: kept\n")
    };
    for (value, source) in [
        ("keys.json", GitHubToken::KeysJson),
        ("gh", GitHubToken::Gh),
    ] {
        let setup = ok(&text(value));
        let github = setup.delivery.as_ref().unwrap().github.as_ref().unwrap();
        assert_eq!(github.token, Some(source), "{value}");
        assert_eq!(setup.unknown_keys(), vec!["delivery.github.future"]);
        assert_eq!(ok(&to_text(&setup)), setup, "{value} round-trips");
        assert!(to_text(&setup).contains(&format!("token: {value}")));
    }
    // GitHub with no token is anonymous.
    let anonymous = ok("version: 1\ndelivery:\n  github: {}\n");
    assert_eq!(anonymous.delivery.unwrap().github.unwrap().token, None);
    // Anything else is refused, and the refusal never repeats it.
    for bad in ["ghp_invented", "GH", "token123"] {
        let invalid = refused(&text(bad));
        assert!(matches!(invalid, Invalid::KeyValue(_)), "{bad}");
        assert!(!invalid.to_string().contains(bad.trim()), "{bad}");
    }
}
