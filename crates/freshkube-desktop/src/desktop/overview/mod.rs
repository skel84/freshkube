//! Overview screen: cluster header, summary tiles and one card per node.
use super::{NextNode, NodeView, PAGE_PADDING, Pilot, PreviousNode, clock};
use crate::palette::palette;
use crate::presentation::{self, ClusterSummary, NodeSummary, Role as NodeRole, Roster};
use crate::ui::{self, DISPLAY_FONT, MONO_FONT, Tone, dp};
use freshkube_core::formatting::format_bytes;
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
            .px(dp(PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(30.))
            .gap(dp(20.))
    }

    /// The talosconfig couldn't be read; shown on every screen.
    pub(super) fn config_error_state(&self, error: String, cx: &mut Context<Self>) -> AnyElement {
        ui::empty_state(
            IconName::FolderOpen,
            "Freshkube can't read your talosconfig",
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
                    .label("Retry")
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
            .gap(dp(7.))
            .child(
                h_flex()
                    .gap_2p5()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(DISPLAY_FONT)
                            .text_size(dp(28.))
                            .line_height(dp(32.))
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
                    .text_size(dp(12.5))
                    .text_color(p.muted)
                    .when(!summary.versions.is_empty(), |this| {
                        this.child(
                            div()
                                .font_family(MONO_FONT)
                                .text_size(dp(12.))
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
                                    .text_size(dp(12.))
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
                            .h(dp(22.))
                            .px_2()
                            .gap_1p5()
                            .rounded_full()
                            .border_1()
                            .border_color(p.line)
                            .bg(p.surface)
                            .text_color(p.ink_2)
                            .text_size(dp(12.))
                            .tooltip(move |window, cx| {
                                Tooltip::new(roster_tip.clone()).build(window, cx)
                            })
                            .child(Icon::new(IconName::Info).size(dp(13.)).text_color(p.muted))
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
                .child(ui::skeleton(dp(64.), dp(11.)))
                .child(ui::skeleton(dp(96.), dp(25.)))
                .child(ui::skeleton(relative(0.7), dp(11.)))
        };
        let card = || {
            v_flex()
                .gap_3()
                .p(dp(14.))
                .rounded(px(10.))
                .border_1()
                .border_color(p.line)
                .bg(p.surface)
                .child(ui::skeleton(dp(110.), dp(11.)))
                .child(ui::skeleton(relative(0.7), dp(15.)))
                .child(ui::skeleton(relative(1.), dp(32.)))
                .child(ui::skeleton(relative(1.), dp(6.)))
                .child(ui::skeleton(relative(0.6), dp(10.)))
        };
        let body = self
            .page_body()
            .child(
                v_flex()
                    .gap_2p5()
                    .child(ui::skeleton(dp(190.), dp(28.)))
                    .child(ui::skeleton(dp(320.), dp(13.))),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(tile_columns(width))
                    .gap(dp(GAP))
                    .children((0..4).map(|_| tile())),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(card_columns(width))
                    .gap(dp(GAP))
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
            .gap(dp(7.))
            .px(dp(14.))
            .pt(dp(12.))
            .pb(dp(13.))
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
                    .gap(dp(7.))
                    .text_color(p.muted)
                    .child(Icon::new(icon).size(dp(15.)).text_color(p.muted))
                    .child(ui::caption(label, cx))
                    .when(interactive, |this| {
                        this.child(div().flex_1()).child(
                            Icon::new(IconName::ChevronRight)
                                .size(dp(15.))
                                .text_color(p.muted),
                        )
                    }),
            )
    }

    fn figure(text: impl Into<SharedString>, color: Option<Hsla>) -> Div {
        div()
            .font_family(DISPLAY_FONT)
            .text_size(dp(25.))
            .line_height(dp(28.))
            .when_some(color, |this, color| this.text_color(color))
            .child(text.into())
    }

    fn summary_tiles(
        &self,
        summary: &ClusterSummary,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let small = |text: String| {
            div()
                .text_size(dp(12.))
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
                    .gap(dp(3.))
                    .flex_wrap()
                    .children(self.nodes.iter().map(|node| {
                        div().w(dp(16.)).h(dp(6.)).rounded(px(2.)).map(|this| {
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
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(
                                div()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(11.5))
                                    .child(service),
                            )
                            .child("on")
                            .child(div().font_family(MONO_FONT).text_size(dp(11.5)).child(node)),
                    )
                    .child(small(format!(
                        "{} healthy · {} not reported",
                        summary.services.healthy, summary.services.unknown
                    )))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.select_node_by_name(target.clone(), window, cx);
                        view.selected_service = Some(chosen.clone());
                        view.health_filter = super::HealthFilter::All;
                        if let Some(node) = view.selected_node.clone() {
                            view.open_node_by_name(
                                &node,
                                crate::desktop::nodes::NodeTab::Services,
                                window,
                                cx,
                            );
                        }
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
                        .text_size(dp(11.5))
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
            .gap(dp(GAP))
            .child(nodes_tile.test_support())
            .child(etcd_tile.test_support())
            .child(services_tile.test_support())
            .child(memory_tile.test_support())
            .into_any_element()
    }
}

mod nodes;

/// Columns for the summary tiles at a content width in `dp`.
fn tile_columns(width: f32) -> u16 {
    if width >= 880. {
        4
    } else if width >= 400. {
        2
    } else {
        1
    }
}

/// Columns for the node cards at a content width in `dp`.
fn card_columns(width: f32) -> u16 {
    let fit = ((width + GAP) / (CARD_MIN_WIDTH + GAP)).floor() as u16;
    fit.clamp(1, 3)
}
