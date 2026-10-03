use super::*;

impl DiagnosticsScreen {
    fn summary(&self, snapshot: &DiagnosticSnapshot, cx: &App) -> impl IntoElement + use<> {
        let count = |status: CheckStatus| {
            snapshot
                .checks
                .iter()
                .filter(|check| check.status == status)
                .count()
        };
        let (pass, warn, fail, unknown) = (
            count(CheckStatus::Pass),
            count(CheckStatus::Warn),
            count(CheckStatus::Fail),
            count(CheckStatus::Unknown) + count(CheckStatus::Checking),
        );
        let cni = snapshot
            .cni
            .cni_type
            .as_ref()
            .map_or("unknown", CniType::name);
        let addons = addons_label(&snapshot.addons);
        h_flex()
            .id("diagnostic-summary")
            .test_support()
            .aria_label(format!(
                "{pass} passing, {warn} warnings, {fail} failing, {unknown} unknown; CNI {cni}; addons {addons}"
            ))
            .gap_2p5()
            .flex_wrap()
            .child(stat("Passing", pass.to_string(), cx))
            .child(stat("Warnings", warn.to_string(), cx))
            .child(stat("Failing", fail.to_string(), cx))
            .child(stat("Unknown", unknown.to_string(), cx))
            .child(stat("CNI", cni, cx))
            .child(stat("Addons", addons, cx))
    }

    fn toolbar(&self, shown: usize, total: usize, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        h_flex()
            .gap_3()
            .items_center()
            .flex_wrap()
            .child(
                div()
                    .text_size(dp(12.5))
                    .text_color(p.muted)
                    .child(format!("{shown} of {total} checks")),
            )
            .child(div().flex_1())
            .child(
                Button::new("diagnostic-filter")
                    .outline()
                    .small()
                    .icon(IconName::ListFilter)
                    .label("Only problems")
                    .selected(self.only_problems)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_only_problems(!view.only_problems, cx)
                    })),
            )
    }

    fn render_row(
        &self,
        ix: usize,
        check: &DiagnosticCheck,
        selected: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let (tone, status) = status_tone(&check.status);
        let key = key(check);
        let columns: &[Column] = if compact {
            &COMPACT_COLUMNS
        } else {
            &FULL_COLUMNS
        };
        let row = h_flex()
            .id(("diagnostic-check", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!("{} · {} · {}", check.name, status, check.message))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .flex_none()
            .text_size(dp(12.5))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .child(cell(columns[0]).child(ui::tag(tone, None, status, cx)));
        let row = if compact {
            row.child(
                cell(columns[1])
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(check.name.clone()),
                    )
                    .child(
                        div()
                            .ml_2()
                            .when(!selected, |this| this.text_color(p.muted))
                            .child(check.message.clone()),
                    ),
            )
        } else {
            row.child(
                cell(columns[1])
                    .font_weight(FontWeight::MEDIUM)
                    .child(check.name.clone()),
            )
            .child(
                cell(columns[2])
                    .when(!selected, |this| this.text_color(p.muted))
                    .child(check.message.clone()),
            )
        };
        row.on_click(cx.listener(move |view, _, window, cx| {
            view.selected = Some(key.clone());
            window.focus(&view.focus, cx);
            cx.notify();
        }))
    }

    fn section_head(title: String, category: CheckCategory, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        h_flex()
            .id(SharedString::from(format!(
                "diagnostic-section-{}",
                section_slug(category)
            )))
            .test_support()
            .aria_label(title.clone())
            .flex_none()
            .gap_2()
            .px_3()
            .pt_3()
            .pb_1()
            .border_b_1()
            .border_color(p.line)
            .child(
                Icon::new(section_icon(category))
                    .size(dp(13.))
                    .text_color(p.muted),
            )
            .child(ui::caption(&title, cx))
    }

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
                                            .rounded(px(6.))
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
        if let Some(page) = gated_page_mode(
            "diagnostics-page",
            "Diagnostics",
            Scope::Node,
            self.source.as_ref(),
            &self.loader,
            "the diagnostics",
            self.embedded,
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(snapshot)) = (self.source.clone(), self.loader.data().cloned())
        else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let all = visible_checks(&snapshot, self.only_problems);
        let keys: Vec<String> = all.iter().map(|check| key(check)).collect();
        let selected_ix = self.selected_index(&keys);
        let missing = missing(&snapshot);
        let summary = self.summary(&snapshot, cx);
        let toolbar = self.toolbar(all.len(), snapshot.checks.len(), cx);

        let width = content_width(window);
        let wide = width >= SIDE_DETAILS;
        // The list gets what the side details leave; fold the result column
        // into the check's cell before anything would be clipped.
        let list_width = if wide {
            width - (DETAILS_WIDTH + 14.)
        } else {
            width
        };
        let compact = list_width < table_width(&FULL_COLUMNS);
        let columns: &[Column] = if compact {
            &COMPACT_COLUMNS
        } else {
            &FULL_COLUMNS
        };

        // Rows are grouped under section captions; remember where the
        // selected row lands among the children so it can scroll into view.
        let mut children: Vec<AnyElement> = Vec::new();
        let mut selected_child = None;
        let mut section = None;
        for (ix, check) in all.iter().enumerate() {
            if section != Some(check.category) {
                section = Some(check.category);
                children.push(
                    Self::section_head(
                        section_title(check.category, &snapshot),
                        check.category,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            if selected_ix == Some(ix) {
                selected_child = Some(children.len());
            }
            children.push(
                self.render_row(ix, check, selected_ix == Some(ix), compact, cx)
                    .into_any_element(),
            );
        }
        if let Some(child) = selected_child {
            self.scroll.scroll_to_item(child);
        }

        let empty = if self.only_problems {
            "No warnings or failures."
        } else {
            "The diagnostics reported nothing."
        };
        let list = panel(cx)
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(table_head(columns, cx))
            .child(
                v_flex()
                    .id("diagnostic-list")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("Diagnostic checks; arrows select a check, P shows only problems")
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextCheck, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousCheck, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstCheck, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastCheck, _, cx| view.step(isize::MAX, cx)))
                    .on_action(cx.listener(|view, _: &ToggleProblems, _, cx| {
                        view.set_only_problems(!view.only_problems, cx)
                    }))
                    .flex_1()
                    .min_h_0()
                    .pb_2()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when(all.is_empty(), |this| {
                        this.child(
                            div()
                                .px_3()
                                .py_3p5()
                                .text_size(dp(12.5))
                                .text_color(p.muted)
                                .child(empty),
                        )
                    })
                    .children(children),
            );
        let details = self.details(
            selected_ix.map(|ix| all[ix]),
            &snapshot,
            &source.target.address,
            cx,
        );
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(dp(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("diagnostic-details")
                        .w(dp(DETAILS_WIDTH))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(14.)).child(list).child(
                        div()
                            .id("diagnostic-details")
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("diagnostics-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(crate::screens::header_mode(
                "Diagnostics",
                &source,
                Scope::Node,
                &self.loader,
                self.embedded,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(toolbar)
            .child(split)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Example data
