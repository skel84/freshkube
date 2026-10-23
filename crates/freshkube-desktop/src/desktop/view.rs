use super::*;
use freshkube_probe::first_frame::FirstFrame;

impl Render for Pilot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit("shell");
        let _span = crate::perf::span("shell.render");
        self.chrome.frame_started(cx);
        // `FRESHKUBE_FIRST_FRAME=1` prints when the window first draws,
        // whatever page it opens on and in release builds too: the release
        // smoke check waits for this line.
        static WINDOW: FirstFrame = FirstFrame::new("window");
        if WINDOW.pending() {
            window.on_next_frame(|_, _| WINDOW.mark());
        }
        // A tab whose cluster entry is gone shows no cluster, so no column of
        // the cluster that happens to be open stands behind its page.
        let column = self
            .navigation
            .lay_out(window)
            .filter(|_| !self.tabs.missing_shown);
        let page = self.render_page(cx);
        let followed = self.is_followed(cx);
        let root = v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .key_context(freshkube_ui::page::SHELL_CONTEXT)
            .track_focus(&self.focus);
        let root = self.navigation.listen(root, cx);
        let root = self.chrome.listen(root, cx);
        let root = self.listen_tabs(root, cx);
        self.listen(root, cx)
            // While the page follows an agent, any input of the person
            // outside the dock takes it back (`agent_follow.rs`).
            .when(followed, |this| {
                this.capture_any_mouse_down(cx.listener(
                    |pilot, event: &MouseDownEvent, window, cx| {
                        pilot.follow_input(Some(event.position), window, cx)
                    },
                ))
                .child(self.follow_scroll_guard(cx))
            })
            .child(self.chrome.parts.header())
            .children(
                self.tabs
                    .shows()
                    .then(|| self.tabs.strip.clone().cached(tab_strip_style())),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.chrome.parts.rail())
                    .children(column.map(|width| self.chrome.parts.column(width)))
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
    /// Adds the actions only the shell can answer to its root: Refresh,
    /// which reads the session's sources and the shown page again; the
    /// session's previous and next context; Go to kind, the search palette
    /// the shell holds; and the dock's tabs, which can be open on any page.
    fn listen(&self, root: Div, cx: &mut Context<Self>) -> Div {
        root.on_action(cx.listener(|view, _: &Refresh, window, cx| view.refresh_now(window, cx)))
            .on_action(cx.listener(|view, _: &PreviousContext, window, cx| {
                view.adjacent_context(false, window, cx)
            }))
            .on_action(cx.listener(|view, _: &NextContext, window, cx| {
                view.adjacent_context(true, window, cx)
            }))
            .on_action(cx.listener(|view, _: &GoToKind, window, cx| view.open_search(window, cx)))
            .on_action(cx.listener(|view, _: &CopyLink, window, cx| view.copy_link(window, cx)))
            .on_action(cx.listener(|view, _: &dock::OpenAgent, window, cx| {
                view.pages
                    .dock
                    .update(cx, |dock, cx| dock.open_agent(window, cx))
            }))
            // Each handler is there only while it can act, so the menus grey
            // it otherwise and its key goes on to whatever else binds it.
            .when(self.pages.dock.read(cx).has_tabs(), |this| {
                this.on_action(cx.listener(|view, _: &dock::NextDockTab, window, cx| {
                    view.pages
                        .dock
                        .update(cx, |dock, cx| dock.step(1, window, cx))
                }))
                .on_action(cx.listener(|view, _: &dock::PreviousDockTab, window, cx| {
                    view.pages
                        .dock
                        .update(cx, |dock, cx| dock.step(-1, window, cx))
                }))
                .on_action(cx.listener(
                    |view, _: &dock::MinimizeDock, window, cx| {
                        view.pages.dock.update(cx, |dock, cx| {
                            let open = dock.is_open();
                            dock.set_open(!open, window, cx)
                        })
                    },
                ))
            })
    }

    /// The page shown, drawn beside the cached chrome.
    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.navigation.page {
            // A tab whose cluster entry is gone shows no page at all, never
            // the cluster that happens to be open (#878).
            _ if self.tabs.missing_shown => self.render_choose_cluster(cx),
            // Not cached: a cached view that draws again makes every cached
            // view inside it draw again, so a cursor moving over one chart
            // would redraw all the panels. Drawn with the shell, the page
            // only places its cached panels, and only the one under the
            // pointer draws again.
            Page::Monitoring => self.pages.monitoring.clone().into_any_element(),
            page if !self.mode().has_talos() && page.needs_talos() => {
                self.render_needs_talosconfig(cx)
            }
            // Pages redraw when their own state changes, not with the shell.
            page => self
                .pages
                .map
                .get(&page.id())
                .map(|page| page.view().cached(cached_page_style()).into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        }
    }

    /// The flash over the shown page's changed rows, mounted beside the
    /// page and the cached chrome as the loading motion is. Only
    /// Resources lists flash.
    pub(super) fn page_flash(
        &self,
        cx: &App,
    ) -> Option<Entity<freshkube_ui::table::FlashLayer<crate::resources::model::ResourceIdentity>>>
    {
        match self.navigation.page {
            Page::Resources => self.pages.resources.read(cx).flash_layer(),
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
        match self.navigation.page {
            Page::Resources => self.pages.resources.read(cx).loading_motion(),
            Page::Observability => self.pages.observability.read(cx).loading_motion(cx),
            Page::Applications => self.pages.applications.read(cx).loading_motion(cx),
            page if !self.mode().has_talos() && page.needs_talos() => None,
            // Its table's, or the open node's tab's: a node screen's.
            Page::Nodes => self.pages.map.get(&PageId::Nodes)?.loading_motion(cx),
            Page::SystemServices => {
                let services = self.pages.system_services.read(cx);
                freshkube_ui::table::TableSource::loading(services)
                    .is_some()
                    .then(|| services.loading_motion().clone())
            }
            // Any other screen's table: etcd, Security, Health, Lifecycle.
            _ => self
                .active_screen(cx)
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
        let dock = self.pages.dock.read(cx).shown_height(window, cx);
        let strip = self.follow_strip(cx);
        let accent = crate::palette::palette(cx).accent;
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
            // The strip, beside the cached page, so its hairline's frames
            // draw nothing else.
            .children(strip.clone())
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(page)
                    // A 2 px frame around the followed page that takes no
                    // room and no pointer.
                    .children(strip.map(|_| {
                        div()
                            .id("follow-frame")
                            .test_support()
                            .absolute()
                            .inset_0()
                            .border_2()
                            .border_color(accent)
                    })),
            )
            .children(dock.map(|height| {
                self.pages
                    .dock
                    .clone()
                    .cached(StyleRefinement::default().w_full().flex_none().h(height))
            }))
    }
}

/// The strip's cached frame: the window's width, one bar high.
fn tab_strip_style() -> StyleRefinement {
    StyleRefinement::default()
        .w_full()
        .h(crate::ui::dp(freshkube_ui::tab_strip::BAR_HEIGHT))
        .flex_none()
}

impl Pilot {
    /// Hears the wheel anywhere in the window while the page follows an
    /// agent: a scrolling list stops the wheel from reaching its parents, so
    /// the capture phase is the only place that sees all of it.
    fn follow_scroll_guard(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pilot = cx.entity().downgrade();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let pilot = pilot.clone();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        _ = pilot.update(cx, |pilot, cx| {
                            pilot.follow_input(Some(event.position), window, cx)
                        });
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}
