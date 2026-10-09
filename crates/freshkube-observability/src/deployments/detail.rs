//! The selected revision in the inspector: Coroot's findings on it, then
//! the application's charts around its start, prepared when the window's
//! answer arrives or another report is chosen.
use super::*;

pub(super) struct Detail {
    /// The report whose charts show: the one chosen, else the first with
    /// charts.
    pub(super) report: String,
    /// The window read, in local time.
    window: SharedString,
    answer: Answer,
}

enum Answer {
    /// Coroot's 404 for the window: no metrics of the application in it.
    NoData,
    Page {
        /// The reports that have charts, in Coroot's order.
        tabs: Vec<Tab>,
        charts: Vec<Rc<Charts>>,
        /// Where Coroot's data stops short of the window's end.
        ends_early: Option<SharedString>,
        /// Why some charts show no points.
        history_note: Option<SharedString>,
    },
}

struct Tab {
    name: String,
    id: SharedString,
    status: Option<Status>,
    tooltip: SharedString,
}

fn has_charts(report: &api::AppReport) -> bool {
    report.widgets.iter().any(|w| {
        matches!(
            w.kind,
            api::WidgetKind::Chart(_) | api::WidgetKind::ChartGroup { .. }
        )
    })
}

/// Every chart's history in the page, for where Coroot's data ends.
fn histories(view: &api::AppView) -> impl Iterator<Item = &api::ChartHistory> {
    view.reports
        .iter()
        .flat_map(|r| &r.widgets)
        .flat_map(|w| match &w.kind {
            api::WidgetKind::Chart(chart) => std::slice::from_ref(chart),
            api::WidgetKind::ChartGroup { charts, .. } => charts.as_slice(),
            _ => &[],
        })
        .filter_map(|chart| chart.history.as_deref())
}

pub(super) fn prepare(answer: &api::RevisionView, chosen: &str) -> Detail {
    let window = format!(
        "From {} to {}",
        format::local_time(answer.from),
        format::local_time(answer.to)
    )
    .into();
    let api::Around::View(view) = &answer.answer else {
        return Detail {
            report: chosen.to_owned(),
            window,
            answer: Answer::NoData,
        };
    };
    let with_charts: Vec<_> = view.reports.iter().filter(|r| has_charts(r)).collect();
    let report = with_charts
        .iter()
        .find(|r| r.name == chosen)
        .or(with_charts.first())
        .copied();
    let tabs = with_charts
        .iter()
        .map(|r| {
            let status = r.judged().then(|| Status::from(r.status));
            Tab {
                name: r.name.clone(),
                id: format!(
                    "obs-revision-report-{}",
                    r.name.to_lowercase().replace(' ', "-")
                )
                .into(),
                status,
                tooltip: match status {
                    Some(status) => format!("{} · {}", r.name, status.label()),
                    None => r.name.clone(),
                }
                .into(),
            }
        })
        .collect();
    let charts = report.map_or_else(Vec::new, |report| {
        AppPage::revision_charts(view, report, &answer.revision)
    });
    // Said once for the window: every chart stops where Coroot's data does.
    let ends_early = histories(view)
        .map(|history| history.to)
        .max()
        .filter(|end| *end < answer.to)
        .map(|end| {
            format!(
                "Coroot's data ends at {}, before the window's end.",
                format::local_time(end)
            )
            .into()
        });
    Detail {
        report: report.map_or_else(|| chosen.to_owned(), |r| r.name.clone()),
        window,
        answer: Answer::Page {
            tabs,
            charts,
            ends_early,
            history_note: view.history_error.map(|error| {
                format!(
                    "Some charts show no points: Coroot's chart histories couldn't be read. {error}"
                )
                .into()
            }),
        },
    }
}

impl Detail {
    pub(super) fn charts(&self) -> &[Rc<Charts>] {
        match &self.answer {
            Answer::Page { charts, .. } => charts,
            Answer::NoData => &[],
        }
    }
}

