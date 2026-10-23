use super::*;

#[gpui_kit::test]
fn secret_values_stay_hidden_until_one_is_revealed(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (secret, _) = target("secrets", |_, name| name == "ledger-database");
    let reveal = |key: &str, cx: &App| {
        pane.read(cx)
            .detail
            .as_ref()
            .unwrap()
            .reveals
            .get(key)
            .cloned()
    };
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &secret, Duration::ZERO, cx);
        window.render_frame(cx);
        // Keys and sizes show; no value does, in the overview or YAML.
        for ix in 0..3usize {
            assert!(window.find(("detail-secret-key", ix)).visible());
            assert!(window.try_find(("detail-secret-value", ix)).is_none());
        }
        let yaml = pane.read(cx).view().unwrap().document.yaml.clone();
        assert!(!yaml.contains("example-only-password"));
        assert!(!yaml.contains("ZXhhbXBsZS1vbmx5LXBhc3N3b3Jk"));

        window.click(("detail-secret-reveal", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(
            reveal("password", cx),
            Some(Reveal::Shown(SecretValue::Text(
                "example-only-password".into()
            )))
        );
        assert!(window.find(("detail-secret-value", 1usize)).visible());
        assert!(window.try_find(("detail-secret-value", 0usize)).is_none());
        window.click(("detail-secret-copy", 1usize), cx);
    })
    .unwrap();
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("example-only-password".into())
    );

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Copy YAML still copies the hidden form.
        window.click("detail-copy-yaml", cx);
        window.click(("detail-secret-reveal", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(
            reveal("keystore.p12", cx),
            Some(Reveal::Shown(SecretValue::Binary(6)))
        );
        assert!(window.try_find(("detail-secret-copy", 2usize)).is_none());

        window.click(("detail-secret-hide", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(reveal("password", cx), None);
        assert!(window.try_find(("detail-secret-value", 1usize)).is_none());

        // A new version of the Secret hides every value again.
        let mut changed = example::document(&secret.identity, live::now()).unwrap();
        changed.resource_version = "2".into();
        pane.update(cx, |pane, cx| {
            let seq = pane.read_seq;
            pane.finish_read(&secret.identity, seq, Ok(DocumentView::new(changed)), cx)
        });
        assert_eq!(reveal("keystore.p12", cx), None);
    })
    .unwrap();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert!(copied.contains("ledger-database"));
    assert!(!copied.contains("example-only-password"));
}
