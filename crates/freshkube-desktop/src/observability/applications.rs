use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
const MATRIX_WIDTH: f32 = 1040.;
impl ObservabilityPage {
    pub(super) fn render_applications(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let header = h_flex()
            .h(dp(30.))
            .px(dp(8.))
            .bg(p.surface_2)
            .child(div().w(dp(22.)).flex_none())
            .child(ui::caption("Application", cx).flex_1().min_w(dp(178.)))
            .child(ui::caption("Type", cx).w(dp(54.)).flex_none())
            .children(Report::ALL.into_iter().map(|report| {
                ui::caption(report.label(), cx)
                    .w(dp(report.column_width()))
                    .px(dp(4.))
                    .overflow_hidden()
                    .flex_none()
            }));
        let list = uniform_list(
            "obs-applications-list",
            self.matrix.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.matrix_row(ix, cx)).collect()
            }),
        )
        .h(dp(590.))
        .w_full();
        let table = v_flex()
            .id("obs-matrix")
            .test_support()
            .w_full()
            .min_w(dp(MATRIX_WIDTH))
            .bg(p.surface)
            .rounded(px(12.))
            .border_1()
            .border_color(p.line)
            .child(header)
            .when_else(
                self.matrix.is_empty(),
                |this| {
                    this.child(
                        body()
                            .child(text(if self.applications.is_empty() {
                                "No applications were returned"
                            } else {
                                "No applications match these filters"
                            }))
                            .child(muted(
                                "Clear the search or choose All to see every application.",
                                cx,
                            )),
                    )
                },
                |this| this.child(list),
            )
            .child(
                line()
                    .px(dp(14.))
                    .py(dp(10.))
                    .border_t_1()
                    .border_color(p.line)
                    .child(muted(
                        "● healthy · ○ unknown · — not reported · Coroot supplies each check",
                        cx,
                    )),
            );
        v_flex()
            .gap(dp(12.))
            .child(
                div()
                    .id("obs-matrix-horizontal")
                    .w_full()
                    .overflow_x_scroll()
                    .child(table),
            )
            .into_any_element()
    }
    fn matrix_row(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        match &self.matrix[index] {
            MatrixRow::Group { label, summary, .. } => line()
                .w_full()
                .h(dp(34.))
                .px(dp(14.))
                .bg(cx.theme().background)
                .border_b_1()
                .border_color(p.line)
                .child(text(label.clone()).font_weight(ui::HEADING_WEIGHT))
                .child(muted(summary.clone(), cx))
                .into_any_element(),
            MatrixRow::App(index) => {
                let index = *index;
                let app = &self.applications[index];
                let app_id = app.id.clone();
                line()
                    .id(app.row_id.clone())
                    .test_support()
                    .w_full()
                    .h(dp(34.))
                    .gap_0()
                    .px(dp(8.))
                    .border_b_1()
                    .border_color(p.line)
                    .child(div().w(dp(22.)).flex_none().child(status(app.status, cx)))
                    .child(
                        Button::new(app.name_id.clone())
                            .ghost()
                            .group("fog-control")
                            .small()
                            .flex_1()
                            .min_w(dp(178.))
                            .justify_start()
                            .px_0()
                            .overflow_hidden()
                            .tooltip(app.label.clone())
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_0()
                                    .child(
                                        mono(app.namespace_prefix.clone())
                                            .max_w(relative(0.45))
                                            .flex_none()
                                            .truncate()
                                            .text_color(p.muted)
                                            .group_hover("fog-control", |style| {
                                                style.text_color(p.ink_2)
                                            }),
                                    )
                                    .child(mono(app.name.clone()).flex_1().truncate()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_app(app_id.clone(), Report::Errors, cx)
                            })),
                    )
                    .child(
                        text(app.language.clone())
                            .text_size(dp(12.))
                            .w(dp(54.))
                            .flex_none(),
                    )
                    .children(Report::ALL.into_iter().map(|report| {
                        let check = app.check(report);
                        let app_id = app.id.clone();
                        let problem = !matches!(check.status, Status::Ok | Status::Unknown);
                        Button::new(check.element_id.clone())
                            .ghost()
                            .group("fog-control")
                            .small()
                            .w(dp(report.column_width()))
                            .px(dp(4.))
                            .gap(dp(4.))
                            .justify_start()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .text_color(if problem {
                                ink(check.status, cx)
                            } else {
                                p.muted
                            })
                            .child(status(check.status, cx))
                            .when(!check.value.is_empty(), |button| {
                                button.child(
                                    text(check.value.clone())
                                        .truncate()
                                        .group_hover("fog-control", |style| {
                                            style.text_color(p.ink_2)
                                        }),
                                )
                            })
                            .tooltip(check.tooltip.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_app(app_id.clone(), report, cx)
                            }))
                    }))
                    .into_any_element()
            }
        }
    }
}