/// One part of the inspector, under a hairline and its caption.
fn section(title: &str, cx: &App) -> Div {
    v_flex()
        .min_w_0()
        .gap(dp(12.))
        .pt(dp(12.))
        .border_t_1()
        .border_color(palette(cx).line)
        .child(ui::caption(title, cx))
}

/// Lines of one part, without padding: the inspector pads its body.
fn part() -> Div {
    v_flex().gap(dp(12.)).min_w_0()
}

impl ObservabilityPage {
    /// The selected revision in the inspector. Nothing while the list has
    /// no revision.
    pub(super) fn render_revision_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.revision_rows().is_empty() {
            return None;
        }
        let row = self.revision_observations.selected()?;
        let revision = &row.revision;
        Some(
            Inspector::new("obs-revision-detail")
                .heading(self.revision_heading(row, cx))
                .child(
                    part()
                        // The title has the hash; Coroot's label adds the images.
                        .child(
                            mono(row.image.clone())
                                .id("obs-revision-version")
                                .test_support()
                                .whitespace_normal(),
                        )
                        .child(pair("Started", row.started.clone(), cx)),
                )
                .child(self.render_findings(revision, cx))
                .child(self.render_around(cx))
                .render(cx)
                .into_any_element(),
        )
    }

    fn revision_heading(&self, row: &Row, cx: &Context<Self>) -> Div {
        let read = if self.fixture {
            None
        } else if self.live.revision.is_loading() {
            Some((Tone::Unknown, "Reading"))
        } else if self.live.revision.is_stale() {
            Some((Tone::Warn, "Stale"))
        } else {
            None
        };
        h_flex()
            .flex_1()
            .min_w_0()
            .items_start()
            .gap(dp(8.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(dp(2.))
                    .child(ui::caption("Revision", cx))
                    .child(
                        div()
                            .id("obs-revision-title")
                            .test_support()
                            .font_family(MONO_FONT)
                            .text_size(dp(13.5))
                            .truncate()
                            .child(row.hash.clone()),
                    ),
            )
            .child(
                div()
                    .id("obs-revision-status")
                    .test_support()
                    .mt(dp(14.))
                    .child(ui::tag(row.status.tone(), None, row.status.label(), cx)),
            )
            .children(read.map(|(tone, text)| {
                div()
                    .id("obs-revision-read")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(text)
                    .mt(dp(14.))
                    .child(ui::tag(tone, None, text, cx))
            }))
    }

    /// What Coroot found on the revision, in its words.
    fn render_findings(&self, revision: &api::DeploymentRevision, cx: &Context<Self>) -> Div {
        let findings = revision.findings.iter().enumerate().map(|(ix, f)| {
            line()
                .id(SharedString::from(format!("obs-revision-finding-{ix}")))
                .test_support()
                .items_start()
                .child(
                    h_flex()
                        .h(dp(20.))
                        .flex_none()
                        .child(status(if f.ok { Status::Ok } else { Status::Warning }, cx)),
                )
                .child(muted(f.report.clone(), cx).w(dp(72.)).flex_none())
                .child(text(f.message.clone()).flex_1().whitespace_normal())
        });
        let note = revision.findings.is_empty().then(|| {
            text(
                revision
                    .note
                    .clone()
                    .unwrap_or_else(|| "Coroot sent no findings.".into()),
            )
            .id("obs-revision-note")
            .test_support()
            .whitespace_normal()
        });
        section("Coroot's findings", cx)
            .child(part().children(findings).children(note))
            .child(
                muted(
                    "Coroot compares a revision with the last earlier one it kept metrics for, without saying which.",
                    cx,
                )
                .whitespace_normal(),
            )
    }

    /// The window picker, then the charts around the start, or why there
    /// are none.
    fn render_around(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.revision_observations;
        let picker = line()
            .flex_wrap()
            .gap(dp(2.))
            .children(WINDOWS.iter().enumerate().map(|(ix, (_, name))| {
                ui::segment(
                    Button::new(SharedString::from(format!("obs-revision-window-{ix}"))),
                    state.window == ix,
                    cx,
                )
                .small()
                .child(text(*name))
                .on_click(cx.listener(move |this, _, _, cx| this.choose_revision_window(ix, cx)))
            }));
        // A failed refresh keeps the last answer for the same window under
        // it, as stale.
        let error = self.revision_error();
        let failed = error.is_some();
        let refused = self.revision_refused();
        let section = section("Around the start", cx)
            .child(picker)
            .children(error.map(|error| {
                part()
                    .id(if refused {
                        "obs-revision-refused"
                    } else {
                        "obs-revision-failed"
                    })
                    .test_support()
                    .role(Role::Status)
                    .when(refused, |this| {
                        this.child(text("Not permitted to read this application's window"))
                    })
                    .child(
                        text(error)
                            .text_color(palette(cx).crit_ink)
                            .whitespace_normal(),
                    )
                    .child(
                        line().child(
                            action("obs-revision-retry", "Retry")
                                .on_click(cx.listener(|this, _, _, cx| this.read_revision(cx))),
                        ),
                    )
            }));
        let Some(detail) = &state.detail else {
            if failed {
                return section;
            }
            return section.child(
                v_flex()
                    .id("obs-revision-loading")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Reading the window from Coroot")
                    .gap(dp(12.))
                    .child(ui::skeleton(relative(0.4), dp(12.)))
                    .child(ui::skeleton(relative(0.7), dp(12.))),
            );
        };
        let section = section.child(
            muted(detail.window.clone(), cx)
                .id("obs-revision-window")
                .test_support(),
        );
        match &detail.answer {
            Answer::NoData => section.child(
                ui::empty_state(
                    IconName::Inbox,
                    "Coroot has no data for this window",
                    "Coroot keeps no metrics of this application from then: the window is older than Coroot keeps, newer than its latest data, or the application sent none in it.",
                    None,
                    Vec::new(),
                    cx,
                )
                .id("obs-revision-no-data")
                .test_support()
                .role(Role::Status)
                .h_auto(),
            ),
            Answer::Page {
                tabs,
                charts,
                ends_early,
                history_note,
            } => {
                let notes = ends_early
                    .iter()
                    .map(|note| (note, "obs-revision-ends-early"))
                    .chain(history_note.iter().map(|note| (note, "obs-revision-history")))
                    .map(|(note, id)| {
                        muted(note.clone(), cx)
                            .id(id)
                            .test_support()
                            .role(Role::Status)
                            .whitespace_normal()
                    })
                    .collect::<Vec<_>>();
                let body = if tabs.is_empty() {
                    vec![
                        muted("Coroot sent no charts for this window.", cx)
                            .id("obs-revision-no-charts")
                            .test_support()
                            .into_any_element(),
                    ]
                } else {
                    charts
                        .iter()
                        .map(|charts| self.render_charts(charts, cx))
                        .collect()
                };
                section
                    .when(!tabs.is_empty(), |this| {
                        this.child(self.render_revision_tabs(tabs, &detail.report, cx))
                    })
                    .children(notes)
                    .children(body)
            }
        }
    }

    fn render_revision_tabs(&self, tabs: &[Tab], chosen: &str, cx: &Context<Self>) -> Div {
        line()
            .flex_wrap()
            .gap(dp(2.))
            .p(dp(3.))
            .rounded(px(8.))
            .bg(palette(cx).surface_2)
            .children(tabs.iter().map(|tab| {
                let report = tab.name.clone();
                ui::segment(Button::new(tab.id.clone()), chosen == tab.name, cx)
                    .group("fog-control")
                    .small()
                    .gap(dp(6.))
                    .children(tab.status.map(|s| status(s, cx)))
                    .child(text(tab.name.clone()))
                    .tooltip(tab.tooltip.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_revision_report(report.clone(), cx)
                    }))
            }))
    }
}
