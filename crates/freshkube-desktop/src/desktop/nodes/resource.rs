//! Cached per-node resource facts. Rendering reads these cells without joining
//! metrics, parsing quantities or formatting labels on a frame.
use freshkube_core::resources::{NodeUsage, cpu_millis, quantity};
use freshkube_ui::meters::{self, Resource};
use gpui_kit::{
    component::{h_flex, tooltip::Tooltip},
    prelude::*,
};

use super::*;
use crate::palette::Palette;

pub(super) struct ResourceCell {
    pub(super) used: Option<f64>,
    pub(super) request: Option<f64>,
    pub(super) allocatable: Option<f64>,
    pub(super) stale: bool,
    pub(super) text: SharedString,
    pub(super) tooltip: SharedString,
}

pub(super) struct RowResources {
    pub(super) cpu: ResourceCell,
    pub(super) memory: ResourceCell,
}

fn value(resource: Resource, amount: f64) -> String {
    match resource {
        Resource::Cpu if amount >= 1000. => format!("{:.1}", amount / 1000.),
        Resource::Cpu => format!("{amount:.0}m"),
        Resource::Memory if amount >= 1024. * 1024. * 1024. => {
            format!("{:.1}Gi", amount / (1024. * 1024. * 1024.))
        }
        Resource::Memory => format!("{:.0}Mi", amount / (1024. * 1024.)),
    }
}

impl RowResources {
    pub(super) fn new(
        row: &NodeRow,
        sample: Option<&NodeUsage>,
        metrics_stale: bool,
        reason: &str,
        talos_current: bool,
    ) -> Self {
        let cell = |resource| {
            let parse: fn(&str) -> Option<f64> = match resource {
                Resource::Cpu => cpu_millis,
                Resource::Memory => quantity,
            };
            let select = |amounts: freshkube_core::resources::Amounts| match resource {
                Resource::Cpu => amounts.cpu_millis,
                Resource::Memory => amounts.memory_bytes,
            };
            let metric = sample.and_then(|sample| select(sample.usage));
            let fallback = if metric.is_none() && matches!(resource, Resource::Memory) {
                row.talos.as_ref().and_then(|node| node.memory.as_ref())
            } else {
                None
            };
            let used = metric.or(fallback.map(|memory| memory.used as f64));
            let request = row
                .kubernetes
                .as_ref()
                .filter(|node| node.pods_observed)
                .and_then(|node| select(node.requests));
            let key = if matches!(resource, Resource::Cpu) {
                "cpu"
            } else {
                "memory"
            };
            let allocatable = row
                .kubernetes
                .as_ref()
                .and_then(|node| node.allocatable.get(key))
                .and_then(|q| parse(&q.0))
                .filter(|v| v.is_finite() && *v >= 0.);
            let stale_sources = row.kubernetes.as_ref().is_some_and(|node| {
                !row.kubernetes_current || (node.pods_observed && !node.pods_current)
            });
            let stale = stale_sources
                || if fallback.is_some() {
                    !talos_current
                } else {
                    metrics_stale
                };
            let mut tooltip = meters::tip(
                &meters::Reading {
                    resource,
                    used,
                    request,
                    end: allocatable,
                    kind: meters::End::Allocatable,
                    stale,
                },
                |v| value(resource, v),
            );
            if matches!(resource, Resource::Cpu) {
                tooltip.push_str(" · in cores / millicores");
            }
            if let Some(memory) = fallback {
                tooltip.push_str(&format!(" · Talos memory fallback (physical total {}) · metrics.k8s.io unavailable: {reason}", value(Resource::Memory, memory.total as f64)));
                tooltip.push_str(if talos_current {
                    " · current Talos memory"
                } else {
                    " · last known Talos memory"
                });
            } else if metric.is_some() {
                tooltip.push_str(if metrics_stale {
                    " · last known metrics.k8s.io"
                } else {
                    " · current metrics.k8s.io"
                });
                if let Some(time) = sample.and_then(|sample| sample.sampled_at) {
                    tooltip.push_str(&format!(" · sampled {}", time.to_rfc3339()));
                }
                if metrics_stale && !reason.is_empty() {
                    tooltip.push_str(&format!(" · {reason}"));
                }
            } else {
                tooltip.push_str(&format!(" · metrics.k8s.io unavailable: {reason}"));
            }
            match &row.kubernetes {
                Some(node) => {
                    if !row.kubernetes_current {
                        tooltip.push_str(" · allocatable is last known");
                    }
                    if !node.pods_observed {
                        tooltip.push_str(" · pod requests unavailable");
                    } else if !node.pods_current {
                        tooltip.push_str(" · pod requests are last known");
                    }
                    if request.is_none() {
                        tooltip.push_str(" · request quantity unavailable");
                    }
                }
                None => tooltip.push_str(" · Kubernetes node and pod requests unavailable"),
            }
            if matches!(resource, Resource::Cpu) {
                tooltip.push_str(&format!(" · load averages: {} (not CPU use)", row.load));
            }
            ResourceCell {
                used,
                request,
                allocatable,
                stale,
                text: used
                    .map(|v| value(resource, v))
                    .unwrap_or_else(|| "—".into())
                    .into(),
                tooltip: tooltip.into(),
            }
        };
        Self {
            cpu: cell(Resource::Cpu),
            memory: cell(Resource::Memory),
        }
    }
}

