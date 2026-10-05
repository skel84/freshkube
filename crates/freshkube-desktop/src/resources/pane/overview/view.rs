use super::*;

impl DetailPane {
    pub(in crate::resources::pane) fn overview(
        &self,
        detail: &Detail,
        summary: &Summary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let identity = &detail.target.identity;
        let section = |title: &str, cx: &App| v_flex().gap_2().child(ui::caption(title, cx));
        let mut body = v_flex()
            .gap(dp(18.))
            .when(detail.target.kind.is_pod(), |this| {
                this.child(self.pod_sections(cx))
            })
            .child(
                v_flex()
                    .gap_2()
                    .child(field("Kind", summary.kind.clone(), cx))
                    .when(!identity.namespace.is_empty(), |this| {
                        this.child(field("Namespace", identity.namespace.clone(), cx))
                    })
                    .child(field(
                        "UID",
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .child(identity.uid.clone()),
                        cx,
                    ))
                    .children(
                        summary
                            .created
                            .clone()
                            .map(|created| field("Created", created, cx)),
                    )
                    .children(summary.deleting.clone().map(|deleting| {
                        field(
                            "Deleting since",
                            div().text_color(p.crit_ink).child(deleting),
                            cx,
                        )
                    }))
                    .children(
                        summary
                            .generation
                            .clone()
                            .map(|generation| field("Generation", generation, cx)),
                    ),
            );
        if let Some((secret_type, keys)) = &summary.secret {
            body = body.child(self.secret(detail, secret_type, keys, cx));
        }
        if !summary.conditions.is_empty() {
            body =
                body.child(
                    section("Conditions", cx).children(summary.conditions.iter().enumerate().map(
                        |(ix, condition)| {
                            h_flex()
                                .id(("detail-condition", ix))
                                .test_support()
                                .items_start()
                                .gap_2()
                                .child(
                                    div()
                                        .w(dp(132.))
                                        .flex_none()
                                        .text_size(dp(12.5))
                                        .child(condition.kind.clone()),
                                )
                                .child(ui::tag(condition.tone, None, condition.status.clone(), cx))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(dp(12.))
                                        .children(condition.detail.clone())
                                        .children(condition.changed.clone().map(|changed| {
                                            div().text_color(p.muted).child(changed)
                                        })),
                                )
                                .into_any_element()
                        },
                    )),
                );
        }
        if !summary.owners.is_empty() {
            body = body.child(
                section("Owned by", cx)
                    .id("detail-owners")
                    .test_support()
                    .children(
                        summary
                            .owners
                            .iter()
                            .map(|owner| self.owner_button(owner, cx)),
                    ),
            );
        }
        if summary.label_count > 0 {
            let shown = if self.show_all_labels {
                summary.labels.len()
            } else {
                LABELS_SHOWN.min(summary.labels.len())
            };
            body = body.child(
                section("Labels", cx)
                    .child(
                        h_flex()
                            .id("detail-labels")
                            .test_support()
                            .flex_wrap()
                            .gap_1p5()
                            .children(summary.labels[..shown].iter().map(|label| {
                                div()
                                    .max_w_full()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(3.))
                                    .bg(p.surface_2)
                                    .border_1()
                                    .border_color(p.line)
                                    .font_family(MONO_FONT)
                                    .text_size(dp(11.5))
                                    .truncate()
                                    .child(label.clone())
                            })),
                    )
                    .children(self.show_more(
                        "detail-labels-all",
                        shown,
                        summary.label_count,
                        "labels",
                        |pane| &mut pane.show_all_labels,
                        cx,
                    )),
            );
        }
        if summary.annotation_count > 0 {
            let shown = if self.show_all_annotations {
                summary.annotations.len()
            } else {
                ANNOTATIONS_SHOWN.min(summary.annotations.len())
            };
            body = body.child(
                section("Annotations", cx)
                    .child(
                        v_flex()
                            .id("detail-annotations")
                            .test_support()
                            .gap_1p5()
                            .children(summary.annotations[..shown].iter().map(|(key, value)| {
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(MONO_FONT)
                                            .text_size(dp(11.5))
                                            .text_color(p.ink_2)
                                            .truncate()
                                            .child(key.clone()),
                                    )
                                    .child(
                                        div()
                                            .font_family(MONO_FONT)
                                            .text_size(dp(11.5))
                                            .child(value.clone()),
                                    )
                            })),
                    )
                    .children(self.show_more(
                        "detail-annotations-all",
                        shown,
                        summary.annotation_count,
                        "annotations",
                        |pane| &mut pane.show_all_annotations,
                        cx,
                    )),
            );
        }
        if !summary.finalizers.is_empty() {
            body = body.child(
                section("Finalizers", cx).children(summary.finalizers.iter().map(|finalizer| {
                    div()
                        .font_family(MONO_FONT)
                        .text_size(dp(12.))
                        .child(finalizer.clone())
                })),
            );
        }
        div()
            .id("detail-overview")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .child(body)
            .into_any_element()
    }

    /// "Show all" under a capped list, or how many more the YAML has.
    fn show_more(
        &self,
        id: &'static str,
        shown: usize,
        total: usize,
        what: &str,
        flag: fn(&mut Self) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if shown == total {
            return None;
        }
        let p = palette(cx);
        if shown == MAX_SHOWN {
            return Some(
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(format!("{} more in the YAML", total - shown))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .child(
                    Button::new(id)
                        .ghost()
                        .xsmall()
                        .label(format!("Show all {total} {what}"))
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            *flag(pane) = true;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    fn secret(
        &self,
        detail: &Detail,
        secret_type: &SharedString,
        keys: &[SecretLine],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        v_flex()
            .gap_2()
            .child(ui::caption("Secret data", cx))
            .child(field("Type", secret_type.clone(), cx))
            .child(div().text_size(dp(12.)).text_color(p.muted).child(
                "Values are hidden. Reveal reads one value afresh; Copy YAML never includes them.",
            ))
            .children(keys.iter().enumerate().map(|(ix, key)| {
                let reveal = detail.reveals.get(&key.name);
                let name = key.name.clone();
                let actions = match reveal {
                    None | Some(Reveal::Failed(_)) => vec![
                        Button::new(("detail-secret-reveal", ix))
                            .outline()
                            .xsmall()
                            .icon(IconName::Eye)
                            .label("Reveal")
                            .on_click(cx.listener({
                                let name = name.clone();
                                move |pane, _, _, cx| pane.reveal(name.clone(), cx)
                            }))
                            .into_any_element(),
                    ],
                    Some(Reveal::Reading) => vec![
                        Button::new(("detail-secret-reveal", ix))
                            .outline()
                            .xsmall()
                            .loading(true)
                            .label("Reading")
                            .into_any_element(),
                    ],
                    Some(Reveal::Shown(value)) => {
                        let mut actions = vec![
                            Button::new(("detail-secret-hide", ix))
                                .outline()
                                .xsmall()
                                .icon(IconName::EyeOff)
                                .label("Hide")
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |pane, _, _, cx| pane.hide(&name, cx)
                                }))
                                .into_any_element(),
                        ];
                        if let SecretValue::Text(_) = value {
                            actions.push(
                                Button::new(("detail-secret-copy", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Copy)
                                    .label("Copy")
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |pane, _, _, cx| pane.copy_value(&name, cx)
                                    }))
                                    .into_any_element(),
                            );
                        }
                        actions
                    }
                };
                let value = match reveal {
                    Some(Reveal::Shown(SecretValue::Text(text))) => Some(
                        div()
                            .id(("detail-secret-value", ix))
                            .test_support()
                            .max_h(dp(160.))
                            .overflow_y_scroll()
                            .px_2()
                            .py_1p5()
                            .rounded(px(8.))
                            .bg(p.surface_2)
                            .font_family(MONO_FONT)
                            .text_size(dp(12.))
                            .child(match text.char_indices().nth(VALUE_CHARS) {
                                Some((cut, _)) => {
                                    format!("{}… (Copy takes the whole value)", &text[..cut])
                                }
                                None => text.clone(),
                            })
                            .into_any_element(),
                    ),
                    Some(Reveal::Shown(SecretValue::Binary(bytes))) => Some(
                        div()
                            .id(("detail-secret-value", ix))
                            .test_support()
                            .text_size(dp(12.))
                            .text_color(p.muted)
                            .child(format!("Binary data, {}; not shown.", byte_size(*bytes)))
                            .into_any_element(),
                    ),
                    Some(Reveal::Failed(reason)) => Some(
                        div()
                            .id(("detail-secret-error", ix))
                            .test_support()
                            .text_size(dp(12.))
                            .text_color(p.crit_ink)
                            .child(reason.clone())
                            .into_any_element(),
                    ),
                    _ => None,
                };
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .id(("detail-secret-key", ix))
                            .test_support()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(MONO_FONT)
                                    .text_size(dp(12.5))
                                    .truncate()
                                    .child(key.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(dp(12.))
                                    .text_color(p.muted)
                                    .child(key.size.clone()),
                            )
                            .children(actions),
                    )
                    .children(value)
            }))
    }
}
