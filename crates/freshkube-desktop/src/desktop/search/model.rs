//! Search rows are derived when the query or an answer changes.
use super::*;
use crate::resources::{custom::Discovery, navigation::NAVIGATION};
use gpui_kit::component::Disableable;

#[derive(Clone)]
pub(super) enum Destination {
    Page(Page),
    Kind(ResourceKind),
    Node(String),
    Object(ResourceKind, ObjectRef),
}
#[derive(Clone)]
pub(super) struct Entry {
    group: &'static str,
    id: SharedString,
    label: SharedString,
    terms: String,
    destination: Destination,
}
impl Entry {
    fn new(
        group: &'static str,
        id: String,
        label: String,
        terms: String,
        destination: Destination,
    ) -> Self {
        Self {
            group,
            id: id.into(),
            label: label.into(),
            terms: terms.to_lowercase(),
            destination,
        }
    }
}
const GROUPS: [&str; 7] = [
    "Pages",
    "Nodes",
    "Pods",
    "Workloads",
    "Networking",
    "Config",
    "Kinds",
];
pub(super) const KINDS: [&str; 9] = [
    "pods",
    "deployments.apps",
    "statefulsets.apps",
    "daemonsets.apps",
    "services",
    "ingresses.networking.k8s.io",
    "configmaps",
    "secrets",
    "namespaces",
];
pub(super) fn object_group(key: &str) -> &'static str {
    match key {
        "pods" => "Pods",
        "deployments.apps" | "statefulsets.apps" | "daemonsets.apps" => "Workloads",
        "services" | "ingresses.networking.k8s.io" => "Networking",
        _ => "Config",
    }
}
impl Pilot {
    pub(super) fn local_search_entries(&self, cx: &App) -> Vec<Entry> {
        let mut entries = Page::ALL
            .into_iter()
            .filter(|page| {
                *page != Page::Resources
                    && (self.kubernetes_only.is_none()
                        || !matches!(
                            page,
                            Page::Etcd
                                | Page::SystemServices
                                | Page::Security
                                | Page::Lifecycle
                                | Page::Operations
                        ))
            })
            .map(|page| {
                Entry::new(
                    "Pages",
                    format!("search-page-{}", page.slug()),
                    page.title().into(),
                    page.title().into(),
                    Destination::Page(page),
                )
            })
            .collect::<Vec<_>>();
        for key in ["namespaces", "events"] {
            let kind = builtin(key).unwrap();
            entries.push(Entry::new(
                "Pages",
                format!("search-page-{key}"),
                resources::title(&kind).into(),
                key.into(),
                Destination::Kind(kind),
            ));
        }
        entries.extend(self.node_workspace.rows.iter().map(|row| {
            Entry::new(
                "Nodes",
                format!("search-node-{}", row.name),
                row.name.to_string(),
                row.name.to_string(),
                Destination::Node(row.name.to_string()),
            )
        }));
        for group in &NAVIGATION {
            for (label, key) in group.items {
                if let Some(kind) = builtin(key) {
                    entries.push(Entry::new(
                        "Kinds",
                        format!("search-kind-{key}"),
                        (*label).into(),
                        format!("{label} {key} {}", kind.kind),
                        Destination::Kind(kind),
                    ));
                }
            }
        }
        if let Some(Discovery::Loaded(groups)) = self.custom.read(cx).groups() {
            for group in groups {
                if let Some(Discovery::Loaded(rows)) = &group.kinds {
                    entries.extend(rows.kinds.iter().map(|row| {
                        Entry::new(
                            "Kinds",
                            format!("search-kind-{}", row.key),
                            format!("{} · {}", row.label, group.name),
                            row.key.to_string(),
                            Destination::Kind(row.kind.clone()),
                        )
                    }));
                }
            }
        }
        entries
    }
}
impl Search {
    pub(super) fn rebuild(&mut self) {
        let query = self.query.trim().to_lowercase();
        self.groups.clear();
        self.destinations.clear();
        for group in GROUPS {
            let matches = self
                .local
                .iter()
                .chain(
                    self.parts
                        .values()
                        .filter_map(|part| part.as_ref().ok())
                        .flat_map(|part| part.entries.iter()),
                )
                .filter(|entry| {
                    entry.group == group && (query.is_empty() || entry.terms.contains(&query))
                })
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            let mut items = Vec::new();
            let mut destinations = Vec::new();
            for entry in matches.iter().take(8) {
                let id = entry.id.clone();
                let label = entry.label.clone();
                items.push(CommandItem::new().label(label.clone()).child(move |_, cx| {
                    div()
                        .id(id.clone())
                        .test_support()
                        .w_full()
                        .py(dp(5.))
                        .child(
                            div()
                                .text_color(crate::palette::palette(cx).ink)
                                .child(label.clone()),
                        )
                }));
                destinations.push(Some(entry.destination.clone()));
            }
            if matches.len() > 8 {
                items.push(
                    CommandItem::new()
                        .label(format!("{} more", matches.len() - 8))
                        .disabled(true),
                );
                destinations.push(None);
            }
            self.groups
                .push(CommandGroup::new().label(group).items(items));
            self.destinations.push(destinations);
        }
        self.notes = KINDS
            .into_iter()
            .filter_map(|key| match self.parts.get(key) {
                Some(Err(error)) => Some((
                    format!("search-status-{key}").into(),
                    format!("Can't list {key}: {error}").into(),
                )),
                Some(Ok(part)) if part.capped => Some((
                    format!("search-status-{key}").into(),
                    format!("{key}: First 2,000 shown").into(),
                )),
                None if self.open => Some((
                    format!("search-status-{key}").into(),
                    format!("Listing {key}…").into(),
                )),
                _ => None,
            })
            .collect();
    }
}
pub(super) fn object_entries(kind: ResourceKind, objects: Vec<ObjectRef>) -> Vec<Entry> {
    let key = kind.key();
    objects
        .into_iter()
        .map(|object| {
            let address = if object.namespace.is_empty() {
                object.name.clone()
            } else {
                format!("{}/{}", object.namespace, object.name)
            };
            Entry::new(
                object_group(&key),
                format!(
                    "search-object-{key}-{}-{}-{}",
                    object.namespace, object.name, object.uid
                ),
                format!("{address} · {}", kind.kind),
                address,
                Destination::Object(kind.clone(), object),
            )
        })
        .collect()
}
