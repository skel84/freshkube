//! The change page (docs/DESIGN.md, "The change page"): one change, from
//! its pull request to the pods that run it, as a trail page. Its rows are
//! hops in the order the change travels, grouped by phase and by Stage,
//! never problems first; the chips count the hops by state and show only
//! theirs, and a Stage whose every row is fine folds while anything else
//! needs a look. Link confidence has its own column, beside the hop's
//! state. The selection's Inspector shows the hop, or for a gate row its
//! Stage with the three gates apart.
//!
//! It is opened from a Kargo application's page, on a Stage, and reads
//! nothing itself: core's [`Change`] is handed to it whole. Its breadcrumb,
//! and Escape with nothing selected, go back to the application's page.
//! An action that leads into Resources goes through the shell, as every
//! object link does; the page never approves, promotes or syncs.
mod detail;
mod table;
#[cfg(test)]
mod tests;
mod view;

use std::collections::BTreeSet;

use chrono::{DateTime, Local, Utc};
use freshkube_core::applications::SessionKey;
use freshkube_core::delivery::change::{Change, Object, Phase, Shows, Target};
use freshkube_core::delivery::join::Confidence;
use freshkube_core::indicators::HealthIndicator;
use freshkube_core::resources::ResourceKind;
use freshkube_ui::inspector::{self, InspectorSplit, Pane, Stacked};
use freshkube_ui::table::{self as kit, TableState};
use freshkube_ui::ui::Tone;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::links::Connections;
use crate::resources::{ResourceLink, Tab, model::ObjectRef};

/// The page's id prefix: `change-title`, `change-hop-<key>`, `change-detail`.
const PREFIX: &str = "change";
/// The page's key context, drawn in every state.
pub(crate) const CONTEXT: &str = "Change";

gpui_kit::actions!(
    change,
    [
        /// Selects the next hop.
        NextHop,
        /// Selects the previous hop.
        PreviousHop,
        /// Clears the selection; with none, goes back.
        Back,
        /// Opens the selected hop's object in Resources.
        OpenHop
    ]
);

pub(crate) fn key_bindings() -> [KeyBinding; 4] {
    [
        KeyBinding::new("down", NextHop, Some(CONTEXT)),
        KeyBinding::new("up", PreviousHop, Some(CONTEXT)),
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("o", OpenHop, Some(CONTEXT)),
    ]
}

/// What the page asks of the page that owns it.
pub(crate) enum ChangeEvent {
    /// Back to the application's page.
    Back,
    /// An object to open in Resources.
    Open(Box<ResourceLink>),
}

/// Stacked, the trail and the Inspector share the height evenly: the
/// trail is what the page is for, and a short window keeps rows of it.
const STACKED: Stacked = Stacked {
    lead: Pane::new(280.).least(inspector::LIST_MIN_HEIGHT),
    trail: Pane::new(280.).least(inspector::MIN_HEIGHT),
};

/// The status chips, failing first, and what each counts.
const TONES: [(Tone, &str, &str); 4] = [
    (Tone::Crit, "failing", "failing"),
    (Tone::Warn, "warning", "warning"),
    (Tone::Unknown, "waiting", "waiting or unknown"),
    (Tone::Good, "ok", "ok"),
];

/// A hop's state as a glyph's tone: waiting and unknown share the dashed
/// ring, and a hand approval is the blue dot.
pub(super) fn tone(state: HealthIndicator) -> Tone {
    match state {
        HealthIndicator::Healthy => Tone::Good,
        HealthIndicator::Info => Tone::Info,
        HealthIndicator::Pending | HealthIndicator::Unknown => Tone::Unknown,
        HealthIndicator::Warning => Tone::Warn,
        HealthIndicator::Error => Tone::Crit,
    }
}

/// The tone a row counts as: the blue dot is something to know, not a
/// fault, so it counts as ok in the chips, the filter and its group.
fn status(tone: Tone) -> Tone {
    match tone {
        Tone::Info => Tone::Good,
        tone => tone,
    }
}

/// How bad a tone is, for a group's worst row.
fn rank(tone: Tone) -> u8 {
    match tone {
        Tone::Crit | Tone::Died => 4,
        Tone::Warn => 3,
        Tone::Unknown => 2,
        _ => 1,
    }
}

/// An instant as the page shows it: the local time of day.
pub(super) fn clock(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

pub(super) fn link_word(confidence: Confidence) -> &'static str {
    super::page::link_word(confidence)
}

