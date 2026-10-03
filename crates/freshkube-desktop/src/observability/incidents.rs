use super::*;
impl ObservabilityPage {
    fn incident_list(&self, stacked: bool, cx: &Context<Self>) -> Div {
        card("Incidents · 2 open", cx)
            .when_else(
                stacked,
                |this| this.w_full(),
                |this| this.w(dp(228.)).flex_none(),
            )
            .children(
                [
                    ("INC-12", "payments/api is serving errors", "12:58 · 2h 02m"),
                    (
                        "INC-13",
                        "platform/keycloak lost an instance",
                        "14:50 · 10 min",
                    ),
                ]
                .into_iter()
                .enumerate()
                .map(|(ix, (id, title, when))| {
                    Button::new(SharedString::from(format!("obs-incident-{id}")))
                        .ghost()
                        .group("fog-control")
                        .selected(self.incident == ix)
                        .h(dp(94.))
                        .w_full()
                        .justify_start()
                        .px(dp(14.))
                        .child(status(Status::Critical, cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .items_start()
                                .text_left()
                                .gap(dp(4.))
                                .child(text(id).font_weight(ui::HEADING_WEIGHT))
                                .child(text(title).whitespace_normal())
                                .child(muted(when, cx)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.incident = ix;
                            this.incident_muted = false;
                            cx.notify();
                        }))
                }),
            )
    }
    fn incident_heading(&self, primary: bool, cx: &Context<Self>) -> Div {
        let title = if primary {
            "payments/api is serving errors"
        } else {
            "platform/keycloak lost an instance"
        };
        line()
            .flex_wrap()
            .child(ui::page_title(title))
            .child(ui::tag(Tone::Crit, None, "Open", cx))
            .child(div().flex_1())
            .child(
                action("obs-incident-traces", "Traces").on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.trace_error = if primary { 0 } else { 2 };
                        this.prepare_trace();
                        this.open(Destination::Traces, cx);
                    },
                )),
            )
            .child(
                action(
                    "obs-incident-mute",
                    if self.incident_muted {
                        "Unmute"
                    } else {
                        "Mute 1h"
                    },
                )
                .selected(self.incident_muted)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.incident_muted = !this.incident_muted;
                    cx.notify();
                })),
            )
    }
    fn incident_slo(&self, primary: bool, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        card("Service level objectives", cx).child(
            body()
                .child(
                    line()
                        .child(muted("OBJECTIVE", cx).flex_1())
                        .child(muted("COMPLIANCE", cx).w(dp(100.)))
                        .child(muted("BURN 1H / 5M", cx).w(dp(120.))),
                )
                .child(
                    line()
                        .child(status(Status::Critical, cx))
                        .child(
                            text(if primary {
                                "Availability · 99% of requests succeed"
                            } else {
                                "Instances · 2 ready replicas"
                            })
                            .flex_1(),
                        )
                        .child(
                            mono(if primary { "97.2%" } else { "50%" })
                                .w(dp(100.))
                                .text_color(p.crit_ink),
                        )
                        .child(mono(if primary { "18.4× / 21.0×" } else { "—" }).w(dp(120.))),
                )
                .child(
                    line()
                        .child(status(Status::Ok, cx))
                        .child(text("Latency · 99% under 500ms").flex_1())
                        .child(mono("99.6%").w(dp(100.)))
                        .child(mono("0.4× / 0.6×").w(dp(120.))),
                )
                .child(muted(
                    "Incident opens when both 1h and 5m error-budget burn exceed 14.4×",
                    cx,
                )),
        )
    }
    fn incident_root(&self, primary: bool, cx: &Context<Self>) -> Div {
        let cause = if primary {
            "Release 1.8.2 of payments/worker changed DATABASE_URL to port 5432. The ledger-db Service exposes only 6432 (pgbouncer). The worker exits on connection refused and the API's settlement calls fail."
        } else {
            "talos-wk-fra1-03 stopped reporting. One keycloak instance is no longer ready; the remaining instance continues serving requests. The worker release happened earlier and is unrelated."
        };
        card("Root cause", cx).child(
            body()
                .child(text(cause).line_height(dp(21.)))
                .child(
                    line().flex_wrap().children(
                        (if primary {
                            [
                                (
                                    "Release 1.8.2",
                                    "DATABASE_URL · 6432 → 5432",
                                    Destination::Deployments,
                                ),
                                (
                                    "payments/worker",
                                    "exits on start · connection refused",
                                    Destination::Application,
                                ),
                                (
                                    "payments/api",
                                    "POST /v1/settlements → 503",
                                    Destination::Traces,
                                ),
                            ]
                        } else {
                            [
                                (
                                    "talos-wk-fra1-03",
                                    "Node stopped reporting at 14:48",
                                    Destination::ServiceMap,
                                ),
                                (
                                    "platform/keycloak",
                                    "One of two instances is unavailable",
                                    Destination::Application,
                                ),
                                (
                                    "platform/oauth2-proxy",
                                    "Requests reach the remaining instance",
                                    Destination::Traces,
                                ),
                            ]
                        })
                        .into_iter()
                        .map(|(title, description, destination)| {
                            Button::new(SharedString::from(format!(
                                "obs-chain-{}",
                                destination.slug()
                            )))
                            .outline()
                            .group("fog-control")
                            .h(dp(76.))
                            .flex_1()
                            .min_w(dp(170.))
                            .justify_start()
                            .child(
                                v_flex()
                                    .items_start()
                                    .min_w_0()
                                    .gap(dp(6.))
                                    .child(
                                        line()
                                            .child(status(Status::Critical, cx))
                                            .child(mono(title)),
                                    )
                                    .child(muted(description, cx).whitespace_normal()),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if destination == Destination::Application {
                                        this.open_named(
                                            if primary {
                                                example::WORKER
                                            } else {
                                                "platform/keycloak"
                                            },
                                            if primary {
                                                Report::Net
                                            } else {
                                                Report::Instances
                                            },
                                            cx,
                                        );
                                    } else {
                                        if destination == Destination::Traces {
                                            this.trace_error = if primary { 0 } else { 2 };
                                            this.prepare_trace();
                                        } else if destination == Destination::Deployments {
                                            this.release = 0;
                                            this.comparison = 1;
                                            this.prepare_release();
                                        }
                                        this.open(destination, cx);
                                    }
                                },
                            ))
                        }),
                    ),
                )
                .when(primary, |this| {
                    this.child(
                        line()
                            .flex_wrap()
                            .child(
                                Button::new("obs-incident-fix")
                                    .primary()
                                    .small()
                                    .label("Roll back worker to 1.8.1…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.preview("Roll back worker", window, cx)
                                    })),
                            )
                            .child(action("obs-incident-edit", "Edit DATABASE_URL…").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.preview("Correct DATABASE_URL", window, cx)
                                }),
                            ))
                            .child(action("obs-incident-diff", "See the 1.8.2 diff").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.release = 0;
                                    this.comparison = 1;
                                    this.prepare_release();
                                    this.open(Destination::Deployments, cx);
                                }),
                            )),
                    )
                }),
        )
    }
    fn incident_ruled(&self, primary: bool, cx: &Context<Self>) -> Div {
        card("Ruled out",cx).child(body().child(line().items_start().child(status(Status::Warning,cx)).child(v_flex().gap(dp(5.)).child(mono(if primary{"talos-wk-fra1-03 · NotReady"}else{"payments/worker · release 1.8.2"}))
            .child(muted(if primary{"Started at 14:50, 1h 52m after this incident. Only the database replica is on this node; the primary is serving."}else{"The failed worker release affected payments, not keycloak's ready instance."},cx))))
            .child(line().items_start().child(status(Status::Warning,cx)).child(v_flex().gap(dp(5.)).child(mono("wk-fra1-01 · memory at 92%"))
                .child(muted("No pressure stalls on ledger-db-0; its latency is flat.",cx)))))
    }
    pub(super) fn render_incident(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let primary = self.incident == 0;
        let stacked = crate::screens::content_width(window) < 880.;
        let list = self.incident_list(stacked, cx);
        let headline = self.incident_heading(primary, cx);
        let slo = self.incident_slo(primary, cx);
        let root = self.incident_root(primary, cx);
        let ruled = self.incident_ruled(primary, cx);
        let content = v_flex()
            .flex_1()
            .min_w_0()
            .gap(dp(12.))
            .child(headline)
            .child(muted(
                if self.incident_muted {
                    "Example: notifications muted for 1h"
                } else if primary {
                    "Started 12:58 · 4,212 impacted requests · 2.8% · Root cause found"
                } else {
                    "Started 14:50 · 1 instance unavailable · Root cause found"
                },
                cx,
            ))
            .child(slo)
            .child(root)
            .child(
                div()
                    .flex()
                    .gap(dp(12.))
                    .when_else(stacked, |this| this.flex_col(), |this| this.flex_row())
                    .when(primary, |this| {
                        this.child(div().flex_1().min_w_0().child(plots::chart(
                            &self.charts[0],
                            "obs-incident-chart",
                            cx,
                        )))
                    })
                    .child(div().flex_1().min_w_0().child(ruled)),
            );
        div()
            .flex()
            .gap(dp(12.))
            .when_else(stacked, |this| this.flex_col(), |this| this.flex_row())
            .items_start()
            .child(list)
            .child(content)
            .into_any_element()
    }
}
