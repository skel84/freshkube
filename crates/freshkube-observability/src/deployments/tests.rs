use super::{Revisions, WINDOWS, detail, image};
use crate::application::example as app_example;
use crate::tables::TableKey;
use freshkube_core::coroot as api;

fn revision(hash: &str, started: i64) -> api::DeploymentRevision {
    api::DeploymentRevision {
        id: format!("{hash}:{started}"),
        hash: hash.into(),
        started_at: chrono::DateTime::from_timestamp(started, 0).unwrap(),
        version: format!("{hash}: example.test/shop/auth:1.4.0"),
        status: api::Status::Ok,
        findings: vec![],
        note: Some("No notable changes".into()),
    }
}

#[test]
fn the_selection_follows_a_revision_by_id_and_a_new_application_starts_again() {
    let app = api::AppId::new("c:shop:Deployment:auth");
    let (a, b) = (
        revision("9c41e7", 1_789_999_000),
        revision("2b70aa", 1_789_990_000),
    );
    let mut state = Revisions::default();
    assert!(state.prepare_list(Some(&app), Some(&Ok(vec![a.clone(), b.clone()]))));
    assert_eq!(state.selected, Some(TableKey::Revision(a.id.clone())));
    state.selected = Some(TableKey::Revision(b.id.clone()));
    state.window = 2;
    assert!(
        !state.prepare_list(Some(&app), Some(&Ok(vec![b.clone(), a.clone()]))),
        "a reorder keeps the revision"
    );
    assert!(state.prepare_list(Some(&app), Some(&Ok(vec![a.clone()]))));
    assert_eq!(state.selected, Some(TableKey::Revision(a.id.clone())));
    assert_eq!(state.count, "1 deployment");

    let other = api::AppId::new("c:shop:Deployment:cart");
    assert!(state.prepare_list(Some(&other), Some(&Ok(vec![]))));
    assert!(state.rows.is_empty() && state.selected.is_none());
    assert_eq!(state.window, 2, "the window chosen stays");

    state.prepare_list(Some(&other), Some(&Err(api::ReadError::Limit)));
    assert!(state.list_error.is_some() && state.rows.is_empty());
}

#[test]
fn a_row_shows_coroots_images_and_its_first_finding() {
    let mut value = revision("9c41e7", 1_789_999_000);
    assert_eq!(image(&value), "example.test/shop/auth:1.4.0");
    let mut state = Revisions::default();
    state.prepare_list(None, Some(&Ok(vec![value.clone()])));
    assert_eq!(state.rows[0].finding, "No notable changes");
    assert_eq!(state.rows[0].id, "obs-revision-9c41e7-1789999000");

    let finding = |report: &str, message: &str| api::RevisionFinding {
        report: report.into(),
        ok: false,
        message: message.into(),
        time: None,
    };
    value.findings = vec![
        finding("SLO", "Availability: 97% (objective: 99%)"),
        finding("CPU", "CPU usage rose"),
    ];
    value.version = "9c41e7".into();
    state.prepare_list(None, Some(&Ok(vec![value.clone()])));
    assert_eq!(
        state.rows[0].finding,
        "Availability: 97% (objective: 99%) (+1 more)"
    );
    assert_eq!(state.rows[0].image, "—", "Coroot knew no image");
}

#[test]
fn coroots_404_for_a_window_is_no_data_with_no_charts() {
    let answer = api::RevisionView {
        revision: revision("9c41e7", 1_789_999_000),
        from: chrono::DateTime::from_timestamp(1_789_997_200, 0).unwrap(),
        to: chrono::DateTime::from_timestamp(1_790_000_800, 0).unwrap(),
        answer: api::Around::NoData,
    };
    let detail = detail::prepare(&answer, "CPU");
    assert!(detail.charts().is_empty());
    assert_eq!(detail.report, "CPU", "the report chosen stays for the next");
}

#[test]
fn the_examples_windows_answer_as_coroot_would() {
    let app = crate::example::id(crate::example::WORKER);
    let revisions = app_example::revisions();
    let window =
        |ix: usize| api::RevisionWindow::both(std::time::Duration::from_secs(WINDOWS[ix].0));
    let newest = app_example::around(&app, &revisions[0], window(0)).unwrap();
    let api::Around::View(_) = &newest.answer else {
        panic!("the newest revision has data");
    };
    let detail = detail::prepare(&newest, "");
    assert!(!detail.charts().is_empty(), "the first report with charts");
    let oldest = app_example::around(&app, &revisions[2], window(2)).unwrap();
    assert!(matches!(oldest.answer, api::Around::NoData));
}

