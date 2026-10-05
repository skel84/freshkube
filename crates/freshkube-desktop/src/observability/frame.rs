//! The Observability page frame and destination composition.
use super::*;

impl Render for ObservabilityPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_app_select(window, cx);
        self.sync_namespace_select(window, cx);
        let p = palette(cx);
        let unavailable =
            !self.fixture && (self.live.provider.is_none() || self.live.source.is_none());
        let content = if unavailable {
            self.render_unavailable(cx)
        } else if let Some(placeholder) = self.read_placeholder(cx) {
            placeholder
        } else {
            match self.destination {
                Destination::Applications => self.render_applications(window, cx),
                Destination::ServiceMap => self.render_map(window, cx),
                Destination::Application => self.render_report(window, cx),
                Destination::Incidents => self.render_incidents(window, cx),
                Destination::Traces if !self.fixture => self.render_live_traces(window, cx),
                Destination::Profiling if !self.fixture => self.render_live_profiling(cx),
                _ if !self.fixture => self.render_limited(cx),
                Destination::Deployments => self.render_deployments(window, cx),
                Destination::Profiling => self.render_profiling(cx),
                Destination::Traces => self.render_traces(window, cx),
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
                    .track_scroll(&self.scroll)
                    .child(
                        freshkube_ui::page::page("obs-frame")
                            .h_auto()
                            .flex_none()
                            .child(match self.destination {
                                Destination::Applications => self.applications_header(window, cx),
                                Destination::Incidents => self.incidents_header(window, cx),
                                _ => self
                                    .source_controls(self.page_header(window), cx)
                                    .render(cx),
                            })
                            .when(!self.fixture && !unavailable, |this| {
                                this.when(self.settings_open, |this| {
                                    this.child(self.render_connection(cx))
                                })
                                .child(self.render_read_state(cx))
                            })
                            .child(content),
                    ),
            )
    }
}
