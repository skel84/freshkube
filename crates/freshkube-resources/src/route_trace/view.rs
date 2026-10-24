use super::*;
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, dp};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use std::rc::Rc;

/// What a link from the full trace does, given by the page that draws it.
pub type OnLink = Rc<dyn Fn(&TraceLink, &mut Window, &mut App)>;

fn heading(model: &TraceModel, cx: &App) -> impl IntoElement {
    let tone = super::verdict_tone(model.verdict);
    h_flex()
        .id(SharedString::from(format!("{}-verdict", model.id)))
        .test_support()
        .role(Role::Status)
        .aria_label(model.headline.clone())
        .gap(dp(8.))
        .items_start()
        .children(ui::status_glyph(tone, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_weight(FontWeight::MEDIUM)
                .whitespace_normal()
                .child(model.headline.clone()),
        )
        .children(
            model
                .owner
                .zip(model.owner_aria.clone())
                .map(|(owner, aria)| {
                    ui::tag(Tone::Outline, None, owner, cx)
                        .id(SharedString::from(format!("{}-owner", model.id)))
                        .aria_label(aria)
                        .test_support()
                }),
        )
        .children(model.stale.then(|| {
            ui::tag(Tone::Warn, None, "Stale verdict", cx)
                .id(SharedString::from(format!("{}-stale", model.id)))
                .test_support()
        }))
}

/// The verdict and, per parent, where it stops: the Routes Inspector and a
/// route's Overview. Everything else is in [`full`].
pub fn compact(model: &TraceModel, cx: &App) -> AnyElement {
    let p = palette(cx);
    v_flex()
        .id(SharedString::from(format!("{}-compact", model.id)))
        .test_support()
        .gap(dp(6.))
        .text_size(dp(12.))
        .child(heading(model, cx))
        .children(model.parents.iter().map(|parent| {
            h_flex()
                .id(parent.id.clone())
                .test_support()
                .gap(dp(6.))
                .items_start()
                .pl(dp(18.))
                .children(ui::status_glyph(parent.tone, cx))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .whitespace_normal()
                        .text_color(p.ink_2)
                        .child(parent.compact.clone()),
                )
        }))
        .into_any_element()
}

fn field(label: &'static str, text: &SharedString, cx: &App) -> Option<Div> {
    (!text.is_empty()).then(|| {
        h_flex()
            .gap(dp(6.))
            .items_start()
            .child(
                div()
                    .flex_none()
                    .w(dp(92.))
                    .text_color(palette(cx).muted)
                    .child(label),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .whitespace_normal()
                    .child(text.clone()),
            )
    })
}

fn step(row: &StepRow, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex()
        .id(row.id.clone())
        .test_support()
        .role(Role::Status)
        .aria_label(row.aria.clone())
        .gap(dp(2.))
        .child(
            h_flex()
                .gap(dp(8.))
                .child(ui::tag(row.tone, None, row.state, cx))
                .child(div().font_weight(FontWeight::MEDIUM).child(row.name))
                .children(
                    row.owner
                        .map(|owner| ui::tag(Tone::Outline, None, owner, cx)),
                )
                .children(row.stale.then(|| ui::tag(Tone::Warn, None, "Stale", cx))),
        )
        .child(
            v_flex()
                .pl(dp(8.))
                .gap(dp(1.))
                .text_size(dp(12.))
                .text_color(p.ink_2)
                .children(field("Route asks", &row.asks, cx))
                .children(field("Gateway serves", &row.serves, cx))
                .children(field("Why", &row.reason, cx))
                .children(
                    row.controller
                        .as_ref()
                        .and_then(|controller| field("Controller says", controller, cx)),
                ),
        )
}

fn link_button(
    id: SharedString,
    label: &'static str,
    link: Option<&TraceLink>,
    on_link: &Option<OnLink>,
) -> Option<Button> {
    let (link, on_link) = (link?.clone(), on_link.clone()?);
    Some(
        Button::new(id)
            .ghost()
            .small()
            .label(label)
            .on_click(move |_, window, cx| on_link(&link, window, cx)),
    )
}

fn branch(row: &BranchRow, on_link: &Option<OnLink>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex()
        .id(row.id.clone())
        .test_support()
        .gap(dp(6.))
        .pl(dp(12.))
        .child(
            h_flex()
                .gap(dp(8.))
                .flex_wrap()
                .child(ui::tag(row.tone, None, row.state, cx))
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .font_weight(FontWeight::MEDIUM)
                        .child(row.title.clone()),
                )
                .children(row.share.clone().map(|share| {
                    ui::tag(Tone::Outline, None, share, cx)
                        .id(SharedString::from(format!("{}-share", row.id)))
                        .test_support()
                }))
                .child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(row.rule_text.clone()),
                )
                .children(link_button(
                    format!("{}-service", row.id).into(),
                    "Open Service",
                    row.service.as_ref(),
                    on_link,
                )),
        )
        .children(row.steps.iter().map(|row| step(row, cx)))
}

