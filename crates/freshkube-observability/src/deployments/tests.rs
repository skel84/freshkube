use super::{Revisions, WINDOWS, detail, image, short_image};
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
    // Read again, as Refresh or coming back reads it, the same
    // application keeps its rows and the older revision until it answers.
    assert!(!state.prepare_list(Some(&app), None));
    assert_eq!(state.selected, Some(TableKey::Revision(b.id.clone())));
    assert_eq!(state.rows.len(), 2);
    assert!(state.answered);
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
    // The list shows each image's name and tag; the tooltip and the
    // inspector the whole reference.
    assert_eq!(state.rows[0].short_image, "auth:1.4.0");
    assert_eq!(state.rows[0].image, "example.test/shop/auth:1.4.0");

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
    assert_eq!(state.rows[0].short_image, "—");
}

#[test]
fn a_short_image_is_each_references_last_segment_and_tag() {
    assert_eq!(
        short_image("example.test/payments/worker:1.8.2"),
        "worker:1.8.2"
    );
    assert_eq!(
        short_image("example.test/shop/api:1.4.0, example.test/shop/proxy:2.1"),
        "api:1.4.0, proxy:2.1"
    );
    assert_eq!(
        short_image("example.test:5000/a/b@sha256:ab12"),
        "b@sha256:ab12"
    );
    assert_eq!(short_image("nginx:1.27"), "nginx:1.27");
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
            assert!(page.read(cx).revision_observations.charts().is_empty());
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
                    .revision_observations
                    .charts()
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
    fn the_inspector_sits_beside_the_table_or_under_it_when_narrow(cx: &mut TestAppContext) {
        use freshkube_ui::layout_check::assert_inspector;
        for (width, height, beside) in [(1260., 900., true), (760., 560., false)] {
            let (_runtime, handle, _page) = open_example(cx, width, height);
            cx.update_window(handle, |_, window, cx| {
                assert_inspector(
                    window,
                    cx,
                    "obs-deployments-split",
                    "obs-deployments-table",
                    "obs-revision-detail",
                    "obs-revision-title",
                );
                let table = window.find("obs-deployments-table").bounds();
                let detail = window.find("obs-revision-detail").bounds();
                assert_eq!(detail.left() > table.left(), beside, "{width}");
            })
            .unwrap();
        }
    }
}

mod wanted {
    use super::super::{Outcome, RevisionLink};
    use crate::tests::mount;
    use crate::{Destination, ObservabilityPage};
    use freshkube_core::coroot as api;
    use freshkube_core::delivery::change::example::POD_HASH;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};

    const ACCESS: &str = "example-connection";

    fn link(name: &str, hash: &str) -> RevisionLink {
        RevisionLink {
            access: ACCESS.into(),
            cluster: "dev-fra".into(),
            namespace: "checkout".into(),
            name: name.into(),
            hash: hash.into(),
        }
    }

    /// The example page on the shell's connection, asked for `link`.
    fn asked(
        cx: &mut TestAppContext,
        link: RevisionLink,
    ) -> (
        tokio::runtime::Runtime,
        AnyWindowHandle,
        Entity<ObservabilityPage>,
    ) {
        let (runtime, handle, page) = mount(cx, true);
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.set_source(Some(ACCESS.into()), cx);
                page.open_revision(link, cx);
            })
        });
        cx.run_until_parked();
        (runtime, handle, page)
    }

    fn outcome(cx: &mut TestAppContext, page: &Entity<ObservabilityPage>) -> Option<Outcome> {
        cx.read(|cx| page.read(cx).revision_observations.wanted_outcome())
    }

    fn selected_hash(cx: &mut TestAppContext, page: &Entity<ObservabilityPage>) -> Option<String> {
        cx.read(|cx| {
            let state = &page.read(cx).revision_observations;
            state.selected().map(|row| row.revision.hash.clone())
        })
    }

    #[gpui_kit::test]
    fn the_change_pages_revision_opens_on_deployments_by_its_hash(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = asked(cx, link("checkout-api", "84c6d7f9b"));
        cx.read(|cx| {
            let page = page.read(cx);
            assert_eq!(page.destination, Destination::Deployments);
            assert_eq!(
                page.selected_app,
                Some(api::AppId::new("fixture:checkout:Deployment:checkout-api"))
            );
        });
        assert_eq!(outcome(cx, &page), Some(Outcome::Found));
        assert_eq!(
            selected_hash(cx, &page).as_deref(),
            Some("84c6d7f9b"),
            "the hash's revision, not the newest"
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-wanted-revision").visible());
            assert!(window.find("obs-revision-detail").visible());
            // Choosing another revision is the user's; the banner goes.
            let newest = page.read(cx).revision_observations.rows[0].id.clone();
            window.click(newest, cx);
            window.render_frame(cx);
            assert!(window.try_find("obs-wanted-revision").is_none());
        })
        .unwrap();
        assert_eq!(selected_hash(cx, &page).as_deref(), Some(POD_HASH));
    }

    #[gpui_kit::test]
    fn a_hash_coroot_doesnt_keep_says_so_over_the_newest(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = asked(cx, link("checkout-api", "0a1b2c3d4"));
        assert_eq!(outcome(cx, &page), Some(Outcome::NoRevision));
        assert_eq!(selected_hash(cx, &page).as_deref(), Some(POD_HASH));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-wanted-revision").visible());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn an_application_coroot_doesnt_list_is_not_chosen(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = asked(cx, link("checkout-web", POD_HASH));
        assert_eq!(outcome(cx, &page), Some(Outcome::NotListed));
        cx.read(|cx| {
            assert_ne!(
                page.read(cx).selected_app,
                Some(api::AppId::new("fixture:checkout:Deployment:checkout-web"))
            );
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("obs-wanted-revision").visible());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_link_of_another_connection_is_dropped(cx: &mut TestAppContext) {
        let mut other = link("checkout-api", POD_HASH);
        other.access = "another-connection".into();
        let (_runtime, handle, page) = asked(cx, other);
        assert_eq!(outcome(cx, &page), None);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("obs-wanted-revision").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn choosing_another_application_forgets_the_revision_asked_for(cx: &mut TestAppContext) {
        let (_runtime, _handle, page) = asked(cx, link("checkout-api", "84c6d7f9b"));
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.choose_app(crate::example::id(crate::example::WORKER), cx)
            })
        });
        assert_eq!(outcome(cx, &page), None);
    }
}
