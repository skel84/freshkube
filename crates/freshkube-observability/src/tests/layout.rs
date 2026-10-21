use super::*;

#[gpui_kit::test]
fn every_observability_destination_keeps_its_shared_header(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    for destination in Destination::NAVIGATION
        .into_iter()
        .chain([Destination::Application])
    {
        cx.update(|cx| page.update(cx, |page, cx| page.open(destination, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-title").visible());
            // Coroot keeps revisions whatever the range, and Deployments'
            // inspector picks its own window around one.
            if destination == Destination::Deployments {
                assert!(window.try_find("obs-time").is_none());
            } else {
                assert!(window.find("obs-time").visible());
            }
            assert!(window.find("obs-refresh").visible());
            let page_id = gpui_kit::ElementId::from("observability-page");
            let refreshes = gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .filter(|element| {
                    element.path().contains(&page_id)
                        && element.role() == Some(gpui_kit::Role::Button)
                        && element
                            .label()
                            .is_some_and(|label| label.starts_with("Refresh"))
                })
                .count();
            assert_eq!(
                refreshes, 1,
                "each Observability destination has one page refresh"
            );
            assert!(
                window.try_find("obs-scope").is_none(),
                "the source is in the status bar, not under the header"
            );
        })
        .unwrap();
        let status = cx.update(|cx| page.update(cx, |page, _| page.status().text(None).clone()));
        assert!(
            status.starts_with("Example data"),
            "{destination:?} names its source in the status bar: {status}"
        );
    }
}

#[gpui_kit::test]
fn applications_live_header_controls_fit_a_narrow_page_at_large_text(cx: &mut TestAppContext) {
    use freshkube_core::coroot as api;
    let (_runtime, handle, page) = mount_geometry(cx, 760., 560.);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            page.fixture = false;
            page.live.visible = false;
            let provider = api::Provider::new(
                "https://coroot-header-geometry.invalid",
                api::Credentials::None,
            )
            .unwrap();
            let project = api::ProjectInfo {
                id: "geometry".into(),
                name: "Project with a long display name".into(),
            };
            page.live.source = Some(provider.source(&project));
            page.live.provider = Some(provider);
            page.live.project_label = project.name.clone();
            page.live.project_labels = vec![project.name.clone()];
            page.live.projects = vec![project];
        });
    });
    cx.update_window(handle, |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_header(window, cx);
        let frame = window.find("obs-frame").bounds();
        let padding = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
        let ids = [
            "obs-project",
            "obs-namespace",
            "obs-columns",
            "obs-time",
            "obs-refresh",
            "obs-more",
        ];
        // A control the row has no room for is in the "…" menu.
        let shown: Vec<_> = ids
            .into_iter()
            .filter(|id| window.try_find(*id).is_some())
            .collect();
        assert!(shown.len() == ids.len() - 1 || shown.contains(&"obs-more"));
        for id in shown {
            let control = window.find(id);
            let bounds = control.bounds();
            assert!(control.visible(), "{id} must remain visible");
            assert!(bounds.size.width > px(1.), "{id} must retain its width");
            assert!(
                bounds.left() >= frame.left() + padding - px(0.5)
                    && bounds.right() <= frame.right() - padding + px(0.5),
                "{id} must stay within the page padding: {bounds:?}; frame {frame:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn applications_secondary_header_fits_actual_desktop_widths(cx: &mut TestAppContext) {
    for (width, height, text_size) in [
        (1280., 880., 13.),
        (1280., 880., 14.),
        (760., 560., 20.),
        (1920., 880., 13.),
    ] {
        let (_runtime, handle, _page) = mount_geometry(cx, width, height);
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            settle_header(window, cx);
            let frame = window.find("obs-frame").bounds();
            let title = window.find("obs-title").bounds();
            let secondary = window.find("obs-secondary").bounds();
            let controls = window.find("obs-controls").bounds();
            let padding = crate::ui::dp_px(freshkube_ui::page::PANE_PADDING, window);
            assert!(
                secondary.top() >= title.bottom(),
                "categories belong below the title"
            );
            assert!(
                (secondary.left() - (frame.left() + padding)).abs() < px(0.5),
                "categories stay left-aligned"
            );
            assert!(
                window.try_find("obs-scope").is_none(),
                "nothing follows the header rows: the meta is in the status bar"
            );
            let mut previous = None;
            let toolbar = window.find("obs-toolbar").bounds();
            let folds = window.try_find("obs-more").is_some();
            if width == 1920. {
                assert!(!folds, "nothing folds with room to spare");
            } else if width == 760. {
                assert!(folds, "a narrow row folds its controls");
            }
            for id in [
                "obs-filter",
                "obs-filter-problems",
                "obs-filter-all",
                "obs-tally-critical",
                "obs-tally-warning",
                "obs-tally-unknown",
                "obs-tally-ok",
                "obs-tally-logs",
                "obs-category-application",
                "obs-category-control-plane",
                "obs-category-monitoring",
                "obs-more-categories",
                "obs-namespace",
                "obs-columns",
                "obs-time",
                "obs-refresh",
                "obs-more",
            ] {
                let Some(element) = window.try_find(id) else {
                    continue;
                };
                let bounds = element.bounds();
                assert!(
                    element.visible(),
                    "{id} must remain visible at {width}/{text_size}"
                );
                assert!(bounds.size.width > px(1.), "{id} keeps its width");
                assert!(
                    bounds.left() >= frame.left() + padding - px(0.5)
                        && bounds.right() <= frame.right() - padding + px(0.5),
                    "{id} leaves the page at {width}/{text_size}: {bounds:?}; {frame:?}"
                );
                if matches!(
                    id,
                    "obs-namespace" | "obs-columns" | "obs-time" | "obs-refresh" | "obs-more"
                ) {
                    if let Some(prior) = previous {
                        let prior: gpui_kit::Bounds<gpui_kit::Pixels> = prior;
                        assert!(
                            bounds.top() >= prior.bottom() || bounds.left() >= prior.right(),
                            "controls overlap at {width}/{text_size}"
                        );
                    }
                    previous = Some(bounds);
                }
            }
            // The controls stay on the toolbar's row, at the page's right
            // edge; what doesn't fit is in the "…" menu.
            assert!(
                controls.top() >= toolbar.top() && controls.bottom() <= toolbar.bottom(),
                "controls stay on the toolbar's row at {width}/{text_size}"
            );
            assert!(
                (controls.right() - (frame.right() - padding)).abs() < px(0.5),
                "controls align with the page's right edge at {width}/{text_size}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn applications_uses_the_pods_frame_and_table_at_both_text_sizes(cx: &mut TestAppContext) {
    use freshkube_ui::layout_check::{TablePage, assert_table_page};
    let (_runtime, handle, _page) = mount(cx, true);
    let table = TablePage {
        page: "obs-frame",
        title: "obs-title",
        title_text: "Applications",
        table: "obs-applications-table-scroll",
        list: "obs-applications-list",
    };
    for text_size in [crate::ui::BASE_TEXT, 20.] {
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let layout = assert_table_page(window, cx, &table);
            assert!(
                layout.group.is_some(),
                "namespace groups take the row height: {layout:#?}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn applications_table_fits_short_results_and_caps_long_results(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let short = page.read(cx).matrix.len();
        assert!(short < 16);
        assert_eq!(
            window.find("obs-applications-list").bounds().size.height,
            crate::ui::dp_px(short as f32 * 26., window)
        );
        window.click("obs-show-all", cx);
        window.render_frame(cx);
        assert!(page.read(cx).matrix.len() > 16);
        assert_eq!(
            window.find("obs-applications-list").bounds().size.height,
            crate::ui::dp_px(16. * 26., window)
        );
    })
    .unwrap();
}

/// Measure independently from the cached column builder, in physical pixels.
fn shaped_width(
    window: &gpui_kit::Window,
    value: &str,
    face: gpui_kit::Font,
    size: f32,
) -> gpui_kit::Pixels {
    let run = gpui_kit::TextRun {
        len: value.len(),
        font: face,
        color: Default::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(value.to_owned().into(), px(size), &[run], None)
        .width
}

#[gpui_kit::test]
fn application_captions_and_problem_values_fit_their_cells_after_text_size_changes(
    cx: &mut TestAppContext,
) {
    use super::application_columns::ColumnKind;
    use freshkube_ui::table::TableColumn;
    let (_runtime, handle, page) = mount_size(cx, true, 1280., 880.);
    for base in [13., 14., 20., 13.] {
        cx.update(|cx| crate::text_size::set(base, cx));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let scale = base / crate::ui::BASE_TEXT;
            let mut face = gpui_kit::font(Theme::global(cx).font_family.clone());
            face.weight = crate::ui::HEADING_WEIGHT;
            for (ix, column) in page.read(cx).application_columns.iter().enumerate() {
                if column.label().is_empty() {
                    continue;
                }
                let header =
                    window.find((gpui_kit::SharedString::from("obs-applications-sort"), ix));
                let caption = shaped_width(
                    window,
                    &column.label().to_uppercase(),
                    face.clone(),
                    11. * scale,
                );
                assert!(
                    header.bounds().size.width >= caption + px(19.5 * scale),
                    "{} caption must fit at {base}: {:?}, text {caption:?}",
                    column.label(),
                    header.bounds()
                );
                if let ColumnKind::Report(report) = column.kind {
                    for app in &page.read(cx).applications {
                        let check = app.check(report);
                        if let Some(button) = window.try_find(check.element_id.clone()) {
                            let value = shaped_width(
                                window,
                                &check.value,
                                gpui_kit::font(crate::ui::MONO_FONT),
                                12. * scale,
                            );
                            let glyph = if check.status.report_tone().is_some() {
                                px(14. * scale)
                            } else {
                                px(0.)
                            };
                            assert!(
                                button.bounds().size.width >= value + glyph,
                                "{} {} must fit at {base}: {:?}, text {value:?}, glyph {glyph:?}",
                                app.name,
                                report.label(),
                                button.bounds()
                            );
                        }
                    }
                }
            }
        })
        .unwrap();
    }
}
