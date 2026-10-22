//! Settings › Audit log (#853, `docs/WRITE_MODEL.md`): the writes the app sent
//! and Talos Operations' log, across contexts, newest first, with a Source
//! filter. Field paths only; no value is ever in either log. It reads the
//! files when the section shows, off the UI thread, and never writes.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use freshkube_core::write::{AuditStore, History, HistoryRow, Source, SourceFilter};
use freshkube_ui::page;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonGroup},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::SettingsPage;
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

/// The most rows the page draws; the rest are counted, not drawn.
pub(super) const MOST_ROWS: usize = 200;

/// Where the logs are, and what was read from them.
pub(super) struct AuditView {
    pub(super) filter: SourceFilter,
    pub(super) history: Option<History>,
    store: Option<AuditStore>,
    operations: Option<PathBuf>,
    reading: Option<Task<()>>,
}

impl AuditView {
    pub(super) fn new() -> Self {
        Self {
            filter: SourceFilter::All,
            history: None,
            store: crate::mutation::write_audit_store(),
            operations: freshkube_core::operations::default_audit_path("")
                .parent()
                .map(PathBuf::from),
            reading: None,
        }
    }

    /// Reads these logs instead of the app's own, for a test.
    #[cfg(test)]
    pub(super) fn at(store: AuditStore, operations: PathBuf) -> Self {
        Self {
            store: Some(store),
            operations: Some(operations),
            ..Self::new()
        }
    }
}

/// Invented rows for example data, so a capture shows both sources.
pub(super) fn example_history() -> History {
    let at = |s: &str| -> DateTime<Utc> { s.parse().unwrap() };
    let row = |when: &str,
               source,
               context: &str,
               action: &str,
               object: &str,
               phase: &str,
               fields: &[&str],
               result: &str| HistoryRow {
        at: at(when),
        source,
        context: context.into(),
        actor: "operator".into(),
        action: action.into(),
        object: object.into(),
        phase: phase.into(),
        fields: fields.iter().map(|f| (*f).to_owned()).collect(),
        result: result.into(),
    };
    History {
        unreadable: 0,
        rows: vec![
            row(
                "2026-10-22T10:04:12Z",
                Source::Writes,
                "dev-fra",
                "scale",
                "payments-api",
                "observed",
                &["spec.replicas"],
                "The cluster reached the new state.",
            ),
            row(
                "2026-10-22T10:04:10Z",
                Source::Writes,
                "dev-fra",
                "scale",
                "payments-api",
                "accepted",
                &["spec.replicas"],
                "The API accepted it.",
            ),
            row(
                "2026-10-22T10:04:10Z",
                Source::Writes,
                "dev-fra",
                "scale",
                "payments-api",
                "sending",
                &["spec.replicas"],
                "",
            ),
            row(
                "2026-10-22T09:12:03Z",
                Source::Operations,
                "prod-ams",
                "cordon",
                "worker-2",
                "success",
                &[],
                "Node worker-2 cordoned.",
            ),
            row(
                "2026-10-21T16:40:00Z",
                Source::Writes,
                "prod-ams",
                "set-image",
                "checkout",
                "conflict",
                &["spec.template.spec.containers[0].image"],
                "Conflict with hpa: .spec.replicas",
            ),
        ],
    }
}

impl SettingsPage {
    /// Reads both logs off the UI thread. A newer read replaces an older.
    pub(super) fn load_audit(&mut self, cx: &mut Context<Self>) {
        if self.origin == super::Origin::Example {
            self.audit.history = Some(example_history());
            cx.notify();
            return;
        }
        let (store, operations) = (self.audit.store.clone(), self.audit.operations.clone());
        self.audit.reading = Some(cx.spawn(async move |this, cx| {
            let history = cx
                .background_spawn(async move {
                    match (store, operations) {
                        (Some(store), Some(dir)) => {
                            freshkube_core::write::load_history(&store, &dir)
                        }
                        (Some(store), None) => {
                            freshkube_core::write::load_history(&store, std::path::Path::new(""))
                        }
                        _ => History::default(),
                    }
                })
                .await;
            _ = this.update(cx, |page, cx| {
                page.audit.history = Some(history);
                page.audit.reading = None;
                cx.notify();
            });
        }));
    }

    fn set_audit_filter(&mut self, filter: SourceFilter, cx: &mut Context<Self>) {
        if self.audit.filter != filter {
            self.audit.filter = filter;
            cx.notify();
        }
    }

