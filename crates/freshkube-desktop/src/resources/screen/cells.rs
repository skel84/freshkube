//! The table's cells beyond the printed text: the status glyph, the name
//! with its namespace and generated suffix, the owner, a pod's readiness
//! and restarts, its use against its request and limit, and its node.
//! Tooltips are built when hovered, from what the row already holds.

use std::sync::Arc;

use freshkube_core::resources::Amounts;
use gpui_kit::component::tooltip::Tooltip;

use super::super::model::ResourceRow;
use super::super::rows::{PodRow, PodState, RowOwner, died};
use super::layout::DisplayColumn;
use super::*;
use crate::palette::Palette;
use crate::presentation::whole_percent;
use freshkube_ui::table::ROW_GROUP;
pub(super) use freshkube_ui::table::cell;

/// The use bar's size and the figure's width beside it.
const FIGURE_WIDTH: f32 = 40.;
/// How much faster than the reason a name's generated suffix shrinks.
const SUFFIX_SHRINK: f32 = 1000.;

/// The glyph's tone for a pod's state: a skull where a container ran and
/// stopped, the critical disc where the pod can't run. A pod on a node
/// that isn't ready can't be confirmed, so it warns.
pub(super) fn pod_tone(pod: &PodRow, node_ready: bool) -> ui::Tone {
    match pod.state {
        PodState::Failing if died(&pod.status) => ui::Tone::Died,
        PodState::Failing => ui::Tone::Crit,
        PodState::NotReady => ui::Tone::Warn,
        _ if !node_ready => ui::Tone::Warn,
        PodState::Running | PodState::Completed => ui::Tone::Good,
        PodState::Pending | PodState::Terminating | PodState::Unknown => ui::Tone::Unknown,
    }
}

/// A workload's tone from its ready replicas of all: every one ready, some,
/// or none of several. A workload scaled to zero is fine.
pub(super) fn ready_tone(text: &str) -> Option<ui::Tone> {
    let (up, all) = text.split_once('/')?;
    let (up, all): (u32, u32) = (up.trim().parse().ok()?, all.trim().parse().ok()?);
    Some(if up >= all {
        ui::Tone::Good
    } else if up == 0 {
        ui::Tone::Crit
    } else {
        ui::Tone::Warn
    })
}

/// A printed status's tone, for kinds other than pods.
pub(super) fn printed_tone(text: &str) -> ui::Tone {
    match status_tone(text) {
        StatusTone::Success => ui::Tone::Good,
        StatusTone::Warning => ui::Tone::Warn,
        StatusTone::Danger => ui::Tone::Crit,
        StatusTone::Neutral => ui::Tone::Unknown,
    }
}

fn tooltip(this: Stateful<Div>, text: impl Fn() -> String + 'static) -> Stateful<Div> {
    this.tooltip(move |window, cx| Tooltip::new(text()).build(window, cx))
}

/// The glyph, or a check where the row is marked.
pub(super) fn glyph(
    column: &DisplayColumn,
    tone: Option<(ui::Tone, SharedString)>,
    marked: bool,
    completed: bool,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    let cell = cell(column)
        .id("glyph")
        .flex()
        .items_center()
        .justify_center()
        .px_0();
    if marked {
        return cell
            .child(
                Icon::new(IconName::SquareCheck)
                    .size(dp(14.))
                    .text_color(p.accent),
            )
            .into_any_element();
    }
    if completed {
        return tooltip(
            cell.child(Icon::new(IconName::Check).size(dp(14.)).text_color(p.muted)),
            || "Completed".into(),
        )
        .into_any_element();
    }
    match tone {
        Some((tone, label)) => tooltip(cell.children(ui::status_glyph(tone, cx)), move || {
            label.to_string()
        })
        .into_any_element(),
        None => cell.into_any_element(),
    }
}