impl ResourceCell {
    pub(super) fn render(&self, resource: Resource, id: ElementId, p: &Palette) -> AnyElement {
        let tooltip = self.tooltip.clone();
        h_flex()
            .id(id)
            .test_support()
            .role(Role::Status)
            .aria_label(tooltip.clone())
            .font_family(ui::MONO_FONT)
            .text_size(ui::dp(12.5))
            .gap(ui::dp(8.))
            .child(
                div()
                    .w(ui::dp(48.))
                    .flex_none()
                    .text_right()
                    .when(self.stale || self.used.is_none(), |this| {
                        this.text_color(p.muted)
                            .group_hover(freshkube_ui::table::ROW_GROUP, |style| {
                                style.text_color(p.ink_2)
                            })
                    })
                    .child(self.text.clone()),
            )
            .children(self.used.map(|used| {
                freshkube_ui::meters::bullet(
                    resource,
                    used,
                    self.request,
                    self.allocatable,
                    self.stale,
                    p,
                )
            }))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .into_any_element()
    }
}

impl Nodes {
    pub(super) fn rebuild_resource_cells(&mut self) {
        let metrics = &self.metrics.snapshot;
        let reason = metrics.error().unwrap_or(if metrics.data().is_some() {
            "node or resource missing from metrics answer"
        } else if self.metrics.source.is_none() {
            "no Kubernetes source"
        } else {
            "awaiting node metrics"
        });
        let now = chrono::Utc::now();
        self.resource_cells = self
            .rows
            .iter()
            .map(|row| {
                let sample = row
                    .key
                    .kubernetes
                    .as_ref()
                    .and_then(|name| metrics.data().and_then(|rows| rows.get(name)));
                let expired = sample
                    .and_then(|sample| sample.sampled_at)
                    .is_some_and(|at| now.signed_duration_since(at).num_seconds() > 45);
                // As on Pods: a NotReady node's kubelet isn't scraped, so its
                // metrics are last known whatever the answer's freshness.
                let not_ready = row.kubernetes.as_ref().is_some_and(|node| !node.is_ready());
                (
                    row.key.clone(),
                    RowResources::new(
                        row,
                        sample,
                        metrics.is_stale() || expired || not_ready,
                        if not_ready {
                            "node is NotReady"
                        } else if expired {
                            "metrics sample is older than 45 s"
                        } else {
                            reason
                        },
                        self.talos_current,
                    ),
                )
            })
            .collect();
    }
}

impl Pilot {
    pub(super) fn nodes_meter_legend(&self, window: &Window, cx: &App) -> AnyElement {
        use freshkube_ui::table;
        let id = self.node_workspace.table.id("meter-legend");
        if crate::screens::page_width(window) < 760. {
            return table::legend_line(id, "Meters: request · use · allocatable ⓘ", "Band = non-terminated pods' requests · line = current use · end = node allocatable. Brighter = above request; amber = at least 85% of allocatable; stripes = last known data.", cx).into_any_element();
        }
        let p = crate::palette::palette(cx);
        table::legend(
            [
                "Meters: band = request · line = use · end = allocatable".into_any_element(),
                table::legend_item(
                    freshkube_ui::meters::bullet(
                        Resource::Cpu,
                        90.,
                        Some(50.),
                        Some(100.),
                        false,
                        &p,
                    ),
                    "≥85% of allocatable",
                )
                .into_any_element(),
                table::legend_item(
                    freshkube_ui::meters::bullet(
                        Resource::Memory,
                        30.,
                        Some(50.),
                        Some(100.),
                        true,
                        &p,
                    ),
                    "last known",
                )
                .into_any_element(),
            ],
            cx,
        )
        .id(id)
        .test_support()
        .into_any_element()
    }
}
