//! The Overview tab: what every kind has in common, conditions, and a
//! Secret's keys with their values hidden until revealed.

use freshkube_core::resources::{Condition, SecretValue};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{DetailPane, local_time};
use crate::palette::palette;
use crate::resources::detail::{Detail, DocumentView, Reveal};
use crate::screens::field;
use crate::ui::{self, MONO_FONT, Tone};

const LABELS_SHOWN: usize = 12;
const ANNOTATIONS_SHOWN: usize = 6;
/// Even "Show all" draws at most this many labels or annotations; the YAML
/// has every one.
const MAX_SHOWN: usize = 200;
const ANNOTATION_CHARS: usize = 240;
/// Revealed values are cut to this many characters when drawn; Copy takes
/// the whole value.
const VALUE_CHARS: usize = 4_000;

struct ConditionLine {
    kind: SharedString,
    status: SharedString,
    tone: Tone,
    detail: Option<SharedString>,
    changed: Option<SharedString>,
}

impl ConditionLine {
    fn new(condition: &Condition) -> Self {
        let detail = [condition.reason.as_str(), condition.message.as_str()]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        Self {
            kind: condition.kind.clone().into(),
            status: condition.status.clone().into(),
            tone: condition_tone(&condition.kind, &condition.status),
            detail: (!detail.is_empty()).then(|| detail.into()),
            changed: condition
                .changed
                .map(|time| format!("since {}", local_time(time)).into()),
        }
    }
}

/// How a condition reads. Most are good when `True`; a few report a problem
/// when `True`. Types this doesn't know stay neutral rather than guess.
fn condition_tone(kind: &str, status: &str) -> Tone {
    const GOOD_WHEN_TRUE: [&str; 9] = [
        "Ready",
        "Available",
        "Initialized",
        "ContainersReady",
        "PodScheduled",
        "PodReadyToStartContainers",
        "Progressing",
        "Complete",
        "Established",
    ];
    const BAD_WHEN_TRUE: [&str; 6] = [
        "MemoryPressure",
        "DiskPressure",
        "PIDPressure",
        "NetworkUnavailable",
        "ReplicaFailure",
        "Failed",
    ];
    let good_when_true = if GOOD_WHEN_TRUE.contains(&kind) {
        true
    } else if BAD_WHEN_TRUE.contains(&kind) {
        false
    } else {
        return if status == "Unknown" {
            Tone::Unknown
        } else {
            Tone::Outline
        };
    };
    match status {
        "True" | "False" if (status == "True") == good_when_true => Tone::Good,
        "True" | "False" => Tone::Crit,
        "Unknown" => Tone::Unknown,
        _ => Tone::Outline,
    }
}

struct SecretLine {
    name: String,
    size: SharedString,
}

/// The overview's text, derived once per document read, never while drawing.
pub(super) struct Summary {
    kind: SharedString,
    created: Option<SharedString>,
    deleting: Option<SharedString>,
    generation: Option<SharedString>,
    owners: Vec<SharedString>,
    conditions: Vec<ConditionLine>,
    labels: Vec<SharedString>,
    label_count: usize,
    annotations: Vec<(SharedString, SharedString)>,
    annotation_count: usize,
    finalizers: Vec<SharedString>,
    /// A Secret's type and keys with their sizes.
    secret: Option<(SharedString, Vec<SecretLine>)>,
}

