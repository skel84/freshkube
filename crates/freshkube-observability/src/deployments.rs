use super::*;
const VERSIONS: [&str; 4] = ["1.8.2", "1.8.1", "1.8.0", "1.7.4"];
impl ObservabilityPage {
    fn release_header(&self, cx: &Context<Self>) -> Div {
        line()
            .flex_wrap()
            .child(mono("payments / worker"))
            .child(div().flex_1())
            .child(
                action(
                    "obs-release-compare",
                    format!("Compare with {}", VERSIONS[self.comparison]),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.comparison = (this.comparison + 1) % VERSIONS.len();
                    if this.comparison == this.release {
                        this.comparison = (this.comparison + 1) % VERSIONS.len();
                    }
                    this.prepare_release();
                    cx.notify();
                })),
            )
            .child(
                Button::new("obs-release-rollback")
                    .primary()
                    .small()
                    .label("Roll back to 1.8.1…")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.preview("Roll back worker", window, cx)
                    })),
            )
    }
    fn release_timeline(&self, cx: &Context<Self>) -> Div {
        card("Last 30 days · 4 releases", cx).child(
            body().child(
                line()
                    .justify_between()
                    .children(VERSIONS.iter().enumerate().rev().map(|(ix, version)| {
                        Button::new(SharedString::from(format!("obs-release-marker-{version}")))
                            .ghost()
                            .group("fog-control")
                            .small()
                            .selected(self.release == ix)
                            .child(status(
                                if ix == 0 {
                                    Status::Critical
                                } else {
                                    Status::Ok
                                },
                                cx,
                            ))
                            .label(*version)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.release = ix;
                                this.prepare_release();
                                cx.notify();
                            }))
                    })),
            ),
        )
    }
    fn release_list(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        card("Releases", cx).children(VERSIONS.iter().enumerate().map(|(ix, version)| {
            Button::new(SharedString::from(format!("obs-release-{version}")))
                .ghost()
                .group("fog-control")
                .selected(self.release == ix)
                .w_full()
                .h(dp(128.))
                .justify_start()
                .px(dp(14.))
                .border_t_1()
                .border_color(p.line)
                .child(status(
                    if ix == 0 {
                        Status::Critical
                    } else {
                        Status::Ok
                    },
                    cx,
                ))
                .child(
                    v_flex()
                        .items_start()
                        .w(dp(90.))
                        .flex_none()
                        .gap(dp(8.))
                        .child(mono(*version).font_weight(ui::HEADING_WEIGHT))
                        .child(muted(format!("rev {}", 14 - ix), cx)),
                )
                .child(
                    v_flex()
                        .items_start()
                        .flex_1()
                        .min_w_0()
                        .gap(dp(7.))
                        .child(text(
                            [
                                "today 12:52 · crash-looping",
                                "3 days ago · replaced",
                                "9 days ago · replaced",
                                "23 days ago · replaced",
                            ][ix],
                        ))
                        .child(
                            line()
                                .child(status(
                                    if ix == 0 {
                                        Status::Critical
                                    } else {
                                        Status::Ok
                                    },
                                    cx,
                                ))
                                .child(text(if ix == 0 {
                                    "Crash: restarted 14 times"
                                } else {
                                    "Availability: 100% · objective 99%"
                                })),
                        )
                        .child(
                            line()
                                .child(status(
                                    if ix == 0 {
                                        Status::LogError
                                    } else {
                                        Status::Ok
                                    },
                                    cx,
                                ))
                                .child(text(if ix == 0 {
                                    "Logs: 212 errors, up from 0"
                                } else {
                                    "Memory: no leak detected"
                                })),
                        )
                        .child(muted(
                            if ix == 0 {
                                "CPU not comparable · not running"
                            } else {
                                "CPU +4% vs preceding release"
                            },
                            cx,
                        )),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.release = ix;
                    this.prepare_release();
                    cx.notify();
                }))
        }))
    }
    fn release_comparison(&self, cx: &Context<Self>) -> Div {
        let p = palette(cx);
        card(format!("What changed in {}", VERSIONS[self.release]), cx).child(
            body()
                .child(
                    line()
                        .flex_wrap()
                        .child(muted(
                            format!(
                                "Compare {} → {}",
                                VERSIONS[self.comparison], VERSIONS[self.release]
                            ),
                            cx,
                        ))
                        .child(div().flex_1())
                        .child(
                            action(
                                "obs-release-yaml",
                                if self.full_yaml {
                                    "Changes"
                                } else {
                                    "Full YAML"
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.full_yaml = !this.full_yaml;
                                cx.notify();
                            })),
                        ),
                )
                .child(
                    div()
                        .id("obs-spec-diff")
                        .test_support()
                        .overflow_x_scroll()
                        .child(v_flex().min_w(dp(420.)).when_else(
                            self.full_yaml,
                            |this| this.child(mono(self.release_yaml.clone()).whitespace_nowrap()),
                            |this| {
                                this.children(self.release_diff.iter().map(|(sign, line)| {
                                    mono(line.clone())
                                        .p(dp(4.))
                                        .whitespace_nowrap()
                                        .when(*sign == '+', |this| this.bg(p.good_soft))
                                        .when(*sign == '-', |this| this.bg(p.crit_soft))
                                }))
                            },
                        )),
                )
                .when(self.release == 0, |this| {
                    this.child(
                        line()
                            .items_start()
                            .p(dp(10.))
                            .rounded(px(8.))
                            .bg(p.crit_soft)
                            .child(status(Status::Critical, cx))
                            .child(text(
                                "ledger-db has no port 5432. The Service exposes 6432/TCP only.",
                            )),
                    )
                }),
        )
    }
    fn release_source(&self, cx: &Context<Self>) -> Div {
        card("Also in this release", cx).child(
            body()
                .child(pair(
                    "Commit",
                    [
                        "a41f9c2 · move db config to env",
                        "63b70d1 · tune queue worker",
                        "090f417 · retry transient requests",
                        "aad3c8e · update base image",
                    ][self.release],
                    cx,
                ))
                .child(pair("Synced by", "Argo CD · payments-prod · auto-sync", cx))
                .child(pair(
                    "Rollout",
                    if self.release == 0 {
                        "stuck 2h · progress deadline exceeded"
                    } else {
                        "completed"
                    },
                    cx,
                )),
        )
    }
    pub(super) fn prepare_release(&mut self) {
        let before = deployment_spec(self.comparison);
        let after = deployment_spec(self.release);
        self.release_yaml = after.join("\n");
        self.release_diff.clear();
        if before == after {
            self.release_diff
                .push((' ', "No spec changes between these revisions.".into()));
            return;
        }
        let changed: Vec<_> = before
            .iter()
            .zip(&after)
            .enumerate()
            .filter_map(|(index, (old, new))| (old != new).then_some(index))
            .collect();
        let mut omitted = false;
        for (index, (old, new)) in before.iter().zip(&after).enumerate() {
            if !changed.iter().any(|change| index.abs_diff(*change) <= 2) {
                if !omitted {
                    self.release_diff.push((' ', "⋯ unchanged context".into()));
                }
                omitted = true;
                continue;
            }
            omitted = false;
            if old == new {
                self.release_diff.push((' ', format!("  {new}")));
            } else {
                self.release_diff.push(('-', format!("− {old}")));
                self.release_diff.push(('+', format!("+ {new}")));
            }
        }
    }

    pub(super) fn render_deployments(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let stacked = freshkube_ui::page::content_width(window) < 880.;
        let header = self.release_header(cx);
        let timeline = self.release_timeline(cx);
        let releases = self.release_list(cx);
        let diff = self.release_comparison(cx);
        let source = self.release_source(cx);
        v_flex()
            .gap(dp(12.))
            .child(header)
            .child(timeline)
            .child(
                div()
                    .id("obs-release-columns")
                    .test_support()
                    .flex()
                    .gap(dp(12.))
                    // Stacked, each column spans the content width (#405);
                    // side by side, they share it and keep their own height.
                    .when_else(
                        stacked,
                        |this| this.flex_col(),
                        |this| this.flex_row().items_start(),
                    )
                    .child(
                        div()
                            .id("obs-release-list")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .child(releases),
                    )
                    .child(
                        v_flex()
                            .id("obs-release-changes")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .gap(dp(12.))
                            .child(diff)
                            .child(source),
                    ),
            )
            .into_any_element()
    }
}

/// Deterministic manifests for fictional ReplicaSets. The view compares the
/// actual fields in these fixtures, including a reversed or identical pair.
fn deployment_spec(release: usize) -> Vec<String> {
    let port = if release == 0 { 5432 } else { 6432 };
    format!("apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: worker\n  namespace: payments\nspec:\n  replicas: 1\n  selector:\n    matchLabels: {{app: worker}}\n  template:\n    metadata:\n      labels: {{app: worker}}\n    spec:\n      containers:\n        - name: worker\n          image: ghcr.io/example/worker:{}\n          env:\n            - name: DATABASE_URL\n              value: postgres://ledger-db:{port}/ledger\n          resources:\n            requests: {{cpu: 100m, memory: 128Mi}}\n            limits: {{cpu: 400m, memory: 512Mi}}", VERSIONS[release])
        .lines().map(str::to_owned).collect()
}