/// The glyph a link shows, if any: Confirmed is the usual case and shows
/// none, so the other two stand out.
pub(super) fn link_glyph(confidence: Confidence) -> Option<Tone> {
    match confidence {
        Confidence::Confirmed => None,
        Confidence::Claimed => Some(Tone::Info),
        Confidence::Unknown => Some(Tone::Unknown),
    }
}

/// One row's cells, derived with the change.
#[derive(Clone, Debug)]
pub(crate) struct HopRow {
    pub(super) key: SharedString,
    pub(super) group: usize,
    pub(super) tone: Tone,
    pub(super) name: SharedString,
    pub(super) detail: SharedString,
    pub(super) from: SharedString,
    pub(super) time: SharedString,
    pub(super) link: Option<Confidence>,
    /// The row's accessibility label.
    pub(super) label: SharedString,
}

/// A group row's words, derived with the change.
#[derive(Clone, Debug)]
pub(crate) struct GroupLine {
    pub(super) label: SharedString,
    pub(super) detail: Vec<String>,
    pub(super) tone: Tone,
    /// Only a Stage folds.
    pub(super) folds: bool,
    pub(super) rows: usize,
}

/// One line of the table: a group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(usize),
    Row(usize),
}

pub(crate) struct ChangePage {
    change: Change,
    connections: Connections,
    rows: Vec<HopRow>,
    groups: Vec<GroupLine>,
    /// Derived from the rows, the folds and the filter.
    lines: Vec<Entry>,
    /// How many rows each chip's tone has, folded or not.
    counts: [usize; TONES.len()],
    /// The Stage groups folded.
    folded: BTreeSet<usize>,
    filter: Option<Tone>,
    columns: Vec<table::Column>,
    table: TableState,
    focus: FocusHandle,
    split: InspectorSplit,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    selected: Option<SharedString>,
}

impl EventEmitter<ChangeEvent> for ChangePage {}

