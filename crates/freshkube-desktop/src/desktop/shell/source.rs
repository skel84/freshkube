//! The expanded column's lines (docs/DESIGN.md, "The column as a source
//! list"): derived when what they show changes, so the cached column only
//! reads them, and what opening each row does.
use super::rail::RowTarget;
use super::*;
use crate::monitoring::page::EntryId;
use crate::observability::Destination;
use freshkube_ui::source_list::{Line, Menu, Note, Prose, Row, Section, SourceListHost};

/// What a row of the column opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ColumnKey {
    Page(Page),
    /// A built-in kind, by its kubectl key.
    Kind(&'static str),
    /// An API group's row, which opens and closes its kinds.
    ApiGroup(SharedString),
    CustomKind(SharedString),
    RetryDiscovery,
    RetryGroup(SharedString),
    Dashboard(EntryId),
    ReadFolder,
    /// Settings, for a talosconfig or a dashboards folder.
    Settings,
    Destination(Destination),
    /// Observability's Dashboards, which opens Monitoring.
    Dashboards,
    /// A namespace for Pods under Workloads, or every one.
    Namespace(Option<SharedString>),
}

/// The column's lines, where a reveal lands among them, and whether
/// discovery has settled enough to stop revealing it.
pub(in crate::desktop) struct ColumnLines {
    pub(super) lines: Vec<Line<ColumnKey>>,
    pub(super) reveal: Option<usize>,
    pub(super) settled: bool,
}

impl ColumnLines {
    fn new(lines: Vec<Line<ColumnKey>>) -> Self {
        Self {
            lines,
            reveal: None,
            settled: true,
        }
    }
}

/// How many namespaces Workloads lists; the rest are in Pods' menu.
const COLUMN_NAMESPACES: usize = 20;

/// Why discovery shows nothing, in a few words; the tooltip says more.
fn failure_label(failure: &Failure) -> &'static str {
    match failure.kind {
        FailureKind::Forbidden => "Not permitted",
        FailureKind::NotFound => "No longer served",
        FailureKind::Timeout => "Timed out",
        FailureKind::Unreachable => "Unreachable",
        _ => "Couldn't discover",
    }
}

/// A page's shortcut as the platform labels it.
fn shortcut(key: &str) -> SharedString {
    format!("{}{key}", freshkube_ui::platform::primary_modifier()).into()
}

impl Pilot {
    /// Says that what the column shows changed: its next draw derives its
    /// lines again, and draws that only redraw it, such as a hover's, read
    /// them as they are.
    pub(in crate::desktop) fn column_changed(&mut self) {
        self.column_stale = true;
    }

    /// Derives the column's lines from what the shell and its pages show
    /// now, once after they change.
    pub(super) fn derive_column(&mut self, cx: &App) {
        if !std::mem::take(&mut self.column_stale) {
            return;
        }
        let derived = match self.area {
            Area::Group(slug) => self.group_lines(slug, cx),
            Area::Custom => self.custom_lines(cx),
            Area::Monitoring => ColumnLines::new(self.monitoring_lines(cx)),
            Area::Observability => ColumnLines::new(self.observability_lines(cx)),
            Area::ControlPlane => ColumnLines::new(self.control_plane_lines(cx)),
            _ => ColumnLines::new(Vec::new()),
        };
        self.column_list.set_lines(derived.lines);
        self.column_reveal_line = derived.reveal;
        self.column_settled = derived.settled;
    }

    /// The mark the rail's dots put on a row.
    fn marked(&self, row: Row<ColumnKey>, target: RowTarget) -> Row<ColumnKey> {
        match self.rail_marks.row(target) {
            Some((tone, why)) => row.mark(*tone, why.clone()),
            None => row,
        }
    }

    fn page_line(&self, page: Page, key: Option<&str>) -> Row<ColumnKey> {
        let mut row = Row::new(
            ColumnKey::Page(page),
            format!("nav-{}", page.slug()),
            page.title(),
        )
        .icon(column::page_icon(page))
        .current(self.page == page);
        if let Some(key) = key {
            row = row.shortcut(shortcut(key));
        }
        self.marked(row, RowTarget::Page(page))
    }

