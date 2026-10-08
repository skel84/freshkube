use super::{Incidents, detail};
use freshkube_core::coroot::{Incident, IncidentView};
fn incident(key: &str) -> Incident {
    serde_json::from_value(serde_json::json!({"key":key,"app":"c:ns:Deployment:api","cluster":"Production","severity":"warning","state":"open","opened_at":null,"duration_seconds":120,"impact_percent":0,"description":"SLO violation"})).unwrap()
}
#[test]
fn selection_survives_reorder_and_clears_removed_identity() {
    let mut state = Incidents::default();
    assert!(state.prepare_list(&[incident("a"), incident("b")]));
    assert!(!state.prepare_list(&[incident("b"), incident("a")]));
    assert_eq!(state.selected().unwrap().0, "a");
    assert!(state.prepare_list(&[incident("b")]));
    assert_eq!(state.selected().unwrap().0, "b");
    assert!(state.prepare_list(&[]));
    assert!(state.selected.is_none());
}
#[test]
fn opened_and_ended_times_read_in_local_time() {
    let mut value = serde_json::to_value(incident("a")).unwrap();
    value["opened_at"] = serde_json::json!("2026-10-05T08:30:00Z");
    value["resolved_at"] = serde_json::json!("2026-10-05T09:00:00Z");
    let value: Incident = serde_json::from_value(value).unwrap();
    let local = |t: &str| {
        t.parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%d %b %H:%M")
            .to_string()
    };
    let mut state = Incidents::default();
    state.prepare_list(std::slice::from_ref(&value));
    assert_eq!(
        state.rows[0].opened.to_string(),
        local("2026-10-05T08:30:00Z")
    );
    let view: IncidentView = serde_json::from_value(serde_json::json!({"incident":value})).unwrap();
    let summary = detail(&view).summary;
    assert!(summary.contains(&format!("Started {}", local("2026-10-05T08:30:00Z"))));
    assert!(summary.contains(&format!("Ended {}", local("2026-10-05T09:00:00Z"))));
    assert!(!summary.contains("UTC"));
}
#[test]
fn chips_and_filter_project_rows_without_moving_the_selection() {
    let mut resolved = serde_json::to_value(incident("b")).unwrap();
    resolved["state"] = serde_json::json!("resolved");
    resolved["description"] = serde_json::json!("Latency above objective");
    let resolved: Incident = serde_json::from_value(resolved).unwrap();
    let mut state = Incidents::default();
    state.prepare_list(&[incident("a"), resolved]);
    assert_eq!(state.counts, [1, 1]);
    assert_eq!(state.shown, [0, 1]);
    state.state_filter = Some(freshkube_core::coroot::IncidentState::Resolved);
    state.project();
    assert_eq!(state.shown, [1]);
    assert_eq!(
        state.selected().unwrap().0,
        "a",
        "a hidden row stays selected"
    );
    state.state_filter = None;
    state.query = "latency".into();
    state.project();
    assert_eq!(state.shown, [1]);
    state.query = "ns/api".into();
    state.project();
    assert_eq!(state.shown, [0, 1], "the filter reads the application too");
    let state = state.cleared();
    assert!(state.rows.is_empty() && state.selected.is_none());
    assert_eq!(state.query, "ns/api", "the filter field still shows it");
}
#[test]
fn missing_evidence_stays_unknown_and_reported_zero_is_preserved() {
    let view: IncidentView =
        serde_json::from_value(serde_json::json!({"incident":incident("a")})).unwrap();
    let projected = detail(&view);
    assert_eq!(projected.objectives[0].compliance, "Not reported");
    assert_eq!(projected.objectives[0].severity, super::Status::Unknown);
    assert_eq!(projected.objectives[0].state, "Not reported");
    assert_eq!(
        projected.objectives[0].impact,
        "Affected requests: Not reported"
    );
    assert!(projected.objectives[0].rates.is_empty());
    assert!(projected.evidence.is_empty());
    assert!(projected.related.is_empty());
    assert!(projected.summary.contains("0.0% affected"));
}

