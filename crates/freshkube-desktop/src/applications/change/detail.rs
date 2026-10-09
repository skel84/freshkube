//! The selected hop in the shared Inspector: its kind, name and state, the
//! fields its tool reports, how it joins the hop before, and where it
//! leads. A gate row shows its Stage, with its three gates apart: whether
//! the Freight is eligible, how it is promoted, and the verification after.
//!
//! Every action leads out and changes nothing. One into Resources names
//! its cluster, and is greyed out with why for a cluster that isn't open;
//! the tools' own pages and a step's logs aren't linked yet, and say so.
//!
//! What the Inspector says is derived as a [`Detail`] when the selection
//! changes; drawing only lays it out.
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

/// The Inspector's words for the selection.
pub(super) struct Detail {
    kind: SharedString,
    title: SharedString,
    tone: Tone,
    state: SharedString,
    banner: Option<(Tone, SharedString, SharedString)>,
    body: Body,
    actions: Vec<ActionLine>,
}

enum Body {
    Hop {
        fields: Vec<FieldLine>,
        link: Option<LinkLines>,
        unlinked: Option<SharedString>,
    },
    Stage {
        fields: Vec<FieldLine>,
        eligible: GateLine,
        approvers: SharedString,
        promotion: GateLine,
        promoters: SharedString,
        steps: Vec<CheckLine>,
        analyses: Vec<CheckLine>,
    },
}

struct FieldLine {
    label: SharedString,
    value: SharedString,
    mono: bool,
}

struct LinkLines {
    confidence: Confidence,
    by: SharedString,
    label: SharedString,
    before: SharedString,
    before_says: SharedString,
    here_says: SharedString,
    why: SharedString,
}

struct GateLine {
    tone: Tone,
    says: SharedString,
    note: Option<SharedString>,
}

struct CheckLine {
    tone: Tone,
    failed: bool,
    name: SharedString,
    found: SharedString,
}

/// An action's button: its label, why it can't be pressed, and the object
/// it opens when it can.
struct ActionLine {
    label: SharedString,
    why: Option<SharedString>,
    opens: Option<Object>,
}

impl ChangePage {
    /// What the Inspector says of the selection, `None` without one.
    pub(super) fn derive_detail(&self) -> Option<Detail> {
        let hop = self.change.hop(self.selected.as_ref()?)?;
        Some(match &hop.shows {
            Shows::Hop(detail) => self.hop_detail(tone(hop.state), detail),
            Shows::Stage(stage) => self.stage_detail(&self.change.stages[*stage]),
        })
    }

    fn hop_detail(&self, tone: Tone, hop: &HopDetail) -> Detail {
        Detail {
            kind: hop.kind.clone().into(),
            title: hop.title.clone().into(),
            tone,
            state: hop.state.clone().into(),
            banner: hop.notice.as_ref().map(|notice| {
                (
                    super::tone(notice.state),
                    notice.lead.clone().into(),
                    notice.body.clone().into(),
                )
            }),
            body: Body::Hop {
                fields: hop.fields.iter().map(field_line).collect(),
                link: hop.link.as_ref().map(link_lines),
                unlinked: hop.unlinked.clone().map(Into::into),
            },
            actions: self.buttons(&hop.actions),
        }
    }

    fn stage_detail(&self, stage: &Stage) -> Detail {
        let freight = &self.change.freight;
        let banner = stage
            .verification
            .iter()
            .find(|check| check.state == HealthIndicator::Error)
            .map(|check| {
                (
                    Tone::Crit,
                    "Verification failed.".into(),
                    format!(
                        "{}: {}. {freight} still runs on {}.",
                        check.name, check.found, stage.cluster
                    )
                    .into(),
                )
            });
        let running = match &stage.running {
            Some(running) => format!("{running} runs; {freight} isn't promoted yet"),
            None => freight.clone(),
        };
        let open = Action::new(Target::Resource {
            what: "the Stage".into(),
            object: stage.object.clone(),
        });
        let line = |label: &str, value: String, mono| FieldLine {
            label: label.to_owned().into(),
            value: value.into(),
            mono,
        };
        Detail {
            kind: "Kargo Stage".into(),
            title: stage.name.clone().into(),
            tone: tone(stage.state),
            state: stage.words.clone().into(),
            banner,
            body: Body::Stage {
                fields: vec![
                    line("Project", self.change.project.clone(), false),
                    line("Deploys to", stage.cluster.clone(), true),
                    line("Freight", running, false),
                ],
                eligible: eligible(&stage.eligible),
                approvers: stage.approvers.clone().into(),
                promotion: promotion(&stage.promotion),
                promoters: stage.promoters.clone().into(),
                steps: stage.steps.iter().map(check_line).collect(),
                analyses: stage.verification.iter().map(check_line).collect(),
            },
            actions: self.buttons(&[open]),
        }
    }