    /// A built-in group's kinds, after Health for Workloads and then its
    /// namespaces, and the row of a kind to reveal.
    fn group_lines(&self, slug: &'static str, cx: &App) -> ColumnLines {
        let Some(group) = navigation::NAVIGATION
            .iter()
            .find(|group| group.slug == slug)
        else {
            return ColumnLines::new(Vec::new());
        };
        let current = (self.page == Page::Resources).then(|| self.resource_kind.key());
        let mut lines = Vec::new();
        if slug == "workloads" {
            lines.push(self.page_line(Page::Health, Some("5")).into());
        }
        let first_kind = lines.len();
        lines.extend(group.items.iter().map(|(label, key)| {
            let row = Row::new(ColumnKey::Kind(key), format!("nav-k8s-{key}"), *label)
                .icon(super::fog_column::kind_icon(key))
                .current(current.as_deref() == Some(*key));
            self.marked(row, RowTarget::Kind(key)).into()
        }));
        let reveal = match &self.column_reveal {
            Some(ColumnReveal::Kind(key)) => group
                .items
                .iter()
                .position(|(_, item)| item == key)
                .map(|ix| first_kind + ix),
            _ => None,
        };
        if slug == "workloads" {
            self.namespace_lines(&mut lines, cx);
        }
        ColumnLines {
            lines,
            reveal,
            settled: true,
        }
    }

    /// Workloads' namespaces, with their pods counted.
    fn namespace_lines(&self, lines: &mut Vec<Line<ColumnKey>>, cx: &App) {
        let chosen = self.resources.read(cx).namespace();
        let pods = self.page == Page::Resources && self.resource_kind.is_pod();
        let namespaces = &self.column_state.namespaces;
        lines.push(Section::new("nav-namespaces", "Namespaces").into());
        lines.push(
            Row::new(
                ColumnKey::Namespace(None),
                "nav-all-namespaces",
                "All namespaces",
            )
            .icon(IconName::Folders)
            .current(pods && chosen.is_none())
            .count(self.column_state.total.clone(), None)
            .into(),
        );
        lines.extend(
            namespaces
                .iter()
                .take(COLUMN_NAMESPACES)
                .map(|(name, count)| {
                    Row::new(
                        ColumnKey::Namespace(Some(name.clone())),
                        format!("nav-namespace-{name}"),
                        name.clone(),
                    )
                    .icon(IconName::Folder)
                    .mono()
                    .current(chosen == Some(name.as_ref()))
                    .count(count.clone(), None)
                    .into()
                }),
        );
        // Past the rows the column lists, a menu names every namespace,
        // as the All namespaces row's menu did.
        if namespaces.len() > COLUMN_NAMESPACES {
            let entries = std::iter::once(("All namespaces".into(), ColumnKey::Namespace(None)))
                .chain(
                    namespaces
                        .iter()
                        .map(|(name, _)| (name.clone(), ColumnKey::Namespace(Some(name.clone())))),
                )
                .collect();
            lines.push(
                Menu::new(
                    "nav-namespaces-more",
                    format!("{} more namespaces…", namespaces.len() - COLUMN_NAMESPACES),
                    entries,
                )
                .tooltip("Choose any namespace")
                .into(),
            );
        }
    }

