//! The Observability page frame and destination composition.
use super::*;

impl Render for ObservabilityPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_app_select(window, cx);
        self.sync_namespace_select(window, cx);
        self.sync_app_tables(cx);
        self.sync_app_charts(cx);
        self.loading.show(self.first_read());
        let p = palette(cx);
        let unavailable =
            !self.fixture && (self.live.provider.is_none() || self.live.source.is_none());
        // Applications is a table page, edge to edge, and so are Incidents
        // and Traces, whose tables share the page with an inspector. The
        // other destinations are pages of cards and canvases, which keep the
        // padded frame.
        let edge = matches!(
            self.destination,
            Destination::Applications | Destination::Incidents | Destination::Traces
        );
        // An inspector's split fills the window, as tall as the scroll area;
        // when a short window scrolls the frame, the split has its own height.
        let fills = matches!(
            self.destination,
            Destination::Incidents | Destination::Traces
        ) && !self.inspector_scrolls(window);
        let state = |state: AnyElement| {
            if edge {
                freshkube_ui::page::inset().child(state).into_any_element()
            } else {
                state
            }
        };
        let content = if unavailable {
            state(self.render_unavailable(cx))
        } else if let Some(placeholder) = self.read_placeholder(cx) {
            state(placeholder)
        } else {
            match self.destination {
                Destination::Applications => self.render_applications(window, cx),
                Destination::ServiceMap => self.render_map(window, cx),
                Destination::Application => self.render_report(window, cx),
                Destination::Incidents => self.render_incidents(window, cx),
                Destination::Traces => self.render_live_traces(window, cx),
                Destination::Profiling if !self.fixture => self.render_live_profiling(cx),
                _ if !self.fixture => self.render_limited(cx),
                Destination::Deployments => self.render_deployments(window, cx),
                Destination::Profiling => self.render_profiling(cx),
            }
        };
        let header = match self.destination {
            Destination::Applications => self.applications_header(window, cx),
            Destination::Incidents => self.incidents_header(window, cx),
            Destination::Traces => self.traces_header(window, cx),
            _ => self
                .source_controls(self.page_header(cx), cx)
                .render(window, cx),
        };
        let frame = if edge {
            freshkube_ui::page::page("obs-frame")
        } else {
            freshkube_ui::page::padded("obs-frame")
        };
        // On the edge-to-edge frame the header is a toolbar, and what isn't
        // the table sits in an inset.
        let header = if edge {
            freshkube_ui::page::toolbar(cx)
                .child(header)
                .into_any_element()
        } else {
            header.into_any_element()
        };
        let inset = |element: AnyElement| {
            if edge {
                freshkube_ui::page::inset()
                    .child(element)
                    .into_any_element()
            } else {
                element
            }
        };
        v_flex()
            .id("observability-page")
            .test_support()
            .track_focus(&self.focus)
            .key_context("Observability")
            .size_full()
            .min_w_0()
            .min_h_0()
            .text_size(dp(13.))
            .text_color(p.ink)
            .child(
                div()
                    .id("obs-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.scroll)
                    .child(
                        frame
                            .flex_none()
                            .when(!fills, |this| this.h_auto())
                            .child(header)
                            .when(!self.fixture && !unavailable, |this| {
                                this.when(self.settings_open, |this| {
                                    this.child(inset(self.render_connection(cx)))
                                })
                                .children(self.render_read_state(cx).map(inset))
                            })
                            .child(content),
                    ),
            )
    }
}
