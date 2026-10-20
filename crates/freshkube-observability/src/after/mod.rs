//! What Coroot found after a deploy, for the change page's Deployment hop
//! (`docs/COROOT.md#what-its-metrics-did-after-it`): the revision, Coroot's
//! status and findings in its words, and the charts of the one report its
//! first finding names, split at the revision's start.
//!
//! Nothing is computed across the split. A chart says how many samples
//! fall on each side, and Coroot is the only one that compares, in its
//! findings. [`After`] is made from an answer that was read, and draws
//! only that.
mod reader;

#[cfg(any(test, feature = "testing"))]
pub use reader::Script;
pub use reader::{OutcomeReader, ReadConnection};

use super::application::{AppPage, charts};
use super::*;
use crate::monitoring::panel::PanelView;
use chrono::{DateTime, Utc};
use freshkube_core::coroot as api;

/// The most charts the change page draws; Compare in Observability has the
/// rest.
const MAX_CHARTS: usize = 4;

/// The report Coroot's findings fall back to when it sent none.
const FALLBACK_REPORT: &str = "SLO";

/// A revision's window, prepared for the change page.
pub struct After {
    hash: SharedString,
    /// `Revision 7d4b9c · started 14:05 (12 min ago) · window ±30 min`.
    revision: SharedString,
    status: Status,
    findings: Vec<Finding>,
    /// Coroot's explanation when it sent no findings.
    note: SharedString,
    /// The report whose charts show.
    report: SharedString,
    shown: Vec<Shown>,
    /// How many of the report's charts are left to Compare.
    more: usize,
    /// Where Coroot's data stops, when it stops before the window's end.
    ends_early: Option<SharedString>,
    /// Why some charts show no points.
    history_note: Option<SharedString>,
}

struct Finding {
    report: SharedString,
    message: SharedString,
    ok: bool,
}

struct Shown {
    charts: Rc<charts::Charts>,
    shown: usize,
    view: Option<Entity<PanelView>>,
}

impl After {
    /// Prepares `view`'s answer. `ends_at` is where Coroot's data stops when
    /// that is before the window's end. The window is the default, 30
    /// minutes on each side.
    pub fn new(
        view: &api::RevisionView,
        ends_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        cx: &mut App,
    ) -> Self {
        let revision = &view.revision;
        let started = format::local_time(revision.started_at);
        let age = format::ago(now - revision.started_at);
        let window = api::RevisionWindow::default();
        let findings: Vec<Finding> = revision
            .findings
            .iter()
            .map(|f| Finding {
                report: f.report.clone().into(),
                message: f.message.clone().into(),
                ok: f.ok,
            })
            .collect();
        let (report, mut shown, more, history_note) = match &view.answer {
            api::Around::View(page) => {
                let named = revision
                    .findings
                    .first()
                    .map_or(FALLBACK_REPORT, |f| f.report.as_str());
                let with_charts = |r: &&api::AppReport| {
                    r.widgets.iter().any(|w| {
                        matches!(
                            w.kind,
                            api::WidgetKind::Chart(_) | api::WidgetKind::ChartGroup { .. }
                        )
                    })
                };
                let report = page
                    .reports
                    .iter()
                    .filter(with_charts)
                    .find(|r| r.name == named)
                    .or_else(|| page.reports.iter().find(with_charts));
                let all = report.map_or_else(Vec::new, |report| {
                    AppPage::revision_charts(page, report, revision)
                });
                let more = all.len().saturating_sub(MAX_CHARTS);
                let shown: Vec<Shown> = all
                    .into_iter()
                    .take(MAX_CHARTS)
                    .map(|charts| {
                        let ix = charts.featured();
                        Shown {
                            charts,
                            shown: ix,
                            view: None,
                        }
                    })
                    .collect();
                (
                    report.map_or_else(|| named.to_owned(), |r| r.name.clone()),
                    shown,
                    more,
                    page.history_error.map(|error| {
                        SharedString::from(format!(
                            "Some charts show no points: Coroot's chart histories couldn't be read. {error}"
                        ))
                    }),
                )
            }
            api::Around::NoData => (FALLBACK_REPORT.to_owned(), Vec::new(), 0, None),
        };
        for shown in &mut shown {
            shown.view = shown.charts.panel(shown.shown).map(|panel| {
                let id = shown.charts.id.clone();
                cx.new(|cx| {
                    let mut view = PanelView::new(id, Rc::new(panel.spec.clone()), cx);
                    view.set_result(panel.result.clone(), panel.window, cx);
                    view.set_markers(panel.markers.clone().into(), cx);
                    view
                })
            });
        }
        Self {
            hash: revision.hash.clone().into(),
            revision: format!(
                "Revision {} · started {started} ({age}) · window ±{} min",
                revision.hash,
                window.after.as_secs() / 60
            )
            .into(),
            status: Status::from(revision.status),
            findings,
            note: revision
                .note
                .clone()
                .unwrap_or_else(|| "Coroot sent no findings.".into())
                .into(),
            report: report.into(),
            shown,
            more,
            ends_early: ends_at.map(|end| {
                format!(
                    "Coroot's data ends at {}, before the window's end.",
                    format::local_time(end)
                )
                .into()
            }),
            history_note,
        }
    }

