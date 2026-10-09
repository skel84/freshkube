//! The selected hop in the shared Inspector: its kind, name and state, the
//! fields its tool reports, how it joins the hop before, and where it
//! leads. A gate row shows its Stage, with its three gates apart: whether
//! the Freight is eligible, how it is promoted, and the verification after.
//!
//! Every action leads out and changes nothing. One into Resources names
//! its cluster, and is greyed out with why for a cluster that isn't open;
//! the tools' own pages and a step's logs aren't linked yet, and say so.
use super::*;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_core::delivery::change::{
    Action, Check, Eligible, Field, HopDetail, LinkDetail, Promotion, Stage, Target, Value,
};
use freshkube_ui::inspector::Inspector;
use freshkube_ui::palette::palette;
use gpui_kit::component::{Disableable as _, Sizable, button::Button, h_flex, v_flex};

/// The width of a field's label.
const LABEL_WIDTH: f32 = 112.;

/// Why a tool's own page doesn't open yet: its address isn't read.
const NOT_LINKED: &str = "Not linked yet: the tool's address isn't read";
/// Why a step's log doesn't open yet.
const NO_LOGS: &str = "Not linked yet: a step's log isn't read";

impl ChangePage {
    /// The Inspector for the selected hop, or `None` without one.
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let selected = self.selected.as_ref()?;
        let hop = self.change.hop(selected)?;
        let inspector = Inspector::new("change-detail");
        let inspector = match &hop.shows {
            Shows::Hop(detail) => self.render_hop(inspector, tone(hop.state), detail, cx),
            Shows::Stage(stage) => self.render_stage(inspector, &self.change.stages[*stage], cx),
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
        let fields = hop.fields.iter().map(|field| render_field(field, cx));
        let link = match (&hop.link, &hop.unlinked) {
            (Some(link), _) => Some(render_link(link, cx)),
            (None, Some(unlinked)) => Some(
                section("Link", cx).child(
                    div()
                        .id("change-detail-link")
                        .test_support()
                        .text_color(p.ink_2)
                        .child(unlinked.clone()),
                ),
            ),
            (None, None) => None,
        };
        inspector
            .heading(heading(&hop.kind, &hop.title, tone, &hop.state, cx))
            .banner(hop.notice.as_ref().map(|notice| {
                ui::banner(
                    super::tone(notice.state),
                    Some(notice.lead.clone().into()),
                    notice.body.clone(),
                    None,
                    cx,
                )
            }))
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
        let freight = &self.change.freight;
        let failed = stage
            .verification
            .iter()
            .find(|check| check.state == HealthIndicator::Error);
        let banner = failed.map(|check| {
            ui::banner(
                Tone::Crit,
                Some("Verification failed.".into()),
                format!(
                    "{}: {}. {freight} still runs on {}.",
                    check.name, check.found, stage.cluster
                ),
                None,
                cx,
            )
        });
        let running = match &stage.running {
            Some(running) => format!("{running} runs; {freight} isn't promoted yet"),
            None => freight.clone(),
        };
        let eligible = match &stage.eligible {
            Eligible::Warehouse { at } => gate(
                Tone::Good,
                format!(
                    "From the Warehouse at {}: the first Stage takes new Freight.",
                    clock(*at)
                ),
                None,
                cx,
            ),
            Eligible::Verified {
                upstream,
                at,
                checks,
            } => gate(
                Tone::Good,
                format!(
                    "Verified in {upstream} at {}: {checks} passed there.",
                    clock(*at)
                ),
                None,
                cx,
            ),
            Eligible::Approved {
                by,
                at,
                past,
                upstream_verified,
            } => gate(
                Tone::Info,
                format!(
                    "Approved by {by} at {}, not verified in {past} at the time.",
                    clock(*at)
                ),
                Some(match upstream_verified {
                    Some(then) => format!(
                        "Approved by hand for this Stage, past {past}; {past} verified it \
                         later, at {}.",
                        clock(*then)
                    ),
                    None => format!(
                        "Approved by hand for this Stage, past {past}, which hasn't verified \
                         it."
                    ),
                }),
                cx,
            ),
        };
        let promotion = match stage.promotion {
            Promotion::Automatic { at } => gate(
                Tone::Good,
                format!(
                    "Automatic: auto-promotion is on. Promoted at {}.",
                    clock(at)
                ),
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
        let open = Action::new(Target::Resource {
            what: "the Stage".into(),
            object: stage.object.clone(),
        });
        inspector
            .heading(heading(
                "Kargo Stage",
                &stage.name,
                tone(stage.state),
                &stage.words,
                cx,
            ))
            .banner(banner)
            .child(
                v_flex()
                    .gap(dp(7.))
                    .child(field(
                        "Project",
                        div().child(self.change.project.clone()),
                        cx,
                    ))
                    .child(field(
                        "Deploys to",
                        div().font_family(MONO_FONT).child(stage.cluster.clone()),
                        cx,
                    ))
                    .child(field("Freight", div().child(running), cx)),
            )
            .child(
                section("Eligible", cx)
                    .id("change-detail-eligible")
                    .test_support()
                    .child(eligible)
                    .child(who("May approve", &stage.approvers, cx)),
            )
            .child(
                section("Promotion", cx)
                    .id("change-detail-promotion")
                    .test_support()
                    .child(promotion)
                    .child(who("May promote", &stage.promoters, cx))
                    .child(checks("change-detail-steps", &stage.steps, cx)),
            )
            .child(
                section("Verification", cx)
                    .id("change-detail-verification")
                    .test_support()
                    .child(
                        div()
                            .text_color(p.muted)
                            .child("Each AnalysisRun after the promotion."),
                    )
                    .child(checks("change-detail-analyses", &stage.verification, cx)),
            )
            .footer(Some(self.render_actions(&[open], cx)))
    }

    /// The hop's buttons: into Resources on its cluster, greyed out with
    /// why where it can't lead yet.
    fn render_actions(&self, actions: &[Action], cx: &mut Context<Self>) -> AnyElement {
        let buttons = actions.iter().enumerate().map(|(ix, action)| {
            let button = Button::new(SharedString::from(format!("change-action-{ix}")))
                .outline()
                .xsmall();
            match &action.target {
                Target::Resource { what, object } => {
                    let (_, closed) = self.object_link(object);
                    let object = object.clone();
                    let why = action.disabled.clone().map(SharedString::from).or(closed);
                    let button = button
                        .label(format!("Open {what} in Resources · {}", object.cluster))
                        .on_click(cx.listener(move |this, _, _, cx| this.open(&object, cx)));
                    match why {
                        Some(why) => button.disabled(true).tooltip(why),
                        None => button,
                    }
                }
                Target::Browser { label } => button
                    .label(format!("{label} ↗"))
                    .disabled(true)
                    .tooltip(action.disabled.clone().unwrap_or(NOT_LINKED.into())),
                Target::Logs { .. } => button
                    .label("Logs")
                    .disabled(true)
                    .tooltip(action.disabled.clone().unwrap_or(NO_LOGS.into())),
            }
        });
        h_flex()
            .id("change-detail-actions")
            .test_support()
            .gap(dp(6.))
            .flex_wrap()
            .children(buttons)
            .into_any_element()
    }
}

/// The kind as a caption over the name in monospace and its state.
fn heading(kind: &str, title: &str, tone: Tone, state: &str, cx: &App) -> impl IntoElement {
    v_flex()
        .id("change-detail-title")
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
                        .text_size(dp(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title.to_owned()),
                )
                .child(ui::tag(tone, None, state.to_owned(), cx)),
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

fn render_field(field_: &Field, cx: &App) -> Div {
    let value = match &field_.value {
        Value::Text(text) => div().child(text.clone()),
        Value::Mono(text) => div().font_family(MONO_FONT).child(text.clone()),
        Value::At(at) => div().child(clock(*at)),
    };
    field(&field_.label, value, cx)
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
fn render_link(link: &LinkDetail, cx: &App) -> Div {
    let p = palette(cx);
    section("Link", cx)
        .child(
            h_flex()
                .id("change-detail-link")
                .test_support()
                .aria_label(SharedString::from(format!(
                    "{} by {}",
                    link_word(link.confidence),
                    link.by
                )))
                .gap(dp(6.))
                .children(link_glyph(link.confidence).and_then(|tone| ui::status_glyph(tone, cx)))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(link_word(link.confidence)),
                )
                .child(div().text_color(p.muted).child(format!("by {}", link.by))),
        )
        .child(field(
            &link.before,
            div().font_family(MONO_FONT).child(link.before_says.clone()),
            cx,
        ))
        .child(field("This hop", div().child(link.here_says.clone()), cx))
        .child(div().text_color(p.ink_2).child(link.why.clone()))
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
fn who(label: &str, who: &str, cx: &App) -> Div {
    field(
        label,
        div().font_family(MONO_FONT).child(who.to_owned()),
        cx,
    )
}

/// Checks, one a line: a glyph, the name in monospace, what it found and
/// when.
fn checks(id: &'static str, checks: &[Check], cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(id)
        .test_support()
        .gap(dp(5.))
        .children(checks.iter().map(|check| {
            let found = match check.at {
                Some(at) => format!("{}, {}", check.found, clock(at)),
                None => check.found.clone(),
            };
            h_flex()
                .gap(dp(8.))
                .child(
                    div()
                        .flex_none()
                        .w(dp(10.))
                        .children(ui::status_glyph(tone(check.state), cx)),
                )
                .child(
                    div()
                        .w(dp(150.))
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO_FONT)
                        .child(check.name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(match check.state {
                            HealthIndicator::Error => p.crit_ink,
                            _ => p.ink_2,
                        })
                        .child(found),
                )
        }))
        .into_any_element()
}