impl ChangePage {
    pub(super) fn new(change: Change, connections: Connections, cx: &mut Context<Self>) -> Self {
        let rows: Vec<HopRow> = change
            .hops
            .iter()
            .map(|hop| {
                let time = hop.at.map(clock).unwrap_or_default();
                let link = hop
                    .link
                    .map(|link| format!(" · link {}", link_word(link)))
                    .unwrap_or_default();
                HopRow {
                    key: hop.key.clone().into(),
                    group: hop.group,
                    tone: tone(hop.state),
                    name: hop.name.clone().into(),
                    detail: hop.detail.clone().into(),
                    from: hop.from.clone().into(),
                    label: format!("{}: {}{link}", hop.name, hop.detail).into(),
                    time: time.into(),
                    link: hop.link,
                }
            })
            .collect();
        let groups: Vec<GroupLine> = change
            .groups
            .iter()
            .enumerate()
            .map(|(ix, group)| {
                let of: Vec<&HopRow> = rows.iter().filter(|row| row.group == ix).collect();
                let tone = of
                    .iter()
                    .map(|row| status(row.tone))
                    .max_by_key(|tone| rank(*tone))
                    .unwrap_or(Tone::Good);
                let (label, detail, folds) = match group.phase {
                    Phase::Build => ("Build".to_owned(), vec![group.detail.clone()], false),
                    Phase::Freight => ("Freight".to_owned(), vec![group.detail.clone()], false),
                    Phase::Stage(stage) => {
                        let stage = &change.stages[stage];
                        (
                            format!("Stage {}", stage.name),
                            vec![group.detail.clone(), stage.words.to_lowercase()],
                            true,
                        )
                    }
                };
                GroupLine {
                    label: label.into(),
                    detail,
                    tone,
                    folds,
                    rows: of.len(),
                }
            })
            .collect();
        // A Stage whose every row is fine folds while anything else on the
        // page needs a look; a waiting or failing one never folds itself.
        let anything_wrong = groups.iter().any(|group| group.tone != Tone::Good);
        let folded = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| anything_wrong && group.folds && group.tone == Tone::Good)
            .map(|(ix, _)| ix)
            .collect();
        let mut counts = [0; TONES.len()];
        for row in &rows {
            if let Some(ix) = TONES
                .iter()
                .position(|(tone, ..)| *tone == status(row.tone))
            {
                counts[ix] += 1;
            }
        }
        let mut page = Self {
            change,
            connections,
            rows,
            groups,
            lines: Vec::new(),
            counts,
            folded,
            filter: None,
            columns: table::columns(),
            table: TableState::new(PREFIX),
            focus: cx.focus_handle(),
            split: InspectorSplit::new(PREFIX, cx).stacked(STACKED),
            page_scroll: ScrollHandle::new(),
            selected: None,
        };
        page.derive();
        page
    }

    /// The lines, when the folds or the filter change; drawing only reads
    /// them. A filter shows its rows in every group, folded or not.
    fn derive(&mut self) {
        self.lines.clear();
        for group in 0..self.groups.len() {
            let rows: Vec<usize> = (0..self.rows.len())
                .filter(|&ix| self.rows[ix].group == group)
                .filter(|&ix| {
                    self.filter
                        .is_none_or(|tone| tone == status(self.rows[ix].tone))
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            self.lines.push(Entry::Group(group));
            if self.filter.is_none() && self.folded.contains(&group) {
                continue;
            }
            self.lines.extend(rows.into_iter().map(Entry::Row));
        }
    }

    /// Puts the keyboard on the table, where its keys are bound.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub(crate) fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Selects a hop, unfolding its group so its row shows.
    pub(crate) fn select(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.key.as_ref() == key) else {
            return;
        };
        let group = row.group;
        self.selected = Some(row.key.clone());
        if self.folded.remove(&group) {
            self.derive();
        }
        kit::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Selects a Stage's first gate, which shows the Stage.
    pub(crate) fn select_stage(&mut self, stage: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.change.stages.iter().position(|s| s.name == stage) else {
            return;
        };
        let key = self
            .change
            .hops
            .iter()
            .find(|hop| hop.shows == Shows::Stage(ix))
            .map(|hop| hop.key.clone());
        if let Some(key) = key {
            self.select(&key, cx);
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(&key, cx);
        }
    }

    fn toggle_fold(&mut self, group: usize, cx: &mut Context<Self>) {
        if !self.folded.remove(&group) {
            self.folded.insert(group);
        }
        self.derive();
        cx.notify();
    }

    fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folded.clear();
        self.derive();
        cx.notify();
    }

    fn toggle_filter(&mut self, tone: Tone, cx: &mut Context<Self>) {
        self.filter = (self.filter != Some(tone)).then_some(tone);
        self.derive();
        cx.notify();
    }

    /// Escape: the selection first, then back.
    fn back(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.emit(ChangeEvent::Back);
        }
    }

    /// The link that opens an object in Resources, on its cluster's
    /// connection, and why it doesn't open when that cluster isn't open.
    fn object_link(&self, object: &Object) -> (ResourceLink, Option<SharedString>) {
        let session = SessionKey::new(object.cluster.clone());
        let link = ResourceLink::Object(
            ResourceKind::new(
                &object.group,
                &object.version,
                &object.kind,
                &object.plural,
                !object.namespace.is_empty(),
            ),
            ObjectRef {
                namespace: object.namespace.clone(),
                name: object.name.clone(),
                uid: String::new(),
                connection: Some(self.connections.of(&session)),
            },
            Tab::Overview,
        );
        let closed = (!self.connections.opens(&session)).then(|| {
            format!(
                "{} isn't the open cluster, so its objects don't open in Resources",
                object.cluster
            )
            .into()
        });
        (link, closed)
    }

    /// What O opens: a gate's Stage, or the hop's first object that may
    /// be opened at all.
    fn selected_object(&self) -> Option<&Object> {
        let hop = self.change.hop(self.selected.as_ref()?)?;
        match &hop.shows {
            Shows::Stage(stage) => Some(&self.change.stages[*stage].object),
            Shows::Hop(detail) => detail
                .actions
                .iter()
                .find_map(|action| match &action.target {
                    Target::Resource { object, .. } if action.disabled.is_none() => Some(object),
                    _ => None,
                }),
        }
    }

    /// O: the selected hop's object in Resources, through the shell, which
    /// says so when its cluster isn't open.
    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(object) = self.selected_object().cloned() {
            self.open(&object, cx);
        }
    }

    fn open(&mut self, object: &Object, cx: &mut Context<Self>) {
        let (link, _) = self.object_link(object);
        cx.emit(ChangeEvent::Open(Box::new(link)));
    }

    /// How many rows the table shows, for the footer while Stages fold.
    fn shown_rows(&self) -> usize {
        self.lines
            .iter()
            .filter(|entry| matches!(entry, Entry::Row(_)))
            .count()
    }
}

#[cfg(test)]
impl ChangePage {
    /// Each line: a group's label, or a row's key.
    pub(crate) fn lines_text(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|entry| match *entry {
                Entry::Group(ix) => format!("# {}", self.groups[ix].label),
                Entry::Row(ix) => self.rows[ix].key.to_string(),
            })
            .collect()
    }

    pub(crate) fn selected(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    pub(crate) fn freight(&self) -> &str {
        &self.change.freight
    }
}
