//! Applications header controls; filtering derives the table projection.
use super::*;
use gpui_kit::component::button::ButtonGroup;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

impl ObservabilityPage {
    pub(in crate::observability) fn applications_header(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = self.source_header(self.page_header(window), cx);
        let segment = ButtonGroup::new("obs-view")
            .outline()
            .small()
            .children([Filter::Problems, Filter::All].into_iter().map(|filter| {
                Button::new(SharedString::from(format!("obs-filter-{}", filter.slug())))
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
                .cleanable(true)
                .aria_label("Filter applications by name, namespace or type")
                .prefix(Icon::new(IconName::Search).size(dp(14.))),
        );
        let count = if !self.fixture && self.live.apps.data().is_none() {
            if self.live.apps.is_loading() {
                "Loading applications".into()
            } else {
                "No observation".into()
            }
        } else {
            self.app_count.clone()
        };
        let mut meta = vec![" · ".into_any_element(), count.into_any_element()];
        if self.live.apps.is_stale() {
            meta.push(" · stale".into_any_element());
        }
        if let Some(time) = self
            .live
            .apps
            .last_successful()
            .or_else(|| self.live.range.to.filter(|_| self.fixture).map(Into::into))
        {
            meta.extend([" · ".into_any_element(), ui::clock(time).into_any_element()]);
        }
        self.time_controls(
            header
                .filter(filter)
                .chips(Some(line().flex_wrap().child(segment).child(chips)))
                .secondary(categories)
                .meta(meta)
                .control(self.namespace_picker(cx))
                .control(self.application_columns_menu(cx)),
            cx,
        )
        .render_fit(window, cx)
    }
    fn namespace_picker(&self, _: &Context<Self>) -> AnyElement {
        Select::new(&self.namespace_select)
            .id("obs-namespace")
            .small()
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

    fn application_columns_menu(&self, cx: &Context<Self>) -> AnyElement {
        use application_columns::ColumnKind;
        let owner = cx.entity().downgrade();
        let hidden = self.hidden_application_columns.clone();
        Button::new("obs-columns")
            .outline()
            .small()
            .label("Columns")
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for kind in
                    std::iter::once(ColumnKind::Type).chain(Report::ALL.map(ColumnKind::Report))
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
    SearchableVec::new(choices)
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
