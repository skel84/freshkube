//! The sidebar's Custom Resources: the API groups a cluster adds,
//! discovered when the section opens, and each group's kinds, discovered
//! when the group opens. What discovery found is kept until the connection
//! changes or the user retries, and nothing is read while the section is
//! closed.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use freshkube_core::resources::{
    ApiGroup, Failure, FailureKind, GroupKinds, ResourceKind, list_custom_groups, list_group_kinds,
};
use gpui_kit::{Context, SharedString, Task};
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use super::example;
use super::screen::{KubeAccess, KubeSource};
use crate::backend::{self, OwnedJob};

/// How long one discovery read may take.
const READ_DEADLINE: Duration = Duration::from_secs(30);

/// Where one discovery read stands.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Discovery<T> {
    Reading,
    Loaded(T),
    Failed(Failure),
}

/// One custom API group, with what the sidebar shows of it derived once.
pub(crate) struct CustomGroup {
    group: ApiGroup,
    pub(crate) name: SharedString,
    /// Element id of its row.
    pub(crate) id: SharedString,
    /// Its name and served versions, preferred first.
    pub(crate) tooltip: SharedString,
    /// `None` until the group is first opened.
    pub(crate) kinds: Option<Discovery<GroupRows>>,
}

impl CustomGroup {
    fn new(group: ApiGroup) -> Self {
        Self {
            name: group.name.clone().into(),
            id: format!("nav-k8s-api-{}", group.name).into(),
            tooltip: format!("{} · {}", group.name, group.versions.join(", ")).into(),
            group,
            kinds: None,
        }
    }
}

/// One listable kind of a group as the sidebar shows it.
pub(crate) struct KindRow {
    pub(crate) kind: ResourceKind,
    /// Its kubectl key, `plural.group`.
    pub(crate) key: SharedString,
    pub(crate) id: SharedString,
    pub(crate) label: SharedString,
    pub(crate) tooltip: SharedString,
}

/// What an open group shows once its kinds are discovered.
pub(crate) struct GroupRows {
    pub(crate) kinds: Vec<KindRow>,
    /// Why nothing can be listed, when nothing can.
    pub(crate) nothing: Option<SharedString>,
    /// Versions that weren't served or couldn't be read while another was:
    /// the row's few words, and each failure in full.
    pub(crate) partial: Option<(SharedString, SharedString)>,
}