fn parent(row: &ParentRow, on_link: &Option<OnLink>, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    v_flex()
        .id(row.id.clone())
        .test_support()
        .gap(dp(8.))
        .pb(dp(10.))
        .child(
            h_flex()
                .gap(dp(8.))
                .flex_wrap()
                .child(ui::tag(row.tone, None, row.state, cx))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(row.title.clone()),
                )
                .children(row.controller.clone().map(|controller| {
                    div()
                        .text_size(dp(12.))
                        .text_color(p.muted)
                        .child(controller.clone())
                }))
                .children(link_button(
                    format!("{}-gateway", row.id).into(),
                    "Open Gateway",
                    row.gateway.as_ref(),
                    on_link,
                )),
        )
        .children(row.steps.iter().map(|row| step(row, cx)))
        .children(row.others.iter().enumerate().map(|(ix, other)| {
            div()
                .id(SharedString::from(format!("{}-other-{ix}", row.id)))
                .test_support()
                .pl(dp(8.))
                .text_size(dp(12.))
                .text_color(p.muted)
                .whitespace_normal()
                .child(other.clone())
        }))
        .children((!row.branches.is_empty()).then(|| ui::caption("Backends", cx)))
        .children(row.branches.iter().map(|row| branch(row, on_link, cx)))
}

fn proposal(row: &ProposalRow, cx: &App) -> impl IntoElement {
    let p = palette(cx);
    let yaml = row.yaml.clone();
    v_flex()
        .id(row.id.clone())
        .test_support()
        .gap(dp(4.))
        .child(
            h_flex()
                .gap(dp(8.))
                .child(div().min_w_0().flex_1().child(row.title.clone()))
                .child(
                    Button::new(SharedString::from(format!("{}-copy", row.id)))
                        .small()
                        .label("Copy")
                        .tooltip("Copies the text only; nothing applies it")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(yaml.to_string()))
                        }),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!("{}-yaml", row.id)))
                .test_support()
                .p(dp(8.))
                .rounded(px(3.))
                .bg(p.surface_2)
                .font_family(MONO_FONT)
                .text_size(dp(12.))
                .whitespace_normal()
                .child(row.yaml.trim_end().to_owned()),
        )
}

/// The whole trace: each parent's ordered steps, each backend's steps, links
/// to the Gateway and the Services, and a proposed ReferenceGrant to copy.
pub fn full(model: &TraceModel, on_link: Option<OnLink>, cx: &App) -> AnyElement {
    v_flex()
        .id(SharedString::from(format!("{}-full", model.id)))
        .test_support()
        .gap(dp(12.))
        .text_size(dp(13.))
        .child(heading(model, cx))
        .children(model.parents.iter().map(|row| parent(row, &on_link, cx)))
        .children((!model.proposals.is_empty()).then(|| {
            v_flex()
                .gap(dp(6.))
                .child(ui::caption("Proposed fix, to copy", cx))
                .child(
                    div()
                        .text_size(dp(12.))
                        .text_color(palette(cx).muted)
                        .child(
                            "Freshkube changes nothing. Someone who owns the Service's namespace applies this.",
                        ),
                )
                .children(model.proposals.iter().map(|row| proposal(row, cx)))
        }))
        .into_any_element()
}

/// Each backend branch on one line: the split and where it stands. For the
/// Inspector, which has no room for every step.
pub fn backends(model: &TraceModel, cx: &App) -> Option<AnyElement> {
    let rows = model
        .parents
        .iter()
        .flat_map(|p| &p.branches)
        .collect::<Vec<_>>();
    (!rows.is_empty()).then(|| {
        v_flex()
            .id(SharedString::from(format!("{}-backends", model.id)))
            .test_support()
            .gap(dp(4.))
            .text_size(dp(12.))
            .child(ui::caption("Backends", cx))
            .children(rows.into_iter().map(|row| {
                h_flex()
                    .id(SharedString::from(format!("{}-line", row.id)))
                    .test_support()
                    .gap(dp(6.))
                    .items_start()
                    .children(ui::status_glyph(row.tone, cx))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .whitespace_normal()
                            .child(row.line.clone()),
                    )
            }))
            .into_any_element()
    })
}

/// The proposed ReferenceGrants, each to copy and nothing more.
pub fn proposals(model: &TraceModel, cx: &App) -> Option<AnyElement> {
    (!model.proposals.is_empty()).then(|| {
        v_flex()
            .id(SharedString::from(format!("{}-proposals", model.id)))
            .test_support()
            .gap(dp(6.))
            .child(ui::caption("Proposed fix, to copy", cx))
            .child(
                div()
                    .text_size(dp(12.))
                    .text_color(palette(cx).muted)
                    .child(
                        "Freshkube changes nothing. Someone who owns the Service's namespace applies this.",
                    ),
            )
            .children(model.proposals.iter().map(|row| proposal(row, cx)))
            .into_any_element()
    })
}
