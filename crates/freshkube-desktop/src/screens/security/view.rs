//! The page: the banners, the summary, the audit's list and the
//! selection's details.
use super::*;

impl SecurityScreen {
    fn summary(&self, snapshot: &SecurityAuditSnapshot, cx: &App) -> Stateful<Div> {
        let certs: Vec<&CertificateAudit> = snapshot
            .talosconfig_certificates
            .value()
            .into_iter()
            .chain(snapshot.talos_kubeconfig_certificates.value())
            .flatten()
            .collect();
        let known = snapshot.talosconfig_certificates.value().is_some()
            || snapshot.talos_kubeconfig_certificates.value().is_some();
        let count = |pred: fn(CertificateExpiryStatus) -> bool| {
            if known {
                certs
                    .iter()
                    .filter(|cert| pred(cert.expiry))
                    .count()
                    .to_string()
            } else {
                "unknown".to_owned()
            }
        };
        let valid = count(|status| status == CertificateExpiryStatus::Valid);
        let expiring = count(|status| {
            matches!(
                status,
                CertificateExpiryStatus::ExpiringSoon | CertificateExpiryStatus::ExpiringVerySoon
            )
        });
        let expired = count(|status| status == CertificateExpiryStatus::Expired);
        let volumes = match snapshot.volume_encryption.value() {
            Some(volumes) if !volumes.is_empty() => format!(
                "{} of {}",
                volumes
                    .iter()
                    .filter(|volume| volume.provider != EncryptionProvider::None)
                    .count(),
                volumes.len()
            ),
            _ => "unknown".to_owned(),
        };
        h_flex()
            .id("security-summary")
            .gap_2p5()
            .flex_wrap()
            .child(stat("Valid certificates", valid, cx))
            .child(stat("Expiring soon", expiring, cx))
            .child(stat("Expired", expired, cx))
            .child(stat("Volumes encrypted", volumes, cx))
    }

    fn render_row(
        &self,
        ix: usize,
        item: &Item,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let (tone, _) = item.verdict.tone();
        let key = item.key.clone();
        h_flex()
            .id(("security-item", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {} · {}",
                item.name, item.status, item.summary
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .flex_none()
            .gap_3()
            .px_3()
            .text_size(dp(12.5))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .child(div().w(dp(112.)).flex_none().child(ui::tag(
                tone,
                None,
                item.status.clone(),
                cx,
            )))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(item.name.clone()),
            )
            .child(
                div()
                    .max_w(dp(220.))
                    .truncate()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.))
                    .when(!selected, |this| this.text_color(p.muted))
                    .child(item.summary.clone()),
            )
            .on_click(cx.listener(move |view, _, window, cx| {
                view.selected = Some(key.clone());
                window.focus(&view.focus, cx);
                cx.notify();
            }))
    }

    fn section_head(section: Section, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        h_flex()
            .id(SharedString::from(format!(
                "security-section-{}",
                section.slug()
            )))
            .test_support()
            .aria_label(section.title())
            .flex_none()
            .gap_2()
            .px_3()
            .pt_3()
            .pb_1()
            .border_b_1()
            .border_color(p.line)
            .child(Icon::new(section.icon()).size(dp(13.)).text_color(p.muted))
            .child(ui::caption(section.title(), cx))
    }

    fn details(&self, item: Option<&Item>, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(item) = item else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("Select an item to see its details.");
        };
        let (tone, icon) = item.verdict.tone();
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .id("security-detail-title")
                            .test_support()
                            .aria_label(item.name.clone())
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(item.name.clone()),
                    )
                    .child(ui::tag(tone, icon, item.status.clone(), cx)),
            )
            .children(
                item.fields.iter().map(|(label, value)| {
                    field(label, mono(value.clone()).whitespace_normal(), cx)
                }),
            )
            .when_some(item.note.clone(), |this, note| {
                this.child(
                    div()
                        .id("security-detail-note")
                        .test_support()
                        .aria_label(note.clone())
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child(note),
                )
            })
    }
}

impl Render for SecurityScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_status();
        let header = self.render_header(window, cx);
        let state = gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Cluster,
            "the security audit",
            cx,
        );
        let body = match state {
            Some(state) => page::inset()
                .id("security-state")
                .test_support()
                .child(state)
                .into_any_element(),
            None => self.render_body(window, cx),
        };
        // The keys live on a wrapper drawn in every state; the page scrolls
        // when the window is too short for the list's least height.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
            .child(
                page::page("security-page")
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(page::toolbar(cx).child(header))
                    .child(body),
            )
    }
}

impl SecurityScreen {
    fn render_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let header = PageHeader::new(PREFIX, "Security");
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh the security audit",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        header.control(refresh).render(window, cx)
    }

    /// The banners, the summary and the audit with the selection's details,
    /// inset under the toolbar.
    fn render_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(snapshot) = self.loader.data() else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let all = items(snapshot);
        let selected_ix = self.selected_index(&all);
        let missing = missing(snapshot);
        let summary = self.summary(snapshot, cx);

        // Rows are grouped under section captions; remember where the
        // selected row lands among the children so it can scroll into view.
        let mut children: Vec<AnyElement> = Vec::new();
        let mut selected_child = None;
        let mut section = None;
        for (ix, item) in all.iter().enumerate() {
            if section != Some(item.section) {
                section = Some(item.section);
                children.push(Self::section_head(item.section, cx).into_any_element());
            }
            if selected_ix == Some(ix) {
                selected_child = Some(children.len());
            }
            children.push(
                self.render_row(ix, item, selected_ix == Some(ix), cx)
                    .into_any_element(),
            );
        }
        if let Some(child) = selected_child {
            self.scroll.scroll_to_item(child);
        }

        let list = panel(cx)
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(
                v_flex()
                    .id("security-list")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label(
                        "Certificates, RBAC role and volume encryption; arrows select an item",
                    )
                    .flex_1()
                    .min_h_0()
                    .pb_2()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.scroll)
                    .when(all.is_empty(), |this| {
                        this.child(
                            div()
                                .px_3()
                                .py_3p5()
                                .text_size(dp(12.5))
                                .text_color(p.muted)
                                .child("The audit reported nothing."),
                        )
                    })
                    .children(children),
            );
        let details = self.details(selected_ix.map(|ix| &all[ix]), cx);
        let wide = content_width(window) >= SIDE_DETAILS;
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(dp(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("security-details")
                        .w(dp(380.))
                        .flex_none()
                        .overflow_y_scroll()
                        .restrict_scroll_to_axis()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(14.)).child(list).child(
                        div()
                            .id("security-details")
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("security-body")
            .test_support()
            .flex_1()
            .px(dp(page::PANE_PADDING))
            .py(dp(page::PANE_PADDING_Y))
            .gap(dp(14.))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(summary)
            .child(split)
            .into_any_element()
    }
}