impl GroupRows {
    fn new(group: &str, found: GroupKinds) -> Self {
        let kinds: Vec<KindRow> = found
            .kinds
            .into_iter()
            .map(|kind| {
                let key = kind.key();
                let scope = if kind.namespaced {
                    "namespaced"
                } else {
                    "cluster-scoped"
                };
                KindRow {
                    id: format!("nav-k8s-{key}").into(),
                    label: kind.kind.clone().into(),
                    tooltip: format!("{} · {} · {scope}", kind.plural, kind.api_version()).into(),
                    key: key.into(),
                    kind,
                }
            })
            .collect();
        let nothing = kinds.is_empty().then(|| {
            if found.unlistable.is_empty() {
                format!("{group} serves no kinds.").into()
            } else {
                format!(
                    "{} can be read one at a time, but not listed and watched.",
                    found.unlistable.join(", ")
                )
                .into()
            }
        });
        let partial = (!found.failures.is_empty()).then(|| {
            // A version /apis advertised that answers 404 isn't served.
            let versions = |served: bool| {
                found
                    .failures
                    .iter()
                    .filter(|(_, failure)| (failure.kind != FailureKind::NotFound) == served)
                    .map(|(version, _)| version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let label = [
                (versions(false), "not served"),
                (versions(true), "unreadable"),
            ]
            .into_iter()
            .filter(|(versions, _)| !versions.is_empty())
            .map(|(versions, what)| format!("{versions} {what}"))
            .collect::<Vec<_>>()
            .join(" · ");
            let detail = found
                .failures
                .iter()
                .map(|(version, failure)| format!("{version}: {failure}"))
                .collect::<Vec<_>>()
                .join("\n");
            (label.into(), detail.into())
        });
        Self {
            kinds,
            nothing,
            partial,
        }
    }
}

type Job = (OwnedJob, Task<()>);
type Answer<T> = oneshot::Receiver<Result<Result<T, Failure>, String>>;

pub(crate) struct CustomResources {
    runtime: Handle,
    source: Option<KubeSource>,
    open: bool,
    open_groups: BTreeSet<String>,
    groups: Option<Discovery<Vec<CustomGroup>>>,
    groups_job: Option<Job>,
    kinds_jobs: HashMap<String, Job>,
    /// Bumped whenever earlier answers stop applying: another connection,
    /// or a retry.
    epoch: u64,
}

impl CustomResources {
    pub(crate) fn new(runtime: Handle) -> Self {
        Self {
            runtime,
            source: None,
            open: false,
            open_groups: BTreeSet::new(),
            groups: None,
            groups_job: None,
            kinds_jobs: HashMap::new(),
            epoch: 0,
        }
    }

    pub(crate) fn is_group_open(&self, name: &str) -> bool {
        self.open_groups.contains(name)
    }

    /// `None` until the section is first opened on a connection.
    pub(crate) fn groups(&self) -> Option<&Discovery<Vec<CustomGroup>>> {
        self.groups.as_ref()
    }

    /// A discovered kind by its kubectl key.
    pub(crate) fn kind(&self, key: &str) -> Option<ResourceKind> {
        let Some(Discovery::Loaded(groups)) = &self.groups else {
            return None;
        };
        groups
            .iter()
            .filter_map(|entry| match &entry.kinds {
                Some(Discovery::Loaded(rows)) => Some(&rows.kinds),
                _ => None,
            })
            .flatten()
            .find(|row| row.key == key)
            .map(|row| row.kind.clone())
    }

    /// Another connection forgets what discovery found on the last one and,
    /// if the section is open, discovers again. The same connection only
    /// takes the new access.
    pub(crate) fn set_source(&mut self, source: Option<KubeSource>, cx: &mut Context<Self>) {
        let id = |source: &Option<KubeSource>| source.as_ref().map(|source| source.id.clone());
        let same = id(&self.source) == id(&source);
        self.source = source;
        if same {
            return;
        }
        self.forget();
        if self.open {
            self.discover_groups(cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        if self.open {
            self.discover_groups_once(cx);
            self.discover_open_groups(cx);
        }
        cx.notify();
    }

    pub(crate) fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open != open {
            self.toggle(cx);
        }
    }

    pub(crate) fn toggle_group(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self.open_groups.remove(name) {
            self.open_groups.insert(name.to_owned());
            self.discover_kinds(name, false, cx);
        }
        cx.notify();
    }

    /// Opens the section and `kind`'s group, so the sidebar shows the kind.
    pub(crate) fn reveal(&mut self, kind: &ResourceKind, cx: &mut Context<Self>) {
        self.open = true;
        self.open_groups.insert(kind.group.clone());
        self.discover_groups_once(cx);
        self.discover_kinds(&kind.group, false, cx);
        cx.notify();
    }

    /// Discovers the groups again, then the kinds of every open group.
    pub(crate) fn retry(&mut self, cx: &mut Context<Self>) {
        self.forget();
        if self.open {
            self.discover_groups(cx);
        }
        cx.notify();
    }

    pub(crate) fn retry_group(&mut self, name: &str, cx: &mut Context<Self>) {
        self.discover_kinds(name, true, cx);
        cx.notify();
    }

    /// A kind of `name` stopped being served, so what discovery found of the
    /// group may be out of date: a group in view is discovered again, any
    /// other is forgotten until it is shown.
    pub(crate) fn not_served(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.open && self.open_groups.contains(name) {
            self.discover_kinds(name, true, cx);
        } else if let Some(Discovery::Loaded(groups)) = self.groups.as_mut()
            && let Some(entry) = groups.iter_mut().find(|entry| entry.name == name)
        {
            entry.kinds = None;
            self.kinds_jobs.remove(name);
        }
        cx.notify();
    }

    /// Holds discovery at reading, as a slow server would.
    #[cfg(test)]
    pub(crate) fn hold_reading(&mut self, group: Option<&str>) {
        match group {
            None => self.groups = Some(Discovery::Reading),
            Some(name) => {
                if let Some(Discovery::Loaded(groups)) = self.groups.as_mut()
                    && let Some(entry) = groups.iter_mut().find(|entry| entry.name == name)
                {
                    entry.kinds = Some(Discovery::Reading);
                }
            }
        }
    }

    fn forget(&mut self) {
        self.epoch += 1;
        self.groups = None;
        self.groups_job = None;
        self.kinds_jobs.clear();
    }

    /// Discovers the groups unless they are known or being read.
    fn discover_groups_once(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.groups, Some(Discovery::Reading | Discovery::Loaded(_))) {
            self.discover_groups(cx);
        }
    }

    fn discover_groups(&mut self, cx: &mut Context<Self>) {
        let Some(access) = self.source.as_ref().map(|source| source.access.clone()) else {
            return;
        };
        self.groups = Some(Discovery::Reading);
        let epoch = self.epoch;
        if let KubeAccess::Example = access {
            self.finish_groups(epoch, Ok(example::custom_groups()), cx);
            return;
        }
        let receiver = read(&self.runtime, access, |client| async move {
            list_custom_groups(&client).await
        });
        let (job, receiver) = receiver;
        let task = cx.spawn(async move |this, cx| {
            let Some(result) = answer(receiver).await else {
                return;
            };
            _ = this.update(cx, |this, cx| this.finish_groups(epoch, result, cx));
        });
        self.groups_job = Some((job, task));
    }

    fn finish_groups(
        &mut self,
        epoch: u64,
        result: Result<Vec<ApiGroup>, Failure>,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.epoch {
            return;
        }
        self.groups_job = None;
        match result {
            Ok(groups) => {
                let groups = groups.into_iter().map(CustomGroup::new).collect();
                self.groups = Some(Discovery::Loaded(groups));
                // Groups left open, on this connection or the last, show
                // their kinds again.
                self.discover_open_groups(cx);
            }
            Err(failure) => self.groups = Some(Discovery::Failed(failure)),
        }
        cx.notify();
    }

    /// Discovers the kinds of every open group not yet known.
    fn discover_open_groups(&mut self, cx: &mut Context<Self>) {
        let open: Vec<String> = self.open_groups.iter().cloned().collect();
        for name in open {
            self.discover_kinds(&name, false, cx);
        }
    }

    /// Discovers one group's kinds, unless they are known or being read and
    /// `again` is false.
    fn discover_kinds(&mut self, name: &str, again: bool, cx: &mut Context<Self>) {
        let Some(access) = self.source.as_ref().map(|source| source.access.clone()) else {
            return;
        };
        let Some(Discovery::Loaded(groups)) = self.groups.as_mut() else {
            return;
        };
        let Some(entry) = groups.iter_mut().find(|entry| entry.name == name) else {
            return;
        };
        if !again && matches!(entry.kinds, Some(Discovery::Reading | Discovery::Loaded(_))) {
            return;
        }
        entry.kinds = Some(Discovery::Reading);
        let group = entry.group.clone();
        let epoch = self.epoch;
        if let KubeAccess::Example = access {
            let result = example::group_kinds(&group.name);
            self.finish_kinds(epoch, name, result, cx);
            return;
        }
        let (job, receiver) = read(&self.runtime, access, move |client| async move {
            list_group_kinds(&client, &group).await
        });
        let key = name.to_owned();
        let task = cx.spawn(async move |this, cx| {
            let Some(result) = answer(receiver).await else {
                return;
            };
            _ = this.update(cx, |this, cx| this.finish_kinds(epoch, &key, result, cx));
        });
        self.kinds_jobs.insert(name.to_owned(), (job, task));
    }

    fn finish_kinds(
        &mut self,
        epoch: u64,
        name: &str,
        result: Result<GroupKinds, Failure>,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.epoch {
            return;
        }
        self.kinds_jobs.remove(name);
        if let Some(Discovery::Loaded(groups)) = self.groups.as_mut()
            && let Some(entry) = groups.iter_mut().find(|entry| entry.name == name)
        {
            entry.kinds = Some(match result {
                Ok(kinds) => Discovery::Loaded(GroupRows::new(name, kinds)),
                Err(failure) => Discovery::Failed(failure),
            });
        }
        cx.notify();
    }
}

/// Runs `read` on Tokio with a client from `access`, within the deadline.
/// After a failure that retrying could fix, the client is dropped so the
/// next read builds a new one.
fn read<T, F, Fut>(runtime: &Handle, access: KubeAccess, read: F) -> (OwnedJob, Answer<T>)
where
    T: Send + 'static,
    F: FnOnce(kube::Client) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, Failure>> + Send,
{
    backend::spawn_job(
        runtime,
        READ_DEADLINE,
        format!("No answer within {} seconds", READ_DEADLINE.as_secs()),
        async move {
            let client = match access.client().await {
                Ok(client) => client,
                Err(error) => {
                    access.forget();
                    return Ok(Err(Failure::new(FailureKind::Other, error)));
                }
            };
            let result = read(client).await;
            if result
                .as_ref()
                .is_err_and(|failure| !failure.kind.is_permanent())
            {
                access.forget();
            }
            Ok(result)
        },
    )
}

/// The read's result, a timeout as a failure, or `None` if it was dropped.
async fn answer<T>(receiver: Answer<T>) -> Option<Result<T, Failure>> {
    match receiver.await {
        Ok(Ok(result)) => Some(result),
        Ok(Err(message)) => Some(Err(Failure::new(FailureKind::Timeout, message))),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use freshkube_core::resources::{ApiGroup, Failure, FailureKind, GroupKinds, ResourceKind};
    use gpui_kit::{AppContext, Entity, TestAppContext};
    use tokio::runtime::Runtime;

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{CustomResources, Discovery, GroupRows};
    use crate::resources::example;
    use crate::resources::screen::{KubeAccess, KubeSource};

    fn source(context: &str) -> KubeSource {
        KubeSource {
            id: example::connection(context),
            context: context.into(),
            access: KubeAccess::Example,
        }
    }

    fn mount(cx: &mut TestAppContext) -> (Runtime, Entity<CustomResources>) {
        let runtime = Runtime::new().unwrap();
        let custom = cx.new(|_| CustomResources::new(runtime.handle().clone()));
        custom.update(cx, |custom, cx| {
            custom.set_source(Some(source("homelab")), cx)
        });
        (runtime, custom)
    }

    fn group_names(custom: &CustomResources) -> Vec<String> {
        match custom.groups() {
            Some(Discovery::Loaded(groups)) => {
                groups.iter().map(|entry| entry.name.to_string()).collect()
            }
            other => panic!("{:?}", other.map(|_| "not loaded")),
        }
    }

    fn kinds(custom: &CustomResources, name: &str) -> Option<Discovery<usize>> {
        let Some(Discovery::Loaded(groups)) = custom.groups() else {
            return None;
        };
        let entry = groups.iter().find(|entry| entry.name == name)?;
        entry.kinds.as_ref().map(|kinds| match kinds {
            Discovery::Reading => Discovery::Reading,
            Discovery::Loaded(kinds) => Discovery::Loaded(kinds.kinds.len()),
            Discovery::Failed(failure) => Discovery::Failed(failure.clone()),
        })
    }

    #[gpui_kit::test]
    fn groups_and_kinds_are_discovered_only_when_opened(cx: &mut TestAppContext) {
        let (_runtime, custom) = mount(cx);
        custom.read_with(cx, |custom, _| assert!(custom.groups().is_none()));

        custom.update(cx, |custom, cx| custom.toggle(cx));
        custom.read_with(cx, |custom, _| {
            assert_eq!(group_names(custom).len(), 7);
            assert_eq!(kinds(custom, "cert-manager.io"), None);
        });
        custom.update(cx, |custom, cx| {
            custom.toggle_group("cert-manager.io", cx);
            custom.toggle_group("velero.io", cx);
        });
        custom.read_with(cx, |custom, _| {
            assert_eq!(kinds(custom, "cert-manager.io"), Some(Discovery::Loaded(4)));
            let Some(Discovery::Failed(failure)) = kinds(custom, "velero.io") else {
                panic!("velero.io should be refused");
            };
            assert_eq!(failure.kind, FailureKind::Forbidden);
            assert_eq!(kinds(custom, "cilium.io"), None);
        });

        // Closing and opening keeps what was found; nothing is read again.
        custom.update(cx, |custom, cx| {
            custom.toggle(cx);
            custom.toggle(cx);
        });
        custom.read_with(cx, |custom, _| {
            assert!(custom.is_group_open("cert-manager.io"));
            assert_eq!(kinds(custom, "cert-manager.io"), Some(Discovery::Loaded(4)));
        });
    }

    #[gpui_kit::test]
    fn another_connection_discovers_again_and_drops_late_answers(cx: &mut TestAppContext) {
        let (_runtime, custom) = mount(cx);
        custom.update(cx, |custom, cx| {
            custom.toggle(cx);
            custom.toggle_group("cert-manager.io", cx);
        });
        let epoch = custom.read_with(cx, |custom, _| custom.epoch);

        // The same connection with new access keeps everything.
        custom.update(cx, |custom, cx| {
            custom.set_source(Some(source("homelab")), cx)
        });
        custom.read_with(cx, |custom, _| assert_eq!(custom.epoch, epoch));

        custom.update(cx, |custom, cx| {
            custom.set_source(Some(source("staging-eu")), cx);
            // An answer to the last connection's read changes nothing.
            custom.finish_groups(
                epoch,
                Ok(vec![ApiGroup {
                    name: "stale.example.com".into(),
                    versions: vec!["v1".into()],
                }]),
                cx,
            );
        });
        custom.read_with(cx, |custom, _| {
            assert_ne!(custom.epoch, epoch);
            assert!(!group_names(custom).contains(&"stale.example.com".to_owned()));
            // The group left open shows its kinds on the new connection.
            assert_eq!(kinds(custom, "cert-manager.io"), Some(Discovery::Loaded(4)));
        });

        // Disconnected: nothing to discover with.
        custom.update(cx, |custom, cx| custom.set_source(None, cx));
        custom.read_with(cx, |custom, _| assert!(custom.groups().is_none()));
    }

    #[gpui_kit::test]
    fn a_kind_not_served_rediscovers_its_group_only_while_shown(cx: &mut TestAppContext) {
        let (_runtime, custom) = mount(cx);
        custom.update(cx, |custom, cx| {
            custom.toggle(cx);
            custom.toggle_group("cert-manager.io", cx);
            custom.toggle_group("cilium.io", cx);
            custom.toggle_group("cilium.io", cx);
        });
        custom.read_with(cx, |custom, _| {
            let found = custom.kind("certificates.cert-manager.io").unwrap();
            assert_eq!(found.api_version(), "cert-manager.io/v1");
            assert_eq!(custom.kind("pods"), None);
            assert_eq!(kinds(custom, "cilium.io"), Some(Discovery::Loaded(3)));
        });

        custom.update(cx, |custom, cx| {
            // In view: read again at once.
            custom.hold_reading(Some("cert-manager.io"));
            custom.not_served("cert-manager.io", cx);
            // Closed: forgotten, and read when next opened.
            custom.not_served("cilium.io", cx);
            // Not a custom group: nothing to do.
            custom.not_served("policy", cx);
        });
        custom.read_with(cx, |custom, _| {
            assert_eq!(kinds(custom, "cert-manager.io"), Some(Discovery::Loaded(4)));
            assert_eq!(kinds(custom, "cilium.io"), None);
        });
        custom.update(cx, |custom, cx| custom.toggle_group("cilium.io", cx));
        custom.read_with(cx, |custom, _| {
            assert_eq!(kinds(custom, "cilium.io"), Some(Discovery::Loaded(3)))
        });

        // A group forgotten while the section was closed is read when the
        // section opens again.
        custom.update(cx, |custom, cx| {
            custom.toggle(cx);
            custom.not_served("cilium.io", cx);
            custom.toggle(cx);
        });
        custom.read_with(cx, |custom, _| {
            assert_eq!(kinds(custom, "cilium.io"), Some(Discovery::Loaded(3)))
        });
    }

    #[test]
    fn group_rows_say_why_nothing_lists_and_which_versions_failed() {
        let metrics = GroupRows::new(
            "metrics.k8s.io",
            GroupKinds {
                unlistable: vec!["NodeMetrics".into(), "PodMetrics".into()],
                ..GroupKinds::default()
            },
        );
        assert!(metrics.kinds.is_empty());
        assert_eq!(
            metrics.nothing.as_deref(),
            Some("NodeMetrics, PodMetrics can be read one at a time, but not listed and watched.")
        );
        assert_eq!(metrics.partial, None);
        let empty = GroupRows::new("empty.example.com", GroupKinds::default());
        assert_eq!(
            empty.nothing.as_deref(),
            Some("empty.example.com serves no kinds.")
        );

        let widget = ResourceKind::new("example.com", "v1", "Widget", "widgets", true);
        let partial = GroupRows::new(
            "example.com",
            GroupKinds {
                kinds: vec![widget],
                unlistable: Vec::new(),
                failures: vec![
                    (
                        "v1alpha1".into(),
                        Failure::new(FailureKind::NotFound, "the server could not find"),
                    ),
                    ("v1beta1".into(), Failure::new(FailureKind::Timeout, "slow")),
                    (
                        "v1beta2".into(),
                        Failure::new(FailureKind::NotFound, "gone"),
                    ),
                ],
            },
        );
        assert_eq!(partial.nothing, None);
        let row = &partial.kinds[0];
        assert_eq!(row.key, "widgets.example.com");
        assert_eq!(row.id, "nav-k8s-widgets.example.com");
        assert_eq!(row.label, "Widget");
        assert_eq!(row.tooltip, "widgets · example.com/v1 · namespaced");
        let (label, detail) = partial.partial.unwrap();
        assert_eq!(label, "v1alpha1, v1beta2 not served · v1beta1 unreadable");
        assert_eq!(
            detail,
            "v1alpha1: Not found · the server could not find\nv1beta1: Timed out · slow\nv1beta2: Not found · gone"
        );
    }
}
