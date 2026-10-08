//! One application's page: its parts as a table page grouped by kind, each
//! with how sure its link to the application is, Confirmed, Claimed or
//! Unknown, from core's `claims`; a read that may have left parts out is a
//! group row with the Unknown ring, never an empty group. The selection's
//! Inspector shows both sides of the link, why, and any lower claim.
//!
//! The list owns the page and hands it each new read; it reads nothing
//! itself. Its breadcrumb, and Escape with nothing selected, go back to the
//! list, which keeps its selection.
mod table;
#[cfg(test)]
mod tests;
mod view;

use freshkube_core::applications::claims::{Claim, Claims, Gap, Side, claims};
use freshkube_core::applications::{
    Application, ApplicationId, Basis, Derived, Evidence, MemberKind, MemberRef,
};
use freshkube_core::delivery::join::Confidence;
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::table::{self as kit, TableState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::display::{Labels, kind_label, rule_label};

/// The page's id prefix: `application-title`, `-list`, `-back`.
const PREFIX: &str = "application";
/// The page's key context, drawn in every state.
pub(crate) const CONTEXT: &str = "Application";

gpui_kit::actions!(
    application,
    [
        /// Selects the next part.
        NextPart,
        /// Selects the previous part.
        PreviousPart,
        /// Clears the selection; with none, goes back to the list.
        Back
    ]
);

pub(crate) fn key_bindings() -> [KeyBinding; 3] {
    [
        KeyBinding::new("down", NextPart, Some(CONTEXT)),
        KeyBinding::new("up", PreviousPart, Some(CONTEXT)),
        KeyBinding::new("escape", Back, Some(CONTEXT)),
    ]
}

/// What the page asks of the list that owns it.
pub(crate) enum ApplicationEvent {
    /// Back to the list.
    Back,
}

/// One part as the table and the Inspector show it.
#[derive(Clone, Debug)]
pub(crate) struct PartRow {
    pub(super) key: SharedString,
    pub(super) rank: u8,
    pub(super) name: SharedString,
    pub(super) cluster: SharedString,
    pub(super) namespace: SharedString,
    pub(super) confidence: Confidence,
    pub(super) found_by: SharedString,
    pub(super) read_from: SharedString,
    /// The row's accessibility label.
    pub(super) label: SharedString,
    pub(super) tooltip: SharedString,
    /// The Inspector's fields about the part, then about its link.
    pub(super) fields: Vec<(&'static str, SharedString)>,
    pub(super) link: Vec<(&'static str, SharedString)>,
    /// Lower rules' claims, in words.
    pub(super) lower: Vec<SharedString>,
}

/// A group row: a kind's, or a read's that may have left parts out.
#[derive(Clone, Debug)]
pub(crate) struct GroupLine {
    pub(super) slug: String,
    pub(super) label: SharedString,
    pub(super) tone: freshkube_ui::ui::Tone,
    pub(super) detail: Vec<String>,
}

/// One line of the table: a group header, or a row by index.
#[derive(Clone, Copy)]
enum Entry {
    Group(usize),
    Row(usize),
}

pub(crate) struct ApplicationPage {
    id: ApplicationId,
    name: SharedString,
    rows: Vec<PartRow>,
    groups: Vec<GroupLine>,
    lines: Vec<Entry>,
    columns: Vec<table::Column>,
    width: f32,
    table: TableState,
    focus: FocusHandle,
    split: InspectorSplit,
    /// The frame's scroll, used while the window is short.
    page_scroll: ScrollHandle,
    selected: Option<SharedString>,
}

impl EventEmitter<ApplicationEvent> for ApplicationPage {}

impl ApplicationPage {
    pub(crate) fn new(
        app: &Application,
        derived: &Derived,
        labels: &Labels,
        cx: &mut Context<Self>,
    ) -> Self {
        let split = InspectorSplit::new(PREFIX, cx);
        let mut page = Self {
            id: app.id.clone(),
            name: app.name.clone().into(),
            rows: Vec::new(),
            groups: Vec::new(),
            lines: Vec::new(),
            columns: Vec::new(),
            width: 0.,
            table: TableState::new(PREFIX),
            focus: cx.focus_handle(),
            split,
            page_scroll: ScrollHandle::new(),
            selected: None,
        };
        page.show(app, derived, labels);
        page
    }

    pub(crate) fn id(&self) -> &ApplicationId {
        &self.id
    }

    /// A new read of the application, keeping the selection while its part
    /// is still there.
    pub(crate) fn update(
        &mut self,
        app: &Application,
        derived: &Derived,
        labels: &Labels,
        cx: &mut Context<Self>,
    ) {
        self.name = app.name.clone().into();
        self.show(app, derived, labels);
        cx.notify();
    }

    /// Derives the rows, groups and lines from the claims.
    fn show(&mut self, app: &Application, derived: &Derived, labels: &Labels) {
        let Claims { links, gaps } = claims(derived, app);
        self.rows = links.iter().map(|claim| row(app, claim, labels)).collect();
        (self.groups, self.lines) = lines(&self.rows, &gaps, labels);
        (self.columns, self.width) = table::columns(&self.rows);
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.rows.iter().any(|row| &row.key == key))
        {
            self.selected = None;
        }
    }

    /// Puts the keyboard on the table, where its keys are bound.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub(crate) fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn selected_row(&self) -> Option<&PartRow> {
        let key = self.selected.as_ref()?;
        self.rows.iter().find(|row| &row.key == key)
    }

    fn select(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.selected = Some(key);
        kit::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    /// Escape: the selection first, then back to the list.
    fn back(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.emit(ApplicationEvent::Back);
        }
    }
}

#[cfg(test)]
impl ApplicationPage {
    /// Each line: a group's label, or a row's name and link word.
    pub(crate) fn lines_text(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|entry| match *entry {
                Entry::Group(ix) => format!("# {}", self.groups[ix].label),
                Entry::Row(ix) => {
                    let row = &self.rows[ix];
                    format!("{} {}", row.name, link_word(row.confidence))
                }
            })
            .collect()
    }

    pub(crate) fn selected(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }
}