    /// Coroot's pod-template hash of the revision.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Coroot's status, as a word.
    pub fn status_word(&self) -> &'static str {
        self.status.label()
    }

    /// Coroot's status as a glyph's tone.
    pub fn tone(&self) -> Tone {
        self.status.tone()
    }

    /// Coroot's first finding, as it worded it.
    pub fn first_finding(&self) -> Option<&str> {
        self.findings.first().map(|f| f.message.as_ref())
    }

    /// The report whose charts show.
    pub fn report(&self) -> &str {
        &self.report
    }

    /// The revision line, for tests.
    pub fn revision_line(&self) -> &str {
        &self.revision
    }

    /// How many charts show, for tests.
    pub fn chart_count(&self) -> usize {
        self.shown.len()
    }

    /// The sample counts under each chart, for tests.
    pub fn sides(&self) -> Vec<String> {
        self.shown
            .iter()
            .filter_map(|shown| shown.charts.sides(shown.shown))
            .map(|s| s.to_string())
            .collect()
    }

    /// The window's note when Coroot's data stops early, for tests.
    pub fn ends_early(&self) -> Option<&str> {
        self.ends_early.as_deref()
    }

    /// The section's body for a found revision, ids under `id`.
    pub fn render(&self, id: &str, cx: &App) -> Div {
        let findings = self.findings.iter().enumerate().map(|(ix, f)| {
            line()
                .id(SharedString::from(format!("{id}-finding-{ix}")))
                .test_support()
                .items_start()
                .child(h_flex().h(dp(20.)).flex_none().child(view::status(
                    if f.ok { Status::Ok } else { Status::Warning },
                    cx,
                )))
                .child(muted(f.report.clone(), cx).w(dp(72.)).flex_none())
                .child(text(f.message.clone()).flex_1().whitespace_normal())
        });
        let note = self.findings.is_empty().then(|| {
            text(self.note.clone())
                .id(SharedString::from(format!("{id}-note")))
                .test_support()
                .whitespace_normal()
        });
        let charts = self.shown.iter().map(|shown| {
            charts::chart_element(&shown.charts, shown.shown, shown.view.as_ref(), cx)
        });
        let notes = [
            self.ends_early.as_ref().map(|n| (n, "ends-early")),
            self.history_note.as_ref().map(|n| (n, "history")),
        ]
        .into_iter()
        .flatten()
        .map(|(note, which)| {
            muted(note.clone(), cx)
                .id(SharedString::from(format!("{id}-{which}")))
                .test_support()
                .role(Role::Status)
                .whitespace_normal()
        });
        v_flex()
            .min_w_0()
            .gap(dp(12.))
            .child(
                line()
                    .items_start()
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-status")))
                            .test_support()
                            .child(ui::tag(self.status.tone(), None, self.status.label(), cx)),
                    )
                    .child(
                        muted(self.revision.clone(), cx)
                            .id(SharedString::from(format!("{id}-revision")))
                            .test_support()
                            .flex_1()
                            .whitespace_normal(),
                    ),
            )
            .child(v_flex().gap(dp(12.)).children(findings).children(note))
            .child(
                muted(
                    "Coroot compares a revision with the last earlier one it kept metrics for, without saying which.",
                    cx,
                )
                .whitespace_normal(),
            )
            .children(notes)
            .when(self.shown.is_empty(), |this| {
                this.child(
                    muted("Coroot sent no charts for this window.", cx)
                        .id(SharedString::from(format!("{id}-no-charts")))
                        .test_support()
                        .whitespace_normal(),
                )
            })
            .children(charts)
            .when(self.more > 0, |this| {
                this.child(
                    muted(
                        format!(
                            "{} more chart{} of the {} report: Compare in Observability.",
                            self.more,
                            if self.more == 1 { "" } else { "s" },
                            self.report
                        ),
                        cx,
                    )
                    .id(SharedString::from(format!("{id}-more")))
                    .test_support()
                    .whitespace_normal(),
                )
            })
    }
}

