//! Fog's compact Pods toolbar and bullet-meter legend.
use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use layout::ColumnSource;

impl ResourcesScreen {
    pub(super) fn pods_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let narrow = content_width(window) < 920.;
        let leading = h_flex()
            .gap(dp(8.))
            .min_w_0()
            .child(
                ui::page_title(self.title())
                    .id("resource-title")
                    .test_support(),
            )
            .child(
                div()
                    .when_else(narrow, |this| this.flex_1(), |this| this.w(dp(150.)))
                    .min_w_0()
                    .key_context(FILTER_CONTEXT)
                    .on_action(cx.listener(|view, _: &LeaveFilter, window, cx| {
                        view.leave_filter(window, cx)
                    }))
                    .child(
                        Input::new(&self.query)
                            .id("resource-filter")
                            .small()
                            .cleanable(true)
                            .aria_label("Filter pods by name, namespace or status")
                            .prefix(Icon::new(IconName::Search).size(dp(14.))),
                    ),
            );
        let controls = h_flex()
            .flex_none()
            .gap(dp(8.))
            .child(
                Select::new(&self.namespace_select)
                    .id("resource-namespace")
                    .small()
                    .w(dp(132.))
                    .menu_width(dp(260.))
                    .search_placeholder("Find a namespace")
                    .accessibility_label("Namespace"),
            )
            .child(
                Button::new("resource-density")
                    .outline()
                    .small()
                    .icon(if self.compact {
                        IconName::Rows4
                    } else {
                        IconName::Rows2
                    })
                    .tooltip(if self.compact {
                        "Compact · 26px rows. Switch to comfortable"
                    } else {
                        "Comfortable · 34px rows. Switch to compact"
                    })
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_density(cx))),
            )
            .child(div().flex_none().child(self.columns_menu(cx)))
            .child(
                Button::new("resource-refresh")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh pods")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
            );
        let scope = h_flex()
            .id("resource-scope")
            .test_support()
            .flex_none()
            .text_size(dp(11.))
            .text_color(p.muted)
            .child(match &self.source {
                Some(source) if matches!(source.access, KubeAccess::Example) => {
                    "Example data".to_owned()
                }
                Some(source) => format!("{} · {} pods", source.context, self.store.len()),
                None => "Not connected".into(),
            })
            .child(match self.store.read_state() {
                ReadState::Loading => " · loading",
                ReadState::Loaded => "",
                ReadState::Stale(_) => " · reconnecting",
                ReadState::Refused(_) => " · not permitted",
                ReadState::Failed(_) => " · failed",
                ReadState::Missing(_) => " · not served",
            })
            .when_some(self.updated, |this, time| {
                this.child(" · ").child(clock(time))
            });
        let toolbar = if narrow {
            v_flex()
                .w_full()
                .gap(dp(8.))
                .child(leading.w_full())
                .children(self.pod_switch(cx))
                .child(controls)
        } else {
            h_flex()
                .w_full()
                .gap(dp(8.))
                .child(leading)
                .children(self.pod_switch(cx))
                .child(div().flex_1())
                .child(controls)
        };
        v_flex()
            .w_full()
            .flex_none()
            .gap(dp(8.))
            .child(toolbar)
            .child(scope)
    }

    fn columns_menu(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let columns = self.layout.all_columns.clone();
        let hidden = self.hidden_columns.clone();
        Button::new("resource-columns")
            .outline()
            .small()
            .label("Columns")
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for column in &columns {
                    if matches!(
                        column.source,
                        ColumnSource::Glyph | ColumnSource::Name(_) | ColumnSource::Namespace
                    ) {
                        continue;
                    }
                    let owner = owner.clone();
                    let source = column.source;
                    menu = menu.item(
                        PopupMenuItem::new(column.label.clone())
                            .checked(!hidden.contains(&source))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |view, cx| {
                                    if !view.hidden_columns.remove(&source) {
                                        view.hidden_columns.insert(source);
                                    }
                                    view.layout.hide(&view.hidden_columns);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    pub(super) fn meter_legend(&self, narrow: bool, cx: &App) -> AnyElement {
        let p = palette(cx);
        if narrow {
            return h_flex()
                .id("resource-meter-legend")
                .h(dp(26.))
                .flex_none()
                .px(dp(12.))
                .border_t_1()
                .border_color(p.line)
                .text_size(dp(11.))
                .text_color(p.muted)
                .child("Meters: request · use · limit ⓘ")
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Band = request · line = use · end = limit. Brighter = above request; amber = at least 85% of limit; stripes = last known data.",
                    ).build(window, cx)
                })
                .into_any_element();
        }
        h_flex()
            .flex_none()
            .px(dp(12.))
            .py(dp(7.))
            .gap(dp(12.))
            .flex_wrap()
            .border_t_1()
            .border_color(p.line)
            .text_size(dp(11.))
            .text_color(p.muted)
            .child("Meters: band = request · line = use · end = limit")
            .child(
                h_flex()
                    .gap(dp(5.))
                    .child(crate::meters::bullet(
                        crate::meters::Resource::Cpu,
                        90.,
                        Some(50.),
                        Some(100.),
                        false,
                        &p,
                    ))
                    .child("≥85% of limit"),
            )
            .child(
                h_flex()
                    .gap(dp(5.))
                    .child(crate::meters::bullet(
                        crate::meters::Resource::Memory,
                        30.,
                        Some(50.),
                        Some(100.),
                        true,
                        &p,
                    ))
                    .child("last known"),
            )
            .into_any_element()
    }
}
