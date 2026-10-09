//! The selected hop in the shared Inspector: its kind, name and state, the
//! fields its tool reports, how it joins the hop before, and where it
//! leads. A gate row shows its Stage, with its three gates apart: whether
//! the Freight is eligible, how it is promoted, and the verification after.
//!
//! Every action leads out and changes nothing. One into Resources or to a
//! step's logs names its cluster, and is greyed out with why for a cluster
//! that isn't open; a tool's own page opens in the browser at the address
//! core made from what the cluster records, and is greyed out with why
//! without one.
//!
//! What the Inspector says is derived as a [`Detail`] when the selection
//! changes; drawing only lays it out.
use super::*;
use crate::ui::{self, MONO_FONT, dp};
use freshkube_core::delivery::change::{
    Action, Check, Destination, Eligible, Field, HopDetail, LinkDetail, Promotion, Stage, Target,
    Upstream, Value,
};
use freshkube_ui::inspector::Inspector;
use freshkube_ui::palette::palette;
use gpui_kit::component::{Disableable as _, Sizable, button::Button, h_flex, v_flex};

/// The width of a field's label.
const LABEL_WIDTH: f32 = 112.;

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

/// An action's button: its label, why it can't be pressed, and where it
/// leads when it can.
struct ActionLine {
    label: SharedString,
    why: Option<SharedString>,
    /// Its address, shown in the tooltip.
    tooltip: Option<SharedString>,
    leads: Option<Leads>,
}

