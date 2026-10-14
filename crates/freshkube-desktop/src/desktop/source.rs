//! What every page is handed, built in one place: the session owner until
//! #44's session type takes it. It holds no fields of its own; it reads
//! the shell's and decides, in one match, whether the window shows example
//! data, a kubeconfig context or a Talos cluster. [`Mode`] answers the same
//! question for the chrome.

use std::sync::Arc;

use freshkube_core::cluster_overview::ClusterOverviewCollector;
use freshkube_core::cluster_source::{
    AppliedAccess, ClusterAccess, ClusterSource, LiveSource, TalosAccess,
};

use super::Pilot;
use super::kubernetes_only::KubernetesOnly;
use crate::backend::Target;
use crate::resources;
use crate::screens::{PageSource, ScreenSource};

/// What the window shows, as two answers: example data or a live cluster,
/// and Talos with Kubernetes or Kubernetes alone. They are independent:
/// example data is Kubernetes-only too after "Use Kubernetes only…" under
/// `--fixture`, or on the `kubernetes-only` debug page. Readers ask these
/// questions rather than the shell's fields.
#[derive(Clone, Copy)]
pub(super) struct Mode<'a> {
    example: bool,
    kubernetes_only: Option<&'a KubernetesOnly>,
}

impl<'a> Mode<'a> {
    /// Example data, which reads no cluster.
    pub(super) fn is_example(self) -> bool {
        self.example
    }

    /// Talos is part of the session: false when a kubeconfig is all it
    /// reads.
    pub(super) fn has_talos(self) -> bool {
        self.kubernetes_only.is_none()
    }

    /// The kubeconfig a session without Talos reads, and its connection.
    pub(super) fn kubernetes_only(self) -> Option<&'a KubernetesOnly> {
        self.kubernetes_only
    }
}

impl Pilot {
    /// What the window shows: example data or not, with Talos or not.
    pub(super) fn mode(&self) -> Mode<'_> {
        Mode {
            example: self.fixture,
            kubernetes_only: self.kubernetes_only.as_ref(),
        }
    }

    /// What every page is handed: the Talos target the screens inspect, and
    /// the Kubernetes connection with the entry and access it is.
    ///
    /// Example data gives the screens a target without Talos access, and
    /// a Kubernetes source once a context is chosen. A kubeconfig context
    /// gives the screens nothing. Talos gives both from the one
    /// [`LiveSource`] the last overview was read with, the screens once it
    /// has a client and the Kubernetes source once the access it was read
    /// with is known too. The connection's id covers what picks the
    /// cluster and its credentials, not the target node, so changing node
    /// keeps a watch; `kube_identity` keeps it while the source is missing.
    pub(super) fn source(&self) -> PageSource {
        let mode = self.mode();
        let (screen, kube, kube_identity) = match (mode.is_example(), mode.kubernetes_only()) {
            (true, _) => {
                let id = self
                    .applied
                    .context
                    .as_deref()
                    .map(resources::example::connection);
                let kube = self
                    .applied
                    .context
                    .clone()
                    .zip(id.clone())
                    .map(|(context, id)| ClusterSource {
                        id,
                        context,
                        access: ClusterAccess::Example(self.hold),
                    });
                (self.screen_on(None), kube, id)
            }
            // Kubernetes-only mode reads no Talos overview, so no screen
            // has a target.
            (false, Some(kubernetes_only)) => {
                let access = kubernetes_only.access();
                let kube = access.map(|access| ClusterSource {
                    id: access.id(),
                    context: access.context().to_owned(),
                    access: ClusterAccess::Direct(access.clone()),
                });
                (None, kube, access.map(|access| access.id()))
            }
            (false, None) => {
                let live = self.live_source();
                let screen = live.clone().and_then(|live| self.screen_on(Some(live)));
                let kube = live.and_then(|live| self.talos_source(live));
                let id = self.registry.active().access.map(|access| access.key());
                (screen, kube, id)
            }
        };
        PageSource {
            screen,
            kube,
            // The entry the window is on is the one the source's connection
            // is: both change together here.
            entry: self.active_cluster().map(str::to_owned),
            kube_identity,
        }
    }

    /// The selected node of the last overview, with `live` Talos access or
    /// none for example data. `None` until a target exists.
    fn screen_on(&self, live: Option<LiveSource>) -> Option<ScreenSource> {
        let cluster = self.overview.data()?;
        let node = self.selected_summary()?;
        Some(ScreenSource {
            target: Target {
                epoch: self.epoch,
                context: cluster.name.clone(),
                node: node.name.clone(),
                address: node.address.clone(),
            },
            nodes: Arc::new(self.nodes.clone()),
            live,
        })
    }

    /// Talos access for the cluster the last overview read, once it has a
    /// client.
    fn live_source(&self) -> Option<LiveSource> {
        let cluster = self.overview.data()?;
        let mut collector =
            ClusterOverviewCollector::new(self.applied.path.clone(), self.applied.context.clone());
        collector.set_kubeconfig_selection(self.kubeconfig.clone());
        Some(LiveSource {
            client: cluster.client.clone()?,
            cluster: Arc::new(cluster.clone()),
            collector,
            config_path: self.applied.path.clone(),
            applied: self.applied_access(),
        })
    }

    /// The Kubernetes API of the Talos cluster `live` reaches, once a
    /// context is chosen and the access it was read with is known.
    fn talos_source(&self, live: LiveSource) -> Option<ClusterSource> {
        self.applied.context.as_ref()?;
        let active = self.registry.active();
        Some(ClusterSource {
            id: active.access?.key(),
            context: live.cluster.name.clone(),
            access: ClusterAccess::Talos(Box::new(TalosAccess {
                configuration: active.access_configuration?,
                selection: self.kubeconfig.clone(),
                live,
            })),
        })
    }

    /// The local access the last overview was collected with, which a live
    /// source carries for Operations to pin its clients to.
    fn applied_access(&self) -> Option<AppliedAccess> {
        Some(AppliedAccess {
            config_path: self.applied.path.clone(),
            selection: self.kubeconfig.clone(),
            configuration: self.registry.active().access_configuration?,
            identity: self.registry.active().access?,
        })
    }
}