mod ui_tests {
    use super::super::WINDOWS;
    use crate::{Destination, tests::mount_size};
    use freshkube_core::coroot as api;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext};

    type Page = Entity<crate::ObservabilityPage>;

    fn open_example(
        cx: &mut TestAppContext,
        width: f32,
        height: f32,
    ) -> (tokio::runtime::Runtime, AnyWindowHandle, Page) {
        let (runtime, handle, page) = mount_size(cx, true, width, height);
        cx.update(|cx| page.update(cx, |page, cx| page.open(Destination::Deployments, cx)));
        cx.run_until_parked();
        (runtime, handle, page)
    }

    fn row_id(cx: &mut TestAppContext, page: &Page, ix: usize) -> SharedString {
        cx.read(|cx| page.read(cx).revision_observations.rows[ix].id.clone())
    }

    #[gpui_kit::test]
    fn the_newest_revision_opens_with_its_charts_split_at_its_start(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = open_example(cx, 1260., 900.);
        let first = row_id(cx, &page, 0);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(page.read(cx).revision_line_count(), 3);
            assert!(window.find(first.clone()).visible());
            assert!(window.find("obs-revision-detail").visible());
            assert!(window.find("obs-revision-note").visible(), "Coroot's note");
            assert!(window.find("obs-revision-window").visible());
            // The rollout began minutes ago, so the window runs past now.
            assert!(window.find("obs-revision-ends-early").visible());
            let state = &page.read(cx).revision_observations;
            let detail = state.detail.as_ref().expect("the window answered");
            let charts = detail.charts();
            assert!(!charts.is_empty());
            let sides = format!("{}-sides", charts[0].id);
            assert!(window.find(sides).visible(), "the samples on each side");
            assert!(window.try_find("obs-revision-no-data").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn another_revision_shows_its_findings_or_coroots_no_data(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = open_example(cx, 1260., 900.);
        let (second, oldest) = (row_id(cx, &page, 1), row_id(cx, &page, 2));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(second.clone(), cx);
            window.render_frame(cx);
            assert!(window.find("obs-revision-finding-0").visible());
            assert!(window.find("obs-revision-finding-1").visible());
            assert!(window.try_find("obs-revision-note").is_none());
            assert!(window.try_find("obs-revision-ends-early").is_none());

            // Older than the example keeps: Coroot's 404 for the window.
            window.click(oldest.clone(), cx);
            window.render_frame(cx);
            assert!(window.find("obs-revision-no-data").visible());
            assert!(window.try_find("obs-revision-failed").is_none());
            assert!(page.read(cx).revision_charts().is_empty());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_window_and_the_report_choose_what_the_inspector_reads(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = open_example(cx, 1260., 900.);
        let second = row_id(cx, &page, 1);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(second.clone(), cx);
            window.render_frame(cx);
            let from = |page: &crate::ObservabilityPage| page.revision_answer().unwrap().from;
            let started = page
                .read(cx)
                .revision_observations
                .selected()
                .unwrap()
                .revision
                .started_at;
            assert_eq!(
                (started - from(page.read(cx))).num_seconds(),
                WINDOWS[0].0 as i64
            );
            window.click("obs-revision-window-1", cx);
            window.render_frame(cx);
            assert_eq!(
                (started - from(page.read(cx))).num_seconds(),
                WINDOWS[1].0 as i64
            );

            let report = page.read(cx).revision_observations.report.clone();
            let other = if report == "CPU" { "Memory" } else { "CPU" };
            let id = format!("obs-revision-report-{}", other.to_lowercase());
            window.click(id, cx);
            window.render_frame(cx);
            let state = &page.read(cx).revision_observations;
            assert_eq!(state.report, other);
            let slug = format!("obs-chart-revision-{}-", other.to_lowercase());
            assert!(
                page.read(cx)
                    .revision_charts()
                    .iter()
                    .all(|c| c.id.starts_with(&slug))
            );

            // Another revision keeps the window and the report.
            let first = state.rows[0].id.clone();
            window.click(first, cx);
            window.render_frame(cx);
            let state = &page.read(cx).revision_observations;
            assert_eq!((state.window, state.report.as_str()), (1, other));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn without_revisions_the_page_says_why(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = open_example(cx, 1260., 900.);
        let statefulset = api::AppId::new("fixture:payments:StatefulSet:ledger-db");
        let deployment = crate::example::id("payments/api");
        for (app, kind) in [
            (None, "none"),
            (Some(statefulset), "StatefulSet"),
            (Some(deployment), "Deployment"),
        ] {
            cx.update(|cx| {
                page.update(cx, |page, cx| {
                    page.selected_app = app;
                    page.refresh(cx);
                })
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("obs-deployments-empty").visible(), "{kind}");
                assert!(window.try_find("obs-revision-detail").is_none(), "{kind}");
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn deployments_use_the_pods_frame_and_table_at_both_text_sizes(cx: &mut TestAppContext) {
        use freshkube_ui::layout_check::{
            PageFrame, Table, assert_bare, assert_edge_frame, assert_inspector, assert_table,
        };
        let (_runtime, handle, _page) = open_example(cx, 1260., 900.);
        let frame = PageFrame {
            page: "obs-frame",
            title: "obs-title",
            title_text: "Deployments",
            content: "obs-deployments-split",
        };
        let table = Table {
            table: Some("obs-deployments-table-scroll"),
            list: "obs-deployments-list",
        };
        for text_size in [crate::ui::BASE_TEXT, 20.] {
            cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
                .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_edge_frame(window, cx, &frame);
                let rows = assert_table(window, cx, &table);
                assert!(rows.header.is_some(), "{rows:#?}");
                assert_bare(window, "obs-deployments-table");
                assert_inspector(
                    window,
                    cx,
                    "obs-deployments-split",
                    "obs-deployments-table",
                    "obs-revision-detail",
                    "obs-revision-title",
                );
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn a_narrow_window_stacks_the_inspector_under_the_table(cx: &mut TestAppContext) {
        let (_runtime, handle, _page) = open_example(cx, 760., 560.);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let table = window.find("obs-deployments-table").bounds();
            let detail = window.find("obs-revision-detail").bounds();
            assert!(detail.top() >= table.bottom() - gpui_kit::px(1.));
        })
        .unwrap();
    }
}
