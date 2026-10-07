//! The page: the banners and the summary in an inset, then the audit's
//! table edge to edge, then the selection's details.
use super::*;
use audit::Stats;
use freshkube_ui::table::DataTable;

impl SecurityScreen {
    fn summary(stats: &Stats, cx: &App) -> AnyElement {
        h_flex()
            .id("security-summary")
            .test_support()
            .gap_2p5()
            .flex_wrap()
            .child(stat("Valid certificates", stats.valid.clone(), cx))
            .child(stat("Expiring soon", stats.expiring.clone(), cx))
            .child(stat("Expired", stats.expired.clone(), cx))
            .child(stat("Volumes encrypted", stats.volumes.clone(), cx))
            .into_any_element()
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
        self.sync();
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
            Some(state) => vec![
                page::inset()
                    .id("security-state")
                    .test_support()
                    .child(state)
                    .into_any_element(),
            ],
            None if self.loader.data().is_some() => self.render_body(window, cx),
            None => Vec::new(),
        };
        // The keys live on a wrapper drawn in every state; the page scrolls
        // when the window is too short for the table's least height.
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
                    .children(body),
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

    /// The banners and the summary in an inset, then the table edge to
    /// edge with the selection's details under it.
    fn render_body(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Display { missing, stats, .. } = &self.display.1;
        let inset = page::inset()
            .id("security-inset")
            .test_support()
            .flex()
            .flex_col()
            .gap(dp(page::PANE_PADDING_Y))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing.clone(), cx))
            .child(Self::summary(stats, cx));
        let table = div()
            .id("security-table")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h(dp(inspector::STACKED_LIST_HEIGHT))
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0());
        let details = page::inset()
            .id("security-details")
            .h(dp(DETAILS_HEIGHT))
            .flex_none()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(self.details(self.selected_item(), cx));
        vec![
            inset.into_any_element(),
            table.into_any_element(),
            details.into_any_element(),
        ]
    }
}
