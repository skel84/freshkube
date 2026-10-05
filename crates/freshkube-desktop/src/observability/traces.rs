use super::*;
actions!(
    observability_heatmap,
    [EarlierBucket, LaterBucket, HigherBucket, LowerBucket]
);
const BUCKETS: [&str; 10] = [
    "errors", "5000+", "2500", "1000", "500", "250", "100", "50", "25", "10",
];
const SPANS: [(&str, &str, f32, f32, bool); 8] = [
    ("ingress-nginx", "POST /v1/settlements", 0., 1., false),
    ("payments/api", "POST /v1/settlements", 0.02, 0.95, true),
    ("payments/api", "redis GET settlement", 0.05, 0.03, false),
    ("payments/ledger", "GET /accounts/8…", 0.07, 0.12, false),
    ("payments/api", "POST http://worker:8080", 0.18, 0.76, true),
    (
        "payments/api",
        "retry 1 · connection refused",
        0.20,
        0.21,
        true,
    ),
    (
        "payments/api",
        "retry 2 · connection refused",
        0.48,
        0.24,
        true,
    ),
    (
        "payments/api",
        "retry 3 · connection refused",
        0.77,
        0.16,
        true,
    ),
];

type Span = (&'static str, &'static str, f32, f32, bool);
const BALANCES: [Span; 5] = [
    ("ingress-nginx", "GET /v1/balances", 0., 1., false),
    ("payments/api", "GET /v1/balances", 0.02, 0.97, true),
    ("payments/ledger", "GET /accounts/8…", 0.04, 0.94, true),
    ("payments/ledger", "SELECT balance", 0.08, 0.87, true),
    ("payments/ledger", "context deadline", 0.96, 0.02, true),
];
const AUTH: [Span; 4] = [
    ("ingress-nginx", "GET /auth", 0., 1., false),
    ("oauth2-proxy", "GET /auth", 0.02, 0.94, true),
    ("oauth2-proxy", "GET keycloak/userinfo", 0.09, 0.83, true),
    ("oauth2-proxy", "dial: no route to host", 0.12, 0.76, true),
];
const HEALTHY: [Span; 5] = [
    ("ingress-nginx", "GET /v1/balances", 0., 1., false),
    ("payments/api", "GET /v1/balances", 0.02, 0.94, false),
    ("payments/api", "redis GET balance", 0.06, 0.12, false),
    ("payments/ledger", "GET /accounts/8…", 0.22, 0.62, false),
    ("ledger-db", "SELECT balance", 0.26, 0.52, false),
];
pub(super) struct TraceSnapshot {
    heat: [[usize; 36]; 10],
    pub(super) spans: &'static [Span],
    visible: Vec<usize>,
    summary: &'static str,
    error: &'static str,
    duration: &'static str,
}
impl TraceSnapshot {
    pub(super) fn new(hours: u32, cause: usize, errors_only: bool) -> Self {
        let spans: &'static [Span] = match cause {
            0 => &SPANS,
            1 => &BALANCES,
            2 => &AUTH,
            _ => &HEALTHY,
        };
        let heat = std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                let minutes_ago = (1. - column as f32 / 35.) * hours as f32 * 60.;
                if row == 0 {
                    if minutes_ago > 128. {
                        0
                    } else {
                        3 + column % 3
                    }
                } else {
                    ((9 - row).min(row) + column % 3).min(5)
                }
            })
        });
        Self {
            heat,
            spans,
            visible: (0..spans.len())
                .filter(|i| !errors_only || spans[*i].4)
                .collect(),
            summary: [
                "1.02s · HTTP 503 · upstream worker unavailable",
                "2.00s · deadline exceeded",
                "0.54s · HTTP 502 · keycloak unavailable",
                "0.18s · HTTP 200 · request completed",
            ][cause],
            error: [
                "connection refused · peer: worker:8080",
                "deadline exceeded · peer: ledger-db:6432",
                "no route to host · peer: keycloak:8080",
                "none",
            ][cause],
            duration: ["1.02s", "2.00s", "0.54s", "0.18s"][cause],
        }
    }
}
impl ObservabilityPage {
    fn trace_toolbar(&self, cx: &Context<Self>) -> Div {
        line()
            .flex_wrap()
            .child(mono("payments/api"))
            .child(muted("OpenTelemetry · example spans", cx))
            .child(div().flex_1())
            .child(
                action("obs-trace-errors", "Errors only")
                    .selected(self.trace_errors_only)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.trace_errors_only = !this.trace_errors_only;
                        this.prepare_trace();
                        cx.notify();
                    })),
            )
            .child(
                action("obs-trace-clear", "Clear selection").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.bucket = None;
                        this.trace_errors_only = false;
                        this.trace_error = 0;
                        this.prepare_trace();
                        cx.notify();
                    },
                )),
            )
    }
    fn trace_heatmap(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let snapshot = &self.trace_snapshot;
        // A fixed ten-by-36 example heatmap, never a live unbounded list.
        let heatmap = v_flex()
            .id("obs-heatmap-grid")
            .test_support()
            .track_focus(&self.heat_focus)
            .key_context("ObservabilityHeatmap")
            .aria_label(
                "Latency and error heatmap. Arrow keys move between time and duration buckets.",
            )
            .on_action(cx.listener(|this, _: &EarlierBucket, _, cx| this.step_bucket(0, -1, cx)))
            .on_action(cx.listener(|this, _: &LaterBucket, _, cx| this.step_bucket(0, 1, cx)))
            .on_action(cx.listener(|this, _: &HigherBucket, _, cx| this.step_bucket(-1, 0, cx)))
            .on_action(cx.listener(|this, _: &LowerBucket, _, cx| this.step_bucket(1, 0, cx)))
            .gap(dp(3.))
            .children(BUCKETS.into_iter().enumerate().map(|(row, label)| {
                line()
                    .gap(dp(3.))
                    .h(dp(16.))
                    .child(muted(label, cx).w(dp(50.)).flex_none().text_right())
                    .children((0..36).map(|column| {
                        let intensity = snapshot.heat[row][column];
                        let selected = self.bucket.is_some_and(|(r, c)| r == row && c == column);
                        div()
                            .id(SharedString::from(format!("obs-bucket-{row}-{column}")))
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .rounded(px(2.))
                            .bg(crate::palette::heat_color(intensity, row == 0))
                            .border_1()
                            .border_color(if selected {
                                p.accent
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .opacity(if self.trace_errors_only && row != 0 {
                                0.25
                            } else {
                                1.
                            })
                            .tooltip(move |window, cx| {
                                Tooltip::new(format!(
                                    "{label} · time bucket {} · Click to inspect traces",
                                    column + 1
                                ))
                                .build(window, cx)
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    if this.trace_errors_only && row != 0 {
                                        return;
                                    }
                                    window.focus(&this.heat_focus, cx);
                                    this.bucket = Some((row, column));
                                    this.trace_error = if row == 0 { 0 } else { 3 };
                                    this.prepare_trace();
                                    cx.notify();
                                }),
                            )
                    }))
            }));
        card("Latency & errors", cx).child(
            body()
                .pt_0()
                .child(
                    line()
                        .child(muted(
                            "Requests per duration bucket (ms) · darker = fewer",
                            cx,
                        ))
                        .child(div().flex_1())
                        .child(muted(
                            self.bucket.map_or("All requests".to_owned(), |(r, c)| {
                                format!("Selected: {} · bucket {}", BUCKETS[r], c + 1)
                            }),
                            cx,
                        )),
                )
                .child(
                    div().relative().child(heatmap).child(
                        div()
                            .absolute()
                            .left(dp(56.))
                            .right_0()
                            .top(dp(72.))
                            .h(px(1.))
                            .bg(p.warn),
                    ),
                )
                .child(
                    line()
                        .justify_between()
                        .child(muted(format!("−{}h", self.hours), cx))
                        .child(muted("SLO · 500ms", cx))
                        .child(muted("now", cx)),
                ),
        )
    }
    fn trace_causes(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        card("Where the errors come from", cx).child(
            body().children(
                [
                    (
                        "POST /v1/settlements",
                        "503 · upstream worker unavailable",
                        "96%",
                    ),
                    ("GET /v1/balances", "context deadline exceeded", "3%"),
                    ("GET /auth", "502 · keycloak instance unreachable", "1%"),
                ]
                .into_iter()
                .enumerate()
                .map(|(ix, (span, error, share))| {
                    Button::new(SharedString::from(format!("obs-error-cause-{ix}")))
                        .ghost()
                        .group("fog-control")
                        .selected(self.trace_error == ix)
                        .w_full()
                        .h(dp(64.))
                        .justify_start()
                        .child(
                            v_flex()
                                .min_w_0()
                                .items_start()
                                .gap(dp(5.))
                                .child(mono(span))
                                .child(text(error).text_color(p.crit_ink).truncate()),
                        )
                        .child(div().flex_1())
                        .child(mono(share))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.trace_error = ix;
                            this.trace_span = 0;
                            this.prepare_trace();
                            cx.notify();
                        }))
                }),
            ),
        )
    }
    fn trace_waterfall(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        let snapshot = &self.trace_snapshot;
        let selected = snapshot.spans[self.trace_span];
        card(
            [
                "Sample trace · 4f2a…9c1e",
                "Sample trace · 93be…714a",
                "Sample trace · 50cd…319f",
                "Sample trace · a3fe…040c",
            ][self.trace_error],
            cx,
        )
        .child(
            body()
                .pt_0()
                .child(muted(snapshot.summary, cx))
                .child(
                    line()
                        .justify_between()
                        .child(muted("0ms", cx))
                        .child(muted(snapshot.duration, cx)),
                )
                .children(snapshot.visible.iter().map(|&ix| {
                    let (service, operation, start, width, error) = snapshot.spans[ix];
                    Button::new(SharedString::from(format!("obs-span-{ix}")))
                        .ghost()
                        .group("fog-control")
                        .selected(self.trace_span == ix)
                        .w_full()
                        .h(dp(32.))
                        .justify_start()
                        .px_0()
                        .gap(dp(8.))
                        .child(mono(service).text_color(p.muted).w(dp(96.)).truncate())
                        .child(mono(operation).w(dp(134.)).truncate())
                        .child(
                            div().flex_1().relative().h(dp(10.)).child(
                                div()
                                    .absolute()
                                    .left(relative(start))
                                    .w(relative(width))
                                    .h_full()
                                    .rounded(px(3.))
                                    .bg(if error {
                                        p.crit
                                    } else {
                                        crate::monitoring::colors::Ink::Slot(0).color(false)
                                    }),
                            ),
                        )
                        .tooltip(format!("{service} · {operation}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.trace_span = ix;
                            cx.notify();
                        }))
                }))
                .child(
                    v_flex()
                        .mt(dp(8.))
                        .pt(dp(12.))
                        .border_t_1()
                        .border_color(p.line)
                        .gap(dp(6.))
                        .child(mono(selected.0))
                        .child(text(selected.1))
                        .child(muted(
                            if selected.4 {
                                snapshot.error
                            } else {
                                "status: OK · transport: HTTP"
                            },
                            cx,
                        )),
                ),
        )
    }
    fn step_bucket(&mut self, row: isize, column: isize, cx: &mut Context<Self>) {
        let (r, c) = self.bucket.unwrap_or((0, 18));
        let r = if self.trace_errors_only {
            0
        } else {
            (r as isize + row).clamp(0, 9) as usize
        };
        let c = (c as isize + column).clamp(0, 35) as usize;
        self.bucket = Some((r, c));
        self.trace_error = if r == 0 { 0 } else { 3 };
        self.prepare_trace();
        cx.notify();
    }
    pub(super) fn prepare_trace(&mut self) {
        if self.trace_errors_only && self.trace_error == 3 {
            self.trace_error = 0;
        }
        if self.trace_errors_only {
            self.bucket = self.bucket.map(|(_, column)| (0, column));
        }
        self.trace_snapshot =
            TraceSnapshot::new(self.hours, self.trace_error, self.trace_errors_only);
        if !self.trace_snapshot.visible.contains(&self.trace_span) {
            self.trace_span = self.trace_snapshot.visible.first().copied().unwrap_or(0);
        }
    }
    pub(super) fn render_traces(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let stacked = crate::screens::content_width(window) < 820.;
        let toolbar = self.trace_toolbar(cx);
        let heat = self.trace_heatmap(cx);
        let errors = self.trace_causes(cx);
        let trace = self.trace_waterfall(cx);
        v_flex()
            .gap(dp(12.))
            .child(toolbar)
            .child(heat)
            .child(
                h_flex()
                    .items_start()
                    .flex_wrap()
                    .gap(dp(12.))
                    .child(
                        div()
                            .when_else(
                                stacked,
                                |this| this.w_full(),
                                |this| this.flex_1().min_w(dp(280.)),
                            )
                            .child(errors),
                    )
                    .child(
                        div()
                            .when_else(
                                stacked,
                                |this| this.w_full(),
                                |this| this.flex_1().min_w(dp(460.)),
                            )
                            .child(trace),
                    ),
            )
            .into_any_element()
    }
}
