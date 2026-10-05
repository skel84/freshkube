//! Applications header controls; filtering derives the table projection.
use super::*;
use gpui_kit::component::button::ButtonGroup;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

impl ObservabilityPage {
    /// What Applications puts in the status bar.
    pub(in crate::observability) fn applications_read(&self) -> super::status::Read<'_> {
        let count = if !self.fixture && self.live.apps.data().is_none() {
            if self.live.apps.is_loading() {
                "Loading applications"
            } else {
                "No observation"
            }
        } else {
            &self.app_count
        };
        super::status::Read {
            count,
            stale: self.live.apps.is_stale(),
            time: self.read_time(self.live.apps.last_successful()),
            note: None,
        }
    }

    pub(in crate::observability) fn applications_header(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = self.source_header(self.page_header(), cx);
        let segment = ButtonGroup::new("obs-view")
            .outline()
            .small()
            .children([Filter::Problems, Filter::All].into_iter().map(|filter| {
                Button::new(SharedString::from(format!("obs-filter-{}", filter.slug())))
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label(filter.label())
                    .selected(self.filter == filter)
            }))
            .on_click(cx.listener(|this, choices: &Vec<usize>, _, cx| {
                this.filter = if choices.first() == Some(&0) {
                    Filter::Problems
                } else {
                    Filter::All
                };
                this.project_filters();
                cx.notify();
            }));
        let chips = freshkube_ui::table::status_chips(
            "obs-tallies",
            Filter::ALL
                .into_iter()
                .enumerate()
                .skip(2)
                .map(|(ix, filter)| {
                    freshkube_ui::table::status_chip(
                        SharedString::from(format!("obs-tally-{}", filter.slug())),
                        match filter {
                            Filter::Critical => Tone::Crit,
                            Filter::Warning => Tone::Warn,
                            Filter::Unknown => Tone::Unknown,
                            Filter::Logs => Tone::Warn,
                            _ => Tone::Good,
                        },
                        self.counts[ix],
                        filter.label(),
                        self.filter == filter,
                        cx,
                    )
                    .when(filter == Filter::Logs, |chip| chip.child("logs"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = if this.filter == filter {
                            Filter::All
                        } else {
                            filter
                        };
                        this.project_filters();
                        cx.notify();
                    }))
                }),
            cx,
        );
        let categories = ButtonGroup::new("obs-categories")
            .outline()
            .small()
            .multiple(true)
            .min_w_0()
            .flex_wrap()
            .children(self.categories.iter().take(6).map(|category| {
                Button::new(category.id.clone())
                    .h(dp(crate::ui::CONTROL_HEIGHT))
                    .label(category.name.clone())
                    .selected(
                        self.all_categories || self.active_categories.contains(&category.name),
                    )
                    .accessibility_label(category.label.clone())
            }))
            .on_click(cx.listener(|this, choices: &Vec<usize>, _, cx| {
                this.choose_categories();
                let active = Rc::make_mut(&mut this.active_categories);
                for category in this.categories.iter().take(6) {
                    active.remove(&category.name);
                }
                for ix in choices {
                    if let Some(category) = this.categories.get(*ix) {
                        active.insert(category.name.clone());
                    }
                }
                this.project_filters();
                cx.notify();
            }));
        let categories = line().flex_wrap().child(categories);
        let categories = categories.when(!self.categories.is_empty(), |row| {
            row.child(self.more_categories(cx))
        });
        let filter = div().child(
            Input::new(&self.query)
                .id(header.id("filter"))
                .small()
                .h(dp(crate::ui::CONTROL_HEIGHT))
                .cleanable(true)
                .aria_label("Filter applications by name, namespace or type")
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        let columns = self.application_columns_items(cx);
        self.time_controls(
            header
                .filter(filter)
                .chips(Some(line().flex_wrap().child(segment).child(chips)))
                .secondary(categories)
                .foldable(self.namespace_picker(cx), self.namespace_fold(cx))
                .foldable(
                    self.application_columns_menu(columns.clone()),
                    freshkube_ui::page::columns_fold(
                        columns,
                        self.hidden_application_columns.len(),
                        self.hidden_application_columns.is_empty(),
                    ),
                ),
            cx,
        )
        .render(window, cx)
    }

    /// The namespace picker's folded form: the namespace shown, noted while
    /// one is picked.
    fn namespace_fold(&self, cx: &Context<Self>) -> freshkube_ui::page::Fold {
        let value = self
            .namespace
            .clone()
            .unwrap_or_else(|| "All namespaces".into());
        freshkube_ui::page::Fold::from(freshkube_ui::page::submenu_value(
            "Namespace",
            value,
            self.namespace_items(cx),
        ))
        .changed(
            self.namespace
                .as_ref()
                .map(|name| format!("Namespace {name}").into()),
        )
    }

    /// Applies a namespace from the picker or its folded form.
    pub(in crate::observability) fn set_namespace(
        &mut self,
        namespace: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.namespace = namespace;
        self.project_filters();
        cx.notify();
    }

    /// The namespace picker's choices as checked items, its folded form.
    fn namespace_items(&self, cx: &Context<Self>) -> freshkube_ui::page::MenuItems {
        let owner = cx.entity().downgrade();
        Rc::new(move |mut menu, window, cx| {
            let Some(page) = owner.upgrade() else {
                return menu;
            };
            let page = page.read(cx);
            let current = page.namespace.clone();
            for choice in namespace_entries(&page.namespaces, current.as_deref()) {
                if choice.notice {
                    menu = menu.label(choice.label);
                    continue;
                }
                let owner = owner.clone();
                let value = choice.value;
                let checked = value == current;
                menu = menu.item(PopupMenuItem::new(choice.label).checked(checked).on_click(
                    move |_, _, cx| {
                        _ = owner.update(cx, |page, cx| page.set_namespace(value.clone(), cx));
                    },
                ));
            }
            menu.scrollable(true).max_h(crate::ui::dp_px(360., window))
        })
    }
    fn namespace_picker(&self, _: &Context<Self>) -> AnyElement {
        Select::new(&self.namespace_select)
            .id("obs-namespace")
            .small()
            .h(dp(crate::ui::CONTROL_HEIGHT))
            .w(dp(132.))
            .menu_width(dp(260.))
            .search_placeholder("Find a namespace")
            .accessibility_label("Application namespace")
            .into_any_element()
    }

    pub(in crate::observability) fn sync_namespace_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stale = !Rc::ptr_eq(&self.namespace_select_source, &self.namespaces);
        let selected = self.namespace_select.read(cx).selected_value();
        if !stale && selected == Some(&self.namespace) {
            return;
        }
        let current = self.namespace.clone();
        self.namespace_select_source = self.namespaces.clone();
        self.namespace_select.update(cx, |state, cx| {
            if stale {
                state.set_items(
                    namespace_choices(&self.namespaces, current.as_deref()),
                    window,
                    cx,
                );
            }
            state.set_selected_value(&current, window, cx);
        });
    }

    /// The table's columns as checked items: the Columns menu, and its
    /// folded form.
    fn application_columns_items(&self, cx: &Context<Self>) -> freshkube_ui::page::MenuItems {
        use application_columns::ColumnKind;
        let owner = cx.entity().downgrade();
        let hidden = self.hidden_application_columns.clone();
        Rc::new(move |mut menu, _, _| {
            for kind in std::iter::once(ColumnKind::Type).chain(Report::ALL.map(ColumnKind::Report))
            {
                let label = match kind {
                    ColumnKind::Report(report) => report.label(),
                    _ => "Type",
                };
                let owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(!hidden.contains(&kind))
                        .on_click(move |_, _, cx| {
                            _ = owner.update(cx, |this, cx| {
                                if !this.hidden_application_columns.remove(&kind) {
                                    this.hidden_application_columns.insert(kind);
                                }
                                this.prepare_application_columns();
                                cx.notify();
                            });
                        }),
                );
            }
            menu
        })
    }

    fn application_columns_menu(&self, items: freshkube_ui::page::MenuItems) -> AnyElement {
        Button::new("obs-columns")
            .outline()
            .small()
            .h(dp(crate::ui::CONTROL_HEIGHT))
            .label("Columns")
            .dropdown_caret(true)
            .dropdown_menu(move |menu, window, cx| items(menu, window, cx))
            .into_any_element()
    }
    fn more_categories(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let categories = self.categories.clone();
        let active = self.active_categories.clone();
        let all = self.all_categories;
        Button::new("obs-more-categories")
            .ghost()
            .small()
            .h(dp(crate::ui::CONTROL_HEIGHT))
            .label("More categories")
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                let all_owner = owner.clone();
                menu = menu.item(PopupMenuItem::new("All categories").checked(all).on_click(
                    move |_, _, cx| {
                        _ = all_owner.update(cx, |this, cx| {
                            this.all_categories = true;
                            this.project_filters();
                            cx.notify();
                        });
                    },
                ));
                for category in categories.iter().skip(6).take(94) {
                    let owner = owner.clone();
                    let category = category.name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(category.clone())
                            .checked(all || active.contains(&category))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |this, cx| {
                                    this.choose_categories();
                                    let active = Rc::make_mut(&mut this.active_categories);
                                    if !active.remove(&category) {
                                        active.insert(category.clone());
                                    }
                                    this.project_filters();
                                    cx.notify();
                                });
                            }),
                    );
                }
                if categories.len() > 100 {
                    menu = menu.item(
                        PopupMenuItem::new(
                            "First 100 categories · All categories includes the rest",
                        )
                        .disabled(true),
                    );
                }
                menu
            })
            .into_any_element()
    }
}