/// The name, after a muted namespace when listing every namespace, with
/// the part its owner generated dimmed, and for a pod that isn't healthy,
/// why.
pub(super) fn name(
    column: &DisplayColumn,
    row: &ResourceRow,
    printed: &str,
    namespaced: bool,
    reason: Option<(Hsla, &str)>,
    p: &Palette,
) -> AnyElement {
    let (stem, suffix) = match row.generated.filter(|ix| printed.is_char_boundary(*ix)) {
        Some(ix) => printed.split_at(ix),
        None => (printed, ""),
    };
    let address = row.identity.address();
    let why = reason.map(|(_, text)| text.to_owned());
    tooltip(
        cell(column)
            .id("name")
            .flex()
            .items_baseline()
            .when(namespaced && !row.identity.namespace.is_empty(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_color(p.muted)
                        .group_hover(ROW_GROUP, |style| style.text_color(p.ink_2))
                        .child(format!("{}/", row.identity.namespace)),
                )
            })
            .child(div().flex_none().child(stem.to_owned()))
            // The generated suffix gives way first, so the reason shows.
            .when(!suffix.is_empty(), |this| {
                this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .map(|mut this| {
                            this.style().flex_shrink = Some(SUFFIX_SHRINK);
                            this
                        })
                        .text_color(p.faint)
                        .child(suffix.to_owned()),
                )
            })
            .when_some(reason, |this, (color, text)| {
                this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .pl(dp(10.))
                        .text_size(dp(11.5))
                        .text_color(color)
                        .child(text.to_owned()),
                )
            }),
        move || match &why {
            Some(why) => format!("{address}\n{why}"),
            None => address.clone(),
        },
    )
    .test_support()
    .into_any_element()
}

/// `deploy/` muted and the owner's name.
pub(super) fn owner(
    column: &DisplayColumn,
    owner: Option<&RowOwner>,
    namespace: &str,
    selected: bool,
    cx: &Context<ResourcesScreen>,
) -> AnyElement {
    let mut p = palette(cx);
    if selected {
        p.muted = p.ink_2;
    }
    let Some(owner) = owner else {
        return cell(column)
            .text_color(p.muted)
            .group_hover(ROW_GROUP, |style| style.text_color(p.ink_2))
            .child("—")
            .into_any_element();
    };
    let label = format!("{} {}", owner.kind, owner.name);
    let key = match owner.kind.as_str() {
        "Deployment" => "deployments.apps",
        "ReplicaSet" => "replicasets.apps",
        "StatefulSet" => "statefulsets.apps",
        "DaemonSet" => "daemonsets.apps",
        "ReplicationController" => "replicationcontrollers",
        "CronJob" => "cronjobs.batch",
        "Job" => "jobs.batch",
        _ => "",
    };
    let destination = freshkube_core::resources::builtin(key).map(|kind| {
        super::super::ResourceLink::Object(
            kind,
            super::super::model::ObjectRef {
                namespace: namespace.into(),
                name: owner.name.clone(),
                uid: String::new(),
            },
            super::super::Tab::Overview,
        )
    });
    tooltip(
        cell(column)
            .id("owner")
            .flex()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .text_color(p.muted)
                    .group_hover(ROW_GROUP, |style| style.text_color(p.ink_2))
                    .child(format!("{}/", owner.short)),
            )
            .when_else(
                destination.is_some(),
                |this| this.child(ui::reference(owner.name.clone(), &p)),
                |this| this.child(div().text_color(p.ink_2).child(owner.name.clone())),
            )
            .when_some(destination, |this, destination| {
                this.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(destination.clone());
                    }),
                )
            }),
        move || label.clone(),
    )
    .into_any_element()
}

/// `0/1 ⟲14`: ready containers of all, then restarts when there were any.
pub(super) fn ready(column: &DisplayColumn, pod: &Arc<PodRow>, p: &Palette) -> AnyElement {
    let all_ready = pod.ready.split_once('/').is_none_or(|(up, all)| up == all);
    let pod_for_tip = pod.clone();
    tooltip(
        cell(column)
            .id("ready")
            .flex()
            .gap(dp(8.))
            .child(
                div()
                    .when(!all_ready && pod.state != PodState::Completed, |this| {
                        this.text_color(p.warn_ink)
                    })
                    .child(pod.ready.clone()),
            )
            .when(pod.restarts > 0, |this| {
                let color = if pod.restarts >= 5 {
                    p.warn_ink
                } else {
                    p.muted
                };
                this.child(
                    h_flex()
                        .gap(dp(2.))
                        .text_color(color)
                        .child(
                            Icon::new(IconName::RotateCcw)
                                .size(dp(11.))
                                .text_color(color),
                        )
                        .child(pod.restarts.to_string()),
                )
            }),
        move || readiness(&pod_for_tip),
    )
    .into_any_element()
}

