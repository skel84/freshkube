use super::*;

#[test]
fn central_mode_needs_a_store_and_a_label_name() {
    assert_eq!(
        rule("version: 1\nmetrics:\n  mode: central\n"),
        "metrics.mode is central, and metrics.store is missing"
    );
    let store =
        "  store:\n    entry: a\n    service: {namespace: vm, name: vmselect, port: 8481}\n";
    for label in ["1cluster", "my-label", "", "a b"] {
        let text = format!("version: 1\nmetrics:\n  mode: central\n{store}  label: '{label}'\n");
        assert_eq!(
            rule(&text),
            "metrics.label isn’t a Prometheus label name",
            "{label:?}"
        );
    }
    let text = format!("version: 1\nmetrics:\n  mode: central\n{store}  label: _k8s_cluster\n");
    assert_eq!(ok(&text).metrics.label_name(), "_k8s_cluster");
}

#[test]
fn the_store_is_a_service_in_an_entry_or_a_url() {
    let text = |store: &str| format!("version: 1\nmetrics:\n  mode: central\n  store:\n{store}");
    let one_of = "metrics.store must set exactly one of entry with service, or url";
    assert_eq!(rule(&text("    entry: a\n")), one_of);
    assert_eq!(
        rule(&text("    service: {namespace: vm, name: v, port: 1}\n")),
        one_of
    );
    assert_eq!(rule(&text("    {}\n")), one_of);
    assert_eq!(
        rule(&text(
            "    entry: a\n    service: {namespace: vm, name: v, port: 1}\n    url: https://m.example.test\n"
        )),
        one_of
    );
    assert_eq!(
        rule(&text(
            "    entry: a\n    service: {namespace: vm, name: v, port: 1}\n    token: keys.json\n"
        )),
        "metrics.store.token only goes with url"
    );
    assert_eq!(
        rule(&text("    url: ftp://m.example.test\n")),
        "metrics.store.url isn’t an http:// or https:// URL"
    );
    assert_eq!(
        rule(&text(
            "    entry: a\n    service: {namespace: vm, name: '', port: 1}\n"
        )),
        "metrics.store.service.name is empty"
    );
    let url = ok(&text(
        "    url: https://m.example.test/select/0/prometheus\n    token: keys.json\n",
    ));
    assert_eq!(ok(&to_text(&url)), url);
}

#[test]
fn a_service_path_is_normalised_and_read_with_or_without_slashes() {
    let service = |path: &str| {
        let text = format!(
            "version: 1\nmetrics:\n  mode: central\n  store:\n    entry: a\n    service: {{namespace: vm, name: v, port: 1, path: '{path}'}}\n"
        );
        ok(&text).metrics.store.unwrap().service.unwrap()
    };
    for path in [
        "/select/0/prometheus",
        "select/0/prometheus/",
        "//select/./0/../prometheus//",
    ] {
        assert_eq!(service(path).path, "select/0/prometheus", "{path}");
    }
    assert_eq!(service("/").path, "");
    assert_eq!(service("").path, "");
    let setup = ok(
        "version: 1\nmetrics:\n  mode: central\n  store:\n    entry: a\n    service: {namespace: vm, name: v, port: 1, https: true, path: p}\n",
    );
    let written = to_text(&setup);
    assert!(written.contains("path: /p"), "{written}");
    assert!(written.contains("https: true"), "{written}");
    assert_eq!(ok(&written), setup);
}

#[test]
fn a_store_and_label_under_another_mode_are_kept_unread() {
    // Not checked, since nothing reads them, but not lost either.
    let text = "version: 1\nmetrics:\n  mode: per-entry\n  label: 'not a label'\n  store:\n    entry: a\n    url: https://m.example.test\n";
    let setup = ok(text);
    assert_eq!(setup.metrics.mode, Mode::PerEntry);
    assert_eq!(setup.metrics.label.as_deref(), Some("not a label"));
    let written = to_text(&setup);
    assert!(written.contains("mode: per-entry"), "{written}");
    assert_eq!(ok(&written), setup);
}