/// Example data's answers: Coroot around the revisions of acme's
/// `checkout-api`.
pub mod example {
    use super::*;

    /// The window around `checkout-api`'s revision with `hash`, as the
    /// Deployments example answers it, or `None` for a hash the example
    /// keeps no revision of.
    pub fn around(hash: &str) -> Option<api::RevisionView> {
        let app = crate::example::id(crate::example::CHECKOUT);
        let revisions = crate::application::example::checkout_revisions();
        let revision = revisions.iter().find(|revision| revision.hash == hash)?;
        // Coroot's example keeps charts for the worker alone, so the window
        // around checkout's revision borrows the worker's charts and is
        // otherwise checkout's page.
        let worker = crate::example::id(crate::example::WORKER);
        let mut view = crate::application::example::around(
            &crate::application::example::app_view(&worker),
            revision,
            api::RevisionWindow::default(),
        )
        .ok()?;
        if let api::Around::View(page) = &mut view.answer {
            page.map.app.id = app;
            page.revisions = Some(Ok(revisions.clone()));
        }
        Some(view)
    }

    /// The hash of the revision of `checkout-api` that started long enough
    /// ago for the whole window to be in Coroot's data.
    pub const OLDER_HASH: &str = "84c6d7f9b";
}

#[cfg(test)]
mod window_tests {
    use super::example;
    use freshkube_core::coroot as api;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn examples_window_has_histories(_cx: &mut TestAppContext) {
        for hash in ["5d8f7c9b6", example::OLDER_HASH] {
            let view = example::around(hash).expect("kept");
            let api::Around::View(page) = &view.answer else {
                panic!("{hash}: no data");
            };
            let n: usize = page
                .reports
                .iter()
                .flat_map(|r| &r.widgets)
                .map(|w| match &w.kind {
                    api::WidgetKind::Chart(c) => c.history.is_some() as usize,
                    api::WidgetKind::ChartGroup { charts, .. } => {
                        charts.iter().filter(|c| c.history.is_some()).count()
                    }
                    _ => 0,
                })
                .sum();
            assert!(
                n > 0,
                "{hash}: {} reports, no histories",
                page.reports.len()
            );
            let outcome = api::Outcome::of_window(view);
            assert!(
                matches!(
                    outcome,
                    api::Outcome::Found(_) | api::Outcome::EndsEarly(..)
                ),
                "{hash}: {outcome:?}"
            );
        }
    }
}
