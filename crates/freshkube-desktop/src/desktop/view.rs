use super::*;
use crate::ui::dp;
use freshkube_probe::first_frame::FirstFrame;

impl Render for Pilot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit("shell");
        let _span = crate::perf::span("shell.render");
        self.fps
            .read(cx)
            .frame_started(cx.background_executor().now());
        // `FRESHKUBE_FIRST_FRAME=1` prints when the window first draws,
        // whatever page it opens on and in release builds too: the release
        // smoke check waits for this line.
        static WINDOW: FirstFrame = FirstFrame::new("window");
        if WINDOW.pending() {
            window.on_next_frame(|_, _| WINDOW.mark());
        }
        self.layout_chrome(window);
        let column = self.column_width(window);
        if column.is_none() {
            // A kind outside every column has no row to reveal.
            self.column_reveal = None;
        }
        let page = match self.page {
            Page::Observability => self
                .observability
                .clone()
                .cached(cached_page_style())
                .into_any_element(),
            // Not cached: a cached view that draws again makes every cached
            // view inside it draw again, so a cursor moving over one chart
            // would redraw all the panels. Drawn with the shell, the page
            // only places its cached panels, and only the one under the
            // pointer draws again.
            Page::Monitoring => self.monitoring.clone().into_any_element(),
            Page::Settings => self
                .settings_page
                .clone()
                .cached(cached_page_style())
                .into_any_element(),
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
            .on_action(
                cx.listener(|view, _: &ToggleColumn, window, cx| view.toggle_column(window, cx)),
            )
            .on_action(cx.listener(|view, _: &dock::NextDockTab, window, cx| {
                if !view.dock.read(cx).has_tabs() {
                    return cx.propagate();
                }
                view.dock.update(cx, |dock, cx| dock.step(1, window, cx))
            }))
            .on_action(cx.listener(|view, _: &dock::PreviousDockTab, window, cx| {
                if !view.dock.read(cx).has_tabs() {
                    return cx.propagate();
                }
                view.dock.update(cx, |dock, cx| dock.step(-1, window, cx))
            }))
            .on_action(cx.listener(|view, _: &dock::MinimizeDock, window, cx| {
                // Without tabs there is no dock: the key goes on.
                if !view.dock.read(cx).has_tabs() {
                    return cx.propagate();
                }
                view.dock
                    .update(cx, |dock, cx| dock.set_open(false, window, cx))
            }))
            .child(
                self.chrome.header.clone().cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(dp(freshkube_ui::page::APP_HEADER_HEIGHT))
                        .flex_none(),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(
                        self.chrome.rail.clone().cached(
                            StyleRefinement::default()
                                .w(dp(RAIL_WIDTH))
                                .h_full()
                                .flex_none(),
                        ),
                    )
                    .children(column.map(|width| {
                        self.chrome
                            .column
                            .clone()
                            .cached(StyleRefinement::default().w(dp(width)).h_full().flex_none())
                    }))
                    .child(self.render_page_cell(page, window, cx)),
            )
            // Drawn by the shell, so it keeps up with the pages it reads.
            .child(self.render_status_bar(window, cx))
            // After the page, so it has drawn its rows when these render.
            .children(self.page_loading_motion(cx))
            .children(self.page_flash(cx))
    }
}

impl Pilot {
    /// The flash over the shown page's changed rows, mounted beside the
    /// page and the cached chrome as the loading motion is. Only
    /// Resources lists flash.
    pub(super) fn page_flash(
        &self,
        cx: &App,
    ) -> Option<Entity<freshkube_ui::table::FlashLayer<crate::resources::model::ResourceIdentity>>>
    {
        match self.page {
            Page::Resources => self.resources.read(cx).flash_layer(),
            _ => None,
        }
    }

    /// The motion over the shown page's loading rows. The page owns it and
    /// its rows decide when it moves; the shell mounts it beside the
    /// page and the cached chrome, so its frames redraw neither. It is
    /// mounted only while the shown page's table draws loading rows: not
    /// for a hidden page, and not once a state, the cards or an expanded
    /// pane replace the table, since only a drawn table stills it.
    pub(super) fn page_loading_motion(
        &self,
        cx: &App,
    ) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        match self.page {
            Page::Resources => self.resources.read(cx).loading_motion(),
            Page::Observability => self.observability.read(cx).loading_motion(cx),
            _ if self.kubernetes_only.is_some()
                && !matches!(self.page, Page::Overview | Page::Health | Page::Nodes) =>
            {
                None
            }
            Page::Nodes => {
                let workspace = &self.node_workspace;
                let table = workspace.view == NodeView::Table
                    && !(workspace.open && workspace.expanded)
                    && freshkube_ui::table::TableSource::loading(self).is_some();
                if table {
                    return Some(workspace.loading_motion().clone());
                }
                // The open node's tab: a node screen's table loading.
                self.node_workspace
                    .open
                    .then(|| self.active_screen())
                    .flatten()
                    .and_then(|screen| screen.loading_motion(cx))
            }
            Page::SystemServices => {
                let services = self.system_services.read(cx);
                freshkube_ui::table::TableSource::loading(services)
                    .is_some()
                    .then(|| services.loading_motion().clone())
            }
            // Any other screen's table: etcd, Security, Health, Lifecycle.
            _ => self
                .active_screen()
                .and_then(|screen| screen.loading_motion(cx)),
        }
    }

    /// The page above the dock. The dock spans the page cell and pushes
    /// the page up; the page reads how much it takes when it lays out.
    fn render_page_cell(
        &mut self,
        page: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dock = self.dock.read(cx).shown_height(window, cx);
        freshkube_ui::page::set_below(
            dock.map(|height| height / crate::ui::dp_px(1., window))
                .unwrap_or(0.),
        );
        v_flex()
            .id("page-cell")
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(div().flex_1().min_w_0().min_h_0().child(page))
            .children(dock.map(|height| {
                self.dock
                    .clone()
                    .cached(StyleRefinement::default().w_full().flex_none().h(height))
            }))
    }
}
