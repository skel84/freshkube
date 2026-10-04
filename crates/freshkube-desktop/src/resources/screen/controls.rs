//! Fog's compact Pods toolbar and bullet-meter legend.
use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use layout::ColumnSource;

impl ResourcesScreen {
    pub(super) fn pods_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let narrow = content_width(window) < page::HEADER_NARROW;
        let title = ui::page_title(self.title())
            .id("resource-title")
            .test_support();
        let filter = div()
            .key_context(FILTER_CONTEXT)
            .on_action(
                cx.listener(|view, _: &LeaveFilter, window, cx| view.leave_filter(window, cx)),
            )
            .child(
                Input::new(&self.query)
                    .id("resource-filter")
                    .small()
                    .cleanable(true)
                    .aria_label("Filter pods by name, namespace or status")
                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
            );
        let meta = page::meta_line("resource-scope", cx)
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
        page::PageHeader::new(title, narrow)
            .filter(filter)
            .chips(self.pod_switch(cx))
            .control(
                Select::new(&self.namespace_select)
                    .id("resource-namespace")
                    .small()
                    .w(dp(132.))
                    .menu_width(dp(260.))
                    .search_placeholder("Find a namespace")
                    .accessibility_label("Namespace"),
            )
            .control(
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
            .control(div().flex_none().child(self.columns_menu(cx)))
            .control(
                Button::new("resource-refresh")
                    .ghost()
                    .small()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh pods")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
            )
            .meta(meta)
            .render()
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
        if narrow {
            return table::legend_line(
                "resource-meter-legend",
                "Meters: request · use · limit ⓘ",
                "Band = request · line = use · end = limit. Brighter = above request; amber = at least 85% of limit; stripes = last known data.",
                cx,
            )
            .into_any_element();
        }
        let p = palette(cx);
        let bullet = |resource, used, stale| {
            crate::meters::bullet(resource, used, Some(50.), Some(100.), stale, &p)
        };
        table::legend(
            [
                "Meters: band = request · line = use · end = limit".into_any_element(),
                table::legend_item(
                    bullet(crate::meters::Resource::Cpu, 90., false),
                    "≥85% of limit",
                )
                .into_any_element(),
                table::legend_item(
                    bullet(crate::meters::Resource::Memory, 30., true),
                    "last known",
                )
                .into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    }
}
