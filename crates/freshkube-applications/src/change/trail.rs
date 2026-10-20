//! The cards and links derived from the same rows List draws. Every gate
//! maps to its Stage's first gate, and joins use only `after`, never names
//! or adjacency. No layout or derivation runs while drawing.
use std::collections::BTreeMap;

use freshkube_delivery::change::{Change, Shows};
use freshkube_ui::graph::{EdgeStyle, GraphEdge, GraphNode};

use super::*;

#[derive(Default)]
pub(super) struct Trail {
    pub(super) nodes: Vec<GraphNode<SharedString>>,
    pub(super) edges: Vec<GraphEdge<SharedString>>,
    pub(super) cards: BTreeMap<SharedString, SharedString>,
}

impl Trail {
    pub(super) fn derive(change: &Change, rows: &[HopRow], groups: &[GroupLine]) -> Self {
        let mut trail = Self::default();
        let mut stages = BTreeMap::new();
        for (hop, row) in change.hops.iter().zip(rows) {
            let key = match hop.shows {
                Shows::Stage(stage) => stages
                    .entry(stage)
                    .or_insert_with(|| row.key.clone())
                    .clone(),
                _ => row.key.clone(),
            };
            trail.cards.insert(row.key.clone(), key.clone());
            if trail.nodes.iter().any(|node| node.key == key) {
                continue;
            }
            let (label, detail, tone) = match hop.shows {
                Shows::Stage(stage) => {
                    let group = &groups[row.group];
                    (
                        change.stages[stage].name.clone().into(),
                        std::iter::once(change.stages[stage].words.as_str())
                            .chain(group.detail.iter().skip(2).map(String::as_str))
                            .collect::<Vec<_>>()
                            .join(" · ")
                            .into(),
                        gate_tone(change, rows, stage),
                    )
                }
                _ => (row.name.clone(), row.detail.clone(), row.tone),
            };
            let ghost = matches!(&hop.shows, Shows::Hop(detail) if matches!(detail.state.as_str(), "Not found" | "Refused" | "Not served" | "Unknown"));
            trail.nodes.push(GraphNode {
                key: key.clone(),
                id: format!("change-card-{key}").into(),
                label,
                detail,
                tone: Some(tone),
                tooltip: row.label.clone(),
                ghost,
            });
        }
        for hop in &change.hops {
            let to = &trail.cards[hop.key.as_str()];
            let Some(from) = hop
                .after
                .as_deref()
                .and_then(|after| trail.cards.get(after))
            else {
                continue;
            };
            if from == to || trail.edges.iter().any(|edge| &edge.to == to) {
                continue;
            }
            let link = match &hop.shows {
                Shows::Hop(detail) => detail.link.as_ref(),
                Shows::Stage(stage) => change.stages[*stage].link.as_ref(),
            };
            let confidence = link
                .map(|link| link.confidence)
                .or(hop.link)
                .unwrap_or(Confidence::Unknown);
            let label = link
                .and_then(|link| link.key.as_deref())
                .map(short_key)
                .unwrap_or_else(|| "no key".into());
            let why = link
                .map(|link| {
                    format!(
                        "{} by {}. {} reports {}. This hop reports {}. {}",
                        link_word(confidence),
                        link.by,
                        link.before,
                        link.before_says,
                        link.here_says,
                        link.why
                    )
                })
                .unwrap_or_else(|| "Unknown: a side can't be read".into());
            trail.edges.push(GraphEdge {
                from: from.clone(),
                to: to.clone(),
                marker_id: format!("change-link-{to}").into(),
                row_id: format!("change-link-row-{to}").into(),
                tone: None,
                width: 1.1,
                label: link_word(confidence).into(),
                tooltip: why.into(),
                style: match confidence {
                    Confidence::Confirmed => EdgeStyle::Solid,
                    Confidence::Claimed => EdgeStyle::Dashed,
                    Confidence::Unknown => EdgeStyle::Dotted,
                },
                pill: Some(label.into()),
            });
        }
        trail
    }
}

fn short_key(key: &str) -> String {
    let digest = key.starts_with("sha256:");
    let key = key.strip_prefix("sha256:").unwrap_or(key);
    if digest || key.len() >= 7 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        key.chars().take(7).collect()
    } else {
        key.to_owned()
    }
}

// The Stage box represents its gates. Downstream health stays on its own
// card, even when it makes the List group's overall glyph weaker.
fn gate_tone(change: &Change, rows: &[HopRow], stage: usize) -> Tone {
    change
        .hops
        .iter()
        .zip(rows)
        .filter(|(hop, _)| hop.shows == Shows::Stage(stage))
        .map(|(_, row)| row.tone)
        .max_by_key(|tone| (rank(*tone), *tone == Tone::Info))
        .unwrap_or(Tone::Unknown)
}
