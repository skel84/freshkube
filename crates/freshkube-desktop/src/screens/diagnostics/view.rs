//! How Diagnostics draws: the header with its status chips, the checks and
//! the selection's details.
use super::*;
use freshkube_ui::page::{self, PageHeader};
use table::DataTable;

impl DiagnosticsScreen {
    fn details(
        &self,
        check: Option<&DiagnosticCheck>,
        snapshot: &DiagnosticSnapshot,
        address: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        let Some(check) = check else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("Select a check to see its details.");
        };
        let (tone, status) = status_tone(&check.status);
        let service = log_service(check);
        let fix = check.fix.as_ref().map(|fix| FixGuidance::of(fix, address));
        let applicable = Self::applicable_fix(check).is_some();
        let check_key = key(check);
        let notice = self
            .notice
            .as_ref()
            .filter(|notice| notice.key == check_key)
            .map(|notice| (notice.ok, notice.text.clone()));
        let fix_blocked = Operations::current(cx).map(|running| running.label.to_string());
        let evidence = check.details.clone().filter(|text| !text.trim().is_empty());
        let message = check.message.clone();
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .id("diagnostic-detail-title")
                            .test_support()
                            .aria_label(check.name.clone())
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(check.name.clone()),
                    )
                    .child(ui::tag(tone, None, status, cx))
                    .child(div().flex_1())
                    .when_some(service, |this, service| {
                        this.child(
                            Button::new("diagnostic-open-logs")
                                .outline()
                                .xsmall()
                                .icon(IconName::ScrollText)
                                .label(format!("{service} logs"))
                                .tooltip("Show this service's logs on the target node")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ScreenEvent::OpenLogs(service.clone()))
                                })),
                        )
                    }),
            )
            .child(field(
                "Result",
                div()
                    .id("diagnostic-detail-message")
                    .test_support()
                    .aria_label(message.clone())
                    .child(message),
                cx,
            ))
            .child(field(
                "Section",
                section_title(check.category, snapshot),
                cx,
            ))
            .when_some(evidence, |this, evidence| {
                this.child(
                    v_flex()
                        .id("diagnostic-detail-evidence")
                        .test_support()
                        .aria_label(evidence.clone())
                        .gap_1()
                        .child(ui::caption("Details", cx))
                        .children(evidence.lines().map(|line| {
                            mono(line.to_owned())
                                .whitespace_normal()
                                .text_color(p.ink_2)
                        })),
                )
            })
            .when(check.status == CheckStatus::Unknown, |this| {
                this.child(
                    div().text_size(dp(12.5)).text_color(p.muted).child(
                        "Nothing is known about this source, so nothing is shown as failed.",
                    ),
                )
            })
            .when_some(fix, |this, fix| {
                let text = fix.text();
                this.child(
                    v_flex()
                        .id("diagnostic-detail-fix")
                        .test_support()
                        .aria_label(text)
                        .gap_1p5()
                        .pt_1()
                        .child(ui::caption("Suggested fix", cx))
                        .child(
                            div()
                                .text_size(dp(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(fix.description.clone()),
                        )
                        .when(!fix.body.is_empty(), |this| {
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(dp(12.))
                                            .text_color(p.muted)
                                            .child(fix.label),
                                    )
                                    .child(
                                        v_flex()
                                            .px_2p5()
                                            .py_2()
                                            .rounded(px(8.))
                                            .border_1()
                                            .border_color(p.line)
                                            .bg(p.hover)
                                            .children(fix.body.iter().map(|line| {
                                                mono(line.clone()).whitespace_normal()
                                            })),
                                    ),
                            )
                        })
                        .when_some(fix.note, |this, note| {
                            this.child(div().text_size(dp(12.5)).text_color(p.warn_ink).child(note))
                        })
                        .when(applicable, |this| {
                            this.child(
                                h_flex().gap_2().child(
                                    Button::new("diagnostic-apply-fix")
                                        .primary()
                                        .small()
                                        .icon(IconName::Wrench)
                                        .label("Apply fix")
                                        .disabled(fix_blocked.is_some())
                                        .when_some(fix_blocked, |this, label| {
                                            this.tooltip(format!("{label} is still running"))
                                        })
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.confirm_fix(check_key.clone(), window, cx)
                                        })),
                                ),
                            )
                        })
                        .when(!applicable, |this| {
                            this.child(
                                div()
                                    .text_size(dp(12.))
                                    .text_color(p.muted)
                                    .child("Guidance only; this fix can't be applied from here."),
                            )
                        })
                        .when_some(notice, |this, (ok, text)| {
                            let (tone, lead) = if ok {
                                (Tone::Good, "Fix applied")
                            } else {
                                (Tone::Crit, "Fix failed")
                            };
                            this.child(
                                h_flex()
                                    .id("diagnostic-fix-result")
                                    .test_support()
                                    .role(Role::Status)
                                    .aria_label(format!("{lead}: {text}"))
                                    .items_start()
                                    .gap_2()
                                    .child(ui::tag(tone, None, lead, cx))
                                    .child(
                                        div().flex_1().min_w_0().text_size(dp(12.5)).child(text),
                                    ),
                            )
                        }),
                )
            })
    }
}