pub(super) fn link_word(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Confirmed => "Confirmed",
        Confidence::Claimed => "Claimed",
        Confidence::Unknown => "Unknown",
    }
}

/// The link's glyph: none for Confirmed, the usual case, so the other two
/// stand out; the Info dot for Claimed, and the dashed ring for Unknown.
pub(super) fn link_tone(confidence: Confidence) -> Option<freshkube_ui::ui::Tone> {
    use freshkube_ui::ui::Tone;
    match confidence {
        Confidence::Confirmed => None,
        Confidence::Claimed => Some(Tone::Info),
        Confidence::Unknown => Some(Tone::Unknown),
    }
}

/// A group's label for a kind, plural.
fn kind_group(kind: MemberKind) -> &'static str {
    use freshkube_core::workloads::WorkloadKind;
    match kind {
        MemberKind::KargoStage => "Kargo Stages",
        MemberKind::KargoWarehouse => "Kargo Warehouses",
        MemberKind::ArgoApplication => "Argo CD Applications",
        MemberKind::Workload(WorkloadKind::Deployment) => "Deployments",
        MemberKind::Workload(WorkloadKind::StatefulSet) => "StatefulSets",
        MemberKind::Workload(WorkloadKind::DaemonSet) => "DaemonSets",
    }
}

/// How the part was found, in a few words.
fn found_by(app: &Application, member: &MemberRef) -> &'static str {
    let basis = app
        .members
        .iter()
        .find(|m| &m.at == member)
        .map(|m| m.basis);
    match (basis, member.kind) {
        (Some(Basis::Direct), MemberKind::KargoStage | MemberKind::KargoWarehouse) => {
            "Project namespace"
        }
        (Some(Basis::Direct), MemberKind::ArgoApplication) => match app.evidence.first() {
            Some(Evidence::ArgoApplicationSet { .. }) => "Owner reference",
            _ => "The Application",
        },
        (Some(Basis::Direct), MemberKind::Workload(_)) => "part-of label",
        (Some(Basis::Tracked), _) => "Argo CD, tracked",
        (Some(Basis::ManagedBy), _) => "Argo CD inventory",
        (Some(Basis::NamesProject), _) => "Stage annotation",
        (Some(Basis::SameName), _) => "part-of, same name",
        (Some(Basis::Inferred), _) => "Name, across clusters",
        (Some(Basis::Override), _) => "Your override",
        (None, _) => "",
    }
}

