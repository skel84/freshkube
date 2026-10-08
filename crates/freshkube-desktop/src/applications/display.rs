//! What the page shows of one derivation, worked out when it arrives so
//! `render` only reads it: a row per application, the body's state, the
//! banners for sources that may have left applications out, and the
//! legend's lines for sources that aren't served.
//!
//! An unread source is never "no applications". With none found, a refused
//! read is the refused state and a failed one the failed state; only when
//! every source answered (or isn't served) is it the empty state.
use std::collections::{BTreeMap, BTreeSet};

use freshkube_core::applications::read::{ARGOCD, MAX_ARGO_NAMESPACES, PART_OF};
use freshkube_core::applications::{
    Application, ArgoFound, Coverage, CoverageState, Derived, Evidence, Member, MemberKind,
    MemberRef, Note, Rule, SessionKey, SourceKind,
};
use freshkube_core::workloads::WorkloadKind;
use gpui_kit::SharedString;

/// The rules in precedence order, as the table groups them.
pub(super) const RULES: [Rule; 4] = [Rule::Kargo, Rule::ArgoCd, Rule::PartOf, Rule::Manual];

pub(super) fn rule_index(rule: Rule) -> usize {
    match rule {
        Rule::Kargo => 0,
        Rule::ArgoCd => 1,
        Rule::PartOf => 2,
        Rule::Manual => 3,
    }
}

/// A group row's label: what found the applications under it.
pub(super) fn rule_label(rule: Rule) -> &'static str {
    match rule {
        Rule::Kargo => "Kargo Projects",
        Rule::ArgoCd => "Argo CD",
        Rule::PartOf => "part-of label",
        Rule::Manual => "Your overrides",
    }
}

/// How completely an application is known, which its glyph and the
/// header's chips show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    /// A source it depends on wasn't read in full: parts may be missing.
    Incomplete,
    /// Argo CD was read only in the namespaces it runs in, where it keeps
    /// Applications by default: a fact to note rather than a gap. Found by
    /// Argo CD there, or by Kargo in a cluster whose Argo CD Applications,
    /// which its Stages promote to, weren't all read.
    Scoped,
    /// Read in full, with something to know: a lower claim, a destination
    /// in another cluster, a join by name.
    Notes,
    /// Read in full, nothing to add.
    Read,
}

pub(super) const MARKS: [Mark; 4] = [Mark::Incomplete, Mark::Scoped, Mark::Notes, Mark::Read];

impl Mark {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Incomplete => 0,
            Self::Scoped => 1,
            Self::Notes => 2,
            Self::Read => 3,
        }
    }

    /// What its chip counts, short enough to show beside the count; the
    /// display has the whole words for the tooltip.
    pub(super) fn short(self) -> &'static str {
        match self {
            Self::Incomplete => "may be incomplete",
            Self::Scoped => "Argo CD in part",
            Self::Notes => "with notes",
            Self::Read => "read in full",
        }
    }

    pub(super) fn slug(self) -> &'static str {
        match self {
            Self::Incomplete => "incomplete",
            Self::Scoped => "scoped",
            Self::Notes => "notes",
            Self::Read => "read",
        }
    }

    /// The mark in words, for a chip's tooltip and accessibility label.
    /// Scoped's names no namespace, since it counts every cluster's; a
    /// row's own words do.
    pub(super) fn what(self) -> &'static str {
        match self {
            Self::Incomplete => "may be incomplete: a source it depends on wasn't read in full",
            Self::Scoped => "Argo CD read only where it runs, with notes",
            Self::Notes => "read in full, with notes",
            Self::Read => "read in full",
        }
    }
}

/// What the clusters are called on the page: their context, or in example
/// data the acme workspace's names.
#[derive(Clone, Debug, Default)]
pub(crate) struct Labels(BTreeMap<SessionKey, String>);

impl Labels {
    pub(crate) fn new(labels: impl IntoIterator<Item = (SessionKey, String)>) -> Self {
        Self(labels.into_iter().collect())
    }

    pub(super) fn of(&self, session: &SessionKey) -> String {
        self.0
            .get(session)
            .cloned()
            .unwrap_or_else(|| session.0.clone())
    }

    /// Several clusters, as one short list: two named, then a count.
    fn list<'a>(&self, sessions: impl IntoIterator<Item = &'a SessionKey>) -> String {
        let names: BTreeSet<String> = sessions.into_iter().map(|s| self.of(s)).collect();
        let names: Vec<String> = names.into_iter().collect();
        match names.len() {
            0..=2 => names.join(", "),
            n => format!("{}, {} +{}", names[0], names[1], n - 2),
        }
    }

    fn all<'a>(&self, sessions: impl IntoIterator<Item = &'a SessionKey>) -> String {
        let names: BTreeSet<String> = sessions.into_iter().map(|s| self.of(s)).collect();
        names.into_iter().collect::<Vec<_>>().join(", ")
    }
}

