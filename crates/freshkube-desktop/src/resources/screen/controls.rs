//! The Resources toolbar and Pods' bullet-meter legend.
use super::*;
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::ui::Tone;
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
        let mut header = header.filter(filter).chips(self.pod_switch(cx));
        // A page's line is in the status bar; the node pane's list keeps
        // its own under its header.
        if self.embedded {
            header = header.meta([self.status.1.text(None).clone().into_any_element()]);
        }
        if self.kind.namespaced && !self.embedded {
            let namespace = Select::new(&self.namespace_select)
                .id(header.id("namespace"))
                .small()
                .h(dp(ui::CONTROL_HEIGHT))
                .w(dp(NAMESPACE_WIDTH))
                .menu_width(dp(260.))
                .search_placeholder("Find a namespace")
                .accessibility_label("Namespace");
            let picked = self.namespace.clone();
            let value = picked.clone().unwrap_or_else(|| "All namespaces".into());
            header = header.foldable(
                namespace,
                page::Fold::from(page::submenu_value(
                    "Namespace",
                    value,
                    self.namespace_items(cx),
                ))
                .changed(picked.map(|name| format!("Namespace {name}").into())),
            );
        }
        if self.lists_pods() && !self.embedded {
            let items = self.columns_items(cx);
            let hidden = self.hidden_columns.len();
            header = header.foldable(
                div().flex_none().child(self.columns_menu(items.clone())),
                page::columns_fold(items, hidden, hidden == 0),
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
        // ⌘R reads this list again only on the Resources page; the node
        // pane's pods list is the Nodes page's, which ⌘R refreshes.
        let fold = if self.embedded {
            page::item("Refresh", refresh)
        } else {
            page::action_item("Refresh", page::Refresh, &self.focus)
        };
        header.foldable(button, fold).render(window, cx)
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

    /// What the status bar shows of this list: the context, how many rows,
    /// the read's state and when the rows last changed. It is derived
    /// again only when one of those changes, so the shell can read it on
    /// every frame.
    pub(crate) fn status(&mut self) -> &Segment {
        let inputs = self.status_inputs();
        let key = &self.status.0;
        let known = (
            key.context
                .as_ref()
                .map(|(context, example)| (context.as_str(), *example)),
            key.read,
            key.total,
            key.shown,
            key.updated,
        );
        if known != inputs {
            let (context, read, total, shown, updated) = inputs;
            let key = StatusKey {
                context: context.map(|(context, example)| (context.to_owned(), example)),
                read,
                total,
                shown,
                updated,
            };
            self.status = (key, self.derive_status());
        }
        &self.status.1
    }

    /// What the status line is derived from.
    fn status_inputs(&self) -> StatusInputs<'_> {
        (
            self.source.as_ref().map(|source| {
                (
                    source.context.as_str(),
                    matches!(source.access, KubeAccess::Example),
                )
            }),
            self.read_label(),
            self.store.len(),
            self.shown(),
            self.updated,
        )
    }

    /// The read's state as the status bar words it, and its tone.
    fn read_label(&self) -> (&'static str, Option<Tone>) {
        let example = self
            .source
            .as_ref()
            .is_some_and(|source| matches!(source.access, KubeAccess::Example));
        match self.store.read_state() {
            ReadState::Loading => ("loading", None),
            ReadState::Loaded if example => ("example data", None),
            ReadState::Loaded => ("watching", None),
            ReadState::Stale(_) => ("reconnecting", Some(Tone::Warn)),
            ReadState::Refused(_) => ("not permitted", Some(Tone::Crit)),
            ReadState::Failed(_) => ("failed", Some(Tone::Crit)),
            ReadState::Missing(_) => ("not served", None),
        }
    }

    /// How many rows the filter lets through. Folded healthy pods aren't
    /// hidden by the filter.
    fn shown(&self) -> usize {
        if self.projection.grouping().is_some() {
            self.projection.tally().total()
        } else {
            self.projection.len()
        }
    }

    fn derive_status(&self) -> Segment {
        let time = self.updated.map(|time| Part::new(clock(time)));
        let Some(source) = &self.source else {
            return Segment::new(None::<SharedString>, ["Not connected"]);
        };
        let example = matches!(source.access, KubeAccess::Example);
        let (state, tone) = self.read_label();
        let state = tone.into_iter().fold(Part::new(state), Part::tone);
        let (total, shown) = (self.store.len(), self.shown());
        if self.lists_pods() && !self.embedded {
            // Pods name their count and leave out a plain "watching".
            let state = (!matches!(self.store.read_state(), ReadState::Loaded)).then_some(state);
            let (context, lead) = if example {
                (None, Part::new("Example data"))
            } else {
                (
                    Some(source.context.clone()),
                    Part::new(format!("{total} pods")),
                )
            };
            return Segment::new(context, std::iter::once(lead).chain(state).chain(time));
        }
        let count = self.store.read_state().shows_rows().then(|| {
            Part::new(if shown == total {
                total.to_string()
            } else {
                format!("{shown} of {total}")
            })
        });
        Segment::new(
            Some(source.context.clone()),
            count.into_iter().chain(Some(state)).chain(time),
        )
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
                    ColumnSource::Glyph
                        | ColumnSource::Name(_)
                        | ColumnSource::Namespace
                        | ColumnSource::Logs
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

type StatusInputs<'a> = (
    Option<(&'a str, bool)>,
    (&'static str, Option<Tone>),
    usize,
    usize,
    Option<SystemTime>,
);

/// What the status line was last derived from.
#[derive(Default)]
pub(super) struct StatusKey {
    context: Option<(String, bool)>,
    read: (&'static str, Option<Tone>),
    total: usize,
    shown: usize,
    updated: Option<SystemTime>,
}
