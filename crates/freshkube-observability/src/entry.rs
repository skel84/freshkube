//! The open workspace entry's Coroot link: which Coroot cluster the
//! Kubernetes links of the map and the reports read. It is the entry's
//! `clusters[].coroot` key in the setup file, read through the same
//! `Link::of` the change page uses; matching names never link anything, and
//! an entry without the key is not linked, even in a workspace of one.
use super::*;
use freshkube_core::coroot as api;
use freshkube_core::workspace::Entry;

/// What the open entry says of Coroot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EntryLink {
    /// The connection is not a workspace entry, or the setup does not list it.
    Closed,
    /// The entry names no Coroot cluster.
    NotLinked(String),
    /// The entry names one, and neither it nor the connection names a project.
    NoProject(String),
    Linked(api::Link),
}

impl EntryLink {
    pub(super) fn of(entries: &[Entry], open: Option<&str>, connection: Option<&str>) -> Self {
        let Some(entry) = open.and_then(|open| entries.iter().find(|entry| entry.id == open))
        else {
            return Self::Closed;
        };
        match api::Link::of(entry, connection) {
            Ok(link) => Self::Linked(link),
            Err(api::Outcome::NoProjectForEntry) => Self::NoProject(entry.id.clone()),
            Err(_) => Self::NotLinked(entry.id.clone()),
        }
    }

    pub(super) fn link(&self) -> Option<&api::Link> {
        match self {
            Self::Linked(link) => Some(link),
            _ => None,
        }
    }

    /// Why Kubernetes links are off for the entry, in the setup file's words;
    /// none for a linked entry.
    pub(super) fn reason(&self) -> Option<String> {
        match self {
            Self::Closed => Some(
                "The open connection is not an entry of the setup file, so no Coroot cluster is linked. \
                 Open an entry that sets clusters[].coroot.cluster in the setup file (freshkube.yaml)."
                    .into(),
            ),
            Self::NotLinked(entry) => Some(format!(
                "No Coroot cluster is linked to “{entry}”. Set clusters[].coroot.cluster for it in the setup file (freshkube.yaml)."
            )),
            Self::NoProject(entry) => Some(format!(
                "“{entry}” names a Coroot cluster but no project, and the connection names none. \
                 Set clusters[].coroot.project or coroot.project in the setup file (freshkube.yaml)."
            )),
            Self::Linked(_) => None,
        }
    }
}

impl ObservabilityPage {
    /// The workspace entries as the setup file lists them, handed again on
    /// every reload. A link edited meanwhile re-reads the page for the new
    /// link.
    pub fn set_entries(&mut self, entries: &[Entry], cx: &mut Context<Self>) {
        if self.entries != entries {
            self.entries = entries.to_vec();
        }
        if self.derive_link() {
            self.relink(cx);
        }
    }

    /// The entry the open connection is, handed with its source.
    pub(super) fn set_open_entry(&mut self, entry: Option<&str>) -> bool {
        if self.open_entry.as_deref() != entry {
            self.open_entry = entry.map(str::to_owned);
        }
        self.derive_link()
    }

    /// Reads the link again from the entries, the open one and the
    /// connection's project; whether it changed.
    pub(super) fn derive_link(&mut self) -> bool {
        let connection = self
            .memory
            .as_ref()
            .and_then(|memory| memory.connection_project());
        let link = EntryLink::of(&self.entries, self.open_entry.as_deref(), connection);
        if self.entry_link == link {
            return false;
        }
        self.entry_link = link;
        true
    }

    /// The Kubernetes access for `project` under the open entry's link: the
    /// link's cluster, when the link is for the project shown.
    pub(super) fn association_for(&self, project: &str) -> Option<api::Association> {
        let link = self
            .entry_link
            .link()
            .filter(|link| link.project() == project)?;
        let access = self.live.access.clone()?;
        Some(api::Association::new(access, link.cluster().to_owned()))
    }

    /// The linked project is the one to show: when the connection lists it
    /// and another is shown, selects it. Whether it did.
    pub(super) fn select_linked_project(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(link) = self.entry_link.link() else {
            return false;
        };
        let shown = self.live.source.as_ref().map(api::Source::project);
        if shown == Some(link.project()) {
            return false;
        }
        let Some(project) = self
            .live
            .projects
            .iter()
            .find(|project| project.id == link.project())
            .cloned()
        else {
            return false;
        };
        self.select_project(&project, cx);
        true
    }

    /// The link changed: what was read under the old one is dropped, with
    /// the reads in flight, and the page reads for the new one.
    pub(super) fn relink(&mut self, cx: &mut Context<Self>) {
        if self.select_linked_project(cx) {
            return;
        }
        if let Some(source) = self.live.source.take() {
            let association = self.association_for(source.project());
            self.live.source = Some(source.with_association(association));
        }
        self.focused_revision = None;
        self.live.clear();
        self.incident_observations.clear_evidence();
        self.revision_observations.clear_evidence();
        self.revision_observations.seek_again();
        self.live_traces.reset();
        self.report_snapshot = None;
        self.refresh(cx);
    }

    /// The open entry's state, for the pages that draw Kubernetes links.
    pub(super) fn link_note(&self) -> Option<String> {
        if let Some(reason) = self.entry_link.reason() {
            return Some(reason);
        }
        let link = self.entry_link.link()?;
        let shown = self.live.source.as_ref()?.project();
        (shown != link.project()).then(|| {
            format!(
                "“{}” is linked to Coroot cluster {} in project {}; this page shows project {shown}, so Kubernetes links are off.",
                link.entry(),
                link.cluster(),
                link.project()
            )
        })
    }
}

#[cfg(test)]
impl ObservabilityPage {
    /// Test: the open entry names Coroot cluster `cluster` in the project
    /// the page shows.
    pub(crate) fn link_for_test(&mut self, cluster: &str, cx: &mut Context<Self>) {
        let project = self
            .live
            .source
            .as_ref()
            .map(|source| source.project().to_owned());
        self.entry_for_test(Some((project.as_deref(), cluster)), cx);
    }

    /// Test: the setup lists one entry, `entry-a`, with the `coroot` key
    /// given (a project of its own, its cluster), and it is the open one.
    pub(crate) fn entry_for_test(
        &mut self,
        coroot: Option<(Option<&str>, &str)>,
        cx: &mut Context<Self>,
    ) {
        use freshkube_core::workspace::Role;
        let mut entry = Entry::new("entry-a", Role::Core, "context-a");
        entry.coroot = coroot.map(|(project, cluster)| {
            let mut key = freshkube_core::setup::EntryCoroot::new(cluster);
            key.project = project.map(str::to_owned);
            Box::new(key)
        });
        self.set_open_entry(Some("entry-a"));
        self.set_entries(&[entry], cx);
    }
}