impl Render for DiagnosticsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync();
        let header = self.render_header(window, cx);
        let state = gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Node,
            "the diagnostics",
            cx,
        );
        let page = page::page("diagnostics-page")
            .h_auto()
            .flex_none()
            .child(page::toolbar(cx).child(header));
        let page = match (state, self.loader.data()) {
            (Some(state), _) => page.child(
                page::inset()
                    .id("diagnostics-state")
                    .test_support()
                    .child(state),
            ),
            (None, Some(snapshot)) => {
                let banners: Vec<AnyElement> = failure_banner(&self.loader, cx)
                    .map(IntoElement::into_any_element)
                    .into_iter()
                    .chain(partial_notice(missing(snapshot), cx))
                    .collect();
                page.when(!banners.is_empty(), |page| {
                    page.child(
                        page::inset()
                            .flex()
                            .flex_col()
                            .gap(dp(page::PANE_PADDING_Y))
                            .children(banners),
                    )
                })
                .child(self.render_split(window, cx))
            }
            (None, None) => page,
        };
        // The keys live on a wrapper drawn in every state, so the page keeps
        // them while a state or an empty filter shows; the page scrolls when
        // the details stack under the table.
        div()
            .id("diagnostics-scroll")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .on_action(cx.listener(|view, _: &NextCheck, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousCheck, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstCheck, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastCheck, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &ToggleProblems, _, cx| {
                view.show(view.shown.toggle_problems(), cx)
            }))
            .child(page)
    }
}

impl DiagnosticsScreen {
    /// The toolbar: the title, the status chips and Refresh; the CNI and
    /// the addons go in the meta line.
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Diagnostics");
        let chips = self.derived.as_ref().map(|derived| {
            let shown = self.shown;
            table::status_chips(
                header.id("tally"),
                TALLIES.iter().zip(derived.counts).map(|(&tally, count)| {
                    table::status_chip(
                        header.id(&format!("tally-{}", tally.what())),
                        tally.tone(),
                        count,
                        tally.what(),
                        shown.chosen(tally),
                        cx,
                    )
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.show(view.shown.press(tally), cx)),
                    )
                }),
                cx,
            )
        });
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh diagnostics",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        let parts = self
            .derived
            .as_ref()
            .map(|derived| derived.meta.clone())
            .unwrap_or_default();
        header
            .chips(chips)
            .control(refresh)
            .meta(meta(
                self.source.as_ref(),
                Scope::Node,
                &self.loader,
                self.embedded,
                parts,
            ))
            .render(window, cx)
    }

    /// The checks, with the selection's details beside them on a wide page
    /// and below them on a narrow one.
    fn render_split(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let beside = crate::screens::beside(window);
        let table = div()
            .id("diagnostic-table")
            .w_full()
            .child(
                DataTable::new()
                    .fit(table::TableSource::line_count(self).max(1))
                    .render(self, window, cx)
                    .w_full()
                    .flex_none(),
            )
            .into_any_element();
        let (Some(source), Some(snapshot)) = (self.source.as_ref(), self.loader.data()) else {
            return table;
        };
        let details = self.details(self.selected_check(), snapshot, &source.target.address, cx);
        let details = div()
            .id("diagnostic-details")
            .test_support()
            .when_else(
                beside,
                |this| this.pr(dp(page::PANE_PADDING)).py(dp(page::PANE_PADDING_Y)),
                |this| this.px(dp(page::PANE_PADDING)).pb(dp(page::PANE_PADDING_Y)),
            )
            .child(details)
            .into_any_element();
        crate::screens::split_narrow("diagnostics-split", beside, table, Some(details))
    }
}

// ---------------------------------------------------------------------------
// Example data
