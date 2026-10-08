//! The selected hop in the shared Inspector: its kind, name and state, the
//! fields its tool reports, how it joins the hop before, and where it
//! opens. A Stage's gate rows show the Stage, with its three gates apart:
//! whether the Freight is eligible, how it is promoted, and the
//! verification after.

use freshkube_ui::inspector::Inspector;
use freshkube_ui::palette::palette;
use freshkube_ui::ui::{self, MONO_FONT, Tone, dp};
use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable as _, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Div, FontWeight, SharedString, TestSupportExt, div};

use super::ChangeStory;
use super::trail::{Action, Check, Eligible, HopDetail, Link, Opens, Promotion, Shows, Stage};

/// The width of a field's label.
const LABEL_WIDTH: f32 = 112.;

impl ChangeStory {
    /// The Inspector for the selected hop, or `None` without one.
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected?;
        let hop = self.trail.hops.iter().find(|hop| hop.key == selected)?;
        let inspector = Inspector::new("change-trail-detail");
        let inspector = match &hop.shows {
            Shows::Hop(detail) => self.render_hop(inspector, hop.tone, detail, cx),
            Shows::Stage(stage) => self.render_stage(inspector, &self.trail.stages[*stage], cx),
        };
        Some(inspector.render(cx).into_any_element())
    }

    fn render_hop(
        &self,
        inspector: Inspector,
        tone: Tone,
        hop: &HopDetail,
        cx: &mut Context<Self>,
    ) -> Inspector {
        let p = palette(cx);
        let fields = (hop.fields.iter()).map(|(label, value, mono)| {
            let value = div().min_w_0().child(*value);
            field(
                label,
                if *mono {
                    value.font_family(MONO_FONT)
                } else {
                    value
                },
                cx,
            )
        });
        let link = match (&hop.link, hop.unlinked) {
            (Some(link), _) => Some(render_link(link, cx)),
            (None, Some(unlinked)) => {
                Some(section("Link", cx).child(div().text_color(p.ink_2).child(unlinked)))
            }
            (None, None) => None,
        };
        inspector
            .heading(heading(hop.kind, hop.title, tone, hop.state, cx))
            .banner(
                hop.notice
                    .map(|(tone, lead, body)| ui::banner(tone, Some(lead.into()), body, None, cx)),
            )
            .child(v_flex().gap(dp(7.)).children(fields))
            .children(link)
            .footer(Some(self.render_actions(&hop.actions, cx)))
    }

    fn render_stage(
        &self,
        inspector: Inspector,
        stage: &Stage,
        cx: &mut Context<Self>,
    ) -> Inspector {
        let p = palette(cx);
        let failed = (stage.verification.iter()).find(|(tone, _, _)| *tone == Tone::Crit);
        let banner = failed.map(|(_, name, found)| {
            ui::banner(
                Tone::Crit,
                Some("Verification failed.".into()),
                format!(
                    "{name}: {found}. wonky-otter still runs on {}.",
                    stage.cluster
                ),
                None,
                cx,
            )
        });
        let freight = match stage.running {
            Some(running) => format!("{running} runs; wonky-otter isn't promoted yet"),
            None => "wonky-otter".to_owned(),
        };
        let eligible = match &stage.eligible {
            Eligible::Warehouse { at } => gate(
                Tone::Good,
                format!("From the Warehouse at {at}: the first Stage takes new Freight."),
                None,
                cx,
            ),
            Eligible::Verified {
                upstream,
                at,
                checks,
            } => gate(
                Tone::Good,
                format!("Verified in {upstream} at {at}: {checks} passed there."),
                None,
                cx,
            ),
            Eligible::Approved {
                by,
                at,
                past,
                upstream_then,
            } => gate(
                Tone::Good,
                format!("Approved by {by} at {at}, not verified in {past} at the time."),
                Some(format!(
                    "Approved by hand for this Stage, past {past}; {upstream_then}."
                )),
                cx,
            ),
        };
        let promotion = match stage.promotion {
            Promotion::Automatic { at } => gate(
                Tone::Good,
                format!("Automatic: auto-promotion is on. Promoted at {at}."),
                None,
                cx,
            ),
            Promotion::Waiting => gate(
                Tone::Unknown,
                "Waiting for promotion: auto-promotion off.".to_owned(),
                Some("Nothing has failed: it waits for someone who may promote.".to_owned()),
                cx,
            ),
        };
        inspector
            .heading(heading(
                "Kargo Stage",
                stage.name,
                stage.tone,
                stage.state,
                cx,
            ))
            .banner(banner)
            .child(
                v_flex()
                    .gap(dp(7.))
                    .child(field("Project", div().child("checkout"), cx))
                    .child(field(
                        "Deploys to",
                        div().font_family(MONO_FONT).child(stage.cluster),
                        cx,
                    ))
                    .child(field("Freight", div().child(freight), cx)),
            )
            .child(section("Eligible", cx).child(eligible).child(who(
                "May approve",
                stage.approvers,
                cx,
            )))
            .child(
                section("Promotion", cx)
                    .child(promotion)
                    .child(who("May promote", stage.promoters, cx))
                    .child(checks("change-trail-steps", &stage.steps, cx)),
            )
            .child(
                section("Verification", cx)
                    .child(
                        div()
                            .text_color(p.muted)
                            .child("Each AnalysisRun after the promotion."),
                    )
                    .child(checks("change-trail-analyses", &stage.verification, cx)),
            )
            .footer(Some(self.render_actions(
                &[
                    Action {
                        opens: Opens::Browser {
                            label: "Open in Kargo",
                        },
                        disabled: None,
                    },
                    Action {
                        opens: Opens::Resources {
                            what: "the Stage",
                            cluster: "core-fra",
                        },
                        disabled: None,
                    },
                ],
                cx,
            )))
    }

    /// The hop's buttons, and what the last one pressed would have opened.
    fn render_actions(&self, actions: &[Action], cx: &mut Context<Self>) -> Div {
        let buttons = actions.iter().enumerate().map(|(ix, action)| {
            let opens = action.opens;
            let button = Button::new(SharedString::from(format!("change-trail-action-{ix}")))
                .outline()
                .xsmall()
                .label(opens.label())
                .on_click(cx.listener(move |this, _, _, cx| this.press(opens.would(), cx)));
            match action.disabled {
                Some(why) => button.disabled(true).tooltip(why),
                None => button,
            }
        });
        v_flex()
            .gap(dp(6.))
            .child(h_flex().gap(dp(6.)).flex_wrap().children(buttons))
            .children(self.opened.clone().map(|opened| {
                div()
                    .id("change-trail-opened")
                    .test_support()
                    .text_size(dp(12.))
                    .text_color(palette(cx).muted)
                    .child(opened)
            }))
    }
}