    fn buttons(&self, actions: &[Action]) -> Vec<ActionLine> {
        actions
            .iter()
            .map(|action| match &action.target {
                Target::Resource { what, object } => {
                    let (_, closed) = self.object_link(object);
                    ActionLine {
                        label: format!("Open {what} in Resources · {}", object.cluster).into(),
                        why: action.disabled.clone().map(SharedString::from).or(closed),
                        opens: Some(object.clone()),
                    }
                }
                Target::Browser { label } => ActionLine {
                    label: format!("{label} ↗").into(),
                    why: Some(action.disabled.clone().unwrap_or(NOT_LINKED.into()).into()),
                    opens: None,
                },
                Target::Logs { .. } => ActionLine {
                    label: "Logs".into(),
                    why: Some(action.disabled.clone().unwrap_or(NO_LOGS.into()).into()),
                    opens: None,
                },
            })
            .collect()
    }

    /// The Inspector for the selected hop, or `None` without one.
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.as_ref()?;
        let inspector = Inspector::new("change-detail")
            .heading(heading(detail, cx))
            .banner(detail.banner.as_ref().map(|(tone, lead, body)| {
                ui::banner(*tone, Some(lead.clone()), body.clone(), None, cx)
            }));
        let inspector = match &detail.body {
            Body::Hop {
                fields,
                link,
                unlinked,
            } => render_hop(inspector, fields, link.as_ref(), unlinked.as_ref(), cx),
            Body::Stage { .. } => render_stage(inspector, &detail.body, cx),
        };
        Some(
            inspector
                .footer(Some(self.render_actions(&detail.actions, cx)))
                .render(cx)
                .into_any_element(),
        )
    }

    /// The hop's buttons: into Resources on its cluster, greyed out with
    /// why where it can't lead yet.
    fn render_actions(&self, actions: &[ActionLine], cx: &mut Context<Self>) -> AnyElement {
        let buttons = actions.iter().enumerate().map(|(ix, action)| {
            let button = Button::new(SharedString::from(format!("change-action-{ix}")))
                .outline()
                .xsmall()
                .label(action.label.clone());
            let button = match action.opens.clone() {
                Some(object) => {
                    button.on_click(cx.listener(move |this, _, _, cx| this.open(&object, cx)))
                }
                None => button,
            };
            match action.why.clone() {
                Some(why) => button.disabled(true).tooltip(why),
                None => button,
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

fn field_line(field: &Field) -> FieldLine {
    let (value, mono) = match &field.value {
        Value::Text(text) => (text.clone(), false),
        Value::Mono(text) => (text.clone(), true),
        Value::At(at) => (clock(*at), false),
    };
    FieldLine {
        label: field.label.clone().into(),
        value: value.into(),
        mono,
    }
}

fn link_lines(link: &LinkDetail) -> LinkLines {
    LinkLines {
        confidence: link.confidence,
        by: format!("by {}", link.by).into(),
        label: format!("{} by {}", link_word(link.confidence), link.by).into(),
        before: link.before.clone().into(),
        before_says: link.before_says.clone().into(),
        here_says: link.here_says.clone().into(),
        why: link.why.clone().into(),
    }
}

fn check_line(check: &Check) -> CheckLine {
    CheckLine {
        tone: tone(check.state),
        failed: check.state == HealthIndicator::Error,
        name: check.name.clone().into(),
        found: match check.at {
            Some(at) => format!("{}, {}", check.found, clock(at)),
            None => check.found.clone(),
        }
        .into(),
    }
}

fn eligible(eligible: &Eligible) -> GateLine {
    let gate = |tone, says: String, note: Option<String>| GateLine {
        tone,
        says: says.into(),
        note: note.map(Into::into),
    };
    match eligible {
        Eligible::Warehouse { at } => gate(
            Tone::Good,
            format!(
                "From the Warehouse at {}: the first Stage takes new Freight.",
                clock(*at)
            ),
            None,
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
        ),
    }
}

fn promotion(promotion: &Promotion) -> GateLine {
    match promotion {
        Promotion::Automatic { at } => GateLine {
            tone: Tone::Good,
            says: format!(
                "Automatic: auto-promotion is on. Promoted at {}.",
                clock(*at)
            )
            .into(),
            note: None,
        },
        Promotion::Waiting => GateLine {
            tone: Tone::Unknown,
            says: "Waiting for promotion: auto-promotion off.".into(),
            note: Some("Nothing has failed: it waits for someone who may promote.".into()),
        },
    }
}

fn render_hop(
    inspector: Inspector,
    fields: &[FieldLine],
    link: Option<&LinkLines>,
    unlinked: Option<&SharedString>,
    cx: &App,
) -> Inspector {
    let p = palette(cx);
    let link = match (link, unlinked) {
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
        .child(
            v_flex()
                .gap(dp(7.))
                .children(fields.iter().map(|line| render_field(line, cx))),
        )
        .children(link)
}

fn render_stage(inspector: Inspector, body: &Body, cx: &App) -> Inspector {
    let Body::Stage {
        fields,
        eligible,
        approvers,
        promotion,
        promoters,
        steps,
        analyses,
    } = body
    else {
        return inspector;
    };
    let p = palette(cx);
    inspector
        .child(
            v_flex()
                .gap(dp(7.))
                .children(fields.iter().map(|line| render_field(line, cx))),
        )
        .child(
            section("Eligible", cx)
                .id("change-detail-eligible")
                .test_support()
                .child(gate(eligible, cx))
                .child(who("May approve", approvers, cx)),
        )
        .child(
            section("Promotion", cx)
                .id("change-detail-promotion")
                .test_support()
                .child(gate(promotion, cx))
                .child(who("May promote", promoters, cx))
                .child(checks("change-detail-steps", steps, cx)),
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
                .child(checks("change-detail-analyses", analyses, cx)),
        )
}

/// The kind as a caption over the name in monospace and its state.
fn heading(detail: &Detail, cx: &App) -> impl IntoElement {
    let Detail {
        kind,
        title,
        tone,
        state,
        ..
    } = detail;
    v_flex()
        .id("change-detail-title")
        .test_support()
        .aria_label(SharedString::from(format!("{kind} {title}: {state}")))
        .gap(dp(4.))
        .min_w_0()
        .child(ui::caption(kind.as_ref(), cx))
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
                        .child(title.clone()),
                )
                .child(ui::tag(*tone, None, state.clone(), cx)),
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

fn render_field(line: &FieldLine, cx: &App) -> Div {
    let value = div()
        .when(line.mono, |this| this.font_family(MONO_FONT))
        .child(line.value.clone());
    field(&line.label, value, cx)
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
fn render_link(link: &LinkLines, cx: &App) -> Div {
    let p = palette(cx);
    section("Link", cx)
        .child(
            h_flex()
                .id("change-detail-link")
                .test_support()
                .aria_label(link.label.clone())
                .gap(dp(6.))
                .children(link_glyph(link.confidence).and_then(|tone| ui::status_glyph(tone, cx)))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(link_word(link.confidence)),
                )
                .child(div().text_color(p.muted).child(link.by.clone())),
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
fn gate(line: &GateLine, cx: &App) -> Div {
    h_flex()
        .gap(dp(8.))
        .items_start()
        .child(
            div()
                .flex_none()
                .h(dp(16.))
                .flex()
                .items_center()
                .children(ui::status_glyph(line.tone, cx)),
        )
        .child(
            v_flex()
                .gap(dp(2.))
                .min_w_0()
                .child(line.says.clone())
                .children(
                    line.note
                        .clone()
                        .map(|note| div().text_color(palette(cx).muted).child(note)),
                ),
        )
}

/// Who may act on the gate.
fn who(label: &str, who: &SharedString, cx: &App) -> Div {
    field(label, div().font_family(MONO_FONT).child(who.clone()), cx)
}

/// Checks, one a line: a glyph, the name in monospace, what it found and
/// when.
fn checks(id: &'static str, checks: &[CheckLine], cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(id)
        .test_support()
        .gap(dp(5.))
        .children(checks.iter().map(|check| {
            h_flex()
                .gap(dp(8.))
                .child(
                    div()
                        .flex_none()
                        .w(dp(10.))
                        .children(ui::status_glyph(check.tone, cx)),
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
                        .text_color(if check.failed { p.crit_ink } else { p.ink_2 })
                        .child(check.found.clone()),
                )
        }))
        .into_any_element()
}
