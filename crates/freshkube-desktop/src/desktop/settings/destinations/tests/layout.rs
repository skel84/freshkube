use super::*;

#[gpui_kit::test]
async fn the_edit_title_stays_left_of_the_close_button_at_text_twenty(cx: &mut TestAppContext) {
    use crate::GpuiOptions;
    use crate::desktop::tests::mount;
    cx.executor().allow_parking();
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let guard = tempfile::tempdir().unwrap();
        write(
            guard.path(),
            &serde_json::json!({
                "version": 1,
                "clusters": [{"id": "prod", "role": "environment", "context": "example-prod"}],
                "destinations": [{
                    "server": "https://a-very-long-cluster-name.region-one.zone-two.example.test:6443",
                    "entry": "prod",
                }],
            }).to_string(),
        );
        let options = GpuiOptions::new(Some(guard.path().join("missing")), None, 100)
            .with_preferences(Some(guard.path().join("preferences.json")));
        let (_runtime, handle, view) = mount(cx, options, width, height);
        open_section(cx, handle, &view, SettingsSection::Destinations).await;
        cx.update_window(handle, |_, window, cx| {
            crate::text_size::set(20., cx);
            window.render_frame(cx);
            window.click("settings-destination-0", cx);
            window.press("e", cx);
            window.render_frame(cx);
            let title = window.find("settings-destination-edit-url").bounds();
            let close = window.find("close").bounds();
            assert!(
                title.right() < close.left(),
                "{width}×{height}: title {title:?} overlaps close button {close:?}"
            );
            // Include the text chunks, so the assertion also catches text
            // overflowing a correctly constrained title box.
            for ix in 0.. {
                let Some(chunk) =
                    window.try_find(format!("settings-destination-edit-url-chunk-{ix}"))
                else {
                    break;
                };
                assert!(chunk.bounds().right() <= title.right() + gpui_kit::px(1.));
            }
            window.click("close", cx);
            window.render_frame(cx);
            assert!(window.try_find("settings-destination-form").is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
async fn removing_a_server_flows_inline_and_wraps_only_between_chunks(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for (url, first_chunk, short) in [
        ("https://b.example.test", "https://b", true),
        (
            "https://a-very-long-cluster-name.region-one.zone-two.example.test:6443/a-long-path",
            "https://a-very-long-cluster-name",
            false,
        ),
    ] {
        let guard = tempfile::tempdir().unwrap();
        write(
            guard.path(),
            &serde_json::json!({
                "version": 1,
                "clusters": [{"id": "prod", "role": "environment", "context": "example-prod"}],
                "destinations": [{"server": url, "entry": "prod"}],
            })
            .to_string(),
        );
        let (_runtime, handle, view) = launch(cx, guard.path());
        open_section(cx, handle, &view, SettingsSection::Destinations).await;
        for size in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(size, cx);
                window.render_frame(cx);
                window.click("settings-destination-0", cx);
                window.press("backspace", cx);
                window.render_frame(cx);
                let paragraph = window.find("settings-destination-remove-url").bounds();
                let first_word = window
                    .find("settings-destination-remove-url-chunk-0")
                    .bounds();
                let scheme = window
                    .find("settings-destination-remove-url-chunk-6")
                    .bounds();
                let style = window.text_style();
                let font_size = crate::ui::dp_px(13., window);
                let shaped_width = |text: &str| {
                    window
                        .text_system()
                        .shape_line(
                            text.to_owned().into(),
                            font_size,
                            &[style.to_run(text.len())],
                            None,
                        )
                        .width()
                        .ceil()
                };
                // Both slashes and the first host label stay on one line.
                assert!((scheme.size.width - shaped_width(first_chunk)).abs() <= gpui_kit::px(1.));
                assert!((first_word.size.width - shaped_width("Argo ")).abs() <= gpui_kit::px(1.));
                let line_height = first_word.size.height;
                assert!(line_height < font_size * 2.);
                let mut previous: Option<gpui_kit::Bounds<gpui_kit::Pixels>> = None;
                for ix in 0.. {
                    let Some(chunk) =
                        window.try_find(format!("settings-destination-remove-url-chunk-{ix}"))
                    else {
                        break;
                    };
                    let bounds = chunk.bounds();
                    // No chunk in either URL is oversized: none may split mid-word.
                    assert!((bounds.size.height - line_height).abs() <= gpui_kit::px(1.));
                    assert!(bounds.right() <= paragraph.right() + gpui_kit::px(1.));
                    if let Some(previous) = previous {
                        if (bounds.top() - previous.top()).abs() <= gpui_kit::px(1.) {
                            // Spaces belong to words; URL chunks have no added gap.
                            assert!((bounds.left() - previous.right()).abs() <= gpui_kit::px(1.));
                        } else {
                            assert!(
                                (bounds.top() - previous.top() - line_height).abs()
                                    <= gpui_kit::px(1.)
                            );
                            assert!(previous.right() + bounds.size.width > paragraph.right() - gpui_kit::px(1.));
                            // Every new line returns to the paragraph edge.
                            assert!((bounds.left() - paragraph.left()).abs() <= gpui_kit::px(1.));
                        }
                    }
                    previous = Some(bounds);
                }
                if short {
                    // NoopTextSystem uses a wide fixed advance for every glyph.
                    // Assert natural packing rather than the native font's exact
                    // line count (which the lead checks in the running app).
                    let text = format!("Argo CD Applications with destination server {url} show it as Unknown again. Nothing on a cluster is touched.");
                    let least_lines = (shaped_width(&text) / paragraph.size.width).ceil();
                    assert!(paragraph.size.height <= line_height * least_lines + gpui_kit::px(1.));
                } else {
                    assert!(paragraph.size.height > line_height * 2.);
                    let last_url_chunk = window
                        .find("settings-destination-remove-url-chunk-12")
                        .bounds();
                    assert!(
                        last_url_chunk.top() > scheme.top(),
                        "the long URL must actually wrap"
                    );
                }
                window.click("settings-destination-remove-cancel", cx);
                window.render_frame(cx);
            })
            .unwrap();
        }
        assert_eq!(destinations(guard.path()), vec![server(url, "prod")]);
    }
}

#[gpui_kit::test]
fn the_example_lists_its_destinations_on_a_table_page(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        for text in [13., 20.] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(text, cx);
                view.update(cx, |pilot, cx| {
                    pilot.show_settings(SettingsSection::Destinations, window, cx)
                });
                crate::desktop::tests::settle_header(window, cx);
                layout_check::assert_table_page(window, cx, &PAGE);
                for row in 0..3 {
                    assert!(window.find(format!("settings-destination-{row}")).visible());
                }
                // The section holds one table: neither other list is drawn.
                assert!(window.try_find("settings-list").is_none());
                assert!(window.try_find("settings-sources-list").is_none());
                assert!(
                    window
                        .find("settings-scope")
                        .label()
                        .unwrap_or_default()
                        .contains("3 destinations mapped")
                );
            })
            .unwrap();
        }
    }
}