mod ui_tests {
    use super::super::detail;
    use crate::{Destination, Status, connection::Subject, tests::mount_size};
    use freshkube_core::coroot as api;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};
    #[gpui_kit::test]
    fn bounded_incidents_prepare_and_render_at_large_text(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount_size(cx, false, 760., 560.);
        let rows=(0..100).map(|ix|serde_json::from_value::<api::Incident>(serde_json::json!({"key":format!("k{ix}"),"app":"c:ns:Deployment:api","cluster":"Production","severity":"warning","state":"open","opened_at":null,"duration_seconds":120,"impact_percent":0,"description":"Source incident"})).unwrap()).collect::<Vec<_>>();
        let related=(0..100).map(|ix|serde_json::json!({"app_id":format!("c:ns:Deployment:related-{ix}"),"status":"warning","issues":["Source observation"]})).collect::<Vec<_>>();
        let mut i = serde_json::to_value(&rows[0]).unwrap();
        i["rca"] = serde_json::json!({"status":"OK","root_cause":"Source root cause ".repeat(512),"propagation":related});
        let view:api::IncidentView=serde_json::from_value(serde_json::json!({"incident":i,"availability":{"objective":"99% of requests should not fail","compliance":"97.2%","violated":true}})).unwrap();
        cx.update(|cx| {
            crate::text_size::set(20., cx);
            page.update(cx, |page, _| {
                let provider =
                    api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
                page.live.source = Some(provider.source(&api::ProjectInfo {
                    id: "p1".into(),
                    name: "Production".into(),
                }));
                page.live.provider = Some(provider);
                page.destination = Destination::Incidents;
                let start = std::time::Instant::now();
                page.incident_observations.prepare_list(&rows);
                page.prepare_incident_columns();
                page.incident_observations.detail = Some(detail(&view));
                page.incident_observations.prepare_related();
                eprintln!(
                    "Incidents: 100 rows/100 propagation preparation {:?}",
                    start.elapsed()
                );
                let request = page
                    .live
                    .incidents
                    .begin(page.live.identity(Subject::Incidents).unwrap());
                page.live.incidents.apply(&request, Ok(rows));
                let request = page.live.incident.begin(
                    page.live
                        .identity(Subject::Incident(
                            "k0".into(),
                            api::AppId::new("c:ns:Deployment:api"),
                        ))
                        .unwrap(),
                );
                page.live.incident.apply(&request, Ok(view));
                assert_eq!(
                    page.incident_observations
                        .detail
                        .as_ref()
                        .unwrap()
                        .objectives[0]
                        .severity,
                    Status::Warning
                );
            });
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("obs-live-incident-k0").is_some());
            assert!(
                window.try_find("obs-live-incident-k99").is_none(),
                "rows past the table's view are not drawn"
            );
            assert!(
                window
                    .try_find("obs-incident-app-c:ns:Deployment:related-10")
                    .is_none()
            );
            let bounds = window.find("obs-incident-detail").bounds();
            assert!(bounds.right() <= window.viewport_size().width);
            assert!(
                bounds.top() >= window.find("obs-incidents-table").bounds().bottom(),
                "a narrow page puts the detail below the table"
            );
            let start = std::time::Instant::now();
            for _ in 0..10 {
                window.render_frame(cx);
            }
            eprintln!(
                "Incidents: ten bounded headless frames {:?}",
                start.elapsed()
            );
        })
        .unwrap();
    }

    fn open_example(
        cx: &mut TestAppContext,
        width: f32,
    ) -> (
        tokio::runtime::Runtime,
        gpui_kit::AnyWindowHandle,
        gpui_kit::Entity<crate::ObservabilityPage>,
    ) {
        let (runtime, handle, page) = mount_size(cx, true, width, 900.);
        cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Incidents, cx)));
        cx.run_until_parked();
        (runtime, handle, page)
    }

    #[gpui_kit::test]
    fn incidents_use_the_pods_frame_and_table_at_both_text_sizes(cx: &mut TestAppContext) {
        use freshkube_ui::layout_check::{
            PageFrame, Table, assert_bare, assert_edge_frame, assert_inspector, assert_table,
        };
        let (_runtime, handle, _page) = open_example(cx, 1260.);
        // The inspector shares the table's line, so the frame's edges are
        // measured from the split.
        let frame = PageFrame {
            page: "obs-frame",
            title: "obs-title",
            title_text: "Incidents",
            content: "obs-incidents-split",
        };
        let table = Table {
            table: Some("obs-incidents-table-scroll"),
            list: "obs-incidents-list",
        };
        for text_size in [crate::ui::BASE_TEXT, 20.] {
            cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_edge_frame(window, cx, &frame);
                let rows = assert_table(window, cx, &table);
                assert!(rows.header.is_some(), "{rows:#?}");
                assert_bare(window, "obs-incidents-table");
                assert_inspector(
                    window,
                    cx,
                    "obs-incidents-split",
                    "obs-incidents-table",
                    "obs-incident-detail",
                    "obs-incident-title",
                );
            })
            .unwrap();
        }
    }

    /// A short window scrolls the frame around a stacked inspector, so the
    /// split has its least heights instead of its contents'; a wide short
    /// window keeps the split filling the page.
    #[gpui_kit::test]
    fn a_short_window_keeps_the_split_to_its_least_heights(cx: &mut TestAppContext) {
        use freshkube_ui::inspector::{LIST_MIN_HEIGHT, MIN_HEIGHT, short_height};
        let close = |what: &str, a: gpui_kit::Pixels, b: gpui_kit::Pixels| {
            assert!(
                (a - b).abs() <= gpui_kit::px(1.),
                "{what}: {a:?}, expected {b:?}"
            );
        };
        let (_runtime, handle, page) = mount_size(cx, true, 760., 500.);
        cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Incidents, cx)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let split = window.find("obs-incidents-split").bounds();
            let table = window.find("obs-incidents-table").bounds();
            let detail = window.find("obs-incident-detail").bounds();
            let least = crate::ui::dp_px(short_height(false, true), window);
            close("split", split.size.height, least);
            assert_eq!(short_height(false, true), LIST_MIN_HEIGHT + MIN_HEIGHT);
            close("stacked", detail.top(), table.bottom());
            close("bottom", detail.bottom(), split.bottom());
        })
        .unwrap();

        let (_runtime, handle, page) = mount_size(cx, true, 1260., 500.);
        cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Incidents, cx)));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let split = window.find("obs-incidents-split").bounds();
            let scroll = window.find("obs-scroll").bounds();
            close("fills", split.bottom(), scroll.bottom());
            let detail = window.find("obs-incident-detail").bounds();
            close("beside", detail.bottom(), split.bottom());
        })
        .unwrap();
    }

    /// A width dragged to is saved in the app's store of widths, and the
    /// next page opens its inspector at it.
    #[gpui_kit::test]
    fn the_inspector_width_survives_reopening(cx: &mut TestAppContext) {
        use freshkube_ui::inspector::{MemoryWidths, SavedWidths, set_saved_widths};
        let widths = std::rc::Rc::new(MemoryWidths::default());
        cx.update(|cx| set_saved_widths(widths.clone(), cx));
        let (_runtime, handle, page) = open_example(cx, 1260.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let state = page.read(cx).incident_split.beside_state().clone();
            state.update(cx, |state, cx| {
                state.resize_panel(1, crate::ui::dp_px(560., window), window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(widths.width("incidents"), Some(560.));
        assert_eq!(widths.width("traces"), None);

        let (_runtime, handle, _page) = open_example(cx, 1260.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let width = window.find("obs-incident-detail").bounds().size.width;
            let expected = crate::ui::dp_px(560., window);
            assert!(
                (width - expected).abs() <= gpui_kit::px(1.),
                "{width:?}, expected {expected:?}"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn example_incidents_select_by_key_filter_and_clear(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = open_example(cx, 1260.);
        let selected = |cx: &mut gpui_kit::App| {
            let state = &page.read(cx).incident_observations;
            (
                state.selected().map(|(key, _)| key.clone()),
                state.detail.as_ref().map(|d| d.key.clone()),
            )
        };
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for key in ["INC-12", "INC-13", "INC-11"] {
                assert!(
                    window
                        .find(gpui_kit::SharedString::from(format!(
                            "obs-live-incident-{key}"
                        )))
                        .visible()
                );
            }
            let table = window.find("obs-incidents-table").bounds();
            let detail = window.find("obs-incident-detail").bounds();
            assert!(
                detail.left() >= table.right(),
                "a wide page puts the detail beside"
            );
            assert_eq!(selected(cx), (Some("INC-12".into()), Some("INC-12".into())));
            window.click("obs-live-incident-INC-13", cx);
            assert_eq!(selected(cx), (Some("INC-13".into()), Some("INC-13".into())));
            page.update(cx, |page, cx| page.refresh(cx));
            assert_eq!(
                selected(cx),
                (Some("INC-13".into()), Some("INC-13".into())),
                "a new answer keeps the selection by key"
            );

            window.click("obs-incidents-resolved", cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).incident_observations.shown.len(), 1);
            assert!(window.try_find("obs-live-incident-INC-13").is_none());
            assert!(
                window
                    .within("obs-incidents-footer")
                    .find("obs-incidents-showing")
                    .visible()
            );
            assert_eq!(selected(cx).0.as_deref(), Some("INC-13"));
            window.click("obs-incidents-show-all", cx);
            window.render_frame(cx);
            assert_eq!(page.read(cx).incident_observations.shown.len(), 3);
            assert!(window.try_find("obs-incidents-showing").is_none());

            let width = page.read(cx).incident_observations.width;
            window.click("obs-columns", cx);
            window.press("down", cx);
            window.press("enter", cx);
            assert!(page.read(cx).incident_observations.width < width);

            window.click("obs-filter", cx);
            window.input("no-such-incident", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-incidents-empty").visible());
            window.click("obs-incidents-clear", cx);
            assert_eq!(page.read(cx).incident_observations.shown.len(), 3);
            assert!(page.read(cx).incident_observations.query.is_empty());
        })
        .unwrap();
    }
}
