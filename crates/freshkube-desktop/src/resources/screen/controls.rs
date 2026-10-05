//! Fog's compact Pods toolbar and bullet-meter legend.
use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use layout::ColumnSource;

impl ResourcesScreen {
    pub(super) fn pods_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let narrow = inset_width(window) < page::HEADER_NARROW;
        let header = page::PageHeader::new("resource", self.title(), narrow);
        let filter = div()
            .key_context(FILTER_CONTEXT)
            .on_action(
                cx.listener(|view, _: &LeaveFilter, window, cx| view.leave_filter(window, cx)),
            )
            .child(
                Input::new(&self.query)
                    .id(header.id("filter"))
                    .small()
                    .cleanable(true)
                    .aria_label("Filter pods by name, namespace or status")
                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
            );
        let source = match &self.source {
            Some(source) if matches!(source.access, KubeAccess::Example) => {
                "Example data".to_owned()
            }
            Some(source) => format!("{} · {} pods", source.context, self.store.len()),
            None => "Not connected".into(),
        };
        let state = match self.store.read_state() {
            ReadState::Loading => " · loading",
            ReadState::Loaded => "",
            ReadState::Stale(_) => " · reconnecting",
            ReadState::Refused(_) => " · not permitted",
            ReadState::Failed(_) => " · failed",
            ReadState::Missing(_) => " · not served",
        };
        let mut meta = vec![source.into_any_element(), state.into_any_element()];
        if let Some(time) = self.updated {
            meta.extend([" · ".into_any_element(), clock(time).into_any_element()]);
        }
        let namespace = Select::new(&self.namespace_select)
            .id(header.id("namespace"))
            .small()
            .w(dp(132.))
            .menu_width(dp(260.))
            .search_placeholder("Find a namespace")
            .accessibility_label("Namespace");
        let refresh = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .icon(IconName::RefreshCw)
            .tooltip("Refresh pods")
            .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx)));
        header
            .filter(filter)
            .chips(self.pod_switch(cx))
            .control(namespace)
            .control(div().flex_none().child(self.columns_menu(cx)))
            .control(refresh)
            .meta(meta)
            .render(cx)
    }

    fn columns_menu(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let columns = self.layout.all_columns.clone();
        let hidden = self.hidden_columns.clone();
        Button::new(self.table.id("columns"))
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
                self.table.id("meter-legend"),
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