impl Summary {
    pub(super) fn new(view: &DocumentView) -> Self {
        let document = &view.document;
        let overview = &document.overview;
        Self {
            kind: format!("{} · {}", document.kind, document.api_version).into(),
            created: overview.created.map(|time| local_time(time).into()),
            deleting: overview.deleting.map(|time| local_time(time).into()),
            generation: overview.generation.map(|generation| {
                match overview.observed_generation {
                    Some(observed) if observed < generation => {
                        format!("{generation}; its controller has seen {observed}")
                    }
                    Some(_) => format!("{generation}, seen by its controller"),
                    None => generation.to_string(),
                }
                .into()
            }),
            owners: overview
                .owners
                .iter()
                .map(|owner| {
                    let controller = if owner.controller {
                        " (controller)"
                    } else {
                        ""
                    };
                    format!("{} {}{controller}", owner.kind, owner.name).into()
                })
                .collect(),
            conditions: overview.conditions.iter().map(ConditionLine::new).collect(),
            labels: overview
                .labels
                .iter()
                .take(MAX_SHOWN)
                .map(|(key, value)| format!("{key}={value}").into())
                .collect(),
            label_count: overview.labels.len(),
            annotations: overview
                .annotations
                .iter()
                .take(MAX_SHOWN)
                .map(|(key, value)| (key.clone().into(), shorten(value, ANNOTATION_CHARS).into()))
                .collect(),
            annotation_count: overview.annotations.len(),
            finalizers: overview
                .finalizers
                .iter()
                .map(|finalizer| finalizer.clone().into())
                .collect(),
            secret: overview.secret.as_ref().map(|secret| {
                (
                    secret.secret_type.clone().into(),
                    secret
                        .keys
                        .iter()
                        .map(|key| SecretLine {
                            name: key.name.clone(),
                            size: byte_size(key.bytes).into(),
                        })
                        .collect(),
                )
            }),
        }
    }
}

fn byte_size(bytes: usize) -> String {
    match bytes {
        1 => "1 byte".into(),
        0..1_024 => format!("{bytes} bytes"),
        _ => format!("{:.1} KiB", bytes as f64 / 1_024.),
    }
}

