use super::*;
use crate::palette::palette;
use crate::ui::{self, dp};
use gpui_kit::component::{h_flex, v_flex};

const SCOPE_NOTE: &str = "Active alerts whose labels name this object, from the \
    cluster's alert read, which runs once a minute. Silenced alerts aren't read.";

impl DetailPane {
    /// The Alerts block of Overview, for a Pod, a workload or a Node. With
    /// `first`, only when there are alerts to show: they are what a short pane
    /// needs: on a Pod, after its containers. Without it, only when there are none to
    /// list (quiet, stale or unknown), after the object's own sections.
    pub(in crate::pane) fn alerts_section(
        &self,
        first: bool,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let body = self.alerts.body.as_ref()?;
        if matches!(body, Body::Lines { .. }) != first {
            return None;
        }
        let p = palette(cx);
        let note = |id: &'static str, text: SharedString, cx: &App| {
            div()
                .id(id)
                .test_support()
                .text_size(dp(12.))
                .text_color(palette(cx).muted)
                .child(text)
        };
        let body = match body {
            Body::Reading => {
                note("alerts-reading", "Reading alerts…".into(), cx).into_any_element()
            }
            Body::Unknown(why) => h_flex()
                .id("alerts-unknown")
                .test_support()
                .gap_2()
                .child(ui::tag(Tone::Unknown, None, "Unknown", cx))
                .child(div().min_w_0().child(why.clone()))
                .into_any_element(),
            Body::Quiet(text) => h_flex()
                .id("alerts-quiet")
                .test_support()
                .gap_2()
                .child(ui::tag(Tone::Good, None, "Quiet", cx))
                .child(div().min_w_0().child(text.clone()))
                .into_any_element(),
            Body::StaleQuiet(text) => h_flex()
                .id("alerts-stale")
                .test_support()
                .gap_2()
                .child(ui::tag(Tone::Unknown, None, "Stale", cx))
                .child(div().min_w_0().child(text.clone()))
                .into_any_element(),
            Body::Lines {
                lines,
                hidden,
                stale,
            } => v_flex()
                .gap_2()
                .when(*stale, |this| {
                    this.child(note(
                        "alerts-stale-note",
                        "The last read couldn't be repeated, so these may have ended.".into(),
                        cx,
                    ))
                })
                .children(lines.iter().map(|line| {
                    v_flex()
                        .id(line.id.clone())
                        .test_support()
                        .role(Role::Status)
                        .aria_label(line.aria.clone())
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(ui::tag(line.tone, None, line.severity, cx))
                                .child(div().min_w_0().child(line.title.clone()))
                                .child(
                                    div()
                                        .text_size(dp(12.))
                                        .text_color(p.muted)
                                        .child(line.meta.clone()),
                                ),
                        )
                        .children(line.text.clone().map(|text| {
                            div()
                                .pl(dp(8.))
                                .text_size(dp(12.))
                                .text_color(p.muted)
                                .child(text)
                        }))
                }))
                .when(*hidden > 0, |this| {
                    this.child(note(
                        "alerts-more",
                        format!("and {hidden} more on the Alerts page").into(),
                        cx,
                    ))
                })
                .into_any_element(),
        };
        Some(
            v_flex()
                .id("alerts-section")
                .test_support()
                .gap_3()
                .child(ui::caption("Alerts", cx))
                .child(body)
                .child(note("alerts-scope", SCOPE_NOTE.into(), cx))
                .into_any_element(),
        )
    }
}
