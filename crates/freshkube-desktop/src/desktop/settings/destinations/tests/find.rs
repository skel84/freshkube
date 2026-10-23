use super::*;

mod suggestions {
    use super::*;
    use crate::desktop::settings::edit_tests::open_settings;
    use crate::desktop::settings::find::Ignored;
    use base64::Engine as _;
    use freshkube_delivery::discover::{self, DestinationFindings, NamespaceRead};
    use freshkube_ui::real_io::RealIo;
    use std::sync::Arc;

    /// A token that stands for a credential: the Secret list the finder
    /// reads carries it in `config` and in `last-applied-configuration`.
    const MARKER: &str = "marker-token-7b21c0de";
    const FILE: &str = r#"{"version":1,"clusters":[
        {"id":"core-fra","role":"core","context":"core-fra"},
        {"id":"prod-fra","role":"environment","context":"prod-fra"},
        {"id":"dev-fra","role":"environment","context":"dev-fra"},
        {"id":"stage-fra","role":"environment","context":"stage-fra"}],
      "destinations":[{"server":"https://prod-ams.example.test:6443","entry":"prod-fra"}],
      "note":"kept"}"#;

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    fn secret(name: &str, server: &str) -> serde_json::Value {
        let config = serde_json::json!({"bearerToken": MARKER}).to_string();
        serde_json::json!({
            "metadata": {"name": format!("cluster-{name}"), "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration":
                    serde_json::json!({"data": {"config": b64(&config)}}).to_string()}},
            "data": {"config": b64(&config), "name": b64(name), "server": b64(server)},
        })
    }

    /// What a find of the invented cluster answers, built from a real
    /// Secret list body that carries the marker.
    fn findings() -> DestinationFindings {
        let body = serde_json::json!({"items": [
            secret("prod-fra", "https://prod-fra.example.test:6443"),
            secret("prod-ams", "https://prod-ams.example.test:6443"),
            secret("prod-lon", "https://prod-lon.example.test:6443"),
            secret("shared", "https://shared.example.test:6443"),
        ]})
        .to_string();
        discover::findings_from_body(
            "core-fra",
            "argocd",
            body.as_bytes(),
            &[
                ("prod-fra", "https://prod-fra.example.test:6443"),
                ("prod-ams", "https://prod-ams.example.test:6443"),
                ("prod-lon", "https://prod-lon.example.test:6443"),
                ("dev-fra", "https://shared.example.test:6443"),
                ("stage-fra", "https://SHARED.example.test:6443/"),
            ],
        )
        .unwrap()
    }

    fn list(
        view: &gpui_kit::Entity<crate::desktop::Pilot>,
        cx: &gpui_kit::App,
    ) -> gpui_kit::Entity<super::super::Destinations> {
        view.read(cx).settings_page.read(cx).destinations.clone()
    }

    fn answer_with(
        view: &gpui_kit::Entity<crate::desktop::Pilot>,
        findings: DestinationFindings,
        cx: &mut gpui_kit::App,
    ) {
        let destinations = list(view, cx);
        destinations.update(cx, |list, _| {
            list.finder.fake = Some(Arc::new(move || {
                let findings = findings.clone();
                Box::pin(async move { findings })
            }));
        });
    }

    async fn found(
        cx: &mut TestAppContext,
        handle: gpui_kit::AnyWindowHandle,
        view: &gpui_kit::Entity<crate::desktop::Pilot>,
    ) {
        let view = view.clone();
        cx.wait_for_io(handle, move |_, cx| {
            let list = list(&view, cx);
            let list = list.read(cx);
            !list.finder.is_finding() && list.finder.read.data().is_some()
        })
        .await;
    }

    /// The rows of the suggestions: destination, cluster or reason.
    fn suggested(
        view: &gpui_kit::Entity<crate::desktop::Pilot>,
        cx: &gpui_kit::App,
    ) -> Vec<(String, String)> {
        list(view, cx)
            .read(cx)
            .finder
            .rows
            .iter()
            .map(|row| (row.destination.to_string(), row.entry.to_string()))
            .collect()
    }

    async fn start(
        cx: &mut TestAppContext,
        directory: &Path,
    ) -> (
        tokio::runtime::Runtime,
        gpui_kit::AnyWindowHandle,
        gpui_kit::Entity<crate::desktop::Pilot>,
    ) {
        write(directory, FILE);
        let (runtime, handle, view) = launch(cx, directory);
        open_settings(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            answer_with(&view, findings(), cx);
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();
        (runtime, handle, view)
    }

    #[gpui_kit::test]
    async fn find_lists_suggestions_in_their_group(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let guard = tempfile::tempdir().unwrap();
        let (_runtime, handle, view) = start(cx, guard.path()).await;
        cx.update_window(handle, |_, window, cx| {
            // Nothing is read until Find is pressed.
            assert!(list(&view, cx).read(cx).finder.read.data().is_none());
            assert!(window.try_find("settings-suggestion-0").is_none());
            window.click("settings-destinations-find", cx);
        })
        .unwrap();
        found(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                suggested(&view, cx),
                [
                    (
                        "server https://kubernetes.default.svc · name in-cluster".to_owned(),
                        "core-fra".to_owned()
                    ),
                    (
                        "server https://prod-fra.example.test:6443 · name prod-fra".to_owned(),
                        "prod-fra".to_owned()
                    ),
                    (
                        "server https://prod-lon.example.test:6443 · name prod-lon".to_owned(),
                        "context prod-lon isn’t a workspace cluster; add it in Clusters".to_owned()
                    ),
                    (
                        "server https://shared.example.test:6443 · name shared".to_owned(),
                        "matches dev-fra and stage-fra".to_owned()
                    ),
                ],
                "prod-ams is mapped already, so it is no item"
            );
            // The mapped row, then the group's rows.
            assert!(window.find("settings-destination-0").visible());
            for at in 0..4 {
                assert!(window.find(format!("settings-suggestion-{at}")).visible());
            }
            assert!(window.try_find("settings-suggestion-4").is_none());
            // Only an item that can't be accepted carries a mark.
            assert!(window.try_find("settings-suggestion-1-mark").is_none());
            assert!(window.find("settings-suggestion-2-mark").visible());
            assert!(window.find("settings-suggestion-3-mark").visible());
            // Nothing was written, and nothing sensitive was kept.
            let list = list(&view, cx);
            let list = list.read(cx);
            let kept: Vec<String> = list
                .finder
                .rows
                .iter()
                .flat_map(|row| {
                    [
                        row.destination.to_string(),
                        row.entry.to_string(),
                        row.tooltip.to_string(),
                        row.suggested.as_ref().unwrap().found_from.to_string(),
                    ]
                })
                .chain(list.finder.notes.iter().map(|(_, text)| text.to_string()))
                .chain([format!("{:?}", list.finder.read.data())])
                .collect();
            for text in kept {
                assert!(!text.contains(MARKER), "{text}");
            }
            let status = view.read(cx).settings_page.read(cx).status().unwrap();
            assert!(
                status.text(None).contains("4 suggested"),
                "{}",
                status.text(None)
            );
        })
        .unwrap();
        assert_eq!(destinations(guard.path()).len(), 1);
    }

    #[gpui_kit::test]
    async fn accept_writes_server_and_name_rows(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let guard = tempfile::tempdir().unwrap();
        let (_runtime, handle, view) = start(cx, guard.path()).await;
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-find", cx)
        })
        .unwrap();
        found(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Ambiguous and unknown items can't be accepted: nothing is
            // written.
            window.click("settings-suggestion-3", cx);
            window.press("enter", cx);
            window.click("settings-suggestion-2", cx);
            window.press("a", cx);
        })
        .unwrap();
        assert_eq!(destinations(guard.path()).len(), 1);
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-suggestion-1", cx);
            window.press("enter", cx);
        })
        .unwrap();
        saved(cx, handle, &view).await;
        let rows = destinations(guard.path());
        assert_eq!(
            rows.iter()
                .map(|row| (row.key(), row.entry.as_str()))
                .collect::<Vec<_>>(),
            [
                (
                    Key::Server("https://prod-ams.example.test:6443".into()),
                    "prod-fra"
                ),
                (
                    Key::Server("https://prod-fra.example.test:6443".into()),
                    "prod-fra"
                ),
                (Key::Name("prod-fra".into()), "prod-fra"),
            ]
        );
        let text = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
        assert!(text.contains("note"), "unknown keys are kept: {text}");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("settings-saved").label().unwrap().starts_with(
                "Mapped server https://prod-fra.example.test:6443 and name prod-fra to prod-fra."
            ));
            // The item is gone once it is mapped; the others stay.
            assert_eq!(suggested(&view, cx).len(), 3);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    async fn accept_over_a_changed_file_writes_nothing_and_keeps_the_item(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let guard = tempfile::tempdir().unwrap();
        let (_runtime, handle, view) = start(cx, guard.path()).await;
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-find", cx)
        })
        .unwrap();
        found(cx, handle, &view).await;
        // A hand edit the app hasn't polled yet: the save re-reads the file.
        let edited = FILE.replace("kept", "edited");
        write(guard.path(), &edited);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-suggestion-1", cx);
            window.press("enter", cx);
        })
        .unwrap();
        saved(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("settings-saved").is_none());
            assert!(
                window
                    .find("settings-save-failed")
                    .label()
                    .unwrap()
                    .contains("changed on disk")
            );
            assert_eq!(suggested(&view, cx).len(), 4, "the item stays listed");
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
            edited
        );
    }

    #[gpui_kit::test]
    async fn ignore_writes_nothing_and_stays_hidden_after_find(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let guard = tempfile::tempdir().unwrap();
        let (_runtime, handle, view) = start(cx, guard.path()).await;
        let before = std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-find", cx)
        })
        .unwrap();
        found(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-suggestion-1", cx);
            window.press("backspace", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(suggested(&view, cx).len(), 3);
            assert!(
                view.read(cx)
                    .settings_page
                    .read(cx)
                    .ignored
                    .contains(&Ignored::Destination(Key::Name(
                        "argocd/cluster-prod-fra".into()
                    )))
            );
            let status = view.read(cx).settings_page.read(cx).status().unwrap();
            assert!(
                status.text(None).contains("1 ignored"),
                "{}",
                status.text(None)
            );
            // Finding again, and changing section and back, keeps it hidden.
            answer_with(&view, findings(), cx);
            window.click("settings-destinations-find", cx);
        })
        .unwrap();
        found(cx, handle, &view).await;
        open_section(cx, handle, &view, SettingsSection::Clusters).await;
        open_section(cx, handle, &view, SettingsSection::Destinations).await;
        cx.update_window(handle, |_, _, cx| assert_eq!(suggested(&view, cx).len(), 3))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(guard.path().join("freshkube.yaml")).unwrap(),
            before
        );
    }

    #[gpui_kit::test]
    async fn hiding_drops_the_read_and_a_late_answer_never_lands(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let guard = tempfile::tempdir().unwrap();
        let (_runtime, handle, view) = start(cx, guard.path()).await;
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(Some(receive)));
        cx.update_window(handle, |_, window, cx| {
            let late = findings();
            list(&view, cx).update(cx, |list, _| {
                list.finder.fake = Some(Arc::new(move || {
                    let (gate, late) = (gate.clone(), late.clone());
                    Box::pin(async move {
                        let receive = gate.lock().unwrap().take();
                        if let Some(receive) = receive {
                            _ = receive.await;
                        }
                        late
                    })
                }));
            });
            window.click("settings-destinations-find", cx);
            assert!(list(&view, cx).read(cx).finder.is_finding());
        })
        .unwrap();
        open_section(cx, handle, &view, SettingsSection::Clusters).await;
        cx.update_window(handle, |_, _, cx| {
            assert!(!list(&view, cx).read(cx).finder.is_finding());
        })
        .unwrap();
        _ = send.send(());
        cx.run_until_parked();
        open_section(cx, handle, &view, SettingsSection::Destinations).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(list(&view, cx).read(cx).finder.read.data().is_none());
            assert!(suggested(&view, cx).is_empty());
            assert!(window.try_find("settings-suggestion-0").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    async fn a_hand_mapping_after_reload_removes_the_item(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let guard = tempfile::tempdir().unwrap();
        let (_runtime, handle, view) = start(cx, guard.path()).await;
        cx.update_window(handle, |_, window, cx| {
            window.click("settings-destinations-find", cx)
        })
        .unwrap();
        found(cx, handle, &view).await;
        let mapped = FILE.replace(
            r#""destinations":["#,
            r#""destinations":[{"server":"https://prod-fra.example.test:6443","entry":"prod-fra"},"#,
        );
        write(guard.path(), &mapped);
        crate::desktop::settings::tests::poll_setup(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let page = view.read(cx).settings_page.clone();
            page.update(cx, |page, cx| page.reload(cx));
        })
        .unwrap();
        saved(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let rows = suggested(&view, cx);
            assert_eq!(rows.len(), 3);
            assert!(
                rows.iter()
                    .all(|(destination, _)| !destination.contains("prod-fra"))
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    async fn unknown_items_and_example_data_have_no_accept(cx: &mut TestAppContext) {
        use crate::desktop::tests::fixture;
        cx.executor().allow_parking();
        // Without a workspace entry for Argo CD, its own cluster is Unknown.
        let guard = tempfile::tempdir().unwrap();
        write(
            guard.path(),
            r#"{"version":1,"clusters":[{"id":"prod-fra","role":"environment","context":"prod-fra"}]}"#,
        );
        let (_runtime, handle, view) = launch(cx, guard.path());
        open_settings(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            answer_with(&view, findings(), cx);
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            window.render_frame(cx);
            window.click("settings-destinations-find", cx);
        })
        .unwrap();
        found(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-suggestion-0", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(destinations(guard.path()).len(), 0);
        // Example data answers at once from its own findings, and saves
        // nothing: Accept is off.
        let (_runtime, handle, view) = fixture(cx, 1280., 820.);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            window.render_frame(cx);
            window.click("settings-destinations-find", cx);
            window.render_frame(cx);
            assert_eq!(suggested(&view, cx).len(), 4);
            window.click("settings-suggestion-1", cx);
            window.press("enter", cx);
            window.render_frame(cx);
            assert!(window.try_find("settings-saved").is_none());
            assert_eq!(
                view.read(cx)
                    .settings_page
                    .read(cx)
                    .workspace()
                    .destinations
                    .len(),
                3
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_page_with_suggestions_keeps_the_table_layout(cx: &mut TestAppContext) {
        use crate::desktop::tests::fixture;
        for (width, height) in [(1280., 880.), (760., 560.)] {
            let (_runtime, handle, view) = fixture(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |pilot, cx| {
                    pilot.show_settings(SettingsSection::Destinations, window, cx)
                });
                window.render_frame(cx);
                window.click("settings-destinations-find", cx);
                window.render_frame(cx);
                layout_check::assert_table_page(window, cx, &PAGE);
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    async fn the_loading_group_row_shows_while_finding(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let guard = tempfile::tempdir().unwrap();
        let (_runtime, handle, view) = start(cx, guard.path()).await;
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(Some(receive)));
        cx.update_window(handle, |_, window, cx| {
            let late = findings();
            list(&view, cx).update(cx, |list, _| {
                list.finder.fake = Some(Arc::new(move || {
                    let (gate, late) = (gate.clone(), late.clone());
                    Box::pin(async move {
                        let receive = gate.lock().unwrap().take();
                        if let Some(receive) = receive {
                            _ = receive.await;
                        }
                        late
                    })
                }));
            });
            window.click("settings-destinations-find", cx);
            window.render_frame(cx);
            assert!(
                window
                    .find("settings-destinations-group-suggested")
                    .visible()
            );
            // A second press while one runs starts nothing.
            window.click("settings-destinations-find", cx);
        })
        .unwrap();
        // The page takes the status once the update has ended.
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let status = view.read(cx).settings_page.read(cx).status().unwrap();
            assert!(
                status.text(None).contains("Finding suggestions"),
                "{}",
                status.text(None)
            );
        })
        .unwrap();
        _ = send.send(());
        found(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("settings-suggestion-0").visible());
        })
        .unwrap();
    }

    async fn note_after(
        cx: &mut TestAppContext,
        directory: &Path,
        text: &str,
        answer: DestinationFindings,
    ) -> String {
        write(directory, text);
        let (_runtime, handle, view) = launch(cx, directory);
        open_settings(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            answer_with(&view, answer, cx);
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            window.render_frame(cx);
            window.click("settings-destinations-find", cx);
        })
        .unwrap();
        found(cx, handle, &view).await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .find("settings-destinations-note-0")
                .label()
                .unwrap()
                .to_owned()
        })
        .unwrap()
    }

    fn read_of(read: NamespaceRead) -> DestinationFindings {
        DestinationFindings {
            entry: "core-fra".into(),
            read: discover::Read::Done {
                namespaces: vec![("argocd".into(), read)],
                skipped: Vec::new(),
                no_servers: None,
            },
        }
    }

    #[gpui_kit::test]
    async fn refused_capped_and_empty_reads_say_so_in_a_banner(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let own_mapped = FILE.replace(
            r#""destinations":["#,
            r#""destinations":[{"server":"https://kubernetes.default.svc","entry":"core-fra"},"#,
        );
        for (text, answer, expected) in [
            (
                FILE,
                read_of(NamespaceRead::Refused("secrets is forbidden".into())),
                "Listing Secrets in argocd on core-fra was refused",
            ),
            (
                FILE,
                read_of(NamespaceRead::Capped),
                "More than 100 cluster Secrets in argocd; nothing suggested from them",
            ),
            (
                FILE,
                read_of(NamespaceRead::Failed(
                    "the cluster Secrets' answer couldn't be read".into(),
                )),
                "Listing Secrets in argocd on core-fra failed: the cluster Secrets' answer couldn't be read",
            ),
            (
                own_mapped.as_str(),
                read_of(NamespaceRead::Found(Vec::new())),
                "No cluster Secrets found",
            ),
        ] {
            let guard = tempfile::tempdir().unwrap();
            let note = note_after(cx, guard.path(), text, answer).await;
            assert!(note.starts_with(expected), "{note}");
            assert!(!note.contains(MARKER));
        }
    }

    #[gpui_kit::test]
    fn found_from_ends_inside_the_default_window_and_cells_carry_the_rows_lines(
        cx: &mut TestAppContext,
    ) {
        use crate::desktop::tests::fixture;
        let (_runtime, handle, view) = fixture(cx, 1280., 880.);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            window.render_frame(cx);
            window.click("settings-destinations-find", cx);
            window.render_frame(cx);
            window.render_frame(cx);
            let list = list(&view, cx);
            let list = list.read(cx);
            let room = crate::screens::page_width(window);
            assert!(
                list.width <= room,
                "the table is {} wide in a page {room} wide",
                list.width
            );
            // A row's tooltip is its destination, what it says and where it
            // was found; its cells carry the same lines.
            let row = &list.finder.rows[1];
            let lines: Vec<&str> = row.tooltip.lines().collect();
            assert_eq!(lines.len(), 3, "{lines:?}");
            assert!(lines[0].starts_with("server https://prod-fra"));
            assert_eq!(lines[1], "Accept to map server https://prod-fra.example.test:6443 and name prod-fra to prod-fra");
            assert!(lines[2].starts_with("Found from: Secret cluster-prod-fra in argocd on core-fra"));
        })
        .unwrap();
    }
}