fn readiness(pod: &PodRow) -> String {
    let mut lines = vec![format!(
        "{} containers ready · {} restart{}",
        pod.ready,
        pod.restarts,
        if pod.restarts == 1 { "" } else { "s" }
    )];
    for container in &pod.containers {
        let mut line = format!(
            "{}: {}",
            container.name,
            if container.ready {
                "ready"
            } else {
                "not ready"
            }
        );
        if let Some(waiting) = &container.waiting {
            line.push_str(&format!(" · {waiting}"));
        }
        if let Some(code) = container.last_exit_code {
            line.push_str(&format!(" · last exit {code}"));
            if let Some(reason) = &container.last_reason {
                line.push_str(&format!(" ({reason})"));
            }
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// Which of a pod's amounts a use cell shows.
#[derive(Clone, Copy)]
pub(super) enum Resource {
    Cpu,
    Memory,
}

impl Resource {
    fn of(self, amounts: &Amounts) -> Option<f64> {
        match self {
            Resource::Cpu => amounts.cpu_millis,
            Resource::Memory => amounts.memory_bytes,
        }
    }

    fn format(self, value: f64) -> String {
        match self {
            Resource::Cpu if value >= 1000. => format!("{:.1}", value / 1000.),
            Resource::Cpu => format!("{value:.0}m"),
            Resource::Memory => {
                const MI: f64 = 1024. * 1024.;
                if value >= 1024. * MI {
                    format!("{:.1}Gi", value / (1024. * MI))
                } else {
                    format!("{:.0}Mi", value / MI)
                }
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Resource::Cpu => "CPU",
            Resource::Memory => "Memory",
        }
    }
}

/// A neutral value beside a Fog bullet meter. Stripes identify last-known use.
#[allow(clippy::too_many_arguments)]
pub(super) fn usage(
    column: &DisplayColumn,
    id: &'static str,
    resource: Resource,
    pod: &PodRow,
    used: Option<&Amounts>,
    stale: bool,
    p: &Palette,
) -> AnyElement {
    let used = used.and_then(|used| resource.of(used));
    let request = resource.of(&pod.requests);
    let limit = resource.of(&pod.limits);
    let Some(value) = used else {
        return cell(column)
            .text_color(p.muted)
            .group_hover(ROW_GROUP, |style| style.text_color(p.ink_2))
            .child("—")
            .into_any_element();
    };
    let text = |amount: Option<f64>| amount.map_or("none".to_owned(), |v| resource.format(v));
    let percent = limit
        .filter(|v| *v > 0.)
        .map(|v| format!(" · {}% of limit", whole_percent(value / v * 100.)))
        .unwrap_or_default();
    let requested = match (request, limit.filter(|v| *v > 0.)) {
        (Some(request), Some(limit)) => format!(
            "{} ({:.0}%)",
            resource.format(request),
            request / limit * 100.
        ),
        _ => text(request),
    };
    let tip = format!(
        "{} {} used{}{percent} · requested {requested} · limit {}",
        resource.name(),
        resource.format(value),
        if stale { " (last known)" } else { "" },
        text(limit),
    );
    tooltip(
        cell(column)
            .id(id)
            .flex()
            .items_center()
            .gap(dp(8.))
            .child(
                div()
                    .flex_none()
                    .w(dp(FIGURE_WIDTH))
                    .text_right()
                    .when(stale, |this| {
                        this.text_color(p.muted)
                            .group_hover(ROW_GROUP, |style| style.text_color(p.ink_2))
                    })
                    .child(resource.format(value)),
            )
            .child(crate::meters::bullet(
                match resource {
                    Resource::Cpu => crate::meters::Resource::Cpu,
                    Resource::Memory => crate::meters::Resource::Memory,
                },
                value,
                request,
                limit,
                stale,
                p,
            )),
        move || tip.clone(),
    )
    .into_any_element()
}

/// The node without the prefix every pod's node shares, and a warning
/// before one that isn't ready.
pub(super) fn node(
    column: &DisplayColumn,
    node: &str,
    prefix: usize,
    ready: bool,
    cx: &Context<ResourcesScreen>,
) -> AnyElement {
    let p = palette(cx);
    let shown = node.get(prefix..).filter(|_| prefix > 0).unwrap_or(node);
    let destination = node.to_owned();
    let full = if ready {
        node.to_owned()
    } else {
        format!("{node} · NotReady")
    };
    tooltip(
        cell(column)
            .id("node")
            .flex()
            .items_center()
            .gap(dp(6.))
            .when(!ready, |this| {
                this.text_color(p.warn_ink)
                    .children(ui::status_glyph(ui::Tone::Warn, cx))
            })
            .child(ui::reference(shown.to_owned(), &p))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(super::super::ResourceLink::Node(
                        destination.clone(),
                        crate::desktop::nodes::NodeTab::Overview,
                    ));
                }),
            ),
        move || full.clone(),
    )
    .into_any_element()
}