/// The kind as a caption over the name in monospace and its state.
fn heading(
    kind: &str,
    title: &'static str,
    tone: Tone,
    state: &'static str,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .id("change-trail-detail-title")
        .test_support()
        .aria_label(SharedString::from(format!("{kind} {title}: {state}")))
        .gap(dp(4.))
        .min_w_0()
        .child(ui::caption(kind, cx))
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .min_w_0()
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(dp(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title),
                )
                .child(ui::tag(tone, None, state, cx)),
        )
}

/// A labelled value.
fn field(label: &str, value: Div, cx: &App) -> Div {
    h_flex()
        .gap_3()
        .items_start()
        .text_size(dp(12.))
        .child(
            div()
                .w(dp(LABEL_WIDTH))
                .flex_none()
                .text_color(palette(cx).muted)
                .child(label.to_owned()),
        )
        // Within the row, so a long value wraps in a narrow Inspector.
        .child(value.flex_1().min_w_0().max_w_full())
}

/// A section of the body under its caption.
fn section(title: &str, cx: &App) -> Div {
    v_flex()
        .gap(dp(7.))
        .text_size(dp(12.))
        .child(ui::caption(title, cx))
}

/// How the hop joins the hop before: its confidence, what each side
/// reports, and why.
fn render_link(link: &Link, cx: &App) -> Div {
    let p = palette(cx);
    section("Link", cx)
        .child(
            h_flex()
                .id("change-trail-detail-link")
                .test_support()
                .gap(dp(6.))
                .children(
                    link.confidence
                        .tone()
                        .and_then(|tone| ui::status_glyph(tone, cx)),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(link.confidence.word()),
                )
                .child(div().text_color(p.muted).child(format!("by {}", link.by))),
        )
        .child(field(
            link.before,
            div().font_family(MONO_FONT).child(link.before_says),
            cx,
        ))
        .child(field("This hop", div().child(link.here_says), cx))
        .child(div().text_color(p.ink_2).child(link.why))
}

/// A gate's state: its glyph and what it says, with a note under it.
fn gate(tone: Tone, says: String, note: Option<String>, cx: &App) -> Div {
    h_flex()
        .gap(dp(8.))
        .items_start()
        .child(
            div()
                .flex_none()
                .h(dp(16.))
                .flex()
                .items_center()
                .children(ui::status_glyph(tone, cx)),
        )
        .child(
            v_flex()
                .gap(dp(2.))
                .min_w_0()
                .child(says)
                .children(note.map(|note| div().text_color(palette(cx).muted).child(note))),
        )
}

/// Who may act on the gate.
fn who(label: &str, who: &'static str, cx: &App) -> Div {
    field(label, div().font_family(MONO_FONT).child(who), cx)
}

/// Checks, one a line: a glyph, the name in monospace, what it found.
fn checks(id: &'static str, checks: &[Check], cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(id)
        .test_support()
        .gap(dp(5.))
        .children(checks.iter().map(|(tone, name, found)| {
            h_flex()
                .gap(dp(8.))
                .child(
                    div()
                        .flex_none()
                        .w(dp(10.))
                        .children(ui::status_glyph(*tone, cx)),
                )
                .child(
                    div()
                        .w(dp(150.))
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO_FONT)
                        .child(*name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(match tone {
                            Tone::Crit => p.crit_ink,
                            _ => p.ink_2,
                        })
                        .child(*found),
                )
        }))
        .into_any_element()
}
