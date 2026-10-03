use super::*;

pub(super) struct ReportSnapshot {
    statement: String,
    threshold: String,
    condition: &'static str,
    clients: Vec<&'static str>,
    dependencies: Vec<&'static str>,
    charts: [Chart; 2],
}
impl ObservabilityPage {
    pub(super) fn prepare_report(&mut self) {
        let Some(app) = self.applications.get(self.selected_app) else {
            return;
        };
        let worker = app.key == example::WORKER;
        let report = self.report;
        let check = app.check(report);
        let statement = if check.status == Status::Integration {
            "This report needs an additional data source.".into()
        } else if check.status == Status::Unknown {
            "No observations are available for this report in the selected period.".into()
        } else if worker && matches!(report, Report::Net | Report::Upstreams) {
            "No connectivity with upstream service ledger-db. TCP connections to ledger-db:5432 are refused.".into()
        } else if worker && report == Report::Instances {
            "The worker has no ready instances. Its container exits with code 1 and waits in CrashLoopBackOff.".into()
        } else if worker && report == Report::Logs {
            "212 error lines from the worker's last runs. Connections to ledger-db:5432 were refused.".into()
        } else if worker && report == Report::Restarts {
            "The worker restarted 14 times. The next attempt is subject to kubelet back-off.".into()
        } else {
            match report {
                Report::Instances => format!("{} instances are ready.", check.value),
                Report::Upstreams if check.status == Status::Ok => {
                    "Every observed upstream is reachable.".into()
                }
                Report::Upstreams => format!(
                    "Upstream {} is unavailable; calls to it are failing.",
                    check.value
                ),
                Report::Net => format!(
                    "Network round-trip time is {}. No failed TCP connections were observed.",
                    check.value
                ),
                Report::Dns => format!(
                    "DNS lookup time is {}. No failed lookups were observed.",
                    check.value
                ),
                Report::Logs => format!(
                    "{} error log lines were observed in the selected period.",
                    check.value
                ),
                Report::Restarts => format!(
                    "Containers restarted {} times in the selected period.",
                    check.value
                ),
                Report::Errors => {
                    format!("{} of requests failed in the selected period.", check.value)
                }
                Report::Latency => format!("The p99 request latency is {}.", check.value),
                _ => format!("{} use is {} of its limit.", report.label(), check.value),
            }
        };
        let condition = match report {
            Report::Instances => "unavailable instances",
            Report::Upstreams => "unreachable upstreams",
            Report::Net => "failed TCP connections",
            Report::Logs => "error log lines",
            Report::Cpu | Report::Memory | Report::Disk => "use (% of limit)",
            Report::Latency => "p99 latency (ms)",
            Report::Dns => "failed DNS queries",
            Report::Errors => "request error rate (%)",
            Report::Restarts => "container restarts",
        };
        let mut clients = vec![];
        let mut dependencies = vec![];
        for edge in &self.connections {
            if self.nodes[edge.to].app == app.key {
                clients.push(self.nodes[edge.from].app);
            }
            if self.nodes[edge.from].app == app.key {
                dependencies.push(self.nodes[edge.to].app);
            }
        }
        let charts = if worker && matches!(report, Report::Net | Report::Upstreams) {
            [self.charts[0].clone(), self.charts[1].clone()]
        } else {
            report_charts(app, report, self.hours)
        };
        self.report_snapshot = Some(ReportSnapshot {
            statement,
            condition,
            clients,
            dependencies,
            charts,
            threshold: self
                .thresholds
                .get(&(app.key.clone(), report))
                .cloned()
                .unwrap_or_else(|| example::threshold(&app.key, report).into()),
        });
    }

