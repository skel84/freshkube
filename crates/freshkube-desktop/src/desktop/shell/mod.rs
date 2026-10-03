//! Window chrome: the header, the icon rail, the navigation column, the
//! status bar and their popovers (docs/DESIGN.md, App frame).
use super::kubernetes_only::{self, KubeConnection};
use super::{
    AUTO_REFRESH, Appearance, Area, COLUMN_WIDTH, ColumnReveal, Page, Pilot, RAIL_WIDTH, clock,
};
use crate::monitoring::page::{Entry, FolderState};
use crate::mutation::Operations;
use crate::palette::palette;
use crate::resources::custom::{CustomGroup, Discovery};
use crate::resources::navigation;
use crate::text_size;
use crate::ui::{self, MONO_FONT, Tone, dp};
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

/// Most custom API groups, and kinds per group, the column lists; a row
/// says how many more there are.
const MAX_SIDEBAR_GROUPS: usize = 300;
const MAX_SIDEBAR_KINDS: usize = 200;

/// What one row of the navigation column shows.
struct NavRow {
    id: SharedString,
    label: SharedString,
    tooltip: Option<SharedString>,
    indent: Rems,
    key: Option<&'static str>,
    suffix: Option<AnyElement>,
}

impl NavRow {
    fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, indent: Rems) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tooltip: None,
            indent,
            key: None,
            suffix: None,
        }
    }

    fn suffix(mut self, suffix: Option<AnyElement>) -> Self {
        self.suffix = suffix;
        self
    }

    fn key(mut self, key: &'static str) -> Self {
        self.key = Some(key);
        self
    }

    fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

type RowAction = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Rows of Custom Resources' column, and where a reveal lands among
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
    /// Custom Resources' column: discovery's state or one header per API
    /// group, with the kinds of each open group. Also where `reveal` lands
    /// among these rows, and whether discovery has settled enough to stop
    /// revealing it.
    fn custom_resources(
        &self,
        current: Option<&str>,
        reveal: Option<&ColumnReveal>,
        cx: &Context<Self>,
    ) -> CustomRows {
        let custom = self.custom.read(cx);
        // The API group of the kind shown, when it is a custom kind.
        let current_group = current
            .filter(|key| navigation::group_of(key).is_none())
            .map(|_| self.resource_kind.group.as_str());
        // A custom kind to reveal, and the group it is in.
        let reveal_kind = match reveal {
            Some(ColumnReveal::Kind(key)) if navigation::group_of(key).is_none() => {
                Some(key.as_str())
            }
            _ => None,
        };
        let reveal_group = match reveal {
            Some(ColumnReveal::ApiGroup(name)) => Some(name.as_str()),
            _ => reveal_kind.and_then(|key| key.split_once('.').map(|(_, group)| group)),
        };
        let mut out = CustomRows {
            rows: Vec::new(),
            reveal: None,
            settled: true,
        };
        let revealing = matches!(reveal, Some(ColumnReveal::Custom)) || reveal_group.is_some();
        if revealing {
            out.reveal = Some(0);
        }
        let status = |text: &'static str| NavRow::new("nav-k8s-custom-status", text, dp(10.));
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
                            NavRow::new(entry.id.clone(), entry.name.clone(), dp(4.))
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
                    } else if matches!(reveal, Some(ColumnReveal::ApiGroup(open)) if open == name) {
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
                            dp(10.),
                        ),
                        None,
                        cx,
                    ));
                }
            }
        }
        if matches!(reveal, Some(ColumnReveal::Custom)) {
            out.reveal = Some(out.rows.len().saturating_sub(1));
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
        let status = |text: SharedString| NavRow::new(format!("{id}-status"), text, dp(30.));
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
                self.column_item(
                    NavRow::new(kind.id.clone(), kind.label.clone(), dp(30.))
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
                    NavRow::new(format!("{id}-partial"), label.clone(), dp(30.))
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
            .h(dp(30.))
            .flex_none()
            .pl(row.indent)
            .pr(dp(8.))
            .gap_1p5()
            .rounded(px(8.))
            .cursor_pointer()
            .text_size(dp(13.))
            .text_color(if marked { p.ink } else { p.ink_2 })
            .when(marked, |this| this.font_weight(ui::HEADING_WEIGHT))
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

    /// A page or kind in the column; the one shown is raised.
    fn column_item(
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
            .h(dp(30.))
            .flex_none()
            .pl(row.indent)
            .pr(dp(8.))
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .cursor_pointer()
            .text_size(dp(13.))
            .text_color(if active { p.ink } else { p.ink_2 })
            .when(active, |this| {
                this.bg(p.surface_2)
                    .border_color(p.line_strong)
                    .font_weight(ui::HEADING_WEIGHT)
            })
            .when(!active, |this| this.hover(|style| style.bg(p.hover)))
            .when_some(row.tooltip, |this, tip| {
                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(div().min_w_0().truncate().child(row.label))
            .child(div().flex_1())
            .children(row.suffix)
            .children(
                row.key
                    .map(|key| ui::keycap(format!("{}{key}", ui::modifier()), cx)),
            )
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

    fn context_item(
        &self,
        ix: usize,
        context: &str,
        popover: WeakEntity<gpui_kit::component::popover::PopoverState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let current = self.applied.context.as_deref() == Some(context);
        let connection = self.kubernetes_only.as_ref().map(|kube| &kube.connection);
        let (tone, tip) = if !current {
            (Tone::Unknown, "Not loaded yet")
        } else if let Some(connection) = connection {
            match connection {
                KubeConnection::Connected { .. } => (Tone::Good, "Connected"),
                KubeConnection::Failed(_) => (Tone::Crit, "Couldn't connect"),
                KubeConnection::Idle | KubeConnection::Connecting => (Tone::Unknown, "Connecting"),
            }
        } else if self.overview.is_stale() {
            (Tone::Warn, "Last refresh failed")
        } else if self.overview.data().is_some() {
            (Tone::Good, "Connected")
        } else {
            (Tone::Unknown, "Connecting")
        };
        let chosen = context.to_owned();
        h_flex()
            .id(("context", ix))
            .test_support()
            .role(Role::Tab)
            .aria_selected(current)
            .aria_label(context.to_owned())
            .tab_index(0)
            .min_h(dp(32.))
            .py_1()
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
            .children(ui::status_glyph(tone, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.5))
                    .child(context.to_owned()),
            )
            .children(self.context_display.counts.get(context).map(|label| {
                div()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child(label.clone())
            }))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_context(chosen.clone(), window, cx);
                view.focus_page(window, cx);
                let handle = window.window_handle();
                let popover = popover.clone();
                cx.defer(move |cx| {
                    _ = handle.update(cx, |_, window, cx| {
                        _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                    });
                });
            }))
            .into_any_element()
    }
}

mod column;
mod context;
pub(super) use context::ContextDisplay;
pub(super) mod fps;
mod frame;
use frame::settings_content;
mod header;
mod rail;
pub(super) use rail::RailMarks;