/// One application as the table and the Inspector show it.
#[derive(Clone, Debug)]
pub(crate) struct AppRow {
    pub(super) key: SharedString,
    pub(super) name: SharedString,
    pub(super) rule: Rule,
    pub(super) mark: Mark,
    /// The mark in this row's words: Scoped names its namespaces.
    pub(super) mark_words: SharedString,
    pub(super) found_by: SharedString,
    pub(super) parts: SharedString,
    pub(super) clusters: SharedString,
    /// The first note, and how many more.
    pub(super) note: SharedString,
    pub(super) tooltip: SharedString,
    /// Lowercased name, id, evidence and clusters, for the filter.
    pub(super) query: String,
    /// The Inspector's fields and notes.
    pub(super) fields: Vec<(&'static str, SharedString)>,
    pub(super) notes: Vec<SharedString>,
}

/// What the page's body is: the table, or a state in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Body {
    Table,
    /// Every source answered or isn't served, and none found anything.
    Empty {
        title: SharedString,
        description: SharedString,
    },
    /// No applications, and a source refused: nothing is known missing.
    Refused {
        title: SharedString,
        description: SharedString,
        reason: SharedString,
    },
    /// No applications, and a source failed: nothing is known missing.
    Failed {
        title: SharedString,
        description: SharedString,
        reason: SharedString,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Display {
    pub(super) rows: Vec<AppRow>,
    pub(super) body: Body,
    /// Sources that may have left applications out, one line each, as the
    /// banner says them, and its label.
    pub(super) missing_text: Option<(SharedString, SharedString)>,
    /// Sources that aren't served, or read in one namespace, as one line
    /// under the header: facts, not warnings.
    pub(super) legend_text: Option<SharedString>,
    /// Applications per mark, before any filter.
    pub(super) marks: [usize; 4],
}

impl Default for Display {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            body: Body::Table,
            missing_text: None,
            legend_text: None,
            marks: [0; 4],
        }
    }
}

/// The words with their first letter a capital, as a label starts.
pub(super) fn capital(words: &str) -> String {
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn source_words(source: SourceKind) -> &'static str {
    match source {
        SourceKind::KargoProjects => "Kargo Projects",
        SourceKind::KargoStages => "Kargo Stages",
        SourceKind::KargoWarehouses => "Kargo Warehouses",
        SourceKind::ArgoApplications => "Argo CD Applications",
        SourceKind::ArgoApplicationSets => "Argo CD ApplicationSets",
        SourceKind::Workloads => "labelled workloads",
    }
}

/// What an application is made of, as a caption: `Stage`, `Deployment`.
pub(super) fn kind_label(kind: MemberKind) -> &'static str {
    match kind {
        MemberKind::KargoStage => "Stage",
        MemberKind::KargoWarehouse => "Warehouse",
        MemberKind::ArgoApplication => "Argo CD Application",
        MemberKind::Workload(WorkloadKind::Deployment) => "Deployment",
        MemberKind::Workload(WorkloadKind::StatefulSet) => "StatefulSet",
        MemberKind::Workload(WorkloadKind::DaemonSet) => "DaemonSet",
    }
}

fn kind_plural(kind: MemberKind, count: usize) -> String {
    let (one, many) = match kind {
        MemberKind::KargoStage => ("stage", "stages"),
        MemberKind::KargoWarehouse => ("warehouse", "warehouses"),
        MemberKind::ArgoApplication => ("Argo CD app", "Argo CD apps"),
        MemberKind::Workload(WorkloadKind::Deployment) => ("deployment", "deployments"),
        MemberKind::Workload(WorkloadKind::StatefulSet) => ("statefulset", "statefulsets"),
        MemberKind::Workload(WorkloadKind::DaemonSet) => ("daemonset", "daemonsets"),
    };
    plural(count, one, many)
}

fn member_words(member: &MemberRef, labels: &Labels) -> String {
    let name = match &member.namespace {
        Some(namespace) => format!("{namespace}/{}", member.name),
        None => member.name.clone(),
    };
    format!(
        "{} {name} on {}",
        kind_label(member.kind),
        labels.of(&member.session)
    )
}

fn rule_words(rule: Rule) -> &'static str {
    match rule {
        Rule::Kargo => "Kargo",
        Rule::ArgoCd => "Argo CD",
        Rule::PartOf => "part-of",
        Rule::Manual => "override",
    }
}