    /// Custom Resources: discovery's state or one row per API group, with
    /// the kinds of each open group; where a reveal lands among them; and
    /// whether discovery has settled enough to stop revealing it.
    fn custom_lines(&self, cx: &App) -> ColumnLines {
        let reveal = self.column_reveal.as_ref();
        let current = (self.page == Page::Resources).then(|| self.resource_kind.key());
        let current = current.as_deref();
        // The API group of the kind shown, when it is a custom kind.
        let current_group = current
            .filter(|key| navigation::group_of(key).is_none())
            .map(|_| self.resource_kind.group.as_str());
        // A custom kind to reveal, and the group it is in.
        let reveal_kind = match reveal {
            Some(ColumnReveal::Kind(key)) if navigation::group_of(key).is_none() => {
                Some(key.as_str())
            }
            _ => None,
        };
        let reveal_group = match reveal {
            Some(ColumnReveal::ApiGroup(name)) => Some(name.as_str()),
            _ => reveal_kind.and_then(|key| key.split_once('.').map(|(_, group)| group)),
        };
        let mut out = ColumnLines::new(Vec::new());
        let revealing = matches!(reveal, Some(ColumnReveal::Custom)) || reveal_group.is_some();
        if revealing {
            out.reveal = Some(0);
        }
        let status = |text: &'static str| Note::new("nav-k8s-custom-status", text);
        match self.custom.read(cx).groups() {
            None => out.lines.push(status("Not connected").into()),
            Some(Discovery::Reading) => {
                out.settled = !revealing;
                out.lines.push(status("Discovering…").into());
            }
            Some(Discovery::Failed(failure)) => out.lines.push(
                status(failure_label(failure))
                    .tooltip(failure.to_string())
                    .retry(ColumnKey::RetryDiscovery)
                    .into(),
            ),
            Some(Discovery::Loaded(groups)) if groups.is_empty() => {
                out.lines.push(status("No custom resources").into());
            }
            Some(Discovery::Loaded(groups)) => {
                self.custom_group_lines(
                    &mut out,
                    groups,
                    Wanted {
                        current,
                        current_group,
                        reveal,
                        reveal_kind,
                        reveal_group,
                    },
                    cx,
                );
            }
        }
        if matches!(reveal, Some(ColumnReveal::Custom)) {
            out.reveal = Some(out.lines.len().saturating_sub(1));
        }
        out
    }

    /// One row per API group, with the kinds of each open group, and where
    /// a reveal lands among them.
    fn custom_group_lines(
        &self,
        out: &mut ColumnLines,
        groups: &[CustomGroup],
        Wanted {
            current,
            current_group,
            reveal,
            reveal_kind,
            reveal_group,
        }: Wanted,
        cx: &App,
    ) {
        let custom = self.custom.read(cx);
        for entry in groups.iter().take(MAX_SIDEBAR_GROUPS) {
            let name = entry.name.as_ref();
            let open = custom.is_group_open(name);
            if reveal_group == Some(name) {
                out.reveal = Some(out.lines.len());
            }
            // A closed group shows that the page is one of its kinds.
            out.lines.push(
                Row::new(
                    ColumnKey::ApiGroup(entry.name.clone()),
                    entry.id.clone(),
                    entry.name.clone(),
                )
                .disclosure(open)
                .current(!open && current_group == Some(name))
                .tooltip(entry.tooltip.clone())
                .into(),
            );
            if !open {
                continue;
            }
            let settled = matches!(
                entry.kinds,
                Some(Discovery::Loaded(_) | Discovery::Failed(_))
            );
            if reveal_group == Some(name) && !settled {
                out.settled = false;
            }
            let found = api_group_lines(entry, current, reveal_kind);
            if let Some(row) = found.reveal {
                out.reveal = Some(out.lines.len() + row);
            } else if matches!(reveal, Some(ColumnReveal::ApiGroup(open)) if open == name) {
                out.reveal = Some(out.lines.len() + found.lines.len() - 1);
            }
            out.lines.extend(found.lines);
        }
        if groups.len() > MAX_SIDEBAR_GROUPS {
            out.lines.push(
                Note::new(
                    "nav-k8s-custom-more",
                    format!(
                        "{} more groups not shown",
                        groups.len() - MAX_SIDEBAR_GROUPS
                    ),
                )
                .into(),
            );
        }
    }