/// A side in words: what was read, where, and what kind of statement.
fn side_words(side: &Side, labels: &Labels) -> String {
    let at = side
        .session
        .as_ref()
        .map(|session| format!(" on {}", labels.of(session)))
        .unwrap_or_default();
    let fact = side.fact.map_or("not read", |fact| fact.word());
    format!("{}{at} ({fact})", side.text)
}

fn row(app: &Application, claim: &Claim, labels: &Labels) -> PartRow {
    let member = &claim.member;
    let cluster = labels.of(&member.session);
    let namespace = member.namespace.clone().unwrap_or_default();
    let found = found_by(app, member);
    let word = link_word(claim.confidence);
    let read_from = claim
        .member_side
        .fact
        .map_or("not read", |fact| fact.word());
    let lower: Vec<SharedString> = claim
        .lower
        .iter()
        .map(|(rule, name)| format!("{} would put it in {name}", rule_label(*rule)).into())
        .collect();
    let key = format!(
        "{}/{}/{}/{}",
        member.session.0,
        member.kind.rank(),
        namespace,
        member.name
    );
    let mut tooltip = format!("{word}: {}", claim.why);
    for lower in &lower {
        tooltip.push_str(&format!(". {lower}"));
    }
    PartRow {
        key: key.into(),
        rank: member.kind.rank(),
        name: member.name.clone().into(),
        cluster: cluster.clone().into(),
        namespace: namespace.clone().into(),
        confidence: claim.confidence,
        found_by: found.into(),
        read_from: read_from.into(),
        label: format!(
            "{} {} on {cluster} · {word} · {found}",
            kind_label(member.kind),
            member.name
        )
        .into(),
        tooltip: tooltip.into(),
        fields: [
            ("Kind", kind_label(member.kind).to_owned()),
            ("Cluster", cluster),
            ("Namespace", namespace),
            ("Found by", found.to_owned()),
        ]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| (label, value.into()))
        .collect(),
        link: vec![
            ("Link", word.into()),
            ("Application", side_words(&claim.app_side, labels).into()),
            ("This part", side_words(&claim.member_side, labels).into()),
            ("Why", claim.why.clone().into()),
        ],
        lower,
    }
}

/// The groups and lines: each kind with parts, in kind order, under its
/// header, and after it each read that may have left parts of that kind
/// out, as a group row of its own.
fn lines(rows: &[PartRow], gaps: &[Gap], labels: &Labels) -> (Vec<GroupLine>, Vec<Entry>) {
    use freshkube_ui::ui::Tone;
    const KINDS: [MemberKind; 6] = [
        MemberKind::KargoStage,
        MemberKind::KargoWarehouse,
        MemberKind::ArgoApplication,
        MemberKind::Workload(freshkube_core::workloads::WorkloadKind::Deployment),
        MemberKind::Workload(freshkube_core::workloads::WorkloadKind::StatefulSet),
        MemberKind::Workload(freshkube_core::workloads::WorkloadKind::DaemonSet),
    ];
    let mut groups = Vec::new();
    let mut lines = Vec::new();
    for kind in KINDS {
        let rank = kind.rank();
        let of_kind: Vec<usize> = (0..rows.len())
            .filter(|&ix| rows[ix].rank == rank)
            .collect();
        if !of_kind.is_empty() {
            let count = of_kind.len();
            groups.push(GroupLine {
                slug: kind_group(kind).to_lowercase().replace(' ', "-"),
                label: kind_group(kind).into(),
                tone: Tone::Outline,
                detail: vec![format!(
                    "{count} {}",
                    if count == 1 { "part" } else { "parts" }
                )],
            });
            lines.push(Entry::Group(groups.len() - 1));
            lines.extend(of_kind.into_iter().map(Entry::Row));
        }
        for gap in gaps.iter().filter(|gap| gap.kinds.first() == Some(&kind)) {
            let cluster = labels.of(&gap.session);
            groups.push(GroupLine {
                slug: format!("missing-{}-{}", rank, gap.session.0),
                label: format!("May be missing on {cluster}").into(),
                tone: Tone::Unknown,
                detail: vec![gap.why.clone()],
            });
            lines.push(Entry::Group(groups.len() - 1));
        }
    }
    (groups, lines)
}
