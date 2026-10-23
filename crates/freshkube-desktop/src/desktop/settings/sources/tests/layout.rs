use super::*;

#[gpui_kit::test]
fn the_example_lists_each_part_on_a_table_page_and_reads_nothing(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 880.);
    for size in [None, Some(20.)] {
        cx.update_window(handle, |_, window, cx| {
            if let Some(size) = size {
                crate::text_size::set(size, cx);
            }
            open(window, cx, &view);
            layout_check::assert_table_page(window, cx, &PAGE);
            for read in ["kargo", "builds", "argocd", "workloads"] {
                assert!(window.find(format!("settings-source-{read}")).visible());
            }
            assert!(window.try_find("settings-destinations-list").is_none());
            assert_eq!(
                states(&view, cx),
                vec![
                    row(KARGO, "cicd-fra", "Not checked"),
                    row(BUILDS, "cicd-fra", "Not checked"),
                    row(ARGOCD, "core-fra", "Not checked"),
                    row(WORKLOADS, MAPPED, "Not checked"),
                ]
            );
            assert!(!list(&view, cx).read(cx).is_checking());
            // Nothing is checked, so no glyph says anything yet.
            assert!(window.try_find("settings-source-kargo-mark").is_none());
            // Reads is cut at a word in a narrow window; the row's tooltip
            // holds the whole of it, as the cell's own does.
            let builds = tooltip(&view, BUILDS, cx);
            assert!(
                builds.contains("Reads PipelineRuns and TaskRuns from Pipelines as Code"),
                "{builds}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn the_glyph_column_stays_at_the_table_edge_and_contains_every_mark(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            open(window, cx, &view);
            layout_check::assert_table_page(window, cx, &PAGE);
            let table = window.find(PAGE.table).bounds();
            let glyph_width = crate::ui::dp_px(freshkube_ui::table::GLYPH_WIDTH, window);
            let header = gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .find(|element| element.label() == Some("Part"))
                .expect("Part header")
                .bounds();
            assert!((header.left() - table.left() - glyph_width).abs() <= gpui_kit::px(1.));
            // The reserved column is present even before there are marks.
            for checked in [false, true] {
                if checked {
                    window.click("settings-sources-check", cx);
                    window.render_frame(cx);
                }
                for read in ["kargo", "builds", "argocd", "workloads"] {
                    let glyph = window
                        .find(format!("settings-source-{read}-glyph"))
                        .bounds();
                    assert!(
                        (glyph.left() - table.left() - gpui_kit::px(1.)).abs() <= gpui_kit::px(1.),
                        "glyph column moved at {width}×{height}: {glyph:?} in {table:?}"
                    );
                    assert!((glyph.size.width - glyph_width).abs() <= gpui_kit::px(1.));
                    let part = window.find(format!("settings-source-{read}-part")).bounds();
                    assert!((part.left() - glyph.right()).abs() <= gpui_kit::px(1.));
                    let mark = window.try_find(format!("settings-source-{read}-mark"));
                    if checked {
                        let mark = mark.expect("checked source has a mark");
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

#[gpui_kit::test]
fn every_part_label_fits_whole_after_check_sources_at_both_sizes(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            open(window, cx, &view);
            layout_check::settle_header(window, cx);
            let before: Vec<_> = [KARGO, BUILDS, ARGOCD, WORKLOADS]
                .map(|read| {
                    window
                        .find(format!("settings-source-{}-label", read.key()))
                        .bounds()
                })
                .into();
            window.click("settings-sources-check", cx);
            layout_check::settle_header(window, cx);
            for (read, before) in [KARGO, BUILDS, ARGOCD, WORKLOADS].into_iter().zip(before) {
                let label = window.find(format!("settings-source-{}-label", read.key()));
                assert!(label.visible());
                let bounds = label.bounds();
                let run = gpui_kit::TextRun {
                    len: read.label().len(),
                    font: gpui_kit::font(crate::ui::MONO_FONT),
                    color: Default::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                // Shape the whole label in the shared table row's actual font and size.
                let whole = window
                    .text_system()
                    .shape_line(
                        read.label().into(),
                        crate::ui::dp_px(12.5, window),
                        &[run],
                        None,
                    )
                    .width();
                assert!(
                    whole <= bounds.size.width + gpui_kit::px(1.),
                    "{} is cut after Check sources at {width}×{height}: {whole:?} in {bounds:?}",
                    read.label()
                );
                assert_eq!(bounds, before, "{} moved after Check sources", read.label());
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn reads_truncates_inside_the_table_at_both_window_sizes(cx: &mut TestAppContext) {
    const READS: &str = "settings-source-builds-reads";
    for (width, height) in [(1280., 880.), (760., 560.)] {
        let (_runtime, handle, view) = fixture(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            open(window, cx, &view);
            if width == 1280. {
                let entry = freshkube_ui::table::word_cut_style("settings-source-workloads-entry").unwrap();
                assert_eq!(entry.text.as_ref(), MAPPED);
                use freshkube_ui::table::TableColumn;
                let sources = list(&view, cx).read(cx);
                for column in &sources.columns {
                    let values = sources.rows.iter().map(|row| if column.label().as_ref() == "Entry" { &row.entry } else { &row.why });
                    if matches!(column.label().as_ref(), "Entry" | "Chosen by") {
                        assert_eq!(column.width(), freshkube_ui::table::fit(column.label(), values, freshkube_ui::table::WIDEST));
                    }
                }
            }
            // Exercise a value too long even for the wide table.
            list(&view, cx).update(cx, |list, cx| {
                let row = list.rows.iter_mut().find(|row| row.read == BUILDS).unwrap();
                row.reads = "PipelineRuns and TaskRuns from Pipelines as Code, including retained runs and their delivery metadata".into();
                cx.notify();
            });
            layout_check::settle_header(window, cx);
            let table = window.find(PAGE.table).bounds();
            let cell = window.find(format!("{READS}-cell")).bounds();
            assert!(
                cell.left() >= table.left() && cell.right() <= table.right() + gpui_kit::px(1.),
                "Reads leaves the viewport at {width}×{height}: {cell:?} in {table:?}"
            );
            let shown = freshkube_ui::table::word_cut_style(READS).expect("Reads painted");
            assert!(shown.text.ends_with('…'), "Reads is not cut: {}", shown.text);
            let run = gpui_kit::TextRun {
                len: shown.text.len(),
                font: gpui_kit::font(shown.font),
                color: shown.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let painted = window
                .text_system()
                .shape_line(shown.text, shown.size, &[run], None)
                .width();
            let padding = crate::ui::dp_px(2. * freshkube_ui::table::CELL_PAD, window);
            assert!(
                painted <= cell.size.width - padding + gpui_kit::px(1.),
                "the cut text exceeds the cell: {painted:?} in {cell:?}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn sources_table_and_inspector_follow_the_shared_geometry_at_every_size(cx: &mut TestAppContext) {
    for (width, height) in [(1280., 880.), (760., 560.)] {
        for text in [13., 20.] {
            let (_runtime, handle, view) = fixture(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                crate::text_size::set(text, cx);
                open(window, cx, &view);
                layout_check::assert_table_page(window, cx, &PAGE);
                window.click("settings-source-kargo", cx);
                layout_check::assert_inspector(
                    window,
                    cx,
                    "settings-sources-split",
                    "settings-sources-table",
                    "settings-source-inspector",
                    "settings-source-inspector-title",
                );
                let table = window.find("settings-sources-table").bounds();
                let inspector = window.find("settings-source-inspector").bounds();
                let beside =
                    crate::screens::page_width(window) >= freshkube_ui::inspector::SPLIT_WIDTH;
                assert_eq!(beside, width == 1280. && text == 13.);
                if beside {
                    assert!(inspector.left() >= table.right());
                } else {
                    assert!(inspector.top() >= table.bottom());
                }
            })
            .unwrap();
        }
    }
}
