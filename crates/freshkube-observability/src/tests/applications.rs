use super::*;

#[gpui_kit::test]
fn applications_filter_and_cells_open_the_selected_report(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter-all", cx);
        assert_eq!(
            page.read(cx).counts[1],
            example::applications()
                .iter()
                .filter(|app| app.category == "application")
                .count()
        );
        window.click("obs-filter", cx);
        window.input("worker", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            page.read(cx)
                .matrix
                .iter()
                .filter(|r| matches!(r, MatrixRow::App(_)))
                .count(),
            1
        );
        window.click("obs-check-fixture:payments:Deployment:worker-net", cx);
        assert_eq!(page.read(cx).destination, Destination::Application);
        assert_eq!(page.read(cx).report, Report::Net);
        window.click("obs-report-memory", cx);
        assert_eq!(page.read(cx).report_name, "Memory");
    })
    .unwrap();
}

#[gpui_kit::test]
fn live_mode_never_contains_example_applications(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, false);
    for destination in [
        Destination::Applications,
        Destination::ServiceMap,
        Destination::Application,
        Destination::Incidents,
        Destination::Deployments,
        Destination::Profiling,
        Destination::Traces,
    ] {
        cx.update(|cx| page.update(cx, |page, cx| page.open(destination, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-integration-required").visible());
            assert_eq!(
                window.find("obs-integration-required").label(),
                Some("Coroot connection required")
            );
            assert!(window.try_find("obs-applications-table").is_none());
            assert!(page.read(cx).applications.is_empty());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn projection_preserves_signal_states_and_selection_by_id(cx: &mut TestAppContext) {
    use super::Status;
    let (_runtime, _handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let id = example::id(example::WORKER);
            page.open_app(id.clone(), Report::Cpu, cx);
            let raw = example::applications();
            let app = page.selected_application().unwrap();
            assert_eq!(app.check(Report::Cpu).status, Status::Ok);
            assert_eq!(app.check(Report::Errors).status, Status::Unknown);
            assert_eq!(app.check(Report::DiskIo).status, Status::Absent);
            let mut reversed = raw.clone();
            reversed.reverse();
            page.apply_applications(&reversed);
            assert_eq!(page.selected_app.as_ref(), Some(&id));
            reversed.retain(|a| a.id != id);
            page.apply_applications(&reversed);
            assert!(page.selected_app.is_none());
            assert!(page.report_snapshot.is_none());
            // A live publication uses exactly the same projection as these fixtures.
            page.fixture = false;
            page.apply_applications(&raw);
            let live: Vec<_> = page
                .applications
                .iter()
                .map(|a| (a.id.clone(), a.status))
                .collect();
            let fixture: Vec<_> = super::projection::applications(&raw)
                .into_iter()
                .map(|a| (a.id, a.status))
                .collect();
            assert_eq!(live, fixture);
        })
    });
}

#[gpui_kit::test]
fn applications_categories_and_filter_count_follow_the_visible_scope(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            page.read(cx).active_categories.as_ref(),
            &std::collections::BTreeSet::from(["application".to_owned()])
        );
        assert_eq!(page.read(cx).shown_apps, 7);
        assert_eq!(page.read(cx).app_count, "7 apps");
        assert!(
            window
                .find("obs-filter-problems")
                .path()
                .contains(&gpui_kit::ElementId::from("obs-view"))
        );
        assert!(
            window
                .find("obs-category-application")
                .path()
                .contains(&gpui_kit::ElementId::from("obs-categories"))
        );
        window.click("obs-filter-all", cx);
        assert!(page.read(cx).shown_apps > 7);
        window.click("obs-category-control-plane", cx);
        assert!(page.read(cx).active_categories.contains("control-plane"));
        assert!(page.read(cx).active_categories.contains("application"));
        window.click("obs-filter", cx);
        window.input("kube-apiserver", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).shown_apps, 1);
        assert_eq!(page.read(cx).app_count, "1 app");
        assert_eq!(page.read(cx).counts[1], 1);
        assert!(
            matches!(&page.read(cx).matrix[0], MatrixRow::Group { label, summary, .. }
            if label == "kube-system" && summary == "1 app")
        );
        window.click("obs-category-control-plane", cx);
        assert_eq!(page.read(cx).shown_apps, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn unknown_tally_uses_the_same_circle_as_unknown_rows_and_filters_apps(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let mut apps = example::applications();
            let unknown = apps
                .iter_mut()
                .find(|app| app.id == example::id("payments/checkout"))
                .unwrap();
            unknown.status = freshkube_core::coroot::Status::Unknown;
            page.apply_applications(&apps);
        });
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let chip = window.find("obs-tally-unknown");
        assert_eq!(chip.label(), Some("1 Unknown"));
        // The chip and the unknown rows both draw `ui::status_glyph(Tone::Unknown)`,
        // the dashed ring whose drawing and size freshkube-ui's tests check. The
        // glyph is a sprite, so the chip paints no outline of its own.
        let chip_bounds = chip.bounds().scale(window.scale_factor());
        let unknown = crate::palette::palette(cx).unk_ink;
        assert!(
            !window.painted_quads().into_iter().any(|quad| {
                quad.border_color == unknown && chip_bounds.contains(&quad.bounds.center())
            }),
            "the Unknown chip draws the shared glyph, not an outline of its own"
        );
        window.click("obs-tally-unknown", cx);
        assert_eq!(page.read(cx).filter, super::Filter::Unknown);
        assert_eq!(page.read(cx).shown_apps, 1);
        window.click("obs-tally-unknown", cx);
        assert_eq!(page.read(cx).filter, super::Filter::All);
        assert!(page.read(cx).shown_apps > 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn categories_fall_back_to_all_and_keep_new_categories_after_all_is_chosen(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let mut apps = example::applications();
            apps.iter_mut()
                .for_each(|app| app.category = "infrastructure".into());
            page.categories = Default::default();
            page.category_defaults_pending = true;
            page.apply_applications(&apps);
            assert!(page.all_categories);
            assert!(page.shown_apps > 0);
            page.all_categories = false;
            page.active_categories = std::rc::Rc::new(["infrastructure".into()].into());
            page.project_filters();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-more-categories", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert!(page.read(cx).all_categories);
        page.update(cx, |page, _| {
            let mut apps = example::applications();
            apps.iter_mut()
                .for_each(|app| app.category = "new-category".into());
            page.apply_applications(&apps);
            assert!(
                page.shown_apps > 0,
                "all includes categories discovered later"
            );
            page.all_categories = false;
            page.project_filters();
            assert_eq!(
                page.shown_apps, 0,
                "an explicit category selection stays explicit"
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn filtering_applications_resets_the_uniform_list_to_the_first_row(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let seed = example::applications()[0].clone();
            let apps: Vec<_> = (0..100)
                .map(|ix| {
                    let mut app = seed.clone();
                    app.id =
                        freshkube_core::coroot::AppId::new(format!("c:ns:Deployment:app-{ix:03}"));
                    app.status = if ix < 50 {
                        freshkube_core::coroot::Status::Critical
                    } else {
                        freshkube_core::coroot::Status::Warning
                    };
                    app
                })
                .collect();
            page.filter = super::Filter::All;
            page.apply_applications(&apps);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-name-c:ns:Deployment:app-000").visible());
        page.read(cx)
            .application_table
            .scroll
            .scroll_to_item_strict(20, gpui_kit::ScrollStrategy::Top);
        window.render_frame(cx);
        assert!(
            window
                .try_find("obs-name-c:ns:Deployment:app-000")
                .is_none()
        );
        window.click("obs-tally-critical", cx);
        window.render_frame(cx);
        assert_eq!(page.read(cx).shown_apps, 50);
        assert!(window.find("obs-name-c:ns:Deployment:app-000").visible());
        page.read(cx)
            .application_table
            .scroll
            .scroll_to_item_strict(20, gpui_kit::ScrollStrategy::Top);
        window.render_frame(cx);
        assert!(
            window
                .try_find("obs-name-c:ns:Deployment:app-000")
                .is_none()
        );
        window.click("obs-filter", cx);
        window.input("app-", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).shown_apps, 50);
        assert!(window.find("obs-name-c:ns:Deployment:app-000").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn application_namespace_select_searches_and_applies_the_chosen_namespace(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-namespace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("cache", cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(110));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(page.read(cx).namespace.as_deref(), Some("cache"));
        assert_eq!(page.read(cx).shown_apps, 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_folded_applications_controls_do_what_the_controls_do(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount_geometry(cx, 760., 560.);
    cx.update_window(handle, |_, _, cx| crate::text_size::set(20., cx))
        .unwrap();
    cx.run_until_parked();
    // Namespace, Columns, Time range, Refresh: each opens from "…".
    let open = |cx: &mut TestAppContext, item: usize| {
        cx.update_window(handle, |_, window, cx| {
            settle_header(window, cx);
            assert!(window.try_find("obs-time").is_none(), "not folded");
            window.click("obs-more", cx);
            window.render_frame(cx);
            window.within("popup-menu").click(item, cx);
            window.render_frame(cx);
        })
        .unwrap();
    };
    let choose = |cx: &mut TestAppContext, item: usize| {
        cx.update_window(handle, |_, window, cx| {
            window
                .within("submenu")
                .within("popup-menu")
                .click(item, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    // The "…" names what its folded controls are set to, in their order.
    let says = |cx: &mut TestAppContext, label: &str| {
        cx.update_window(handle, |_, window, cx| {
            settle_header(window, cx);
            assert_eq!(window.find("obs-more").label(), Some(label));
            let dot = window.try_find("obs-more-dot").is_some();
            assert_eq!(dot, label != "More");
        })
        .unwrap();
    };
    says(cx, "More");
    open(cx, 2);
    // 1 hour, 3 hours, 24 hours, 7 days.
    choose(cx, 3);
    cx.update(|cx| assert_eq!(page.read(cx).hours(), 168));
    says(cx, "More · Time range 7d");
    let first = cx.update(|cx| page.read(cx).namespaces[0].clone());
    open(cx, 0);
    // All namespaces, then each namespace.
    choose(cx, 1);
    cx.update(|cx| assert_eq!(page.read(cx).namespace.as_ref(), Some(&first)));
    says(cx, &format!("More · Namespace {first} · Time range 7d"));
    let hidden = cx.update(|cx| page.read(cx).hidden_application_columns.len());
    open(cx, 1);
    choose(cx, 0);
    cx.update(|cx| {
        assert_ne!(page.read(cx).hidden_application_columns.len(), hidden);
    });
    says(
        cx,
        &format!("More · Namespace {first} · 1 column hidden · Time range 7d"),
    );
    // Back to 3 hours: the range drops out, the others stay.
    open(cx, 2);
    choose(cx, 1);
    says(cx, &format!("More · Namespace {first} · 1 column hidden"));
}