impl ObservabilityPage {
    pub(super) fn applications_header(&self, window: &Window, cx: &Context<Self>) -> Div {
        let header = self.page_header(window);
        let segment = line()
            .gap(dp(2.))
            .p(dp(3.))
            .rounded(px(8.))
            .bg(palette(cx).surface_2)
            .children([Filter::Problems, Filter::All].into_iter().map(|filter| {
                ui::segment(
                    Button::new(SharedString::from(format!("obs-filter-{}", filter.slug()))),
                    self.filter == filter,
                    cx,
                )
                .small()
                .label(filter.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter = filter;
                    this.project();
                    cx.notify();
                }))
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
                            Filter::Integration => Tone::Unknown, // TODO: shared Tone::Integration in components follow-up.
                            Filter::Logs => Tone::Accent,
                            _ => Tone::Good,
                        },
                        self.counts[ix],
                        filter.label(),
                        self.filter == filter,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = if this.filter == filter {
                            Filter::All
                        } else {
                            filter
                        };
                        this.project();
                        cx.notify();
                    }))
                }),
            cx,
        );
        let categories = line()
            .flex_wrap()
            .children(self.categories.iter().take(6).map(|category| {
                let category = category.clone();
                Button::new(SharedString::from(format!("obs-category-{category}")))
                    .ghost()
                    .small()
                    .label(category.clone())
                    .selected(self.active_categories.contains(&category))
                    .accessibility_label(format!("Category: {category}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let active = Rc::make_mut(&mut this.active_categories);
                        if !active.remove(&category) {
                            active.insert(category.clone());
                        }
                        this.project();
                        cx.notify();
                    }))
            }));
        let categories = categories.when(self.categories.len() > 6, |row| {
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
        let mut meta = vec![count.into_any_element()];
        if self.live.apps.is_stale() {
            meta.push(" · stale".into_any_element());
        }
        if let Some(time) = self.live.apps.last_successful() {
            meta.extend([" · ".into_any_element(), ui::clock(time).into_any_element()]);
        }
        self.time_controls(
            self.source_header(
                header
                    .filter(filter)
                    .chips(Some(
                        v_flex()
                            .gap(dp(4.))
                            .child(line().child(segment).child(chips))
                            .child(categories),
                    ))
                    .meta(meta),
                cx,
            )
            .control(self.namespace_picker(cx))
            .control(self.application_density(cx))
            .control(self.application_columns_menu(cx)),
            cx,
        )
        .render(cx)
    }
}

impl ObservabilityPage {
    fn namespace_picker(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let namespace = self.namespace.clone();
        let choices = self.namespaces.clone();
        action(
            "obs-namespace",
            namespace.clone().unwrap_or_else(|| "All namespaces".into()),
        )
        .dropdown_caret(true)
        .max_w(dp(150.))
        .overflow_hidden()
        .accessibility_label("Application namespace")
        .dropdown_menu(move |mut menu, _, _| {
            for next in std::iter::once(None).chain(choices.iter().take(100).cloned().map(Some)) {
                let owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(next.clone().unwrap_or_else(|| "All namespaces".into()))
                        .checked(namespace == next)
                        .on_click(move |_, _, cx| {
                            _ = owner.update(cx, |this, cx| {
                                this.namespace = next.clone();
                                this.project();
                                cx.notify();
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element()
    }
}

impl ObservabilityPage {
    fn application_density(&self, cx: &Context<Self>) -> Button {
        Button::new("obs-density")
            .outline()
            .small()
            .icon(if self.application_table.compact {
                IconName::Rows4
            } else {
                IconName::Rows2
            })
            .accessibility_label("Application table density")
            .tooltip(if self.application_table.compact {
                "Compact · 26px rows. Switch to comfortable"
            } else {
                "Comfortable · 34px rows. Switch to compact"
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.application_table.compact = !this.application_table.compact;
                cx.notify();
            }))
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
}

impl ObservabilityPage {
    fn more_categories(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let categories = self.categories.clone();
        let active = self.active_categories.clone();
        Button::new("obs-more-categories")
            .ghost()
            .small()
            .label("More categories")
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                let all_owner = owner.clone();
                let all_categories = categories.clone();
                menu = menu.item(
                    PopupMenuItem::new("All categories")
                        .checked(active.len() == categories.len())
                        .on_click(move |_, _, cx| {
                            _ = all_owner.update(cx, |this, cx| {
                                this.active_categories =
                                    Rc::new(all_categories.iter().cloned().collect());
                                this.project();
                                cx.notify();
                            });
                        }),
                );
                for category in categories.iter().skip(6).take(94) {
                    let owner = owner.clone();
                    let category = category.clone();
                    menu = menu.item(
                        PopupMenuItem::new(category.clone())
                            .checked(active.contains(&category))
                            .on_click(move |_, _, cx| {
                                _ = owner.update(cx, |this, cx| {
                                    let active = Rc::make_mut(&mut this.active_categories);
                                    if !active.remove(&category) {
                                        active.insert(category.clone());
                                    }
                                    this.project();
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