    fn report_header(&self, cx: &Context<Self>) -> Div {
        let app = &self.applications[self.selected_app];
        let worker = app.key == example::WORKER;
        line()
            .flex_wrap()
            .child(self.breadcrumbs("Applications", Destination::Applications, cx))
            .child(div().flex_1())
            .when(worker, |this| {
                this.child(
                    Button::new("obs-app-logs")
                        .primary()
                        .small()
                        .label("Logs")
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ObservabilityEvent::OpenPod { logs: true })
                        })),
                )
                .child(action("obs-app-pod", "Open pod").on_click(
                    cx.listener(|_, _, _, cx| cx.emit(ObservabilityEvent::OpenPod { logs: false })),
                ))
                .child(action("obs-app-rollback", "Roll back to 1.8.1…").on_click(
                    cx.listener(|this, _, window, cx| this.preview("Roll back worker", window, cx)),
                ))
            })
    }
    fn report_map(&self, cx: &Context<Self>) -> Div {
        let app = &self.applications[self.selected_app];
        let worker = app.key == example::WORKER;
        let p = palette(cx);
        let snapshot = self
            .report_snapshot
            .as_ref()
            .expect("fixture report is prepared");
        card("Application dependencies", cx).child(
            body().child(
                h_flex()
                    .items_stretch()
                    .gap(dp(12.))
                    .flex_wrap()
                    .child(self.report_relations("CLIENTS", &snapshot.clients, cx))
                    .child(text("→").text_color(p.muted).py(dp(18.)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(dp(190.))
                            .p(dp(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.accent_line)
                            .bg(p.accent_soft)
                            .child(ui::caption(&app.key, cx))
                            .child(line().mt(dp(8.)).child(status(app.status, cx)).child(
                                mono(if worker { example::POD } else { &app.name }).truncate(),
                            ))
                            .child(muted(
                                format!("{} instances ready", app.check(Report::Instances).value),
                                cx,
                            )),
                    )
                    .child(text("→").text_color(p.muted).py(dp(18.)))
                    .child(self.report_relations("DEPENDENCIES", &snapshot.dependencies, cx)),
            ),
        )
    }
    fn report_tabs(&self, cx: &Context<Self>) -> Div {
        let app = &self.applications[self.selected_app];
        let p = palette(cx);
        line()
            .flex_wrap()
            .border_b_1()
            .border_color(p.line)
            .pb(dp(8.))
            .children(Report::ALL.into_iter().map(|report| {
                Button::new(SharedString::from(format!("obs-report-{}", report.slug())))
                    .ghost()
                    .group("fog-control")
                    .small()
                    .selected(self.report == report)
                    .child(status(app.check(report).status, cx))
                    .label(report.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.report = report;
                        this.prepare_report();
                        cx.notify();
                    }))
            }))
    }
    fn report_checks(&self, cx: &Context<Self>) -> Div {
        let app = &self.applications[self.selected_app];
        let p = palette(cx);
        let snapshot = self
            .report_snapshot
            .as_ref()
            .expect("fixture report is prepared");
        let check = app.check(self.report);
        card(format!("Checks · {}", self.report.label()), cx).child(
            body()
                .child(
                    line()
                        .items_start()
                        .child(status(check.status, cx))
                        .child(text(snapshot.statement.clone()).flex_1()),
                )
                .when(
                    !matches!(check.status, Status::Integration | Status::Unknown),
                    |this| {
                        this.child(
                            line()
                                .flex_wrap()
                                .child(muted(format!("Condition: {} >", snapshot.condition), cx))
                                .child(
                                    Button::new("obs-threshold")
                                        .ghost()
                                        .group("fog-control")
                                        .small()
                                        .text_color(p.accent)
                                        .label(snapshot.threshold.clone())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.edit_threshold(window, cx)
                                        })),
                                ),
                        )
                    },
                )
                .child(muted(
                    "Observed values and thresholds are example data.",
                    cx,
                )),
        )
    }
    fn report_related(&self, cx: &Context<Self>) -> Div {
        let app = &self.applications[self.selected_app];
        let worker = app.key == example::WORKER;
        let check = app.check(self.report);
        if worker && matches!(self.report, Report::Net | Report::Upstreams) {
            card("Upstreams", cx).child(
                body()
                    .child(self.relation("obs-upstream-db", "SERVICE", "payments/ledger-db", cx))
                    .child(pair("Endpoint", "ledger-db:5432 · refused", cx))
                    .child(pair("Service port", "6432/TCP · pgbouncer", cx))
                    .child(self.relation("obs-upstream-redis", "SERVICE", "cache/redis-cache", cx))
                    .child(pair("Endpoint", "redis-cache:6379 · 0.2ms RTT", cx)),
            )
        } else {
            card("Report details", cx).child(
                body()
                    .child(pair("Application", app.key.clone(), cx))
                    .child(pair("Report", self.report.label(), cx))
                    .child(pair("Observed", check.value.clone(), cx))
                    .child(pair(
                        "Range",
                        format!("Last {}h · ends at 15:00", self.hours),
                        cx,
                    )),
            )
        }
    }
    pub(super) fn render_report(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let app = &self.applications[self.selected_app];
        let snapshot = self
            .report_snapshot
            .as_ref()
            .expect("fixture report is prepared");
        let check = app.check(self.report);
        let header = self.report_header(cx);
        let map = self.report_map(cx);
        let tabs = self.report_tabs(cx);
        let checks = self.report_checks(cx);
        let related = self.report_related(cx);

        let stacked = crate::screens::content_width(window) < 820.;
        v_flex()
            .gap(dp(12.))
            .child(header)
            .child(map)
            .child(tabs)
            .child(
                div()
                    .flex()
                    .gap(dp(12.))
                    .when_else(stacked, |this| this.flex_col(), |this| this.flex_row())
                    .child(div().flex_1().min_w_0().child(checks))
                    .child(div().flex_1().min_w_0().child(related)),
            )
            .when(
                !matches!(check.status, Status::Integration | Status::Unknown),
                |this| {
                    this.child(
                        div()
                            .flex()
                            .gap(dp(12.))
                            .when_else(stacked, |this| this.flex_col(), |this| this.flex_row())
                            .child(div().flex_1().min_w_0().child(plots::chart(
                                &snapshot.charts[0],
                                "obs-failed-chart",
                                cx,
                            )))
                            .child(div().flex_1().min_w_0().child(plots::chart(
                                &snapshot.charts[1],
                                "obs-success-chart",
                                cx,
                            ))),
                    )
                },
            )
            .into_any_element()
    }
    fn report_relations(
        &self,
        label: &'static str,
        keys: &[&'static str],
        cx: &Context<Self>,
    ) -> Div {
        v_flex()
            .min_w(dp(150.))
            .gap(dp(6.))
            .when(keys.is_empty(), |this| {
                this.child(ui::caption(label, cx))
                    .child(muted("No observed connections", cx))
            })
            .children(keys.iter().enumerate().map(|(ix, key)| {
                self.relation(
                    SharedString::from(format!("obs-relation-{label}-{ix}")),
                    label,
                    key,
                    cx,
                )
            }))
    }
    fn relation(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        key: &'static str,
        cx: &Context<Self>,
    ) -> Div {
        v_flex()
            .min_w(dp(150.))
            .gap(dp(5.))
            .child(ui::caption(label, cx))
            .child(
                Button::new(id)
                    .ghost()
                    .group("fog-control")
                    .small()
                    .justify_start()
                    .child(ui::reference(key, &palette(cx)))
                    .font_family(MONO_FONT)
                    .text_color(palette(cx).ink_2)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_named(key, Report::Net, cx)),
                    ),
            )
    }
}

