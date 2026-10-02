//! Window chrome: title bar, sidebar, status bar and their popovers.
use super::kubernetes_only::{self, KubeConnection};
use super::{AUTO_REFRESH, Appearance, Page, Pilot, SIDEBAR_WIDTH, SidebarReveal, clock};
use crate::mutation::Operations;
use crate::palette::palette;
use crate::presentation::{self, Role as NodeRole};
use crate::resources::custom::{CustomGroup, Discovery};
use crate::resources::navigation::{self, NavGroup};
use crate::text_size;
use crate::ui::{self, DISPLAY_FONT, MONO_FONT, Tone, dp};
use freshkube_core::resources::{Failure, FailureKind};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    input::Input,
    popover::Popover,
    scroll::{Scrollbar, ScrollbarMode},
    status_bar::StatusBar,
    switch::Switch,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// Most custom API groups, and kinds per group, the sidebar lists; a row
/// says how many more there are.
const MAX_SIDEBAR_GROUPS: usize = 300;
const MAX_SIDEBAR_KINDS: usize = 200;

fn role_icon(role: NodeRole) -> IconName {
    match role {
        NodeRole::ControlPlane => IconName::ServerCog,
        _ => IconName::Server,
    }
}

/// What one Kubernetes sidebar row shows.
struct NavRow {
    id: SharedString,
    label: SharedString,
    tooltip: Option<SharedString>,
    indent: Rems,
}

impl NavRow {
    fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, indent: Rems) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tooltip: None,
            indent,
        }
    }

    fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

type RowAction = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Rows of the sidebar's Custom Resources, and where a reveal lands among
/// them.
struct CustomRows {
    rows: Vec<AnyElement>,
    reveal: Option<usize>,
    /// False while the rows a reveal wants are still being discovered.
    settled: bool,
}

/// Why discovery shows nothing, in a few words; the tooltip says more.
fn failure_label(failure: &Failure) -> &'static str {
    match failure.kind {
        FailureKind::Forbidden => "Not permitted",
        FailureKind::NotFound => "No longer served",
        FailureKind::Timeout => "Timed out",
        FailureKind::Unreachable => "Unreachable",
        _ => "Couldn't discover",
    }
}