    pub(super) fn render_audit_log(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let header = page::PageHeader::new("settings-audit-log", "Audit log").render(window, cx);
        let selected = SourceFilter::ALL
            .iter()
            .position(|filter| *filter == self.audit.filter);
        let this = cx.entity().downgrade();
        let filter = SourceFilter::ALL.iter().enumerate().fold(
            ButtonGroup::new("audit-filter").outline().small(),
            |group, (ix, filter)| {
                group.child(ui::choice(
                    Button::new(("audit-filter", ix)).label(filter.label()),
                    selected == Some(ix),
                ))
            },
        );
        let filter = filter.on_click(move |picked: &Vec<usize>, _, cx| {
            let Some(filter) = picked.first().and_then(|ix| SourceFilter::ALL.get(*ix)) else {
                return;
            };
            let filter = *filter;
            _ = this.update(cx, |page, cx| page.set_audit_filter(filter, cx));
        });
        let body = match &self.audit.history {
            None => muted("audit-loading", "Reading the logs…", cx).into_any_element(),
            Some(history) => {
                let rows: Vec<&HistoryRow> = history.shown(self.audit.filter).collect();
                let total = rows.len();
                let drawn = rows
                    .into_iter()
                    .take(MOST_ROWS)
                    .enumerate()
                    .map(|(ix, row)| render_row(ix, row, cx));
                v_flex()
                    .gap(dp(8.))
                    .when(history.unreadable > 0, |this| {
                        this.child(
                            div()
                                .id("audit-unreadable")
                                .test_support()
                                .role(Role::Alert)
                                .aria_label("Some audit lines could not be read")
                                .child(ui::warning_banner(
                                    None,
                                    format!(
                                        "{} audit line(s) or file(s) could not be read and are not listed.",
                                        history.unreadable
                                    ),
                                    None,
                                    cx,
                                )),
                        )
                    })
                    .when(total == 0, |this| {
                        this.child(muted(
                            "audit-empty",
                            "Nothing has been recorded yet.",
                            cx,
                        ))
                    })
                    .children(drawn)
                    .when(total > MOST_ROWS, |this| {
                        this.child(muted(
                            "audit-capped",
                            format!("Showing the newest {MOST_ROWS} of {total}."),
                            cx,
                        ))
                    })
                    .into_any_element()
            }
        };
        page::padded("settings-page")
            .track_focus(&self.focus)
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(header)
            .child(
                v_flex()
                    .id("settings-audit-log-body")
                    .test_support()
                    .w_full()
                    .gap(dp(12.))
                    .flex_none()
                    .child(
                        div()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child("Writes the app sent to clusters, and Talos Operations, across contexts. Field paths only: values are never recorded."),
                    )
                    .child(
                        h_flex()
                            .id("audit-source")
                            .test_support()
                            .gap(dp(8.))
                            .items_center()
                            .child(div().text_size(dp(12.)).text_color(p.muted).child("Source"))
                            .child(filter),
                    )
                    .child(body),
            )
            .into_any_element()
    }
}

fn muted(id: &'static str, text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .text_size(dp(12.))
        .text_color(palette(cx).muted)
        .child(text)
}

fn render_row(ix: usize, row: &HistoryRow, cx: &App) -> AnyElement {
    let p = palette(cx);
    let when = row.at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let tone = match row.phase.as_str() {
        "observed" | "success" => Tone::Good,
        "refused" | "conflict" | "failure" | "dry_run_failed" => Tone::Crit,
        "not_observed" | "unknown" | "cancelled" => Tone::Warn,
        _ => Tone::Outline,
    };
    let detail = {
        let mut parts = vec![];
        if !row.result.is_empty() {
            parts.push(row.result.clone());
        }
        if !row.fields.is_empty() {
            parts.push(format!("fields: {}", row.fields.join(", ")));
        }
        parts.join(" · ")
    };
    let summary = format!(
        "{when} {} {} {} {} {}",
        row.source.label(),
        row.context,
        row.action,
        row.object,
        row.phase
    );
    v_flex()
        .id(("audit-row", ix))
        .test_support()
        .aria_label(summary)
        .gap(dp(2.))
        .pb(dp(6.))
        .border_b_1()
        .border_color(p.line)
        .text_size(dp(13.))
        .child(
            h_flex()
                .gap(dp(8.))
                .flex_wrap()
                .items_center()
                .child(div().text_color(p.muted).child(when))
                .child(ui::tag(Tone::Outline, None, row.source.label(), cx))
                .child(div().child(row.context.clone()))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(row.action.clone()),
                )
                .child(div().child(row.object.clone()))
                .child(ui::tag(tone, None, row.phase.replace('_', " "), cx)),
        )
        .when(!detail.is_empty(), |this| {
            this.child(div().text_size(dp(12.)).text_color(p.muted).child(detail))
        })
        .into_any_element()
}
