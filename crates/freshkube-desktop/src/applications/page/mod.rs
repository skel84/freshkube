//! One application's page: its parts as a table page grouped by kind, each
//! with how sure its link to the application is, Confirmed, Claimed or
//! Unknown, from core's `claims`; a read that may have left parts out is a
//! group row with the Unknown ring, never an empty group. The selection's
//! Inspector shows both sides of the link, why, and any lower claim.
//!
//! The list owns the page and hands it each new read; it reads nothing
//! itself. Its breadcrumb, and Escape with nothing selected, go back to the
//! list, which keeps its selection.
//!
//! O, the row menu and the Inspector's button open the selected part in
//! Resources, on its Overview: the list passes its link on to the shell,
//! which opens it as every object link opens. The button and the menu item
//! are greyed out, with why, for a part in a cluster that isn't open; O
//! sends its link anyway, and the shell says it can't open it.
//!
//! A Kargo Stage whose change was read leads to it: F, the row menu's
//! Follow and the Inspector's button ask the list to show the change
//! page on that Stage.
mod table;
#[cfg(test)]
mod tests;
mod view;

use freshkube_core::applications::claims::{Claim, Claims, Gap, Side, Unchecked, claims};
use freshkube_core::applications::{
    Application, ApplicationId, Basis, Evidence, MemberKind, MemberRef, Rule,
};
use freshkube_core::delivery::join::Confidence;
use freshkube_ui::inspector::InspectorSplit;
use freshkube_ui::table::{self as kit, TableState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::Read;
use super::display::{Labels, kind_label, what_it_is};
use super::links::{self, Connections};
use crate::resources::ResourceLink;

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
        Back,
        /// Opens the selected part in Resources.
        OpenPart,
        /// Follows the change the selected Stage carries.
        FollowFreight
    ]
);

pub(crate) fn key_bindings() -> [KeyBinding; 5] {
    [
        KeyBinding::new("down", NextPart, Some(CONTEXT)),
        KeyBinding::new("up", PreviousPart, Some(CONTEXT)),
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("o", OpenPart, Some(CONTEXT)),
        KeyBinding::new("f", FollowFreight, Some(CONTEXT)),
    ]
}

/// What the page asks of the list that owns it.
pub(crate) enum ApplicationEvent {
    /// Back to the list.
    Back,
    /// A part to open in Resources.
    Open(Box<ResourceLink>),
    /// The change a Stage carries, by the Stage's name.
    Follow(SharedString),
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
    /// The Link column's word: [`link_word`], or "By label" for a part
    /// only its label claims.
    pub(super) word: &'static str,
    pub(super) found_by: SharedString,
    pub(super) read_from: SharedString,
    /// The row's accessibility label.
    pub(super) label: SharedString,
    pub(super) tooltip: SharedString,
    /// The Inspector's fields about the part, then about its link.
    pub(super) fields: Vec<(&'static str, SharedString)>,
    pub(super) link: Vec<(&'static str, SharedString)>,
    /// Why the link is what it is, which the lower rules' claims follow.
    pub(super) why: Option<SharedString>,
    /// Lower rules' claims, in words.
    pub(super) lower: Vec<SharedString>,
    /// The link that opens the part in Resources.
    pub(super) open: ResourceLink,
    /// Why the part doesn't open here: its cluster isn't the open one.
    pub(super) closed: Option<SharedString>,
    /// The Open in Resources button's tooltip: where it opens the part, or
    /// why it doesn't.
    pub(super) open_tip: SharedString,
    /// The Freight a Kargo Stage carries, when its change was read.
    pub(super) follows: Option<SharedString>,
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
    /// What the application is, as the header names it beside the name.
    what: SharedString,
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
    /// Whether the Inspector shows the selection. A right-click selects
    /// without opening it, so the table keeps its place under the menu.
    inspect: bool,
    /// The list's last read failed over an earlier one: the banner's text
    /// and label, as the list shows them.
    stale: Option<(SharedString, SharedString)>,
}

impl EventEmitter<ApplicationEvent> for ApplicationPage {}

impl ApplicationPage {
    pub(super) fn new(
        app: &Application,
        read: &Read,
        stale: Option<(SharedString, SharedString)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let split = InspectorSplit::new(PREFIX, cx);
        let mut page = Self {
            id: app.id.clone(),
            name: app.name.clone().into(),
            what: what_it_is(app).into(),
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
            inspect: false,
            stale,
        };
        page.show(app, read);
        page
    }

    pub(crate) fn id(&self) -> &ApplicationId {
        &self.id
    }

    pub(crate) fn name(&self) -> &SharedString {
        &self.name
    }

    /// A new read of the application, keeping the selection while its part
    /// is still there.
    pub(super) fn update(
        &mut self,
        app: &Application,
        read: &Read,
        stale: Option<(SharedString, SharedString)>,
        cx: &mut Context<Self>,
    ) {
        self.stale = stale;
        self.name = app.name.clone().into();
        self.what = what_it_is(app).into();
        self.show(app, read);
        cx.notify();
    }

    /// Derives the rows, groups and lines from the claims.
    fn show(&mut self, app: &Application, read: &Read) {
        let Claims { links, gaps } = claims(&read.derived, app);
        self.rows = links
            .iter()
            .map(|claim| {
                let mut row = row(app, claim, &read.labels, &read.connections);
                if claim.member.kind == MemberKind::KargoStage {
                    row.follows = read
                        .changes
                        .freight_of(&app.name, &claim.member.name)
                        .map(SharedString::from);
                }
                row
            })
            .collect();
        (self.groups, self.lines) = lines(&self.rows, &gaps, &read.labels);
        (self.columns, self.width) = table::columns(&self.rows);
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !self.rows.iter().any(|row| &row.key == key))
        {
            self.selected = None;
            self.inspect = false;
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
        self.inspect = true;
        kit::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(key) = kit::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    /// O: the selected part in Resources, through the shell, which says so
    /// when its cluster isn't open.
    fn open_part(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row() {
            cx.emit(ApplicationEvent::Open(Box::new(row.open.clone())));
        }
    }

    /// F: the change the selected Stage carries, when one was read.
    fn follow(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row().filter(|row| row.follows.is_some()) {
            cx.emit(ApplicationEvent::Follow(row.name.clone()));
        }
    }

    /// Escape: the selection first, then back to the list.
    fn back(&mut self, cx: &mut Context<Self>) {
        self.inspect = false;
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
                    format!("{} {}", row.name, row.word)
                }
            })
            .collect()
    }