/// A note in words, for the Inspector and the Notes column.
pub(super) fn note_words(note: &Note, labels: &Labels) -> String {
    match note {
        Note::LowerClaim { member, rule, name } if *name == member.name => format!(
            "{} is also claimed by the {} rule, under its own name",
            member_words(member, labels),
            rule_words(*rule)
        ),
        Note::LowerClaim { member, rule, name } => format!(
            "{} is also claimed by the {} rule, as “{name}”",
            member_words(member, labels),
            rule_words(*rule)
        ),
        Note::UnmappedDestination { member } => format!(
            "{} deploys to another cluster, not mapped yet",
            member_words(member, labels)
        ),
        Note::Merged { from } => format!(
            "Merged from {}",
            from.iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Note::MembersUnknown { session, why } => {
            format!("Parts on {} may be missing: {why}", labels.of(session))
        }
        Note::ProjectNotRead {
            member,
            project,
            why,
        } => format!(
            "{} names Kargo Project {project}, which wasn't read: {why}",
            member_words(member, labels)
        ),
        Note::JoinedAcrossSessions { name, sessions } => format!(
            "Joined by the name “{name}” across {}: an inference",
            labels.all(sessions)
        ),
        Note::ManagerUnknown { member } => format!(
            "{} may be managed by an Argo CD Application, but that cluster's Applications weren't read in full",
            member_words(member, labels)
        ),
    }
}

/// Whether a note says something may be missing, not only something to know.
fn note_is_unknown(note: &Note) -> bool {
    matches!(
        note,
        Note::MembersUnknown { .. } | Note::ProjectNotRead { .. } | Note::ManagerUnknown { .. }
    )
}

fn evidence_words(evidence: &Evidence, labels: &Labels) -> Option<String> {
    Some(match evidence {
        Evidence::KargoProject { session, project } => {
            format!("Kargo Project {project} on {}", labels.of(session))
        }
        Evidence::ArgoApplicationSet {
            session,
            namespace,
            name,
            read,
        } => {
            let how = if *read {
                ""
            } else {
                ", named by an Application's owner"
            };
            format!(
                "Argo CD ApplicationSet {namespace}/{name} on {}{how}",
                labels.of(session)
            )
        }
        Evidence::ArgoApplication {
            session,
            namespace,
            name,
        } => format!(
            "Argo CD Application {namespace}/{name} on {}",
            labels.of(session)
        ),
        Evidence::PartOfLabel { value } => format!("app.kubernetes.io/part-of={value}"),
        Evidence::Override(what) => format!("Your override: {what}"),
    })
}

fn sessions_of(app: &Application) -> BTreeSet<SessionKey> {
    let mut sessions: BTreeSet<SessionKey> =
        app.members.iter().map(|m| m.at.session.clone()).collect();
    for evidence in &app.evidence {
        match evidence {
            Evidence::KargoProject { session, .. }
            | Evidence::ArgoApplicationSet { session, .. }
            | Evidence::ArgoApplication { session, .. } => {
                sessions.insert(session.clone());
            }
            Evidence::PartOfLabel { .. } | Evidence::Override(_) => {}
        }
    }
    sessions
}

fn parts_words(members: &[Member]) -> String {
    let mut counts: Vec<(MemberKind, usize)> = Vec::new();
    for member in members {
        match counts.iter_mut().find(|(kind, _)| *kind == member.at.kind) {
            Some((_, count)) => *count += 1,
            None => counts.push((member.at.kind, 1)),
        }
    }
    if counts.is_empty() {
        return "none read".to_owned();
    }
    counts
        .iter()
        .map(|(kind, count)| kind_plural(*kind, *count))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Where Argo CD was read and how its namespaces were found, as one
/// phrase: "read in gitops only, where Argo CD's workloads run".
pub(super) fn scope_words(namespaces: &[String], found: &ArgoFound) -> String {
    let read = namespaces.join(", ");
    match found {
        ArgoFound::Labelled { skipped } if skipped.is_empty() => {
            format!("read in {read} only, where Argo CD's workloads run")
        }
        ArgoFound::Labelled { skipped } => format!(
            "read in {read} only, where Argo CD's workloads run; {} past the cap of {MAX_ARGO_NAMESPACES} not read",
            skipped.join(", ")
        ),
        ArgoFound::Default => {
            format!("read in {read} only, its default: no workload is labelled {PART_OF}={ARGOCD}")
        }
    }
}

/// Why a cluster's Argo CD Applications aren't all known, as a phrase.
fn argo_short_words(state: &CoverageState) -> Option<String> {
    Some(match state {
        CoverageState::NamespaceOnly { namespaces, found } => scope_words(namespaces, found),
        CoverageState::NamespaceNotFound(_) => "Argo CD's namespace wasn't found".to_owned(),
        CoverageState::Capped(read) => format!("stopped after {read}"),
        CoverageState::Refused(why) => format!("refused ({why})"),
        CoverageState::Unreadable(why) => format!("couldn't be read ({why})"),
        CoverageState::Read | CoverageState::NotInstalled(_) => return None,
    })
}

/// Where each rule's sources may have been left short, where Argo CD was
/// read only in the namespaces it runs in, which is a fact to note rather
/// than a gap, and where its Applications aren't all known for any reason.
#[derive(Default)]
struct Short {
    unread: BTreeMap<Rule, BTreeSet<SessionKey>>,
    /// Argo CD read in its namespaces only, in these words.
    namespace_only: BTreeMap<SessionKey, String>,
    /// Argo CD Applications not all known, and why, in words.
    argo_partial: BTreeMap<SessionKey, String>,
}

impl Short {
    fn new(coverage: &[Coverage]) -> Self {
        let mut short = Self::default();
        for c in coverage {
            if c.source == SourceKind::ArgoApplications
                && let Some(words) = argo_short_words(&c.state)
            {
                short.argo_partial.insert(c.session.clone(), words);
            }
            match &c.state {
                CoverageState::NamespaceOnly { namespaces, found } => {
                    short
                        .namespace_only
                        .insert(c.session.clone(), scope_words(namespaces, found));
                }
                state if state.is_unknown() => {
                    short
                        .unread
                        .entry(c.source.rule())
                        .or_default()
                        .insert(c.session.clone());
                }
                _ => {}
            }
        }
        short
    }
}

/// Notes that differ only in the part they name, said once: "3 parts on
/// core-fra may be managed …: shop/web, shop/worker, shop/db". The order
/// of first appearance is kept.
fn grouped_notes(notes: &[Note], labels: &Labels) -> Vec<String> {
    let mut groups: Vec<(Option<String>, String, Vec<String>)> = Vec::new();
    for note in notes {
        let Some((member, rest)) = note_about(note, labels) else {
            groups.push((None, note_words(note, labels), Vec::new()));
            continue;
        };
        let key = format!("{} {rest}", labels.of(&member.session));
        let name = match &member.namespace {
            Some(namespace) => format!("{namespace}/{}", member.name),
            None => member.name.clone(),
        };
        match groups.iter_mut().find(|(k, ..)| k.as_ref() == Some(&key)) {
            Some((_, _, names)) => names.push(name),
            None => groups.push((Some(key), note_words(note, labels), vec![name])),
        }
    }
    const NAMED: usize = 6;
    groups
        .into_iter()
        .map(|(key, one, names)| match (key, names.len()) {
            (Some(key), n) if n > 1 => {
                let shown = names
                    .iter()
                    .take(NAMED)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                let more = match n.saturating_sub(NAMED) {
                    0 => String::new(),
                    more => format!(" +{more}"),
                };
                format!("{n} parts on {}: {shown}{more}", plural_verb(&key))
            }
            _ => one,
        })
        .collect()
}

/// "core-fra is also claimed …" as several parts say it.
fn plural_verb(key: &str) -> String {
    let (cluster, rest) = key.split_once(' ').unwrap_or((key, ""));
    let rest = [
        ("is ", "are "),
        ("deploys ", "deploy "),
        ("names ", "name "),
    ]
    .into_iter()
    .find_map(|(one, many)| rest.strip_prefix(one).map(|tail| format!("{many}{tail}")))
    .unwrap_or_else(|| rest.to_owned())
    .replace("its own name", "their own names");
    format!("{cluster} {rest}")
}

/// A note about one part, as the part and the words after it.
fn note_about<'a>(note: &'a Note, labels: &Labels) -> Option<(&'a MemberRef, String)> {
    let words = note_words(note, labels);
    let member = match note {
        Note::LowerClaim { member, .. }
        | Note::UnmappedDestination { member }
        | Note::ProjectNotRead { member, .. }
        | Note::ManagerUnknown { member } => member,
        _ => return None,
    };
    let lead = member_words(member, labels);
    Some((member, words.strip_prefix(&lead)?.trim_start().to_owned()))
}

fn row(app: &Application, short: &Short, labels: &Labels) -> AppRow {
    let sessions = sessions_of(app);
    let found: Vec<String> = app
        .evidence
        .iter()
        .filter_map(|evidence| evidence_words(evidence, labels))
        .collect();
    let mut notes = grouped_notes(&app.notes, labels);
    let mut scope: Vec<String> = Vec::new();
    if app.rule == Rule::ArgoCd {
        for (session, words) in &short.namespace_only {
            if sessions.contains(session) {
                let words = format!("Argo CD on {} was {words}", labels.of(session));
                notes.push(format!(
                    "{words}; Applications in other namespaces aren't listed"
                ));
                scope.push(words);
            }
        }
    }
    // Kargo's Stages promote to Argo CD Applications, so a Project in a
    // cluster whose Applications aren't all known is never read in full.
    if app.rule == Rule::Kargo {
        for (session, words) in &short.argo_partial {
            if sessions.contains(session) {
                let words = format!("Argo CD on {} was {words}", labels.of(session));
                notes.push(format!(
                    "{words}; Applications its Stages promote to may not all be listed"
                ));
                scope.push(words);
            }
        }
    }
    // A source of the application's own rule that may be short in a cluster
    // it spans leaves its parts unknown, as do the notes that say so.
    let short_here = short
        .unread
        .get(&app.rule)
        .is_some_and(|short| short.iter().any(|s| sessions.contains(s)));
    let scoped = !scope.is_empty();
    let mark = if short_here || app.notes.iter().any(note_is_unknown) {
        Mark::Incomplete
    } else if scoped {
        Mark::Scoped
    } else if notes.is_empty() {
        Mark::Read
    } else {
        Mark::Notes
    };
    let found_by = found.first().cloned().unwrap_or_default();
    let found_by = match found.len() {
        0 | 1 => found_by,
        n => format!("{found_by} +{}", n - 1),
    };
    let parts = parts_words(&app.members);
    let clusters = labels.list(&sessions);
    let note = match notes.len() {
        0 => String::new(),
        1 => notes[0].clone(),
        n => format!("{} (+{} more)", notes[0], n - 1),
    };
    let mut fields: Vec<(&'static str, SharedString)> = vec![
        ("Found by", found.join("\n").into()),
        ("Id", app.id.as_str().to_owned().into()),
        ("Parts", parts.clone().into()),
        ("Clusters", labels.all(&sessions).into()),
    ];
    if mark == Mark::Incomplete {
        fields.push((
            "Read",
            "Not in full: parts may be missing, which says nothing about whether they exist".into(),
        ));
    }
    let mark_words = match mark {
        Mark::Scoped => scope.join("; "),
        mark => mark.what().to_owned(),
    };
    AppRow {
        key: app.id.as_str().to_owned().into(),
        name: app.name.clone().into(),
        rule: app.rule,
        mark,
        tooltip: format!("{} · {}", app.name, capital(&mark_words)).into(),
        mark_words: capital(&mark_words).into(),
        query: format!(
            "{} {} {} {}",
            app.name,
            app.id.as_str(),
            found.join(" "),
            clusters
        )
        .to_lowercase(),
        found_by: found_by.into(),
        parts: parts.into(),
        clusters: clusters.into(),
        note: note.into(),
        fields,
        notes: notes.into_iter().map(Into::into).collect(),
    }
}

/// One line per cluster and source that may have left applications out.
fn missing_lines(coverage: &[Coverage], labels: &Labels) -> Vec<SharedString> {
    let mut lines = Vec::new();
    for c in coverage {
        let source = source_words(c.source);
        let cluster = labels.of(&c.session);
        let project = c
            .project
            .as_ref()
            .map(|p| format!(" of Project {p}"))
            .unwrap_or_default();
        let rule = rule_words(c.source.rule());
        let line = match &c.state {
            CoverageState::Refused(why) => {
                format!(
                    "{source}{project} were refused on {cluster} ({why}); {rule} applications may be missing"
                )
            }
            CoverageState::Unreadable(why) => {
                format!(
                    "{source}{project} couldn't be read on {cluster} ({why}); {rule} applications may be missing"
                )
            }
            CoverageState::Capped(read) => format!(
                "{source}{project} on {cluster} stopped after {read}; {rule} applications may be missing"
            ),
            // Said once, for the Applications.
            CoverageState::NamespaceNotFound(_) if c.source == SourceKind::ArgoApplications => {
                format!(
                    "Argo CD's namespace wasn't found on {cluster}; Applications elsewhere aren't listed"
                )
            }
            CoverageState::Read
            | CoverageState::NamespaceOnly { .. }
            | CoverageState::NamespaceNotFound(_)
            | CoverageState::NotInstalled(_) => continue,
        };
        lines.push(line.into());
    }
    lines
}

/// Facts about what was read: what isn't served, and where Argo CD was
/// read only in one namespace.
fn legend_lines(coverage: &[Coverage], labels: &Labels) -> Vec<SharedString> {
    let mut kargo = BTreeSet::new();
    let mut argo = BTreeSet::new();
    let mut namespace_only: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in coverage {
        match (&c.state, c.source.rule()) {
            (CoverageState::NotInstalled(_), Rule::Kargo) => {
                kargo.insert(labels.of(&c.session));
            }
            (CoverageState::NotInstalled(_), Rule::ArgoCd) => {
                argo.insert(labels.of(&c.session));
            }
            (CoverageState::NamespaceOnly { namespaces, found }, _) => {
                namespace_only
                    .entry(scope_words(namespaces, found))
                    .or_default()
                    .insert(labels.of(&c.session));
            }
            _ => {}
        }
    }
    let on = |clusters: &BTreeSet<String>| match clusters.len() {
        1 => clusters.iter().next().cloned().unwrap_or_default(),
        n => plural(n, "cluster", "clusters"),
    };
    let mut lines: Vec<SharedString> = Vec::new();
    if !kargo.is_empty() {
        lines.push(format!("Kargo isn't served on {}", on(&kargo)).into());
    }
    if !argo.is_empty() {
        lines.push(format!("Argo CD isn't served on {}", on(&argo)).into());
    }
    for (words, clusters) in namespace_only {
        lines.push(
            format!(
                "Argo CD on {} was {words}; Applications in other namespaces aren't listed",
                on(&clusters)
            )
            .into(),
        );
    }
    lines
}

/// The body when nothing was found: refused or failed before empty, since
/// an unread source may hold applications.
fn body_without_applications(coverage: &[Coverage], labels: &Labels) -> Body {
    let refused: Vec<&Coverage> = coverage
        .iter()
        .filter(|c| matches!(c.state, CoverageState::Refused(_)))
        .collect();
    let failed: Vec<&Coverage> = coverage
        .iter()
        .filter(|c| matches!(c.state, CoverageState::Unreadable(_)))
        .collect();
    let sources = |list: &[&Coverage]| {
        let words: BTreeSet<&str> = list.iter().map(|c| source_words(c.source)).collect();
        words.into_iter().collect::<Vec<_>>().join(", ")
    };
    let clusters = |list: &[&Coverage]| labels.all(list.iter().map(|c| &c.session));
    let reason = |list: &[&Coverage]| -> SharedString {
        match &list[0].state {
            CoverageState::Refused(why) | CoverageState::Unreadable(why) => why.clone().into(),
            _ => SharedString::default(),
        }
    };
    if !refused.is_empty() {
        return Body::Refused {
            title: format!("Not permitted to list {}", sources(&refused)).into(),
            description: format!(
                "The identity of {} may not list {}. That says nothing about whether any applications exist.",
                clusters(&refused),
                sources(&refused)
            )
            .into(),
            reason: reason(&refused),
        };
    }
    if !failed.is_empty() {
        return Body::Failed {
            title: format!("Couldn't read {}", sources(&failed)).into(),
            description: format!(
                "Nothing is known yet on {}, so no application is shown as missing. Refresh starts over.",
                clusters(&failed)
            )
            .into(),
            reason: reason(&failed),
        };
    }
    let sessions: BTreeSet<&SessionKey> = coverage.iter().map(|c| &c.session).collect();
    let mut description = format!(
        "Looked for Kargo Projects, Argo CD ApplicationSets and Applications where Argo CD runs, and Deployments, StatefulSets and DaemonSets labelled {PART_OF}."
    );
    let not_found = coverage.iter().filter(|c| {
        c.source == SourceKind::ArgoApplications
            && matches!(c.state, CoverageState::NamespaceNotFound(_))
    });
    let not_found = labels.all(not_found.map(|c| &c.session));
    if !not_found.is_empty() {
        description.push_str(&format!(
            " Argo CD's namespace wasn't found on {not_found}; Applications elsewhere aren't listed."
        ));
    }
    for line in legend_lines(coverage, labels) {
        description.push(' ');
        description.push_str(&line);
        description.push('.');
    }
    Body::Empty {
        title: format!(
            "No applications found in what was read on {}",
            labels.all(sessions)
        )
        .into(),
        description: description.into(),
    }
}

impl Display {
    pub(super) fn new(derived: &Derived, labels: &Labels) -> Self {
        let short = Short::new(&derived.coverage);
        let mut rows: Vec<AppRow> = derived
            .applications
            .iter()
            .map(|app| row(app, &short, labels))
            .collect();
        // Grouped by rule, in precedence order; by name within, as derived.
        rows.sort_by_key(|row| rule_index(row.rule));
        let mut marks = [0; 4];
        for row in &rows {
            marks[row.mark.index()] += 1;
        }
        let body = if rows.is_empty() {
            body_without_applications(&derived.coverage, labels)
        } else {
            Body::Table
        };
        // A refused or failed state already says what wasn't read.
        let missing = if matches!(body, Body::Refused { .. } | Body::Failed { .. }) {
            Vec::new()
        } else {
            missing_lines(&derived.coverage, labels)
        };
        let missing_text = (!missing.is_empty()).then(|| {
            let text = missing
                .iter()
                .map(|line| line.as_ref())
                .collect::<Vec<_>>()
                .join(". ");
            let label = format!("May be missing: {text}");
            (text.into(), label.into())
        });
        let legend = legend_lines(&derived.coverage, labels);
        let legend_text = (!legend.is_empty()).then(|| {
            legend
                .iter()
                .map(|line| line.as_ref())
                .collect::<Vec<_>>()
                .join(" · ")
                .into()
        });
        Self {
            rows,
            body,
            missing_text,
            legend_text,
            marks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freshkube_core::applications::{Inputs, Override, SessionInputs, derive, example};
    use freshkube_core::delivery::source::{Source, Truncation};

    fn labels() -> Labels {
        Labels::default()
    }

    fn shown(inputs: &Inputs) -> Display {
        Display::new(&derive(inputs, &Override::default()), &labels())
    }

    fn one(session: SessionInputs) -> Inputs {
        Inputs {
            sessions: vec![session],
            stage_naming: None,
        }
    }

    fn empty_session() -> SessionInputs {
        SessionInputs {
            key: SessionKey::new("acme-solo"),
            kargo: Source::NotInstalled("kargo.akuity.io is not served".into()),
            argo_scope: freshkube_core::applications::ArgoScope::AllNamespaces,
            argo_applications: Source::NotInstalled("argoproj.io is not served".into()),
            argo_application_sets: Source::NotInstalled("argoproj.io is not served".into()),
            workloads: Source::Read(Vec::new()),
        }
    }

    #[test]
    fn acme_groups_by_rule_in_precedence_order() {
        let display = shown(&example::acme());
        let order: Vec<(&str, Rule)> = display
            .rows
            .iter()
            .map(|row| (row.name.as_ref(), row.rule))
            .collect();
        assert_eq!(
            order,
            [
                ("cart", Rule::Kargo),
                ("checkout", Rule::Kargo),
                ("catalog", Rule::ArgoCd),
                ("status-page", Rule::ArgoCd),
                ("loyalty", Rule::PartOf),
            ]
        );
        assert_eq!(display.body, Body::Table);
        assert!(display.missing_text.is_none());
        assert_eq!(
            display.legend_text.as_ref().map(|t| t.as_ref()),
            Some("Kargo isn't served on 5 clusters · Argo CD isn't served on 5 clusters")
        );
    }

    #[test]
    fn nothing_found_where_everything_answered_is_empty() {
        let display = shown(&one(empty_session()));
        let Body::Empty { title, description } = &display.body else {
            panic!("{:?}", display.body);
        };
        assert_eq!(
            title.as_ref(),
            "No applications found in what was read on acme-solo"
        );
        assert!(description.contains("Kargo isn't served on acme-solo"));
    }

    #[test]
    fn a_refused_read_with_nothing_found_is_never_empty() {
        let mut session = empty_session();
        session.workloads = Source::Refused("deployments.apps is forbidden".into());
        let display = shown(&one(session));
        let Body::Refused { title, reason, .. } = &display.body else {
            panic!("{:?}", display.body);
        };
        assert_eq!(title.as_ref(), "Not permitted to list labelled workloads");
        assert_eq!(reason.as_ref(), "deployments.apps is forbidden");
        assert!(display.missing_text.is_none(), "the state says it already");
    }

    #[test]
    fn a_failed_read_with_nothing_found_is_the_failed_state() {
        let mut session = empty_session();
        session.workloads = Source::Unreadable("connection reset".into());
        let display = shown(&one(session));
        assert!(
            matches!(display.body, Body::Failed { .. }),
            "{:?}",
            display.body
        );
    }

    #[test]
    fn a_refused_source_beside_found_applications_is_a_missing_line() {
        let mut inputs = example::acme();
        inputs.sessions[0].kargo = Source::Refused("projects is forbidden".into());
        let display = shown(&inputs);
        assert_eq!(display.body, Body::Table);
        let (text, label) = display.missing_text.clone().unwrap();
        assert!(
            text.starts_with("Kargo Projects were refused on core-fra") && !text.contains(". "),
            "one line: {text}"
        );
        assert_eq!(label.as_ref(), format!("May be missing: {text}"));
        assert!(display.rows.iter().all(|row| row.rule != Rule::Kargo));
    }

    #[test]
    fn a_capped_source_marks_its_applications_incomplete() {
        let mut inputs = example::acme();
        if let Source::Read(apps) = inputs.sessions[0].argo_applications.clone() {
            inputs.sessions[0].argo_applications = Source::Capped(apps, Truncation { read: 500 });
        }
        let display = shown(&inputs);
        let (text, _) = display.missing_text.clone().unwrap();
        assert!(text.contains("stopped after 500"), "{text}");
        let catalog = display.rows.iter().find(|r| r.name == "catalog").unwrap();
        assert_eq!(catalog.mark, Mark::Incomplete);
        let loyalty = display.rows.iter().find(|r| r.name == "loyalty").unwrap();
        assert_eq!(
            loyalty.mark,
            Mark::Read,
            "dev-fra's labels were read in full"
        );
    }

    #[test]
    fn notes_mark_an_application_without_making_it_unknown() {
        let display = shown(&example::acme());
        let checkout = display.rows.iter().find(|r| r.name == "checkout").unwrap();
        assert_eq!(checkout.mark, Mark::Notes);
        assert!(!checkout.note.is_empty());
        assert_eq!(display.marks.iter().sum::<usize>(), display.rows.len());
    }

    fn scoped(inputs: &mut Inputs, namespaces: &[&str], found: ArgoFound) {
        inputs.sessions[0].argo_scope = freshkube_core::applications::ArgoScope::Namespace {
            namespaces: namespaces.iter().map(|n| n.to_string()).collect(),
            found,
        };
    }

    #[test]
    fn argo_read_where_it_runs_is_a_legend_fact_that_says_where_and_how() {
        let mut inputs = example::acme();
        scoped(
            &mut inputs,
            &["gitops"],
            ArgoFound::Labelled { skipped: vec![] },
        );
        let display = shown(&inputs);
        assert!(display.missing_text.is_none());
        let legend = display.legend_text.clone().unwrap();
        assert!(
            legend.contains(
                "Argo CD on core-fra was read in gitops only, where Argo CD's workloads run; Applications in other namespaces aren't listed"
            ),
            "{legend}"
        );
        let catalog = display.rows.iter().find(|r| r.name == "catalog").unwrap();
        assert_eq!(catalog.mark, Mark::Scoped, "a fact to note, not a gap");
        assert_eq!(
            catalog.mark_words.as_ref(),
            "Argo CD on core-fra was read in gitops only, where Argo CD's workloads run"
        );
        assert!(catalog.tooltip.contains("read in gitops only"));
        assert!(
            catalog
                .notes
                .iter()
                .any(|n| n.contains("read in gitops only"))
        );
    }

    #[test]
    fn the_default_namespace_says_nothing_marked_argo_cd() {
        let mut inputs = example::acme();
        scoped(&mut inputs, &["argocd"], ArgoFound::Default);
        let display = shown(&inputs);
        let legend = display.legend_text.clone().unwrap();
        assert!(
            legend.contains(
                "read in argocd only, its default: no workload is labelled app.kubernetes.io/part-of=argocd"
            ),
            "{legend}"
        );
    }

    #[test]
    fn namespaces_past_the_cap_are_named() {
        let mut inputs = example::acme();
        scoped(
            &mut inputs,
            &["a-ops", "b-ops", "c-ops"],
            ArgoFound::Labelled {
                skipped: vec!["d-ops".into()],
            },
        );
        let legend = shown(&inputs).legend_text.unwrap();
        assert!(
            legend.contains("read in a-ops, b-ops, c-ops only, where Argo CD's workloads run; d-ops past the cap of 3 not read"),
            "{legend}"
        );
    }

    #[test]
    fn a_kargo_project_beside_argo_cd_read_in_part_is_never_read_in_full() {
        let mut inputs = example::acme();
        scoped(
            &mut inputs,
            &["gitops"],
            ArgoFound::Labelled { skipped: vec![] },
        );
        let display = shown(&inputs);
        let checkout = display.rows.iter().find(|r| r.name == "checkout").unwrap();
        assert_eq!(checkout.rule, Rule::Kargo);
        assert_eq!(checkout.mark, Mark::Scoped);
        assert!(
            checkout
                .notes
                .iter()
                .any(|n| n.contains("Applications its Stages promote to may not all be listed")),
            "{:?}",
            checkout.notes
        );
        // Refused is worse, and says so the same way.
        let mut inputs = example::acme();
        inputs.sessions[0].argo_applications = Source::Refused("forbidden".into());
        let display = shown(&inputs);
        let checkout = display.rows.iter().find(|r| r.name == "checkout").unwrap();
        assert_ne!(checkout.mark, Mark::Read);
        assert_ne!(checkout.mark, Mark::Notes);
    }

    #[test]
    fn argo_cd_not_found_is_never_none() {
        let mut inputs = example::acme();
        scoped(&mut inputs, &["argocd"], ArgoFound::Default);
        inputs.sessions[0].argo_applications = Source::Read(Vec::new());
        inputs.sessions[0].argo_application_sets = Source::Read(Vec::new());
        let display = shown(&inputs);
        let (missing, _) = display.missing_text.clone().unwrap();
        assert!(
            missing.contains(
                "Argo CD's namespace wasn't found on core-fra; Applications elsewhere aren't listed"
            ),
            "{missing}"
        );
        assert_eq!(missing.matches("wasn't found").count(), 1, "said once");

        // With nothing found anywhere, the empty state says it too.
        let mut only = inputs.clone();
        only.sessions.truncate(1);
        only.sessions[0].kargo = Source::NotInstalled("not served".into());
        only.sessions[0].workloads = Source::Read(Vec::new());
        let Body::Empty { description, .. } = shown(&only).body else {
            panic!("expected the empty state");
        };
        assert!(
            description.contains("Argo CD's namespace wasn't found on core-fra"),
            "{description}"
        );
    }

    #[test]
    fn notes_about_several_parts_are_said_once() {
        let display = shown(&example::acme());
        let cart = display.rows.iter().find(|r| r.name == "cart").unwrap();
        assert!(
            cart.notes.contains(&SharedString::from(
                "4 parts on core-fra deploy to another cluster, not mapped yet: argocd/cart-dev, argocd/cart-stage, argocd/cart-prod-ams, argocd/cart-prod-fra"
            )),
            "{:?}",
            cart.notes
        );
        assert!(
            cart.notes.iter().any(|n| n.starts_with(
                "4 parts on core-fra are also claimed by the Argo CD rule, under their own names: "
            )),
            "{:?}",
            cart.notes
        );
        assert_eq!(cart.notes.len(), 4, "{:?}", cart.notes);
    }

    #[test]
    fn chips_have_short_words_and_whole_ones() {
        for mark in MARKS {
            assert!(!mark.short().is_empty());
            assert!(mark.what().len() >= mark.short().len());
        }
    }
}