/// A namespace choice, or a disabled notice when the menu is capped.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::observability) struct NamespaceChoice {
    label: SharedString,
    value: Option<String>,
    notice: bool,
}
impl SelectItem for NamespaceChoice {
    type Value = Option<String>;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.value
    }
    fn disabled(&self) -> bool {
        self.notice
    }
}
fn namespace_choices(names: &[String], current: Option<&str>) -> SearchableVec<NamespaceChoice> {
    SearchableVec::new(namespace_entries(names, current))
}

/// The namespaces to pick from, All namespaces first: the picker's choices
/// and its folded form's items.
fn namespace_entries(names: &[String], current: Option<&str>) -> Vec<NamespaceChoice> {
    let mut choices = vec![NamespaceChoice {
        label: "All namespaces".into(),
        value: None,
        notice: false,
    }];
    let choice = |name: &str| NamespaceChoice {
        label: name.to_owned().into(),
        value: Some(name.to_owned()),
        notice: false,
    };
    choices.extend(names.iter().take(100).map(|name| choice(name)));
    if let Some(current) =
        current.filter(|current| !names.iter().take(100).any(|name| name == current))
    {
        choices.push(choice(current));
    }
    if names.len() > 100 {
        choices.push(NamespaceChoice {
            label: "First 100 choices · All namespaces includes the rest".into(),
            value: None,
            notice: true,
        });
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::namespace_choices;
    use gpui_kit::component::{
        IndexPath, searchable_list::SearchableListDelegate, select::SelectItem,
    };

    #[test]
    fn capped_namespaces_explain_the_limit_and_keep_the_selected_namespace() {
        let names: Vec<_> = (0..120).map(|ix| format!("ns-{ix:03}")).collect();
        let choices = namespace_choices(&names, Some("ns-119"));
        assert_eq!(choices.items_count(0), 103);
        let selected = choices.item(IndexPath::default().row(101)).unwrap();
        assert_eq!(selected.value().as_deref(), Some("ns-119"));
        let note = choices.item(IndexPath::default().row(102)).unwrap();
        assert!(note.disabled());
        assert!(note.title().starts_with("First 100 choices"));
    }
}
