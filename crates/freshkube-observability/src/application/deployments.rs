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
    fn report_button_opens_the_first_findings_report_with_its_marker(cx: &mut TestAppContext) {
        let (_runtime, handle, page) = mount(cx, true);
        let app = crate::example::id(crate::example::WORKER);
        let mut view = example::app_view(&app);
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