    /// The built-in dashboards, then the user's folder or how to add one.
    fn monitoring_lines(&self, cx: &App) -> Vec<Line<ColumnKey>> {
        let monitoring = self.monitoring.read(cx);
        let open = (self.page == Page::Monitoring).then(|| monitoring.chosen());
        let entry = |entry: &Entry| -> Line<ColumnKey> {
            let mut row = Row::new(
                ColumnKey::Dashboard(entry.id.clone()),
                entry.element_id.clone(),
                entry.title.clone(),
            )
            .icon(IconName::ChartLine)
            .current(open == Some(&entry.id));
            if let Some(tooltip) = &entry.tooltip {
                row = row.tooltip(tooltip.clone());
            }
            row.into()
        };
        let catalog = monitoring.catalog();
        let mut lines = vec![Section::new("monitoring-builtin", "Built in").into()];
        lines.extend(catalog.builtins.iter().map(entry));
        let folder = |name: &SharedString| Section::new("monitoring-folder", name.clone()).into();
        match &catalog.folder {
            FolderState::None => lines.push(
                Prose::new(
                    "monitoring-folder-none",
                    "Add your own Grafana dashboards from a folder of JSON",
                )
                .action(
                    "monitoring-add-folder",
                    "Choose a folder",
                    ColumnKey::Settings,
                )
                .into(),
            ),
            FolderState::Reading { name, .. } => {
                lines.push(folder(name));
                lines.push(Note::new("monitoring-folder-reading", "Reading…").into());
            }
            FolderState::Read {
                name,
                path,
                entries,
                note,
            } => {
                lines.push(folder(name));
                if entries.is_empty() {
                    lines.push(
                        Note::new("monitoring-folder-empty", "No dashboards")
                            .tooltip(path.clone())
                            .into(),
                    );
                }
                lines.extend(entries.iter().map(entry));
                if let Some(note) = note {
                    lines.push(Note::new("monitoring-folder-note", note.clone()).into());
                }
            }
            FolderState::Failed { name, error } => {
                lines.push(folder(name));
                lines.push(
                    Note::new("monitoring-folder-failed", "Couldn't read the folder")
                        .tooltip(error.clone())
                        .retry(ColumnKey::ReadFolder)
                        .into(),
                );
            }
        }
        lines
    }

    /// Observability's destinations, then Dashboards.
    fn observability_lines(&self, cx: &App) -> Vec<Line<ColumnKey>> {
        let observability = self.observability.read(cx);
        let destination = observability.destination();
        let incidents = observability.incident_count();
        let mut lines: Vec<Line<ColumnKey>> = self
            .obs_destinations()
            .into_iter()
            .map(|item| {
                let active = item == destination
                    || (item == Destination::Applications
                        && destination == Destination::Application);
                let mut row = Row::new(
                    ColumnKey::Destination(item),
                    format!("nav-obs-{}", item.slug()),
                    item.label(),
                )
                .icon(item.icon())
                .current(active);
                if let Some(count) = incidents.filter(|_| item == Destination::Incidents) {
                    row = row.count(count.to_owned(), self.fixture.then_some(Tone::Crit));
                }
                row.into()
            })
            .collect();
        lines.push(
            Row::new(ColumnKey::Dashboards, "nav-obs-dashboards", "Dashboards")
                .icon(IconName::ChartLine)
                .tooltip("Prometheus dashboards, in Monitoring")
                .into(),
        );
        lines
    }

    /// The Talos pages, or without a talosconfig, how to add one.
    fn control_plane_lines(&self, cx: &App) -> Vec<Line<ColumnKey>> {
        if self.kubernetes_only.is_some() {
            return vec![
                Prose::new("talosconfig-needed", "Talos views need a talosconfig")
                    .action("add-talosconfig", "Add a talosconfig", ColumnKey::Settings)
                    .into(),
            ];
        }
        let services = self.system_services.read(cx);
        let mut services_row = self.page_line(Page::SystemServices, Some("7"));
        if services.unhealthy > 0 {
            services_row = services_row.count(services.badge.clone(), Some(Tone::Crit));
        }
        vec![
            self.page_line(Page::Etcd, Some("6")).into(),
            services_row.into(),
            self.page_line(Page::Security, Some("8")).into(),
            self.page_line(Page::Lifecycle, Some("9")).into(),
            self.page_line(Page::Operations, None).into(),
        ]
    }

    /// Observability's destinations in the column; Deployments has no live
    /// source yet, so only example data shows it.
    pub(super) fn obs_destinations(&self) -> Vec<Destination> {
        Destination::NAVIGATION
            .into_iter()
            .filter(|item| self.fixture || *item != Destination::Deployments)
            .collect()
    }
}

/// What Custom Resources' column marks as shown and reveals.
#[derive(Clone, Copy)]
struct Wanted<'a> {
    current: Option<&'a str>,
    current_group: Option<&'a str>,
    reveal: Option<&'a ColumnReveal>,
    reveal_kind: Option<&'a str>,
    reveal_group: Option<&'a str>,
}