#[gpui_kit::test]
fn the_glyph_column_stays_at_the_table_edge_and_contains_every_mark(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |pilot, cx| {
                pilot.show_settings(SettingsSection::Destinations, window, cx)
            });
            layout_check::assert_table_page(window, cx, &PAGE);
            let table = window.find(PAGE.table).bounds();
            let glyph_width = crate::ui::dp_px(freshkube_ui::table::GLYPH_WIDTH, window);
            let header = gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .find(|element| element.label() == Some("Destination"))
                .expect("Destination header")
                .bounds();
            assert!((header.left() - table.left() - glyph_width).abs() <= gpui_kit::px(1.));
            let destinations = view.read(cx).settings_page.read(cx).destinations.clone();
            for missing in [false, true] {
                if missing {
                    // Long destinations must leave every missing-cluster mark in its column.
                    destinations.update(cx, |list, cx| {
                        for row in &mut list.rows {
                            row.destination = "example-destination-with-a-long-name".into();
                            row.listed = false;
                        }
                        cx.notify();
                    });
                    window.render_frame(cx);
                }
                for row in 0..3 {
                    let glyph = window
                        .find(format!("settings-destination-{row}-glyph"))
                        .bounds();
                    assert!(
                        (glyph.left() - table.left() - gpui_kit::px(1.)).abs() <= gpui_kit::px(1.),
                        "glyph column moved at {width}×{height}: {glyph:?} in {table:?}"
                    );
                    assert!((glyph.size.width - glyph_width).abs() <= gpui_kit::px(1.));
                    let cell = window
                        .find(format!("settings-destination-{row}-destination-cell"))
                        .bounds();
                    assert!((cell.left() - glyph.right()).abs() <= gpui_kit::px(1.));
                    let mark = window.try_find(format!("settings-destination-{row}-mark"));
                    if missing {
                        let mark = mark.expect("missing cluster has a mark");
                        assert!(mark.visible());
                        let mark = mark.bounds();
                        assert!(mark.left() >= glyph.left() && mark.right() <= glyph.right());
                        assert!((mark.center().x - glyph.center().x).abs() <= gpui_kit::px(1.));
                    } else {
                        assert!(mark.is_none());
                    }
                }
            }
        })
        .unwrap();
    }
}

/// In a short window a list section scrolls its frame, and its list keeps
/// `SHORT_LIST_HEIGHT`, with twenty mappings as with three.
#[gpui_kit::test]
fn a_short_window_keeps_each_list_usable(cx: &mut TestAppContext) {
    use freshkube_ui::page::SHORT_LIST_HEIGHT;
    use gpui_kit::px;
    let (_runtime, handle, view) = fixture(cx, 760., 480.);
    for (section, table) in [
        (SettingsSection::Clusters, "settings-table-scroll"),
        (
            SettingsSection::Destinations,
            "settings-destinations-table-scroll",
        ),
    ] {
        for many in [false, true] {
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(20., cx);
                let mut example = workspace::example();
                if many {
                    example.destinations = (0..20)
                        .map(|n| Destination::new(name(&format!("shop-api-{n}")), "dev-fra"))
                        .collect();
                }
                view.update(cx, |pilot, cx| {
                    pilot.settings_page.update(cx, |page, cx| {
                        page.set_workspace(&Loaded::Workspace(example), true, cx)
                    });
                    pilot.show_settings(section, window, cx)
                });
                crate::desktop::tests::settle_header(window, cx);
                assert!(freshkube_ui::page::is_short(window));
                let least = crate::ui::dp_px(SHORT_LIST_HEIGHT, window) - px(2.);
                let bounds = window.find(table).bounds();
                assert!(bounds.size.height >= least, "{table} is {bounds:?}");
            })
            .unwrap();
        }
    }
}