/// Where a button leads.
#[derive(Clone)]
pub(super) enum Leads {
    Resources(Object),
    Browser(SharedString),
    Logs(Object, Option<String>),
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
                    match stage.cluster.cluster() {
                        Some(cluster) => format!(
                            "{}: {}. {freight} still runs on {cluster}.",
                            check.name, check.found
                        ),
                        None => format!("{}: {}.", check.name, check.found),
                    }
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
                    match &stage.cluster {
                        Destination::Cluster(cluster) => line("Deploys to", cluster.clone(), true),
                        Destination::Entry { entry, via } => line(
                            "Deploys to",
                            format!("{entry}, mapped from {via} in Settings › Workspace"),
                            false,
                        ),
                        Destination::Unknown(why) => {
                            line("Deploys to", format!("Unknown: {why}"), false)
                        }
                    },
                    line("Freight", running, false),
                ],
                eligible: eligible(&stage.eligible),
                approvers: not_read(&stage.approvers),
                promotion: promotion(&stage.promotion),
                promoters: not_read(&stage.promoters),
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
                        label: format!(
                            "Open {what} in Resources · {}",
                            self.cluster_name(&object.cluster)
                        )
                        .into(),
                        why: action.disabled.clone().map(SharedString::from).or(closed),
                        tooltip: None,
                        leads: Some(Leads::Resources(object.clone())),
                    }
                }
                Target::Browser { label, address } => ActionLine {
                    label: format!("{label} ↗").into(),
                    why: action.disabled.clone().map(SharedString::from),
                    tooltip: address.as_ref().map(|address| address.to_string().into()),
                    leads: address
                        .as_ref()
                        .map(|address| Leads::Browser(address.to_string().into())),
                },
                Target::Logs {
                    what,
                    pod,
                    container,
                } => {
                    let closed = self.logs_closed(pod);
                    ActionLine {
                        label: format!("Logs of {what} · {}", self.cluster_name(&pod.cluster))
                            .into(),
                        why: action.disabled.clone().map(SharedString::from).or(closed),
                        tooltip: None,
                        leads: Some(Leads::Logs(pod.clone(), container.clone())),
                    }
                }
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
            let button = match action.leads.clone() {
                Some(leads) => button.on_click(cx.listener(move |this, _, _, cx| match &leads {
                    Leads::Resources(object) => this.open(object, cx),
                    Leads::Browser(address) => cx.open_url(address),
                    Leads::Logs(pod, container) => this.open_logs(pod, container.clone(), cx),
                })),
                None => button,
            };
            match (action.why.clone(), action.tooltip.clone()) {
                (Some(why), _) => button.disabled(true).tooltip(why),
                (None, Some(tooltip)) => button.tooltip(tooltip),
                (None, None) => button,
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

/// ` at 10:12`, when Kargo recorded a time.
fn at(at: Option<DateTime<Utc>>) -> String {
    at.map(|at| format!(" at {}", clock(at)))
        .unwrap_or_default()
}

fn gate_line(tone: Tone, says: String, note: Option<String>) -> GateLine {
    GateLine {
        tone,
        says: says.into(),
        note: note.map(Into::into),
    }
}

fn eligible(eligible: &Eligible) -> GateLine {
    match eligible {
        Eligible::Warehouse { at: when } => gate_line(
            Tone::Good,
            format!(
                "From the Warehouse{}: the first Stage takes new Freight.",
                at(*when)
            ),
            None,
        ),
        Eligible::Verified {
            upstream,
            at: when,
            checks,
        } => gate_line(
            Tone::Good,
            match checks {
                Some(checks) => format!(
                    "Verified in {upstream}{}: {checks} passed there.",
                    at(*when)
                ),
                None => format!("Verified in {upstream}{}.", at(*when)),
            },
            None,
        ),
        Eligible::Approved {
            by,
            at: when,
            past,
            upstream,
        } => gate_line(
            Tone::Info,
            match (by, when) {
                (Some(by), Some(_)) => format!(
                    "Approved by {by}{}, not verified in {past} at the time.",
                    at(*when)
                ),
                (Some(by), None) => format!("Approved by {by}, past {past}."),
                (None, _) => format!("Approved by hand{}, past {past}.", at(*when)),
            },
            Some(match upstream {
                Upstream::Verified { at: then } => format!(
                    "Approved by hand for this Stage, past {past}; {past} verified it \
                     later{}.",
                    at(*then)
                ),
                Upstream::NotVerified => format!(
                    "Approved by hand for this Stage, past {past}, which hasn't verified \
                     it."
                ),
            }),
        ),
        Eligible::NotYet { upstream } => gate_line(
            Tone::Unknown,
            match upstream.as_slice() {
                [] => "Not eligible yet.".into(),
                upstream => format!(
                    "Not verified in {} yet, nor approved for this Stage.",
                    upstream.join(" or ")
                ),
            },
            None,
        ),
        Eligible::Unknown { why } => gate_line(Tone::Unknown, why.clone(), None),
    }
}

fn promotion(promotion: &Promotion) -> GateLine {
    match promotion {
        Promotion::Automatic { at: when } => gate_line(
            Tone::Good,
            format!("Automatic: auto-promotion is on. Promoted{}.", at(*when)),
            None,
        ),
        Promotion::ByHand { at: when } => gate_line(
            Tone::Good,
            format!("By hand: a user promoted it{}.", at(*when)),
            None,
        ),
        Promotion::Other { how, at: when } => gate_line(
            Tone::Good,
            match when {
                Some(_) => format!("{how}. Promoted{}.", at(*when)),
                None => format!("{how}."),
            },
            None,
        ),
        Promotion::Waiting => gate_line(
            Tone::Unknown,
            "Waiting for promotion: auto-promotion off.".into(),
            Some("Nothing has failed: it waits for someone who may promote.".into()),
        ),
        Promotion::NotYet => gate_line(
            Tone::Unknown,
            "Not promoted to this Stage yet.".into(),
            None,
        ),
        Promotion::Running { phase } => gate_line(
            Tone::Info,
            format!("Promoting: Kargo reports {phase}."),
            None,
        ),
        Promotion::Failed { phase, message } => gate_line(
            Tone::Crit,
            format!("The promotion ended {phase}."),
            message.clone(),
        ),
        Promotion::Unknown { why } => gate_line(Tone::Unknown, why.clone(), None),
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
                .child(checks(
                    "change-detail-steps",
                    steps,
                    "Its steps aren't read.",
                    cx,
                )),
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
                .child(checks(
                    "change-detail-analyses",
                    analyses,
                    "None recorded for this Freight.",
                    cx,
                )),
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

#[cfg(test)]
impl Detail {
    pub(super) fn action_labels(&self) -> Vec<String> {
        self.actions
            .iter()
            .map(|action| action.label.to_string())
            .collect()
    }
}

/// Who may act, or that it wasn't read.
fn not_read(who: &Option<String>) -> SharedString {
    who.clone().unwrap_or_else(|| "Not read".into()).into()
}

fn who(label: &str, who: &SharedString, cx: &App) -> Div {
    field(label, div().font_family(MONO_FONT).child(who.clone()), cx)
}

/// Checks, one a line: a glyph, the name in monospace, what it found and
/// when; `empty` in muted ink when there are none.
fn checks(id: &'static str, checks: &[CheckLine], empty: &'static str, cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(id)
        .test_support()
        .gap(dp(5.))
        .when(checks.is_empty(), |list| {
            list.child(div().text_color(p.muted).child(empty))
        })
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
