use super::*;
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
                            .accessibility_label(app.label.clone())
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
                            .accessibility_label(check.label.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_app(app_id.clone(), report, cx)
                            }))
                    }))
                    .into_any_element()
            }
        }
    }
}