/// `text` on one line, cut to `max` characters.
fn shorten(text: &str, max: usize) -> String {
    let line = text.trim_end().replace('\n', " ↵ ");
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

impl DetailPane {
    pub(super) fn overview(
        &self,
        detail: &Detail,
        summary: &Summary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let identity = &detail.target.identity;
        let section = |title: &str, cx: &App| v_flex().gap_2().child(ui::caption(title, cx));
        let mut body = v_flex().gap(px(18.)).child(
            v_flex()
                .gap_2()
                .child(field("Kind", summary.kind.clone(), cx))
                .when(!identity.namespace.is_empty(), |this| {
                    this.child(field("Namespace", identity.namespace.clone(), cx))
                })
                .child(field(
                    "UID",
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(12.))
                        .child(identity.uid.clone()),
                    cx,
                ))
                .children(
                    summary
                        .created
                        .clone()
                        .map(|created| field("Created", created, cx)),
                )
                .children(summary.deleting.clone().map(|deleting| {
                    field(
                        "Deleting since",
                        div().text_color(p.crit_ink).child(deleting),
                        cx,
                    )
                }))
                .children(
                    summary
                        .generation
                        .clone()
                        .map(|generation| field("Generation", generation, cx)),
                ),
        );
        if let Some((secret_type, keys)) = &summary.secret {
            body = body.child(self.secret(detail, secret_type, keys, cx));
        }
        if !summary.conditions.is_empty() {
            body =
                body.child(
                    section("Conditions", cx).children(summary.conditions.iter().enumerate().map(
                        |(ix, condition)| {
                            h_flex()
                                .id(("detail-condition", ix))
                                .test_support()
                                .items_start()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(132.))
                                        .flex_none()
                                        .text_size(px(12.5))
                                        .child(condition.kind.clone()),
                                )
                                .child(ui::tag(condition.tone, None, condition.status.clone(), cx))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(12.))
                                        .children(condition.detail.clone())
                                        .children(condition.changed.clone().map(|changed| {
                                            div().text_color(p.muted).child(changed)
                                        })),
                                )
                                .into_any_element()
                        },
                    )),
                );
        }
        if !summary.owners.is_empty() {
            body = body.child(
                section("Owned by", cx).children(
                    summary
                        .owners
                        .iter()
                        .map(|owner| div().text_size(px(12.5)).child(owner.clone())),
                ),
            );
        }
        if summary.label_count > 0 {
            let shown = if self.show_all_labels {
                summary.labels.len()
            } else {
                LABELS_SHOWN.min(summary.labels.len())
            };
            body = body.child(
                section("Labels", cx)
                    .child(
                        h_flex()
                            .id("detail-labels")
                            .test_support()
                            .flex_wrap()
                            .gap_1p5()
                            .children(summary.labels[..shown].iter().map(|label| {
                                div()
                                    .max_w_full()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(5.))
                                    .bg(p.surface_2)
                                    .border_1()
                                    .border_color(p.line)
                                    .font_family(MONO_FONT)
                                    .text_size(px(11.5))
                                    .truncate()
                                    .child(label.clone())
                            })),
                    )
                    .children(self.show_more(
                        "detail-labels-all",
                        shown,
                        summary.label_count,
                        "labels",
                        |pane| &mut pane.show_all_labels,
                        cx,
                    )),
            );
        }
        if summary.annotation_count > 0 {
            let shown = if self.show_all_annotations {
                summary.annotations.len()
            } else {
                ANNOTATIONS_SHOWN.min(summary.annotations.len())
            };
            body = body.child(
                section("Annotations", cx)
                    .child(
                        v_flex()
                            .id("detail-annotations")
                            .test_support()
                            .gap_1p5()
                            .children(summary.annotations[..shown].iter().map(|(key, value)| {
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(MONO_FONT)
                                            .text_size(px(11.5))
                                            .text_color(p.ink_2)
                                            .truncate()
                                            .child(key.clone()),
                                    )
                                    .child(
                                        div()
                                            .font_family(MONO_FONT)
                                            .text_size(px(11.5))
                                            .child(value.clone()),
                                    )
                            })),
                    )
                    .children(self.show_more(
                        "detail-annotations-all",
                        shown,
                        summary.annotation_count,
                        "annotations",
                        |pane| &mut pane.show_all_annotations,
                        cx,
                    )),
            );
        }
        if !summary.finalizers.is_empty() {
            body = body.child(
                section("Finalizers", cx).children(summary.finalizers.iter().map(|finalizer| {
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(12.))
                        .child(finalizer.clone())
                })),
            );
        }
        div()
            .id("detail-overview")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .child(body)
            .into_any_element()
    }

    /// "Show all" under a capped list, or how many more the YAML has.
    fn show_more(
        &self,
        id: &'static str,
        shown: usize,
        total: usize,
        what: &str,
        flag: fn(&mut Self) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if shown == total {
            return None;
        }
        let p = palette(cx);
        if shown == MAX_SHOWN {
            return Some(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(format!("{} more in the YAML", total - shown))
                    .into_any_element(),
            );
        }
        Some(
            div()
                .child(
                    Button::new(id)
                        .ghost()
                        .xsmall()
                        .label(format!("Show all {total} {what}"))
                        .on_click(cx.listener(move |pane, _, _, cx| {
                            *flag(pane) = true;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    fn secret(
        &self,
        detail: &Detail,
        secret_type: &SharedString,
        keys: &[SecretLine],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        v_flex()
            .gap_2()
            .child(ui::caption("Secret data", cx))
            .child(field("Type", secret_type.clone(), cx))
            .child(div().text_size(px(12.)).text_color(p.muted).child(
                "Values are hidden. Reveal reads one value afresh; Copy YAML never includes them.",
            ))
            .children(keys.iter().enumerate().map(|(ix, key)| {
                let reveal = detail.reveals.get(&key.name);
                let name = key.name.clone();
                let actions = match reveal {
                    None | Some(Reveal::Failed(_)) => vec![
                        Button::new(("detail-secret-reveal", ix))
                            .outline()
                            .xsmall()
                            .icon(IconName::Eye)
                            .label("Reveal")
                            .on_click(cx.listener({
                                let name = name.clone();
                                move |pane, _, _, cx| pane.reveal(name.clone(), cx)
                            }))
                            .into_any_element(),
                    ],
                    Some(Reveal::Reading) => vec![
                        Button::new(("detail-secret-reveal", ix))
                            .outline()
                            .xsmall()
                            .loading(true)
                            .label("Reading")
                            .into_any_element(),
                    ],
                    Some(Reveal::Shown(value)) => {
                        let mut actions = vec![
                            Button::new(("detail-secret-hide", ix))
                                .outline()
                                .xsmall()
                                .icon(IconName::EyeOff)
                                .label("Hide")
                                .on_click(cx.listener({
                                    let name = name.clone();
                                    move |pane, _, _, cx| pane.hide(&name, cx)
                                }))
                                .into_any_element(),
                        ];
                        if let SecretValue::Text(_) = value {
                            actions.push(
                                Button::new(("detail-secret-copy", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Copy)
                                    .label("Copy")
                                    .on_click(cx.listener({
                                        let name = name.clone();
                                        move |pane, _, _, cx| pane.copy_value(&name, cx)
                                    }))
                                    .into_any_element(),
                            );
                        }
                        actions
                    }
                };
                let value = match reveal {
                    Some(Reveal::Shown(SecretValue::Text(text))) => Some(
                        div()
                            .id(("detail-secret-value", ix))
                            .test_support()
                            .max_h(px(160.))
                            .overflow_y_scroll()
                            .px_2()
                            .py_1p5()
                            .rounded(px(6.))
                            .bg(p.surface_2)
                            .font_family(MONO_FONT)
                            .text_size(px(12.))
                            .child(match text.char_indices().nth(VALUE_CHARS) {
                                Some((cut, _)) => {
                                    format!("{}… (Copy takes the whole value)", &text[..cut])
                                }
                                None => text.clone(),
                            })
                            .into_any_element(),
                    ),
                    Some(Reveal::Shown(SecretValue::Binary(bytes))) => Some(
                        div()
                            .id(("detail-secret-value", ix))
                            .test_support()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(format!("Binary data, {}; not shown.", byte_size(*bytes)))
                            .into_any_element(),
                    ),
                    Some(Reveal::Failed(reason)) => Some(
                        div()
                            .id(("detail-secret-error", ix))
                            .test_support()
                            .text_size(px(12.))
                            .text_color(p.crit_ink)
                            .child(reason.clone())
                            .into_any_element(),
                    ),
                    _ => None,
                };
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .id(("detail-secret-key", ix))
                            .test_support()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(MONO_FONT)
                                    .text_size(px(12.5))
                                    .truncate()
                                    .child(key.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(p.muted)
                                    .child(key.size.clone()),
                            )
                            .children(actions),
                    )
                    .children(value)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::{Tone, condition_tone, shorten};

    #[test]
    fn conditions_read_good_or_bad_only_when_their_meaning_is_known() {
        assert_eq!(condition_tone("Ready", "True"), Tone::Good);
        assert_eq!(condition_tone("Ready", "False"), Tone::Crit);
        assert_eq!(condition_tone("MemoryPressure", "False"), Tone::Good);
        assert_eq!(condition_tone("MemoryPressure", "True"), Tone::Crit);
        assert_eq!(condition_tone("Ready", "Unknown"), Tone::Unknown);
        assert_eq!(condition_tone("SomethingCustom", "True"), Tone::Outline);
        assert_eq!(condition_tone("SomethingCustom", "Unknown"), Tone::Unknown);
    }

    #[test]
    fn long_values_are_shortened_onto_one_line() {
        assert_eq!(shorten("a\nb\n", 10), "a ↵ b");
        assert_eq!(shorten("ééééé", 3), "ééé…");
        assert_eq!(shorten("abc", 3), "abc");
    }
}