impl Pilot {
    pub(super) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let unhealthy = self
            .overview
            .data()
            .map(|cluster| {
                presentation::cluster_summary(cluster, &self.nodes)
                    .services
                    .unhealthy
            })
            .unwrap_or(0);
        let collecting = self.logs.read(cx).collecting_count();
        let services_suffix = (unhealthy > 0).then(|| {
            div()
                .min_w(dp(18.))
                .h(dp(18.))
                .px(dp(5.))
                .rounded_full()
                .bg(p.crit)
                .text_color(gpui_kit::white())
                .text_size(dp(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .flex()
                .items_center()
                .justify_center()
                .child(unhealthy.to_string())
                .into_any_element()
        });
        let logs_suffix = (collecting > 0).then(|| {
            div()
                .size(dp(7.))
                .rounded_full()
                .bg(p.good)
                .into_any_element()
        });
        let section =
            |label: &str, cx: &App| div().px_2().pt_3().pb_1().child(ui::caption(label, cx));
        let talos = v_flex()
            .gap_0p5()
            .child(self.nav_item(
                Page::Overview,
                IconName::LayoutDashboard,
                Some("1"),
                None,
                cx,
            ))
            .child(self.nav_item(
                Page::Services,
                IconName::HeartPulse,
                Some("2"),
                services_suffix,
                cx,
            ))
            .child(self.nav_item(Page::Logs, IconName::ScrollText, Some("3"), logs_suffix, cx))
            .child(section("Node", cx))
            .child(self.nav_item(Page::Processes, IconName::Cpu, Some("4"), None, cx))
            .child(self.nav_item(Page::Storage, IconName::HardDrive, Some("5"), None, cx))
            .child(self.nav_item(Page::Network, IconName::Network, Some("6"), None, cx))
            .child(self.nav_item(
                Page::Diagnostics,
                IconName::Stethoscope,
                Some("7"),
                None,
                cx,
            ))
            .child(section("Cluster", cx))
            .child(self.nav_item(Page::Etcd, IconName::Database, Some("8"), None, cx))
            .child(self.nav_item(Page::Workloads, IconName::Boxes, Some("9"), None, cx))
            .child(self.nav_item(Page::Security, IconName::ShieldCheck, None, None, cx))
            .child(self.nav_item(Page::Lifecycle, IconName::Layers, None, None, cx));
        // Kubernetes rows are children of the scrolling element, so one can
        // be scrolled into view: the Talos block and the section caption
        // come first.
        const FIRST_ROW: usize = 2;
        let current = (self.page == Page::Resources).then(|| self.resource_kind.key());
        let mut rows: Vec<AnyElement> = navigation::NAVIGATION
            .iter()
            .flat_map(|group| self.kubernetes_group(group, current.as_deref(), cx))
            .collect();
        let built_in = rows.len();
        let custom = self.custom_resources(current.as_deref(), self.sidebar_reveal.as_ref(), cx);
        rows.extend(custom.rows);
        // Scrolling needs the navigation's size, which the first frame of a
        // window doesn't know yet; the reveal waits a frame then.
        if let Some(reveal) = &self.sidebar_reveal {
            if self.sidebar_scroll.bounds().size.height > px(0.) {
                if let Some(row) = self
                    .kubernetes_row(reveal)
                    .or(custom.reveal.map(|row| built_in + row))
                {
                    self.sidebar_scroll.scroll_to_item(FIRST_ROW + row);
                }
                // Rows still being discovered will grow the block; it is
                // revealed again once they arrive.
                if custom.settled {
                    self.sidebar_reveal = None;
                }
            } else {
                window.request_animation_frame();
            }
        }
        let nav = v_flex()
            .id("sidebar-scroll")
            .test_support()
            .aria_label("Screens")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.sidebar_scroll)
            // Scrolling by hand cancels a reveal still waiting for discovery.
            .on_scroll_wheel(cx.listener(|view, _, _, _| view.sidebar_reveal = None))
            .gap_0p5()
            .child(talos)
            .child(section("Kubernetes", cx))
            .children(rows)
            .child(
                v_flex()
                    .gap_0p5()
                    .child(section("Maintain", cx))
                    .child(self.nav_item(Page::Operations, IconName::Wrench, None, None, cx)),
            );
        let contexts = if self.config_loading {
            div()
                .px_2()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child("Loading contexts…")
                .into_any_element()
        } else if self.config_error.is_some() || self.contexts.is_empty() {
            div()
                .px_2()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child("No contexts loaded")
                .into_any_element()
        } else {
            v_flex()
                .gap_0p5()
                .children(
                    self.contexts
                        .iter()
                        .enumerate()
                        .map(|(ix, context)| self.context_item(ix, context, cx)),
                )
                .into_any_element()
        };
        let pilot = cx.entity().downgrade();
        let open_pilot = pilot.clone();
        let path_input = self.path.clone();
        let path_label = if self.fixture {
            "Example data".to_owned()
        } else if let Some(kube) = &self.kubernetes_only {
            format!("Kubeconfig: {}", kube.files())
        } else {
            self.applied
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Default talosconfig".into())
        };
        v_flex()
            .id("sidebar")
            .w(dp(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .px(dp(10.))
            .pt_4()
            .pb_2p5()
            .gap(dp(22.))
            .child(
                h_flex()
                    .gap_2p5()
                    .px_1p5()
                    .child(
                        div()
                            .size(dp(28.))
                            .rounded(px(7.))
                            .bg(p.accent)
                            .text_color(cx.theme().primary_foreground)
                            .font_family(DISPLAY_FONT)
                            .text_size(dp(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("F"),
                    )
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .font_family(DISPLAY_FONT)
                                    .text_size(dp(15.))
                                    .child("Freshkube"),
                            )
                            .child(div().text_size(dp(11.5)).text_color(p.muted).child(
                                if self.fixture {
                                    "Example data"
                                } else {
                                    concat!("v", env!("CARGO_PKG_VERSION"))
                                },
                            )),
                    ),
            )
            .child(
                // Navigation scrolls when the window is short or many
                // Kubernetes groups are open; the bar shows there is more.
                div().relative().flex_1().min_h_0().child(nav).child(
                    Scrollbar::vertical(&self.sidebar_scroll)
                        .id("sidebar-scrollbar")
                        .mode(ScrollbarMode::Hover),
                ),
            )
            .child(
                // Contexts stay in view below the navigation, scrolling on
                // their own when there are many.
                v_flex()
                    .flex_none()
                    .max_h(relative(0.4))
                    .min_h_0()
                    .gap_0p5()
                    .pt(dp(14.))
                    .mt(dp(-8.))
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        h_flex()
                            .justify_between()
                            .px_2()
                            .pb_1p5()
                            .child(ui::caption("Contexts", cx))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(ui::keycap("⌥↑", cx))
                                    .child(ui::keycap("⌥↓", cx)),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("context-scroll")
                            .test_support()
                            .aria_label("Contexts")
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(contexts),
                    ),
            )
            .child(
                h_flex().child(
                    Popover::new("settings-popover")
                        .anchor(Anchor::BottomLeft)
                        .open(self.settings_open)
                        .on_open_change(move |open, _, cx| {
                            let open = *open;
                            _ = open_pilot.update(cx, |view, cx| {
                                view.settings_open = open;
                                cx.notify();
                            });
                        })
                        .trigger(
                            Button::new("settings")
                                .ghost()
                                .icon(IconName::Settings)
                                .label("Settings")
                                .tooltip(path_label),
                        )
                        .content(move |_, window, cx| {
                            settings_content(pilot.clone(), path_input.clone(), window, cx)
                        }),
                ),
            )
            .into_any_element()
    }

    fn nav_item(
        &self,
        page: Page,
        icon: IconName,
        key: Option<&str>,
        suffix: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let active = self.page == page;
        // Every page but Resources reads the Talos API.
        let unavailable = self.kubernetes_only.is_some() && page != Page::Resources;
        h_flex()
            .id(SharedString::from(format!("nav-{}", page.slug())))
            .test_support()
            .role(Role::Tab)
            .aria_selected(active)
            .aria_label(page.title())
            .tab_index(0)
            .h(dp(30.))
            .flex_none()
            .px_2()
            .gap_2p5()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(dp(13.))
            .text_color(if active { p.ink } else { p.ink_2 })
            .when(active, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .border_1()
                    .border_color(p.line)
                    .shadow_xs()
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!active, |this| this.hover(|style| style.bg(p.hover)))
            .when(unavailable && !active, |this| this.opacity(0.5))
            .when(unavailable, |this| {
                this.tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("Needs a talosconfig")
                        .build(window, cx)
                })
            })
            .child(Icon::new(icon).size(dp(16.)).text_color(if active {
                p.accent
            } else {
                p.ink_2
            }))
            .child(page.title())
            .child(div().flex_1())
            .children(suffix)
            .children(key.map(|key| ui::keycap(format!("{}{key}", ui::modifier()), cx)))
            .on_click(
                cx.listener(move |view, _, window, cx| {
                    view.navigate_from_keyboard(page, window, cx)
                }),
            )
            .into_any_element()
    }

    /// Where a reveal lands among the built-in Kubernetes rows, counting
    /// group headers and the kinds of open groups.
    fn kubernetes_row(&self, reveal: &SidebarReveal) -> Option<usize> {
        let mut row = 0;
        for group in &navigation::NAVIGATION {
            let header = row;
            let open = self.kubernetes_groups.contains(group.slug);
            let shown = if open { group.items.len() } else { 0 };
            match reveal {
                SidebarReveal::Kind(key) => {
                    if let Some(ix) = group.items.iter().position(|(_, item)| item == key) {
                        return Some(if open { header + 1 + ix } else { header });
                    }
                }
                SidebarReveal::Group(slug) if *slug == group.slug => return Some(header + shown),
                SidebarReveal::Group(_) | SidebarReveal::Custom | SidebarReveal::ApiGroup(_) => {}
            }
            row = header + 1 + shown;
        }
        None
    }

    /// A collapsible group of Kubernetes kinds, as Kubeli groups them: its
    /// header, then its kinds while it is open; each a row of the scrolling
    /// navigation.
    fn kubernetes_group(
        &self,
        group: &'static NavGroup,
        current: Option<&str>,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let open = self.kubernetes_groups.contains(group.slug);
        // A closed group still shows that the page is one of its kinds.
        let holds_current =
            current.is_some_and(|key| group.items.iter().any(|(_, item)| *item == key));
        let slug = group.slug;
        let mut rows = vec![self.nav_header(
            NavRow::new(format!("nav-k8s-group-{slug}"), group.label, dp(8.)),
            open,
            holds_current,
            cx.listener(move |view, _, _, cx| view.toggle_kubernetes_group(slug, cx)),
            cx,
        )];
        if open {
            rows.extend(group.items.iter().map(|(label, key)| {
                self.kubernetes_item(
                    NavRow::new(format!("nav-k8s-{key}"), *label, dp(30.)),
                    current == Some(*key),
                    cx.listener(move |view, _, window, cx| view.open_builtin(key, window, cx)),
                    cx,
                )
            }));
        }
        rows
    }

    /// Custom Resources: discovery's state or one header per API group,
    /// with the kinds of each open group. Also where `reveal` lands among
    /// these rows, and whether discovery has settled enough to stop
    /// revealing it.
    fn custom_resources(
        &self,
        current: Option<&str>,
        reveal: Option<&SidebarReveal>,
        cx: &Context<Self>,
    ) -> CustomRows {
        let custom = self.custom.read(cx);
        let open = custom.is_open();
        // The API group of the kind shown, when it is a custom kind.
        let current_group = current
            .filter(|key| navigation::group_of(key).is_none())
            .map(|_| self.resource_kind.group.as_str());
        // A custom kind to reveal, and the group it is in.
        let reveal_kind = match reveal {
            Some(SidebarReveal::Kind(key)) if navigation::group_of(key).is_none() => {
                Some(key.as_str())
            }
            _ => None,
        };
        let reveal_group = match reveal {
            Some(SidebarReveal::ApiGroup(name)) => Some(name.as_str()),
            _ => reveal_kind.and_then(|key| key.split_once('.').map(|(_, group)| group)),
        };
        let mut out = CustomRows {
            rows: vec![self.nav_header(
                NavRow::new("nav-k8s-group-custom", "Custom Resources", dp(8.)),
                open,
                current_group.is_some(),
                cx.listener(|view, _, _, cx| view.toggle_custom_resources(cx)),
                cx,
            )],
            reveal: None,
            settled: true,
        };
        let revealing = matches!(reveal, Some(SidebarReveal::Custom)) || reveal_group.is_some();
        if revealing {
            out.reveal = Some(0);
        }
        if !open {
            return out;
        }
        let status = |text: &'static str| NavRow::new("nav-k8s-custom-status", text, dp(30.));
        match custom.groups() {
            None => out
                .rows
                .push(self.nav_status(status("Not connected"), None, cx)),
            Some(Discovery::Reading) => {
                out.settled = !revealing;
                out.rows
                    .push(self.nav_status(status("Discovering…"), None, cx));
            }
            Some(Discovery::Failed(failure)) => out.rows.push(self.nav_status(
                status(failure_label(failure)).tooltip(failure.to_string()),
                Some(Box::new(cx.listener(|view, _, _, cx| {
                    view.custom.update(cx, |custom, cx| custom.retry(cx))
                }))),
                cx,
            )),
            Some(Discovery::Loaded(groups)) if groups.is_empty() => {
                out.rows
                    .push(self.nav_status(status("No custom resources"), None, cx));
            }
            Some(Discovery::Loaded(groups)) => {
                for entry in groups.iter().take(MAX_SIDEBAR_GROUPS) {
                    let name = entry.name.as_ref();
                    let group_open = custom.is_group_open(name);
                    if reveal_group == Some(name) {
                        out.reveal = Some(out.rows.len());
                    }
                    let toggled = entry.name.clone();
                    out.rows.push(
                        self.nav_header(
                            NavRow::new(entry.id.clone(), entry.name.clone(), dp(30.))
                                .tooltip(entry.tooltip.clone()),
                            group_open,
                            current_group == Some(name),
                            cx.listener(move |view, _, _, cx| view.toggle_api_group(&toggled, cx)),
                            cx,
                        ),
                    );
                    if !group_open {
                        continue;
                    }
                    let settled = matches!(
                        entry.kinds,
                        Some(Discovery::Loaded(_) | Discovery::Failed(_))
                    );
                    if reveal_group == Some(name) && !settled {
                        out.settled = false;
                    }
                    let found = self.api_group_rows(entry, current, reveal_kind, cx);
                    if let Some(row) = found.reveal {
                        out.reveal = Some(out.rows.len() + row);
                    } else if matches!(reveal, Some(SidebarReveal::ApiGroup(open)) if open == name)
                    {
                        out.reveal = Some(out.rows.len() + found.rows.len() - 1);
                    }
                    out.rows.extend(found.rows);
                }
                if groups.len() > MAX_SIDEBAR_GROUPS {
                    out.rows.push(self.nav_status(
                        NavRow::new(
                            "nav-k8s-custom-more",
                            format!(
                                "{} more groups not shown",
                                groups.len() - MAX_SIDEBAR_GROUPS
                            ),
                            dp(30.),
                        ),
                        None,
                        cx,
                    ));
                }
            }
        }
        if matches!(reveal, Some(SidebarReveal::Custom)) {
            out.reveal = Some(out.rows.len() - 1);
        }
        out
    }

    /// An open API group's kinds, or why it shows none, and whether a
    /// version failed while others were read. `reveal` is a kind's row.
    fn api_group_rows(
        &self,
        entry: &CustomGroup,
        current: Option<&str>,
        reveal_kind: Option<&str>,
        cx: &Context<Self>,
    ) -> CustomRows {
        let id = &entry.id;
        let mut out = CustomRows {
            rows: Vec::new(),
            reveal: None,
            settled: true,
        };
        let status = |text: SharedString| NavRow::new(format!("{id}-status"), text, dp(52.));
        let retry = || -> Option<RowAction> {
            let name = entry.name.clone();
            Some(Box::new(cx.listener(move |view, _, _, cx| {
                view.custom
                    .update(cx, |custom, cx| custom.retry_group(&name, cx))
            })))
        };
        let rows = match &entry.kinds {
            None | Some(Discovery::Reading) => {
                out.rows
                    .push(self.nav_status(status("Discovering…".into()), None, cx));
                return out;
            }
            Some(Discovery::Failed(failure)) => {
                out.rows.push(self.nav_status(
                    status(failure_label(failure).into()).tooltip(failure.to_string()),
                    retry(),
                    cx,
                ));
                return out;
            }
            Some(Discovery::Loaded(rows)) => rows,
        };
        for kind in rows.kinds.iter().take(MAX_SIDEBAR_KINDS) {
            if reveal_kind == Some(kind.key.as_ref()) {
                out.reveal = Some(out.rows.len());
            }
            let key = kind.key.clone();
            out.rows.push(
                self.kubernetes_item(
                    NavRow::new(kind.id.clone(), kind.label.clone(), dp(52.))
                        .tooltip(kind.tooltip.clone()),
                    current == Some(kind.key.as_ref()),
                    cx.listener(move |view, _, window, cx| view.open_custom(&key, window, cx)),
                    cx,
                ),
            );
        }
        if rows.kinds.len() > MAX_SIDEBAR_KINDS {
            let more = rows.kinds.len() - MAX_SIDEBAR_KINDS;
            out.rows.push(self.nav_status(
                status(format!("{more} more kinds not shown").into()),
                None,
                cx,
            ));
        }
        if let Some(why) = &rows.nothing {
            out.rows.push(self.nav_status(
                status("Nothing to list".into()).tooltip(why.clone()),
                None,
                cx,
            ));
        }
        if let Some((label, detail)) = &rows.partial {
            out.rows.push(
                self.nav_status(
                    NavRow::new(format!("{id}-partial"), label.clone(), dp(52.))
                        .tooltip(detail.clone()),
                    retry(),
                    cx,
                ),
            );
        }
        out
    }

    /// A row that opens and closes the rows below it.
    fn nav_header(
        &self,
        row: NavRow,
        open: bool,
        holds_current: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        // A closed header still shows that the page is one of its kinds.
        let marked = holds_current && !open;
        h_flex()
            .id(row.id)
            .test_support()
            .role(Role::Button)
            .aria_expanded(open)
            .aria_label(row.label.clone())
            .tab_index(0)
            .h(dp(28.))
            .flex_none()
            .pl(row.indent)
            .pr_2()
            .gap_2()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(dp(12.5))
            .text_color(if marked { p.ink } else { p.ink_2 })
            .when(marked, |this| this.font_weight(FontWeight::SEMIBOLD))
            .hover(|style| style.bg(p.hover))
            .when_some(row.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(dp(14.))
                .text_color(p.muted)
                .flex_none(),
            )
            .child(div().min_w_0().truncate().child(row.label))
            .on_click(on_click)
            .into_any_element()
    }

    fn kubernetes_item(
        &self,
        row: NavRow,
        active: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        h_flex()
            .id(row.id)
            .test_support()
            .role(Role::Tab)
            .aria_selected(active)
            .aria_label(row.label.clone())
            .tab_index(0)
            .h(dp(28.))
            .flex_none()
            .pl(row.indent)
            .pr_2()
            .rounded(px(7.))
            .cursor_pointer()
            .text_size(dp(12.5))
            .text_color(if active { p.ink } else { p.ink_2 })
            .when(active, |this| {
                this.bg(cx.theme().sidebar_accent)
                    .border_1()
                    .border_color(p.line)
                    .shadow_xs()
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!active, |this| this.hover(|style| style.bg(p.hover)))
            .when_some(row.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(div().min_w_0().truncate().child(row.label))
            .on_click(on_click)
            .into_any_element()
    }

    /// A line saying how discovery went where rows would be, with Retry
    /// after a failure.
    fn nav_status(&self, row: NavRow, retry: Option<RowAction>, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let retry_id = SharedString::from(format!("{}-retry", row.id));
        h_flex()
            .id(row.id)
            .test_support()
            .role(Role::Status)
            .aria_label(row.label.clone())
            .h(dp(28.))
            .flex_none()
            .pl(row.indent)
            .pr_1()
            .gap_1()
            .text_size(dp(12.))
            .text_color(p.muted)
            .when_some(row.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(div().flex_1().min_w_0().truncate().child(row.label))
            .when_some(retry, |this, retry| {
                this.child(
                    Button::new(retry_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::RefreshCw)
                        .accessibility_label("Retry")
                        .tooltip("Retry")
                        .on_click(retry),
                )
            })
            .into_any_element()
    }

    fn context_item(&self, ix: usize, context: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let current = self.applied.context.as_deref() == Some(context);
        let connection = self.kubernetes_only.as_ref().map(|kube| &kube.connection);
        let (dot, tip) = if !current {
            (None, "Not loaded yet")
        } else if let Some(connection) = connection {
            match connection {
                KubeConnection::Connected { .. } => (Some(p.good), "Connected"),
                KubeConnection::Failed(_) => (Some(p.crit), "Couldn't connect"),
                KubeConnection::Idle | KubeConnection::Connecting => (None, "Connecting"),
            }
        } else if self.overview.is_stale() {
            (Some(p.warn), "Last refresh failed")
        } else if self.overview.data().is_some() {
            (Some(p.good), "Connected")
        } else {
            (None, "Connecting")
        };
        let chosen = context.to_owned();
        h_flex()
            .id(("context", ix))
            .test_support()
            .role(Role::Tab)
            .aria_selected(current)
            .aria_label(context.to_owned())
            .tab_index(0)
            .h(dp(32.))
            .px_2()
            .gap_2p5()
            .rounded(px(7.))
            .cursor_pointer()
            .text_color(if current { p.ink } else { p.ink_2 })
            .when(current, |this| this.bg(p.accent_soft))
            .when(!current, |this| this.hover(|style| style.bg(p.hover)))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
            })
            .child(
                div()
                    .flex_none()
                    .size(dp(8.))
                    .rounded_full()
                    .map(|this| match dot {
                        Some(color) => this.bg(color),
                        None => this.border(px(1.5)).border_color(p.faint),
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.5))
                    .child(context.to_owned()),
            )
            .children(self.context_nodes.get(context).map(|count| {
                div()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child(if *count == 1 {
                        "1 node".to_owned()
                    } else {
                        format!("{count} nodes")
                    })
            }))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_context(chosen.clone(), window, cx);
                view.focus_page(window, cx);
            }))
            .into_any_element()
    }
}

mod frame;
use frame::settings_content;
