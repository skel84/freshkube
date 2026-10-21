use super::*;

#[gpui_kit::test]
fn applications_loading_failure_refusal_and_stale_keep_distinct_surfaces(cx: &mut TestAppContext) {
    use freshkube_core::coroot as api;
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, _| {
            page.fixture = false;
            let provider =
                api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
            page.live.source = Some(provider.source(&api::ProjectInfo {
                id: "p".into(),
                name: "Project".into(),
            }));
            page.live.provider = Some(provider);
            page.live.visible = false;
            page.applications.clear();
            page.project();
            page.live.apps.begin(
                page.live
                    .identity(super::connection::Subject::Applications)
                    .unwrap(),
            );
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The table's loading rows, under its header, not a card skeleton.
        let rows = window.find("obs-table-loading").bounds();
        let table = window.find("obs-applications-table").bounds();
        assert!(table.contains(&rows.origin), "{rows:?} {table:?}");
        assert!(window.try_find("obs-loading").is_none());
        assert!(window.find("obs-title").visible());
        assert!(window.try_find("obs-failed").is_none());
        assert!(page.read(cx).loading_motion(cx).is_some());
    })
    .unwrap();
    for (error, surface) in [
        (api::ReadError::Refused, "obs-refused"),
        (api::ReadError::Missing, "obs-failed"),
    ] {
        cx.update(|cx| {
            page.update(cx, |page, _| {
                let request = page.live.apps.begin(
                    page.live
                        .identity(super::connection::Subject::Applications)
                        .unwrap(),
                );
                page.live.apps.apply(&request, Err(error.to_string()));
                page.live.capabilities[0] = api::Capability::Unavailable(error);
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find(surface).visible());
            assert!(window.find("obs-retry").visible());
            assert!(window.try_find("obs-loading").is_none());
            assert!(window.try_find("obs-table-loading").is_none());
            assert!(page.read(cx).loading_motion(cx).is_none());
        })
        .unwrap();
    }
    cx.update(|cx| {
        page.update(cx, |page, _| {
            let identity = page
                .live
                .identity(super::connection::Subject::Applications)
                .unwrap();
            let request = page.live.apps.begin(identity.clone());
            let apps = example::applications();
            page.live.apps.apply(&request, Ok(apps.clone()));
            page.apply_applications(&apps);
            let request = page.live.apps.begin(identity);
            page.live
                .apps
                .apply(&request, Err("Coroot could not be reached.".into()));
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-stale").visible());
        assert_eq!(page.read(cx).shown_apps, 7);
        assert!(page.read(cx).live.apps.last_successful().is_some());
        assert!(window.try_find("obs-failed").is_none());
        window.click("obs-retry", cx);
        assert!(page.read(cx).live.apps.data().is_some());
        assert!(page.read(cx).live.apps.last_successful().is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn applications_empty_filter_has_a_clear_action_and_empty_observations_do_not(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-filter", cx);
        window.input("no-such-application", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-applications-empty").visible());
        assert_eq!(page.read(cx).shown_apps, 0);
        window.click("obs-clear-filters", cx);
        assert!(page.read(cx).shown_apps > 0);
        assert!(page.read(cx).query_text.is_empty());
        assert_eq!(page.read(cx).filter, super::Filter::All);
        page.update(cx, |page, _| page.apply_applications(&[]));
        window.render_frame(cx);
        assert!(window.find("obs-applications-empty").visible());
        assert!(window.try_find("obs-clear-filters").is_none());
    })
    .unwrap();
}

/// Held example data leaves each table on its loading rows, under its
/// header and moved by the shell, and the answer replaces them.
#[gpui_kit::test]
fn held_tables_show_the_shared_loading_rows(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    // A connection clears what was observed; the headers stay known.
    cx.update(|cx| page.update(cx, |page, _| page.clear_observations()));
    assert!(cx.read(|cx| !page.read(cx).trace_columns().is_empty()));
    for (destination, table) in [
        (Destination::Applications, "obs-applications-table"),
        (Destination::Incidents, "obs-incidents-table"),
        (Destination::Traces, "obs-traces-table"),
    ] {
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.hold = true;
                page.open(destination, cx);
            })
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let rows = window.find("obs-table-loading").bounds();
            let table = window.find(table).bounds();
            assert!(table.contains(&rows.origin), "{rows:?} {table:?}");
            assert!(window.try_find("obs-loading").is_none());
            assert!(page.read(cx).loading_motion(cx).is_some());
            // Reading an application's traces anew keeps their header.
            if destination == Destination::Traces {
                assert!(!page.read(cx).trace_columns().is_empty());
            }
        })
        .unwrap();
        cx.update(|cx| page.update(cx, |page, _| page.hold = false));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("obs-table-loading").is_none());
            assert!(window.find(table).visible());
            assert!(page.read(cx).loading_motion(cx).is_none());
        })
        .unwrap();
    }
}

/// A held example source (`FRESHKUBE_FIXTURE_HOLD=coroot`) forgets what
/// the examples answered, so every table waits on its loading rows and the
/// rail counts no incidents; a live source holds nothing.
#[gpui_kit::test]
fn held_examples_forget_what_the_page_answered(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, true);
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(!page.applications.is_empty());
        assert!(page.incident_count().is_some());
    });
    let held = example_source(true);
    cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some(&held), None, cx)));
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.applications.is_empty());
        assert!(page.matrix.is_empty());
        assert!(page.app_choices.is_empty());
        assert!(page.categories.is_empty());
        assert_eq!(page.app_count, "0 apps");
        assert!(page.incident_count().is_none());
        // The held tables keep their headers, over their loading rows.
        assert!(!page.incident_columns().is_empty());
        assert!(!page.revision_columns().is_empty());
    });
    for (destination, table) in [
        (Destination::Applications, "obs-applications-table"),
        (Destination::Incidents, "obs-incidents-table"),
    ] {
        cx.update(|cx| page.update(cx, |page, cx| page.open(destination, cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let rows = window.find("obs-table-loading").bounds();
            assert!(window.find(table).bounds().contains(&rows.origin));
        })
        .unwrap();
    }

    let (_runtime, _handle, live) = mount(cx, false);
    cx.update(|cx| {
        live.update(cx, |page, cx| {
            page.set_access(Some("conn-a".into()), None, cx)
        })
    });
    assert!(cx.read(|cx| !live.read(cx).hold && !live.read(cx).fixture));
}