/// An open API group's kinds, or why it shows none, and whether a version
/// failed while others were read. `reveal` is a kind's line.
fn api_group_lines(
    entry: &CustomGroup,
    current: Option<&str>,
    reveal_kind: Option<&str>,
) -> ColumnLines {
    let id = &entry.id;
    let mut out = ColumnLines::new(Vec::new());
    let status = |text: SharedString| Note::new(format!("{id}-status"), text).depth(1);
    let retry = ColumnKey::RetryGroup(entry.name.clone());
    let rows = match &entry.kinds {
        None | Some(Discovery::Reading) => {
            out.lines.push(status("Discovering…".into()).into());
            return out;
        }
        Some(Discovery::Failed(failure)) => {
            out.lines.push(
                status(failure_label(failure).into())
                    .tooltip(failure.to_string())
                    .retry(retry)
                    .into(),
            );
            return out;
        }
        Some(Discovery::Loaded(rows)) => rows,
    };
    for kind in rows.kinds.iter().take(MAX_SIDEBAR_KINDS) {
        if reveal_kind == Some(kind.key.as_ref()) {
            out.reveal = Some(out.lines.len());
        }
        out.lines.push(
            Row::new(
                ColumnKey::CustomKind(kind.key.clone()),
                kind.id.clone(),
                kind.label.clone(),
            )
            .depth(1)
            .current(current == Some(kind.key.as_ref()))
            .tooltip(kind.tooltip.clone())
            .into(),
        );
    }
    if rows.kinds.len() > MAX_SIDEBAR_KINDS {
        let more = rows.kinds.len() - MAX_SIDEBAR_KINDS;
        out.lines
            .push(status(format!("{more} more kinds not shown").into()).into());
    }
    if let Some(why) = &rows.nothing {
        out.lines
            .push(status("Nothing to list".into()).tooltip(why.clone()).into());
    }
    if let Some((label, detail)) = &rows.partial {
        out.lines.push(
            Note::new(format!("{id}-partial"), label.clone())
                .depth(1)
                .tooltip(detail.clone())
                .retry(retry)
                .into(),
        );
    }
    out
}

impl SourceListHost for Pilot {
    type Key = ColumnKey;

    fn source_list(&mut self) -> &mut freshkube_ui::source_list::SourceList<ColumnKey> {
        &mut self.column_list
    }

    fn open(&mut self, key: &ColumnKey, window: &mut Window, cx: &mut Context<Self>) {
        self.column_changed();
        match key.clone() {
            ColumnKey::Page(page) => self.navigate_from_keyboard(page, window, cx),
            ColumnKey::Kind(key) => self.open_builtin(key, window, cx),
            ColumnKey::ApiGroup(name) => self.toggle_api_group(&name, cx),
            ColumnKey::CustomKind(key) => self.open_custom(&key, window, cx),
            ColumnKey::RetryDiscovery => self.custom.update(cx, |custom, cx| custom.retry(cx)),
            ColumnKey::RetryGroup(name) => self
                .custom
                .update(cx, |custom, cx| custom.retry_group(&name, cx)),
            ColumnKey::Dashboard(id) => {
                self.monitoring
                    .update(cx, |monitoring, cx| monitoring.open(id, cx));
                self.navigate_from_keyboard(Page::Monitoring, window, cx);
            }
            ColumnKey::ReadFolder => self
                .monitoring
                .update(cx, |monitoring, cx| monitoring.read_folder(cx)),
            ColumnKey::Settings => {
                self.settings_open = true;
                cx.notify();
            }
            ColumnKey::Destination(item) => {
                self.observability
                    .update(cx, |page, cx| page.open(item, cx));
                self.navigate_from_keyboard(Page::Observability, window, cx);
            }
            ColumnKey::Dashboards => self.navigate_from_keyboard(Page::Monitoring, window, cx),
            ColumnKey::Namespace(namespace) => {
                self.choose_sidebar_namespace(namespace.map(String::from), window, cx)
            }
        }
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_page(window, cx);
    }

    /// Only the column draws the keyboard's row.
    fn moved(&mut self, cx: &mut Context<Self>) {
        self.chrome.column.update(cx, |_, cx| cx.notify());
    }
}
