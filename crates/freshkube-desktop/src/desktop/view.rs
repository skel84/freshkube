use super::*;

impl Render for Pilot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit("shell");
        let page = match self.page {
            Page::Resources => self
                .resources
                .clone()
                .cached(cached_page_style())
                .into_any_element(),
            _ if self.kubernetes_only.is_some()
                && !matches!(self.page, Page::Overview | Page::Health | Page::Nodes) =>
            {
                self.render_needs_talosconfig(cx)
            }
            Page::Nodes => self
                .nodes_page
                .clone()
                .cached(cached_page_style())
                .into_any_element(),
            Page::Overview => self
                .overview_page
                .clone()
                .cached(cached_page_style())
                .into_any_element(),
            Page::SystemServices => self
                .services_page
                .clone()
                .cached(cached_page_style())
                .into_any_element(),

            // Screens redraw when their own state changes, not with the shell.
            _ => self
                .active_screen()
                .map(|screen| screen.view().cached(cached_page_style()).into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .key_context("Freshkube")
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &Refresh, window, cx| view.refresh_now(window, cx)))
            .on_action(
                cx.listener(|view, _: &ShowOverview, window, cx| view.show_row(0, window, cx)),
            )
            .on_action(cx.listener(|view, _: &ShowNodes, window, cx| view.show_row(1, window, cx)))
            .on_action(
                cx.listener(|view, _: &ShowNamespaces, window, cx| view.show_row(2, window, cx)),
            )
            .on_action(cx.listener(|view, _: &ShowEvents, window, cx| view.show_row(3, window, cx)))
            .on_action(cx.listener(|view, _: &ShowHealth, window, cx| view.show_row(4, window, cx)))
            .on_action(cx.listener(|view, _: &ShowEtcd, window, cx| view.show_row(6, window, cx)))
            .on_action(
                cx.listener(|view, _: &ShowSystemServices, window, cx| {
                    view.show_row(7, window, cx)
                }),
            )
            .on_action(
                cx.listener(|view, _: &ShowSecurity, window, cx| view.show_row(8, window, cx)),
            )
            .on_action(
                cx.listener(|view, _: &ShowLifecycle, window, cx| view.show_row(9, window, cx)),
            )
            .on_action(
                cx.listener(|view, _: &NextScreen, window, cx| view.adjacent_row(true, window, cx)),
            )
            .on_action(cx.listener(|view, _: &PreviousScreen, window, cx| {
                view.adjacent_row(false, window, cx)
            }))
            .on_action(cx.listener(|view, _: &PreviousContext, window, cx| {
                view.adjacent_context(false, window, cx)
            }))
            .on_action(cx.listener(|view, _: &NextContext, window, cx| {
                view.adjacent_context(true, window, cx)
            }))
            .on_action(cx.listener(|view, _: &GoToKind, window, cx| view.open_search(window, cx)))
            .child(self.render_title_bar(window, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(window, cx))
                    .child(div().flex_1().min_w_0().min_h_0().child(page)),
            )
            .child(self.render_status_bar(window, cx))
    }
}
