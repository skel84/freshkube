use super::*;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
const MATRIX_WIDTH: f32 = 960.;
impl ObservabilityPage {
    pub(super) fn render_applications(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let owner = cx.entity().downgrade();
        let category = self.category;
        let categories = action(
            "obs-categories",
            self.category
                .map_or("Categories · All", |ix| example::CATEGORIES[ix]),
        )
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for (ix, label) in [
                "All categories",
                "Applications",
                "Control plane",
                "Monitoring",
            ]
            .into_iter()
            .enumerate()
            {
                let owner = owner.clone();
                let next = ix.checked_sub(1);
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(category == next)
                        .on_click(move |_, _, cx| {
                            _ = owner.update(cx, |this, cx| {
                                this.category = next;
                                this.project();
                                cx.notify();
                            });
                        }),
                );
            }
            menu
        });
        let owner = cx.entity().downgrade();
        let namespace = self.namespace.clone();
        let namespaces = action(
            "obs-namespace",
            self.namespace
                .clone()
                .unwrap_or_else(|| "Namespace · All".into()),
        )
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for ns in [
                "All",
                "payments",
                "platform",
                "cache",
                "kube-system",
                "argocd",
                "ingress",
                "monitoring",
                "logging",
            ] {
                let owner = owner.clone();
                let selected =
                    namespace.as_deref() == Some(ns) || (ns == "All" && namespace.is_none());
                menu = menu.item(PopupMenuItem::new(ns).checked(selected).on_click(
                    move |_, _, cx| {
                        _ = owner.update(cx, |this, cx| {
                            this.namespace = (ns != "All").then(|| ns.into());
                            this.project();
                            cx.notify();
                        });
                    },
                ));
            }
            menu
        });
        let toolbar = line()
            .flex_wrap()
            .child(section("Applications"))
            .child(muted(self.applications.len().to_string(), cx))
            .child(div().flex_1())
            .child(
                div().w(dp(230.)).child(
                    Input::new(&self.query)
                        .id("obs-filter")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size(dp(14.))),
                ),
            )
            .child(namespaces)
            .child(categories);
        let filters = line()
            .flex_wrap()
            .children(Filter::ALL.into_iter().enumerate().map(|(ix, filter)| {
                Button::new(SharedString::from(format!("obs-filter-{}", filter.slug())))
                    .outline()
                    .group("fog-control")
                    .small()
                    .rounded_full()
                    .selected(self.filter == filter)
                    .when(
                        !matches!(filter, Filter::All | Filter::Problems),
                        |button| {
                            button.child(status(
                                match filter {
                                    Filter::Critical => Status::Critical,
                                    Filter::Warning => Status::Warning,
                                    Filter::Logs => Status::LogError,
                                    Filter::Integration => Status::Integration,
                                    _ => Status::Ok,
                                },
                                cx,
                            ))
                        },
                    )
                    .child(text(format!("{} {}", filter.label(), self.counts[ix])))
                    .accessibility_label(format!("{} {}", filter.label(), self.counts[ix]))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = filter;
                        this.project();
                        cx.notify();
                    }))
            }));
        let header = h_flex()
            .h(dp(30.))
            .px(dp(8.))
            .bg(p.surface_2)
            .child(div().w(dp(22.)).flex_none())
            .child(ui::caption("Application", cx).w(dp(178.)).flex_none())
            .child(ui::caption("Type", cx).w(dp(54.)).flex_none())
            .children(Report::ALL.into_iter().map(|report| {
                ui::caption(report.label(), cx)
                    .w(dp(report.column_width()))
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
                            .child(text("No applications match these filters"))
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
                        "Healthy values are muted · every check opens its report",
                        cx,
                    )),
            );
        v_flex()
            .gap(dp(12.))
            .child(toolbar)
            .child(filters)
            .child(
                div()
                    .id("obs-matrix-horizontal")
                    .overflow_x_scroll()
                    .child(table),
            )
            .into_any_element()
    }
    fn matrix_row(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        match &self.matrix[index] {
            MatrixRow::Group {
                label,
                shown,
                hidden,
            } => line()
                .w_full()
                .h(dp(34.))
                .px(dp(14.))
                .bg(cx.theme().background)
                .border_b_1()
                .border_color(p.line)
                .child(text(label.clone()).font_weight(ui::HEADING_WEIGHT))
                .child(muted(format!("{shown} shown · {hidden} OK hidden"), cx))
                .into_any_element(),
            MatrixRow::App(index) => {
                let index = *index;
                let app = &self.applications[index];
                line()
                    .id(SharedString::from(format!("obs-app-{}", app.key)))
                    .test_support()
                    .w_full()
                    .h(dp(34.))
                    .gap_0()
                    .px(dp(8.))
                    .border_b_1()
                    .border_color(p.line)
                    .child(div().w(dp(22.)).flex_none().child(status(app.status, cx)))
                    .child(
                        Button::new(SharedString::from(format!("obs-name-{}", app.key)))
                            .ghost()
                            .group("fog-control")
                            .small()
                            .w(dp(178.))
                            .justify_start()
                            .px_0()
                            .overflow_hidden()
                            .tooltip(app.key.clone())
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_0()
                                    .child(
                                        mono(app.namespace.clone() + "/")
                                            .max_w(dp(92.))
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
                                this.open_app(index, Report::Errors, cx)
                            })),
                    )
                    .child(text(app.language).text_size(dp(12.)).w(dp(54.)).flex_none())
                    .children(Report::ALL.into_iter().map(|report| {
                        let check = app.check(report);
                        let problem = !matches!(check.status, Status::Ok | Status::Unknown);
                        Button::new(SharedString::from(format!(
                            "obs-check-{}-{}",
                            app.key,
                            report.slug()
                        )))
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
                        .when(check.status != Status::Ok, |button| {
                            button.child(status(check.status, cx))
                        })
                        .child(
                            text(check.value.clone())
                                .truncate()
                                .group_hover("fog-control", |style| style.text_color(p.ink_2)),
                        )
                        .tooltip(format!(
                            "{} · {}: {} · Open report",
                            app.key,
                            report.label(),
                            check.value
                        ))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_app(index, report, cx)),
                        )
                    }))
                    .into_any_element()
            }
        }
    }
}
