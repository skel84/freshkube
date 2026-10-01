//! Overview screen: cluster header, summary tiles and one card per node.
use super::{NextNode, NodeView, PAGE_PADDING, Page, Pilot, PreviousNode, clock};
use crate::palette::palette;
use crate::presentation::{self, ClusterSummary, NodeSummary, Role as NodeRole, Roster};
use crate::ui::{self, DISPLAY_FONT, MONO_FONT, Tone};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, Selectable, Sizable,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use talos_pilot_core::formatting::format_bytes;

const CARD_MIN_WIDTH: f32 = 290.;
const GAP: f32 = 12.;

fn role_icon(role: NodeRole) -> IconName {
    match role {
        NodeRole::ControlPlane => IconName::ServerCog,
        _ => IconName::Server,
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

impl Pilot {
    pub(super) fn page_scroll(&self, id: &'static str) -> Stateful<Div> {
        div().id(id).size_full().overflow_y_scroll()
    }

    pub(super) fn page_body(&self) -> Div {
        v_flex()
            .px(px(PAGE_PADDING))
            .pt(px(22.))
            .pb(px(30.))
            .gap(px(20.))
    }

    /// The talosconfig couldn't be read; shown on every screen.
    pub(super) fn config_error_state(&self, error: String, cx: &mut Context<Self>) -> AnyElement {
        ui::empty_state(
            IconName::FolderOpen,
            "Talos Pilot can't read your talosconfig",
            "Choose a talosconfig file, or set the TALOSCONFIG environment variable and retry.",
            Some(error),
            vec![
                Button::new("browse-config-empty")
                    .primary()
                    .icon(IconName::FolderOpen)
                    .label("Browse…")
                    .on_click(cx.listener(|view, _, window, cx| view.browse_config(window, cx)))
                    .into_any_element(),
                Button::new("retry-config")
                    .outline()
                    .label("Retry")
                    .on_click(
                        cx.listener(|view, _, window, cx| view.load_configuration(window, cx)),
                    )
                    .into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    }

    /// Unreachable cluster with no earlier snapshot to fall back to.
    pub(super) fn unreachable_state(&self, error: String, cx: &mut Context<Self>) -> AnyElement {
        let context = self.applied.context.clone().unwrap_or_default();
        ui::empty_state(
            IconName::Unplug,
            format!("Can't reach {context}"),
            "No endpoint in this context answered. Check the network path to the Talos API (port 50000), then retry.",
            Some(error),
            vec![
                Button::new("retry")
                    .primary()
                    .icon(IconName::RefreshCw)
                    .label("Retry")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)))
                    .into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    }

    pub(super) fn stale_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.overview.is_stale() {
            return None;
        }
        let failed = self
            .overview
            .last_failure()
            .map(clock)
            .unwrap_or_else(|| "the last attempt".into());
        let kept = self
            .overview
            .last_successful()
            .map(clock)
            .unwrap_or_else(|| "earlier".into());
        let reason = self
            .overview
            .error()
            .unwrap_or("The refresh failed.")
            .to_owned();
        Some(ui::warning_banner(
            Some(format!("Refresh failed at {failed}.").into()),
            format!("You're looking at the snapshot from {kept}. {reason}"),
            Some(
                Button::new("retry")
                    .small()
                    .icon(IconName::RefreshCw)
                    .label("Retry now")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)))
                    .into_any_element(),
            ),
            cx,
        ))
    }

    pub(super) fn render_overview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(error) = self.config_error.clone() {
            return self.config_error_state(error, cx);
        }
        let Some(cluster) = self.overview.data() else {
            if let Some(error) = self
                .overview
                .error()
                .filter(|_| !self.overview.is_loading())
            {
                return self.unreachable_state(error.to_owned(), cx);
            }
            return self.overview_skeleton(window, cx);
        };
        let p = palette(cx);
        let summary = presentation::cluster_summary(cluster, &self.nodes);
        let roster = if self.fixture {
            Roster::Fixture
        } else if !cluster.discovery_members.is_empty() {
            Roster::Discovery
        } else {
            Roster::FallbackOrEndpoints
        };
        let roster_tip = match roster {
            Roster::FallbackOrEndpoints => format!(
                "Node list from the Kubernetes API or the configured endpoints; the collector doesn't say which. Kubernetes access: {}.",
                cluster
                    .kubeconfig_source
                    .as_deref()
                    .unwrap_or("not available")
            ),
            _ => format!(
                "Node list from {}. Kubernetes access: {}.",
                roster.label(),
                cluster
                    .kubeconfig_source
                    .as_deref()
                    .unwrap_or("not available")
            ),
        };
        let warnings: Vec<String> = [&cluster.discovery_warning, &cluster.kubeconfig_warning]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let stale = self.overview.is_stale();
        let versions = summary.versions.clone();
        let header = v_flex()
            .gap(px(7.))
            .child(
                h_flex()
                    .gap_2p5()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(DISPLAY_FONT)
                            .text_size(px(28.))
                            .line_height(px(32.))
                            .child(cluster.name.clone()),
                    )
                    .child(if stale {
                        ui::tag(Tone::Warn, Some(IconName::Clock), "Stale snapshot", cx)
                    } else {
                        ui::tag(Tone::Good, Some(IconName::Plug), "Connected", cx)
                    })
                    .when(versions.len() > 1, |this| {
                        let tip = format!("Nodes run {}", versions.join(" and "));
                        this.child(
                            div()
                                .id("version-drift")
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                                .child(ui::tag(
                                    Tone::Warn,
                                    Some(IconName::TriangleAlert),
                                    format!("{} Talos versions", versions.len()),
                                    cx,
                                )),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .when(!summary.versions.is_empty(), |this| {
                        this.child(
                            div()
                                .font_family(MONO_FONT)
                                .text_size(px(12.))
                                .child(format!("Talos {}", summary.versions.join(" / "))),
                        )
                        .child("·")
                    })
                    .when(
                        !summary.platforms.is_empty() || !summary.arches.is_empty(),
                        |this| {
                            let mut parts = summary.platforms.clone();
                            parts.extend(summary.arches.clone());
                            this.child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(px(12.))
                                    .child(parts.join(" · ")),
                            )
                            .child("·")
                        },
                    )
                    .child(
                        h_flex()
                            .id("roster")
                            .test_support()
                            .aria_label(roster_tip.clone())
                            .h(px(22.))
                            .px_2()
                            .gap_1p5()
                            .rounded_full()
                            .border_1()
                            .border_color(p.line)
                            .bg(p.surface)
                            .text_color(p.ink_2)
                            .text_size(px(12.))
                            .tooltip(move |window, cx| {
                                Tooltip::new(roster_tip.clone()).build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::Info)
                                    .with_size(px(13.))
                                    .text_color(p.muted),
                            )
                            .child(format!("Roster: {}", roster.label())),
                    ),
            );
        let width = Self::content_width(window);
        let body = self
            .page_body()
            .children(self.stale_banner(cx))
            .child(header)
            .children(
                warnings
                    .into_iter()
                    .map(|warning| ui::warning_banner(None, warning, None, cx)),
            )
            .child(self.summary_tiles(&summary, width, cx))
            .child(self.nodes_section(&summary, width, cx));
        self.page_scroll("overview-page")
            .child(body)
            .into_any_element()
    }

    fn overview_skeleton(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let width = Self::content_width(window);
        let count = self
            .applied
            .context
            .as_ref()
            .and_then(|context| self.context_nodes.get(context))
            .copied()
            .unwrap_or(3)
            .max(1);
        let tile = || {
            v_flex()
                .gap_2()
                .p_3()
                .rounded(px(10.))
                .border_1()
                .border_color(p.line)
                .bg(p.surface)
                .child(ui::skeleton(px(64.), px(11.)))
                .child(ui::skeleton(px(96.), px(25.)))
                .child(ui::skeleton(relative(0.7), px(11.)))
        };
        let card = || {
            v_flex()
                .gap_3()
                .p(px(14.))
                .rounded(px(10.))
                .border_1()
                .border_color(p.line)
                .bg(p.surface)
                .child(ui::skeleton(px(110.), px(11.)))
                .child(ui::skeleton(relative(0.7), px(15.)))
                .child(ui::skeleton(relative(1.), px(32.)))
                .child(ui::skeleton(relative(1.), px(6.)))
                .child(ui::skeleton(relative(0.6), px(10.)))
        };
        let body = self
            .page_body()
            .child(
                v_flex()
                    .gap_2p5()
                    .child(ui::skeleton(px(190.), px(28.)))
                    .child(ui::skeleton(px(320.), px(13.))),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(tile_columns(width))
                    .gap(px(GAP))
                    .children((0..4).map(|_| tile())),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(card_columns(width))
                    .gap(px(GAP))
                    .children((0..count).map(|_| card())),
            );
        self.page_scroll("overview-page")
            .child(body)
            .into_any_element()
    }

    fn tile(
        &self,
        id: &'static str,
        icon: IconName,
        label: &str,
        interactive: bool,
        cx: &App,
    ) -> Stateful<Div> {
        let p = palette(cx);
        v_flex()
            .id(id)
            .gap(px(7.))
            .px(px(14.))
            .pt(px(12.))
            .pb(px(13.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .min_w_0()
            .when(interactive, |this| {
                this.cursor_pointer()
                    .tab_index(0)
                    .hover(|style| style.border_color(p.line_strong))
            })
            .child(
                h_flex()
                    .gap(px(7.))
                    .text_color(p.muted)
                    .child(Icon::new(icon).with_size(px(15.)).text_color(p.muted))
                    .child(ui::caption(label, cx))
                    .when(interactive, |this| {
                        this.child(div().flex_1()).child(
                            Icon::new(IconName::ChevronRight)
                                .with_size(px(15.))
                                .text_color(p.muted),
                        )
                    }),
            )
    }

    fn figure(text: impl Into<SharedString>, color: Option<Hsla>) -> Div {
        div()
            .font_family(DISPLAY_FONT)
            .text_size(px(25.))
            .line_height(px(28.))
            .when_some(color, |this, color| this.text_color(color))
            .child(text.into())
    }

    fn summary_tiles(
        &self,
        summary: &ClusterSummary,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let small = |text: String| {
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .min_w_0()
                .child(text)
        };
        let nodes_tile = self
            .tile("tile-nodes", IconName::Server, "Nodes", false, cx)
            .aria_label(format!(
                "{} of {} nodes responding",
                summary.responding, summary.total
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(Self::figure(
                        format!("{} / {}", summary.responding, summary.total),
                        None,
                    ))
                    .child(small("responding".into())),
            )
            .child(
                h_flex()
                    .gap(px(3.))
                    .flex_wrap()
                    .children(self.nodes.iter().map(|node| {
                        div().w(px(16.)).h(px(6.)).rounded(px(2.)).map(|this| {
                            if node.responding {
                                this.bg(p.good)
                            } else {
                                this.border(px(1.5)).border_color(p.unk)
                            }
                        })
                    })),
            )
            .child(small(format!(
                "{} · {}",
                plural(summary.control_planes, "control plane", "control planes"),
                plural(summary.workers, "worker", "workers")
            )));
        let etcd_tile = {
            let tile = self.tile("tile-etcd", IconName::Database, "etcd", false, cx);
            match &summary.etcd {
                Some(etcd) => {
                    let tolerance = presentation::etcd_failure_tolerance(etcd.total);
                    tile.child(
                        h_flex()
                            .gap_2()
                            .child(Self::figure(
                                format!("{} / {}", etcd.healthy, etcd.total),
                                None,
                            ))
                            .child(if etcd.has_quorum {
                                ui::tag(Tone::Good, Some(IconName::CircleCheck), "Quorum", cx)
                            } else {
                                // `healthy` counts members that answered the
                                // status request; silence isn't a failure.
                                ui::tag(
                                    Tone::Warn,
                                    Some(IconName::CircleAlert),
                                    "Quorum unconfirmed",
                                    cx,
                                )
                            }),
                    )
                    .child(small(if !etcd.has_quorum {
                        format!(
                            "{} of {} answered; quorum needs {}",
                            etcd.healthy,
                            plural(etcd.total, "member", "members"),
                            etcd.total / 2 + 1
                        )
                    } else if etcd.total <= 1 {
                        "Single member, so no failure tolerance".into()
                    } else {
                        format!(
                            "Tolerates {}",
                            plural(tolerance, "member failure", "member failures")
                        )
                    }))
                }
                None => tile
                    .child(Self::figure("—", Some(p.muted)))
                    .child(small("No etcd status reported".into())),
            }
        };
        let services_tile = match summary.first_unhealthy.clone() {
            Some((node, service)) => {
                let target = node.clone();
                let chosen = service.clone();
                self.tile("tile-services", IconName::HeartPulse, "Services", true, cx)
                    .aria_label(format!("{} unhealthy services", summary.services.unhealthy))
                    .child(Self::figure(
                        format!("{} unhealthy", summary.services.unhealthy),
                        Some(p.crit_ink),
                    ))
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(px(11.5))
                                    .child(service),
                            )
                            .child("on")
                            .child(div().font_family(MONO_FONT).text_size(px(11.5)).child(node)),
                    )
                    .child(small(format!(
                        "{} healthy · {} not reported",
                        summary.services.healthy, summary.services.unknown
                    )))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.select_node_by_name(target.clone(), window, cx);
                        view.selected_service = Some(chosen.clone());
                        view.health_filter = super::HealthFilter::All;
                        view.navigate(Page::Services, window, cx);
                    }))
            }
            None => {
                let total = summary.services.healthy + summary.services.unknown;
                self.tile("tile-services", IconName::HeartPulse, "Services", false, cx)
                    .child(if total == 0 {
                        Self::figure("—", Some(p.muted))
                    } else {
                        Self::figure("All healthy", Some(p.good_ink))
                    })
                    .child(small(if total == 0 {
                        "No services reported".into()
                    } else {
                        format!(
                            "{} healthy · {} not reported",
                            summary.services.healthy, summary.services.unknown
                        )
                    }))
            }
        };
        let memory_tile = match summary.peak_memory.clone() {
            Some((node, percent)) => {
                let level = presentation::memory_level(percent);
                let target = node.clone();
                self.tile(
                    "tile-memory",
                    IconName::MemoryStick,
                    "Peak memory",
                    true,
                    cx,
                )
                .aria_label(format!("Peak memory {percent:.0} percent on {node}"))
                .child(
                    h_flex()
                        .gap_2()
                        .child(Self::figure(format!("{percent:.0} %"), None))
                        .children(ui::memory_tone(level).map(|(tone, text)| {
                            ui::tag(tone, Some(IconName::TriangleAlert), text, cx)
                        })),
                )
                .child(ui::meter(percent, level, cx))
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child(node),
                )
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.select_node_by_name(target.clone(), window, cx);
                }))
            }
            None => self
                .tile(
                    "tile-memory",
                    IconName::MemoryStick,
                    "Peak memory",
                    false,
                    cx,
                )
                .child(Self::figure("—", Some(p.muted)))
                .child(small("No memory data reported".into())),
        };
        div()
            .grid()
            .grid_cols(tile_columns(width))
            .gap(px(GAP))
            .child(nodes_tile.test_support())
            .child(etcd_tile.test_support())
            .child(services_tile.test_support())
            .child(memory_tile.test_support())
            .into_any_element()
    }

    fn nodes_section(
        &self,
        summary: &ClusterSummary,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let view = self.node_view;
        let header = h_flex()
            .gap_2p5()
            .flex_wrap()
            .mb_3()
            .child(
                div()
                    .font_family(DISPLAY_FONT)
                    .text_size(px(17.))
                    .child("Nodes"),
            )
            .child(div().text_size(px(12.)).text_color(p.muted).child(format!(
                "{} of {} responding · select a node to show it in Services and Logs",
                summary.responding, summary.total
            )))
            .child(div().flex_1())
            .child(
                ButtonGroup::new("node-view")
                    .outline()
                    .small()
                    .child(
                        Button::new("view-cards")
                            .icon(IconName::LayoutGrid)
                            .label("Cards")
                            .selected(view == NodeView::Cards),
                    )
                    .child(
                        Button::new("view-table")
                            .icon(IconName::Table2)
                            .label("Table")
                            .selected(view == NodeView::Table),
                    )
                    .on_click(cx.listener(|view, selected: &Vec<usize>, _, cx| {
                        view.node_view = if selected.first() == Some(&1) {
                            NodeView::Table
                        } else {
                            NodeView::Cards
                        };
                        cx.notify();
                    })),
            );
        let region = div()
            .id("nodes-region")
            .test_support()
            .aria_label("Cluster nodes; use arrow keys to change the target node")
            .key_context("TalosNodes")
            .track_focus(&self.node_focus)
            .on_action(
                cx.listener(|view, _: &PreviousNode, window, cx| view.step_node(-1, window, cx)),
            )
            .on_action(cx.listener(|view, _: &NextNode, window, cx| view.step_node(1, window, cx)));
        let region = match view {
            NodeView::Cards => region
                .grid()
                .grid_cols(card_columns(width))
                .gap(px(GAP))
                .children(
                    self.nodes
                        .iter()
                        .enumerate()
                        .map(|(ix, node)| self.node_card(ix, node, cx).into_any_element()),
                ),
            NodeView::Table => region.overflow_x_scroll().child(self.node_table(cx)),
        };
        v_flex().child(header).child(region).into_any_element()
    }

    fn node_card(&self, ix: usize, node: &NodeSummary, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let target = self.selected_node.as_ref() == Some(&node.name);
        let name = node.name.clone();
        let label = format!("{} · {} · {}", node.name, node.address, node.role.label());
        let card = v_flex()
            .id(SharedString::from(node.name.clone()))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(target)
            .aria_label(label)
            .gap(px(13.))
            .p(px(14.))
            .pb(px(if node.responding { 8. } else { 14. }))
            .rounded(px(10.))
            .border_1()
            .min_w_0()
            .cursor_pointer()
            .map(|this| {
                if node.responding {
                    this.bg(p.surface)
                } else {
                    this.border_dashed()
                }
            })
            .border_color(if target {
                p.accent
            } else if node.responding {
                p.line
            } else {
                p.line_strong
            })
            .when(target, |this| {
                this.shadow(vec![BoxShadow {
                    color: p.accent,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(1.),
                    inset: false,
                }])
            })
            .when(!target, |this| {
                this.hover(|style| style.border_color(p.line_strong))
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_node_by_name(name.clone(), window, cx);
                window.focus(&view.node_focus, cx);
            }));
        let header = h_flex()
            .gap_1p5()
            .flex_wrap()
            .min_h(px(20.))
            .child(
                h_flex()
                    .gap_1p5()
                    .mr_0p5()
                    .child(
                        Icon::new(role_icon(node.role))
                            .with_size(px(15.))
                            .text_color(p.muted),
                    )
                    .child(ui::caption(node.role.label(), cx)),
            )
            .when(node.etcd_member, |this| {
                this.child(ui::tag(Tone::Outline, None, "etcd", cx))
            })
            .when(target, |this| {
                this.child(ui::tag(
                    Tone::Accent,
                    Some(IconName::Crosshair),
                    "Target",
                    cx,
                ))
            })
            .child(div().flex_1())
            .child(if node.responding {
                div()
                    .id(("responding", ix))
                    .tooltip(|window, cx| {
                        Tooltip::new("Responding to the Talos API").build(window, cx)
                    })
                    .child(div().size(px(8.)).rounded_full().bg(p.good))
                    .into_any_element()
            } else {
                ui::tag(
                    Tone::Unknown,
                    Some(IconName::CircleDashed),
                    "No response",
                    cx,
                )
                .into_any_element()
            });
        let identity = v_flex()
            .gap(px(3.))
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(px(18.))
                    .child(node.name.clone()),
            )
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(match &node.version {
                        Some(version) => format!("{} · {version}", node.address),
                        None => node.address.clone(),
                    }),
            );
        let card = card.child(header).child(identity);
        if !node.responding {
            return card
                .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .line_height(px(19.))
                    .child(format!(
                        "The Talos API at {}:50000 didn't answer the last refresh. CPU, memory and services are unknown, not failed.",
                        node.address
                    )),
                )
                .into_any_element();
        }
        let metric_row = |label: String| {
            h_flex().gap_2().flex_wrap().text_size(px(12.)).child(
                div()
                    .text_color(p.ink_2)
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
        };
        let value = |text: String| {
            div()
                .font_family(MONO_FONT)
                .text_size(px(11.5))
                .text_color(p.ink_2)
                .child(text)
        };
        let samples = self.load_history.get(&node.name);
        let sample_count = samples.len();
        let capped = node
            .cores
            .is_some_and(|cores| samples.iter().all(|value| *value <= cores as f64));
        let load =
            v_flex()
                .gap_1p5()
                .child(metric_row("Load".into()).child(div().flex_1()).child(value(
                    match node.load {
                        Some([one, five, fifteen]) => {
                            format!("{one:.2} · {five:.2} · {fifteen:.2}")
                        }
                        None => "Unavailable".into(),
                    },
                )))
                .child(ui::sparkline(samples, node.cores, cx))
                .child(
                    h_flex()
                        .justify_between()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(if sample_count < 2 {
                            "collecting samples…"
                        } else {
                            "last 5 min"
                        })
                        .child(match node.cores {
                            Some(cores) if capped => format!("dashed line = {cores} cores"),
                            Some(cores) => format!("above {cores} cores"),
                            None => "cores unknown".into(),
                        }),
                );
        let memory = match node.memory {
            Some(memory) => {
                let percent = memory.percent();
                let level = memory.level();
                v_flex()
                    .gap_1p5()
                    .child(
                        metric_row("Memory".into())
                            .children(ui::memory_tone(level).map(|(tone, text)| {
                                ui::tag(tone, Some(IconName::TriangleAlert), text, cx)
                            }))
                            .child(div().flex_1())
                            .child(value(format!(
                                "{} / {} · {percent:.0} %",
                                format_bytes(memory.used),
                                format_bytes(memory.total)
                            ))),
                    )
                    .child(ui::meter(percent, level, cx))
            }
            None => v_flex().child(
                metric_row("Memory".into())
                    .child(div().flex_1())
                    .child(value("Unavailable".into())),
            ),
        };
        let counts = node.health_counts();
        let unhealthy: Vec<String> = node.unhealthy_services().map(|s| s.id.clone()).collect();
        let services = v_flex()
            .gap(px(7.))
            .pt(px(11.))
            .border_t_1()
            .border_color(p.line)
            .child(
                h_flex().gap(px(5.)).flex_wrap().min_h(px(10.)).children(
                    node.services
                        .iter()
                        .enumerate()
                        .map(|(service_ix, service)| {
                            let health = presentation::service_health(service);
                            let tip =
                                format!("{} · {}", service.id, presentation::health_text(&health));
                            div()
                                .id(("service-dot", ix * 64 + service_ix))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                                .child(ui::glyph(health, cx))
                        }),
                ),
            )
            .child(
                metric_row(plural(node.services.len(), "service", "services"))
                    .child(div().flex_1())
                    .child(if unhealthy.is_empty() {
                        div().text_color(p.muted).child(format!(
                            "{} healthy · {} not reported",
                            counts.healthy, counts.unknown
                        ))
                    } else {
                        div().text_color(p.crit_ink).child(format!(
                            "{} unhealthy: {}",
                            unhealthy.len(),
                            unhealthy.join(", ")
                        ))
                    }),
            );
        let services_target = node.name.clone();
        let logs_target = node.name.clone();
        let actions = h_flex()
            .gap_0p5()
            .ml(px(-6.))
            .child(
                Button::new(("node-services", ix))
                    .ghost()
                    .small()
                    .icon(IconName::HeartPulse)
                    .label("Services")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.select_node_by_name(services_target.clone(), window, cx);
                        view.navigate(Page::Services, window, cx);
                    })),
            )
            .child(
                Button::new(("node-logs", ix))
                    .ghost()
                    .small()
                    .icon(IconName::ScrollText)
                    .label("Logs")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.select_node_by_name(logs_target.clone(), window, cx);
                        view.navigate(Page::Logs, window, cx);
                    })),
            );
        card.child(load)
            .child(memory)
            .child(services)
            .child(actions)
            .into_any_element()
    }

    fn node_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        // Node takes the remaining width; the rest fit a 1040 px pane.
        const COLUMNS: [(&str, f32); 8] = [
            ("Role", 126.),
            ("Node", 200.),
            ("Address", 112.),
            ("Status", 124.),
            ("CPU", 80.),
            ("Load 1 / 5 / 15", 150.),
            ("Memory", 122.),
            ("Services", 110.),
        ];
        let cell = |ix: usize| {
            let cell = div().px_3().min_w_0().whitespace_nowrap();
            if ix == 1 {
                cell.flex_1().min_w(px(COLUMNS[ix].1))
            } else {
                cell.flex_none().w(px(COLUMNS[ix].1))
            }
        };
        let head = h_flex()
            .py(px(10.))
            .border_b_1()
            .border_color(p.line)
            .children(
                COLUMNS
                    .iter()
                    .enumerate()
                    .map(|(ix, (label, _))| cell(ix).child(ui::caption(label, cx))),
            );
        let rows =
            self.nodes.iter().enumerate().map(|(ix, node)| {
                let target = self.selected_node.as_ref() == Some(&node.name);
                let name = node.name.clone();
                let counts = node.health_counts();
                h_flex()
                    .id(SharedString::from(node.name.clone()))
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_selected(target)
                    .aria_label(format!(
                        "{} · {} · {}",
                        node.name,
                        node.address,
                        node.role.label()
                    ))
                    .py(px(9.))
                    .border_b_1()
                    .border_color(p.line)
                    .cursor_pointer()
                    .when(target, |this| this.bg(p.accent_soft))
                    .when(!target, |this| this.hover(|style| style.bg(p.hover)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.select_node_by_name(name.clone(), window, cx);
                        window.focus(&view.node_focus, cx);
                    }))
                    .child(
                        cell(0).child(
                            h_flex()
                                .gap_1p5()
                                .child(
                                    Icon::new(role_icon(node.role))
                                        .with_size(px(14.))
                                        .text_color(p.muted),
                                )
                                .child(div().text_size(px(12.5)).child(node.role.label())),
                        ),
                    )
                    .child(
                        cell(1)
                            .font_family(MONO_FONT)
                            .text_size(px(12.5))
                            .child(node.name.clone()),
                    )
                    .child(
                        cell(2)
                            .font_family(MONO_FONT)
                            .text_size(px(12.))
                            .child(node.address.clone()),
                    )
                    .child(cell(3).child(if node.responding {
                        ui::tag(Tone::Good, None, "Responding", cx)
                    } else {
                        ui::tag(
                            Tone::Unknown,
                            Some(IconName::CircleDashed),
                            "No response",
                            cx,
                        )
                    }))
                    .child(
                        cell(4).text_size(px(12.5)).child(
                            node.cores
                                .map(|c| format!("{c} cores"))
                                .unwrap_or_else(|| "—".into()),
                        ),
                    )
                    .child(cell(5).font_family(MONO_FONT).text_size(px(12.)).child(
                        match node.load {
                            Some([a, b, c]) => format!("{a:.2} / {b:.2} / {c:.2}"),
                            None => "—".into(),
                        },
                    ))
                    .child(
                        cell(6).child(match node.memory {
                            Some(memory) => h_flex()
                                .id(("memory-cell", ix))
                                .gap_2()
                                .tooltip({
                                    let detail = format!(
                                        "{} of {} used",
                                        format_bytes(memory.used),
                                        format_bytes(memory.total)
                                    );
                                    move |window, cx| Tooltip::new(detail.clone()).build(window, cx)
                                })
                                .child(div().w(px(48.)).child(ui::meter(
                                    memory.percent(),
                                    memory.level(),
                                    cx,
                                )))
                                .child(
                                    div()
                                        .font_family(MONO_FONT)
                                        .text_size(px(12.))
                                        .child(format!("{:.0} %", memory.percent())),
                                )
                                .into_any_element(),
                            None => div().child("—").into_any_element(),
                        }),
                    )
                    .child(cell(7).text_size(px(12.5)).child(if counts.unhealthy > 0 {
                        div()
                            .text_color(p.crit_ink)
                            .child(format!("{} unhealthy", counts.unhealthy))
                    } else if node.services.is_empty() {
                        div().text_color(p.muted).child("—")
                    } else {
                        div().text_color(p.muted).child(plural(
                            node.services.len(),
                            "service",
                            "services",
                        ))
                    }))
            });
        v_flex()
            .min_w(px(COLUMNS.iter().map(|(_, width)| width).sum::<f32>()))
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .overflow_hidden()
            .child(head)
            .children(rows)
            .into_any_element()
    }
}

fn tile_columns(width: Pixels) -> u16 {
    if width >= px(880.) {
        4
    } else if width >= px(400.) {
        2
    } else {
        1
    }
}

fn card_columns(width: Pixels) -> u16 {
    let fit = ((width + px(GAP)) / px(CARD_MIN_WIDTH + GAP)).floor() as u16;
    fit.clamp(1, 3)
}
