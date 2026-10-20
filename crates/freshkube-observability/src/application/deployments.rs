//! Revision navigation keeps the application's observation window and focuses
//! by the start second, including when the start is outside that window.
use super::*;

pub(crate) struct FocusedRevision {
    pub app: api::AppId,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl ObservabilityPage {
    pub(crate) fn show_revision_on_charts(
        &mut self,
        app: &api::AppId,
        revision: &api::DeploymentRevision,
        cx: &mut Context<Self>,
    ) {
        if self.selected_app.as_ref() != Some(app) {
            return;
        }
        // A report button belongs to the answer that drew it. A replaced
        // answer or application must not navigate using that old revision.
        let current = match self.destination {
            Destination::Deployments => self.revision_is_listed(revision),
            Destination::Application => self.app_page.as_ref().is_some_and(|page| {
                page.revisions
                    .iter()
                    .any(|r| r.id == revision.id && r.started_at == revision.started_at)
            }),
            _ => false,
        };
        if !current {
            return;
        }
        self.forget_wanted();
        self.focused_revision = Some(FocusedRevision {
            app: app.clone(),
            started_at: revision.started_at,
        });
        self.report_name = revision
            .findings
            .first()
            .map_or("SLO", |finding| finding.report.as_str())
            .to_owned();
        self.open(Destination::Application, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{AppPage, Destination, Report, api, example, format};
    use crate::tests::mount;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn first_show_on_charts_after_opening_keeps_focus_across_a_clock_tick(cx: &mut TestAppContext) {
        let _clock = example::with_clock(cx.background_executor.clone());
        let (_runtime, handle, page) = mount(cx, true);
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                // Open directly on Deployments, without visiting another report.
                page.selected_app = Some(crate::example::id(crate::example::WORKER));
                page.report_name = "Deployments".into();
                page.open(Destination::Application, cx);
            })
        });
        cx.run_until_parked();
        let revision = page.read_with(cx, |page, _| {
            page.app_page.as_ref().unwrap().revisions[0].clone()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        // The click and the new report read happen on the next clock minute.
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(60));
        cx.update_window(handle, |_, window, cx| {
            window.click(format!("obs-show-on-charts-{}", revision.id), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let page = page.read(cx);
            assert_eq!(page.report_name, "SLO");
            assert_eq!(
                page.focused_revision.as_ref().unwrap().started_at,
                revision.started_at
            );
            assert!(!page.app_charts.is_empty());
            for chart in &page.app_charts {
                assert_eq!(
                    chart.view().read(cx).focused_marker_labels().len(),
                    1,
                    "the first click must focus the revision on every chart"
                );
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn report_button_opens_the_first_findings_report_with_its_marker(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount(cx, true);
        let app = crate::example::id(crate::example::WORKER);
        let mut view =
            cx.update(|cx| page.update(cx, |page, _| page.example_views.get(&app).clone()));
        let revisions = view.revisions.as_mut().unwrap().as_mut().unwrap();
        revisions[0].findings.push(api::RevisionFinding {
            report: "CPU".into(),
            message: "CPU usage: no change".into(),
            ok: true,
            time: None,
        });
        let revision = revisions[0].clone();
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.open_app(app.clone(), Report::Cpu, cx);
                page.report_name = "Deployments".into();
                page.app_page = Some(AppPage::new(&view, "Deployments"));
                cx.notify();
            })
        });
        cx.run_until_parked();
        let range = page.read_with(cx, |page, _| page.live.range);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(format!("obs-show-on-charts-{}", revision.id), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let page = page.read(cx);
            assert_eq!(page.destination, Destination::Application);
            assert_eq!(page.report_name, "CPU");
            assert_eq!(page.selected_app.as_ref(), Some(&app));
            assert_eq!(page.live.range, range);
            assert_eq!(
                page.focused_revision.as_ref().unwrap().started_at,
                revision.started_at
            );
            assert!(!page.app_charts.is_empty());
            for chart in &page.app_charts {
                assert_eq!(chart.view().read(cx).focused_marker_labels().len(), 1);
            }
            assert!(
                window
                    .try_find("obs-focused-revision-before-window")
                    .is_none()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn deployments_report_keeps_another_table_widget(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount(cx, true);
        let app = crate::example::id(crate::example::WORKER);
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                let mut view = page.example_views.get(&app).clone();
                let report = view
                    .reports
                    .iter_mut()
                    .find(|r| r.name == "Deployments")
                    .unwrap();
                report.widgets.insert(
                    0,
                    api::Widget {
                        width: 1.,
                        kind: api::WidgetKind::Table(api::Table {
                            header: vec!["Instance".into(), "State".into()],
                            rows: vec![vec![
                                api::Cell {
                                    value: "checkout-api".into(),
                                    ..Default::default()
                                },
                                api::Cell {
                                    value: "Ready".into(),
                                    ..Default::default()
                                },
                            ]],
                        }),
                    },
                );
                page.selected_app = Some(app);
                page.report_name = "Deployments".into();
                page.open(Destination::Application, cx);
                page.app_page = Some(AppPage::new(&view, "Deployments"));
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                page.read(cx).app_tables.len(),
                2,
                "only the revisions table is replaced"
            );
            assert!(window.find("obs-app-table-1-row-0").visible());
            assert_eq!(
                window.find("obs-app-table-1-row-0").label(),
                Some("checkout-api")
            );
            assert!(window.try_find("obs-app-table-2-list").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn old_revision_opens_slo_with_its_time_and_keeps_the_window(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount(cx, true);
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.open_app(crate::example::id(crate::example::WORKER), Report::Cpu, cx);
                page.select_report("Deployments".into(), cx);
            })
        });
        cx.run_until_parked();
        let (revision, range) = page.read_with(cx, |page, _| {
            (
                page.app_page.as_ref().unwrap().revisions[2].clone(),
                page.live.range,
            )
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(format!("obs-show-on-charts-{}", revision.id), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let page = page.read(cx);
            assert_eq!(page.destination, Destination::Application);
            assert_eq!(page.report_name, "SLO");
            assert_eq!(page.live.range, range);
            assert!(window.find("obs-focused-revision-before-window").visible());
            assert_eq!(
                page.app_page
                    .as_ref()
                    .unwrap()
                    .focus_note
                    .as_ref()
                    .unwrap()
                    .as_ref(),
                format!(
                    "Started {}, before this 3 h window",
                    format::local_time(revision.started_at)
                )
            );
            assert!(
                page.app_charts.iter().all(|chart| chart
                    .view()
                    .read(cx)
                    .focused_marker_labels()
                    .is_empty())
            );
        })
        .unwrap();
        for (report, shown) in [("Deployments", false), ("CPU", true)] {
            cx.update(|cx| page.update(cx, |page, cx| page.select_report(report.into(), cx)));
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(
                    window
                        .try_find("obs-focused-revision-before-window")
                        .is_some(),
                    shown,
                    "the before-window note belongs only to reports with charts"
                );
                assert!(
                    page.read(cx).focused_revision.is_some(),
                    "switching reports keeps the focus"
                );
            })
            .unwrap();
        }
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.choose_app(crate::example::id(crate::example::CHECKOUT), cx);
            })
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(page.read(cx).focused_revision.is_none());
            assert!(
                window
                    .try_find("obs-focused-revision-before-window")
                    .is_none()
            );
        })
        .unwrap();
    }
}
