//! The Resources toolbar and Pods' bullet-meter legend.
use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use layout::ColumnSource;
use std::rc::Rc;

impl ResourcesScreen {
    /// The page's toolbar: the kind, its filter, Pods' Problems and All, the
    /// namespace, Pods' columns and Refresh, with where the rows come from
    /// in the meta line.
    pub(super) fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = page::PageHeader::new("resource", self.title());
        let filter = div()
            .key_context(FILTER_CONTEXT)
            .on_action(
                cx.listener(|view, _: &LeaveFilter, window, cx| view.leave_filter(window, cx)),
            )
            .child(
                Input::new(&self.query)
                    .id(header.id("filter"))
                    .small()
                    .h(dp(ui::CONTROL_HEIGHT))
                    .cleanable(true)
                    .aria_label(if self.lists_pods() {
                        "Filter pods by name, namespace or status"
                    } else {
                        "Filter by name, namespace or any column; Escape clears it, then returns to the list"
                    })
                    .prefix(Icon::new(IconName::Search).size(dp(14.))),
            );
        let mut header = header
            .filter(filter)
            .chips(self.pod_switch(cx))
            .meta(self.meta());
        if self.kind.namespaced && !self.embedded {
            let namespace = Select::new(&self.namespace_select)
                .id(header.id("namespace"))
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .w(dp(NAMESPACE_WIDTH))
                .menu_width(dp(260.))
                .search_placeholder("Find a namespace")
                .accessibility_label("Namespace");
            header = header.foldable(
                namespace,
                page::submenu("Namespace", self.namespace_items(cx)),
            );
        }
        if self.lists_pods() && !self.embedded {
            let items = self.columns_items(cx);
            header = header.foldable(
                div().flex_none().child(self.columns_menu(items.clone())),
                page::submenu("Columns", items),
            );
        }
        let refresh = page::handler(cx, |view: &mut Self, window, cx| view.refresh(window, cx));
        let button = Button::new(header.id("refresh"))
            .ghost()
            .small()
            .size(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::RefreshCw)
            .tooltip(if self.lists_pods() {
                "Refresh pods"
            } else {
                "Refresh"
            })
            .on_click({
                let refresh = refresh.clone();
                move |_, window, cx| refresh(window, cx)
            });
        header
            .foldable(button, page::item("Refresh", refresh))
            .render(window, cx)
    }

    /// The namespace picker's choices as menu items, its folded form: each
    /// sets the namespace as choosing it in the picker does.
    fn namespace_items(&self, cx: &Context<Self>) -> page::MenuItems {
        let owner = cx.entity().downgrade();
        Rc::new(move |mut menu, window, cx| {
            let Some(view) = owner.upgrade() else {
                return menu;
            };
            let view = view.read(cx);
            let current = view.namespace.clone();
            for (label, value) in namespace_entries(&view.namespaces, current.as_deref()) {
                let owner = owner.clone();
                let checked = value == current;
                menu = menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                    move |_, window, cx| {
                        _ = owner
                            .update(cx, |view, cx| view.set_namespace(value.clone(), window, cx));
                    },
                ));
            }
            menu.scrollable(true).max_h(ui::dp_px(360., window))
        })
    }

    /// The meta line: the context, how many rows, the read's state and when
    /// the rows last changed.
    fn meta(&self) -> Vec<AnyElement> {
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.access, KubeAccess::Example));
        let mut meta = Vec::new();
        if self.lists_pods() && !self.embedded {
            let source = match &self.source {
                Some(_) if example => "Example data".to_owned(),
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
            meta.extend([source.into_any_element(), state.into_any_element()]);
        } else {
            let read = self.store.read_state();
            let state = match read {
                ReadState::Loading => "loading",
                ReadState::Loaded if example => "example data",
                ReadState::Loaded => "watching",
                ReadState::Stale(_) => "reconnecting",
                ReadState::Refused(_) => "not permitted",
                ReadState::Failed(_) => "failed",
                ReadState::Missing(_) => "not served",
            };
            let (total, shown) = (self.store.len(), self.projection.len());
            // Folded healthy pods aren't hidden by the filter.
            let shown = if self.projection.grouping().is_some() {
                self.projection.tally().total()
            } else {
                shown
            };
            let Some(source) = &self.source else {
                return vec!["Not connected".into_any_element()];
            };
            meta.push(format!("{} · ", source.context).into_any_element());
            if read.shows_rows() {
                let count = if shown == total {
                    total.to_string()
                } else {
                    format!("{shown} of {total}")
                };
                meta.push(format!("{count} · ").into_any_element());
            }
            meta.push(state.into_any_element());
        }
        if let Some(time) = self.updated {
            meta.extend([" · ".into_any_element(), clock(time).into_any_element()]);
        }
        meta
    }

    /// Pods' columns as checked items: the Columns menu, and its folded form.
    fn columns_items(&self, cx: &Context<Self>) -> page::MenuItems {
        let owner = cx.entity().downgrade();
        let columns = self.layout.all_columns.clone();
        let hidden = self.hidden_columns.clone();
        Rc::new(move |mut menu, _, _| {
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
    }

    fn columns_menu(&self, items: page::MenuItems) -> AnyElement {
        Button::new(self.table.id("columns"))
            .outline()
            .small()
            .h(dp(ui::CONTROL_HEIGHT))
            .label("Columns")
            .dropdown_caret(true)
            .dropdown_menu(move |menu, window, cx| items(menu, window, cx))
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