fn report_charts(app: &Application, report: Report, hours: u32) -> [Chart; 2] {
    let check = app.check(report);
    let unit = match report {
        Report::Cpu | Report::Memory | Report::Disk | Report::Errors => "%",
        Report::Latency | Report::Net | Report::Dns => "ms",
        _ => "count",
    };
    let raw: String = check
        .value
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let value = raw
        .parse::<f32>()
        .unwrap_or(if check.status == Status::Ok { 0. } else { 1. })
        * if check.value.ends_with('k') {
            1000.
        } else {
            1.
        };
    let maximum = if unit == "%" {
        100.
    } else {
        (value * 1.4).max(1.)
    };
    let make = |previous: bool| {
        let mut chart = example::chart(
            if previous {
                "Previous period"
            } else {
                report.label()
            },
            unit,
            hours,
            false,
            false,
        );
        chart.maximum = maximum;
        chart.y_ticks = std::array::from_fn(|i| format!("{:.1}", maximum * (1. - i as f32 / 3.)));
        chart.series = vec![Series {
            label: report.label(),
            values: (0..64)
                .map(|ix| {
                    let before = (1. - ix as f32 / 63.) * hours as f32 * 60. > 128.;
                    let scale = if previous || before { 0.6 } else { 1. };
                    (value * scale * (0.92 + (ix % 5) as f32 * 0.02)).min(maximum)
                })
                .collect(),
        }];
        chart
    };
    [make(false), make(true)]
}