    pub(crate) fn what(&self) -> &SharedString {
        &self.what
    }

    pub(crate) fn selected(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    pub(crate) fn inspects(&self) -> bool {
        self.inspect
    }
}

pub(super) fn link_word(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Confirmed => "Confirmed",
        Confidence::Claimed => "Claimed",
        Confidence::Unknown => "Unknown",
    }
}

/// The link's glyph: Good for Confirmed, the Info dot for Claimed, and the
/// dashed ring for Unknown.
pub(super) fn link_tone(confidence: Confidence) -> freshkube_ui::ui::Tone {
    use freshkube_ui::ui::Tone;
    match confidence {
        Confidence::Confirmed => Tone::Good,
        Confidence::Claimed => Tone::Info,
        Confidence::Unknown => Tone::Unknown,
    }
}

/// The word for a part's link: "By label" when only its
/// `app.kubernetes.io/part-of` label claims it.
fn row_word(app: &Application, claim: &Claim) -> &'static str {
    let basis = app
        .members
        .iter()
        .find(|m| m.at == claim.member)
        .map(|m| m.basis);
    match (claim.confidence, basis, claim.member.kind) {
        (Confidence::Claimed, Some(Basis::SameName), _)
        | (Confidence::Claimed, Some(Basis::Direct), MemberKind::Workload(_)) => "By label",
        (confidence, ..) => link_word(confidence),
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

/// How Argo CD was read in the part's cluster, naming the cluster and
/// the namespaces.
fn unchecked_words(unchecked: &Unchecked, cluster: &str) -> String {
    match unchecked {
        Unchecked::NotRead => format!("Argo CD not read on {cluster}"),
        Unchecked::Namespaces(namespaces) => {
            format!(
                "Argo CD read in {} only on {cluster}",
                namespaces.join(", ")
            )
        }
        Unchecked::NotFound(namespace) => {
            format!("Argo CD not found on {cluster}: {namespace} holds no Applications")
        }
        Unchecked::Capped(read) => {
            format!("Argo CD read in part on {cluster}: stopped after {read}")
        }
    }
}

/// A lower rule's claim, as a sentence: where that rule would put the part.
fn lower_words(rule: Rule, name: &str) -> String {
    match rule {
        Rule::Kargo => format!("Kargo Project {name} would also claim it."),
        Rule::ArgoCd => format!("Argo CD would also put it in {name}."),
        Rule::PartOf => format!("Its app.kubernetes.io/part-of label puts it in {name}."),
        Rule::Manual => format!("Your override puts it in {name}."),
    }
}

fn row(app: &Application, claim: &Claim, labels: &Labels, connections: &Connections) -> PartRow {
    let member = &claim.member;
    let cluster = labels.of(&member.session);
    let namespace = member.namespace.clone().unwrap_or_default();
    let found = found_by(app, member);
    let word = row_word(app, claim);
    // What wasn't checked, and where, says more than the label's fact.
    let unchecked = claim
        .unchecked
        .as_ref()
        .map(|unchecked| unchecked_words(unchecked, &cluster));
    let read_from = unchecked.clone().unwrap_or_else(|| {
        claim
            .member_side
            .fact
            .map_or("not read", |fact| fact.word())
            .to_owned()
    });
    let lower: Vec<SharedString> = claim
        .lower
        .iter()
        .map(|(rule, name)| lower_words(*rule, name).into())
        .collect();
    let key = format!(
        "{}/{}/{}/{}",
        member.session.0,
        member.kind.rank(),
        namespace,
        member.name
    );
    let mut tooltip = format!("{word}: {}.", claim.why.trim_end_matches('.'));
    for lower in &lower {
        tooltip.push(' ');
        tooltip.push_str(lower);
    }
    let closed: Option<SharedString> = (!connections.opens(&member.session)).then(|| {
        format!("{cluster} isn't the open cluster, so its objects don't open in Resources").into()
    });
    PartRow {
        key: key.into(),
        rank: member.kind.rank(),
        name: member.name.clone().into(),
        cluster: cluster.clone().into(),
        namespace: namespace.clone().into(),
        confidence: claim.confidence,
        word,
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
            ("Cluster", cluster.clone()),
            ("Namespace", namespace),
            ("Found by", found.to_owned()),
        ]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| (label, value.into()))
        .collect(),
        link: [
            Some(("Link", word.into())),
            Some(("Application", side_words(&claim.app_side, labels).into())),
            Some(("This part", side_words(&claim.member_side, labels).into())),
            unchecked.map(|unchecked| ("Not checked", unchecked.into())),
        ]
        .into_iter()
        .flatten()
        .collect(),
        why: (!claim.why.is_empty())
            .then(|| format!("{}.", claim.why.trim_end_matches('.')).into()),
        lower,
        open: links::link(member, connections),
        open_tip: closed
            .clone()
            .unwrap_or_else(|| format!("Open {} on {cluster}, in Resources", member.name).into()),
        closed,
        follows: None,
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
