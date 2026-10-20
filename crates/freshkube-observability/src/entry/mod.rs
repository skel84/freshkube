//! The open workspace entry's Coroot link: which Coroot cluster the
//! Kubernetes links of the map and the reports read. It is the entry's
//! `clusters[].coroot` key in the setup file, read through the same
//! `Link::of` the change page uses; matching names never link anything, and
//! an entry without the key is not linked, even in a workspace of one.
use super::*;
use freshkube_core::coroot as api;
use freshkube_core::workspace::Entry;

#[cfg(test)]
mod tests;

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

/// What the open entry's link says of Kubernetes links when they are off:
/// the finding, and what to do about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LinkNote {
    pub(super) lead: SharedString,
    pub(super) body: SharedString,
    /// Both, as one line.
    pub(super) text: SharedString,
}

impl LinkNote {
    fn new(lead: String, body: String) -> Self {
        Self {
            text: format!("{lead} {body}").into(),
            lead: lead.into(),
            body: body.into(),
        }
    }
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

    /// Why the entry has no link, in the setup file's words; none for a
    /// linked entry.
    fn unlinked(&self) -> Option<LinkNote> {
        let file = "in the setup file (freshkube.yaml)";
        match self {
            Self::Closed => Some(LinkNote::new(
                "The open connection is not an entry of the setup file.".into(),
                format!(
                    "Open an entry that sets clusters[].coroot.cluster {file} to get Kubernetes links."
                ),
            )),
            Self::NotLinked(entry) => Some(LinkNote::new(
                format!("No Coroot cluster is linked to “{entry}”."),
                format!("Set clusters[].coroot.cluster for it {file}."),
            )),
            Self::NoProject(entry) => Some(LinkNote::new(
                format!(
                    "“{entry}” names a Coroot cluster but no project, and the connection names none."
                ),
                format!("Set clusters[].coroot.project or coroot.project {file}."),
            )),
            Self::Linked(_) => None,
        }
    }
}

impl ObservabilityPage {
    /// The workspace entries as the setup file lists them, with the
    /// connection's `coroot.project`, handed again on every reload. A link
    /// edited meanwhile reads the page again for the new link.
    pub fn set_entries(
        &mut self,
        entries: &[Entry],
        connection_project: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if self.entries != entries {
            self.entries = entries.to_vec();
        }
        if self.connection_project.as_deref() != connection_project {
            self.connection_project = connection_project.map(str::to_owned);
        }
        if self.derive_link(self.fixture) {
            self.relink(cx);
        }
    }

    /// The entry the open connection is, handed with its source. Example
    /// data names no project: its entries are linked by their cluster.
    pub(super) fn set_open_entry(&mut self, entry: Option<&str>, fixture: bool) -> bool {
        if self.open_entry.as_deref() != entry {
            self.open_entry = entry.map(str::to_owned);
        }
        self.derive_link(fixture)
    }

    /// Reads the link again from the entries, the open one and the
    /// connection's project; whether it changed.
    fn derive_link(&mut self, fixture: bool) -> bool {
        let connection = self
            .connection_project
            .as_deref()
            .or(fixture.then_some("example"));
        let link = EntryLink::of(&self.entries, self.open_entry.as_deref(), connection);
        let changed = self.entry_link != link;
        self.entry_link = link;
        self.sync_note();
        changed
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

    /// The linked project, when the connection lists it and another is shown.
    pub(super) fn linked_choice(&self) -> Option<api::ProjectInfo> {
        let link = self.entry_link.link()?;
        let shown = self.live.source.as_ref().map(api::Source::project);
        if shown == Some(link.project()) {
            return None;
        }
        self.live
            .projects
            .iter()
            .find(|project| project.id == link.project())
            .cloned()
    }

    /// The linked project is the one to show: when the connection lists it
    /// and another is shown, reads it. Whether it did.
    pub(super) fn select_linked_project(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(project) = self.linked_choice() else {
            return false;
        };
        self.select_project_as(&project, false, cx);
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
        self.sync_note();
        self.focused_revision = None;
        self.live.clear();
        self.incident_observations.clear_evidence();
        self.revision_observations.clear_evidence();
        self.revision_observations.seek_again();
        self.live_traces.reset();
        self.report_snapshot = None;
        self.refresh(cx);
    }

    /// Derives what the open entry's link says, from the link, the shown
    /// project and the listed ones. Called wherever one of them changes.
    pub(super) fn sync_note(&mut self) {
        self.link_note = self.entry_link.unlinked().or_else(|| {
            let link = self.entry_link.link()?;
            // A page that is not connected has nothing to link yet.
            self.live.provider.as_ref()?;
            let shown = self.live.source.as_ref().map(api::Source::project);
            if shown == Some(link.project()) {
                return None;
            }
            let (entry, project) = (link.entry(), link.project());
            if !self.live.projects.iter().any(|listed| listed.id == project) {
                return Some(LinkNote::new(
                    format!("Coroot lists no project “{project}” for “{entry}”."),
                    "Check clusters[].coroot.project (or coroot.project) in the setup file (freshkube.yaml); Kubernetes links stay off.".into(),
                ));
            }
            let showing = shown.map_or_else(
                || "No project is chosen".to_owned(),
                |shown| format!("This page shows project {shown}"),
            );
            Some(LinkNote::new(
                format!(
                    "“{entry}” is linked to Coroot cluster {} in project {project}.",
                    link.cluster()
                ),
                format!("{showing}; choose {project} to use its Kubernetes links."),
            ))
        });
    }

    /// The open entry's link, for the line that says what it is.
    pub(super) fn linked_line(&self) -> Option<String> {
        let link = self.entry_link.link()?;
        self.link_note.is_none().then(|| {
            format!(
                "Coroot cluster {} is linked to “{}”, so reports open its objects.",
                link.cluster(),
                link.entry()
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
        self.set_open_entry(Some("entry-a"), false);
        let project = self.connection_project.clone();
        self.set_entries(&[entry], project.as_deref(), cx);
    }
}
