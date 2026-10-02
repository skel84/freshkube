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
                && !matches!(self.page, Page::Workloads | Page::Nodes) =>
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
            Page::Services => self
                .services_page
                .clone()
                .cached(cached_page_style())
                .into_any_element(),
            Page::Logs => self.render_logs_page(),
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
            .on_action(cx.listener(|view, _: &ShowOverview, window, cx| {
                view.navigate_from_keyboard(Page::Overview, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowServices, window, cx| {
                view.navigate_from_keyboard(Page::Services, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowLogs, window, cx| {
                view.navigate_from_keyboard(Page::Logs, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowProcesses, window, cx| {
                view.navigate_from_keyboard(Page::Processes, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowStorage, window, cx| {
                view.navigate_from_keyboard(Page::Storage, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowNetwork, window, cx| {
                view.navigate_from_keyboard(Page::Network, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowDiagnostics, window, cx| {
                view.navigate_from_keyboard(Page::Diagnostics, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowEtcd, window, cx| {
                view.navigate_from_keyboard(Page::Etcd, window, cx)
            }))
            .on_action(cx.listener(|view, _: &ShowWorkloads, window, cx| {
                view.navigate_from_keyboard(Page::Workloads, window, cx)
            }))
            .on_action(cx.listener(|view, _: &NextScreen, window, cx| {
                view.navigate_from_keyboard(view.adjacent_page(true), window, cx)
            }))
            .on_action(cx.listener(|view, _: &PreviousScreen, window, cx| {
                view.navigate_from_keyboard(view.adjacent_page(false), window, cx)
            }))
            .on_action(cx.listener(|view, _: &PreviousContext, window, cx| {
                view.adjacent_context(false, window, cx)
            }))
            .on_action(cx.listener(|view, _: &NextContext, window, cx| {
                view.adjacent_context(true, window, cx)
            }))
            .on_action(
                cx.listener(|view, _: &GoToKind, window, cx| view.open_kind_switcher(window, cx)),
            )
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
