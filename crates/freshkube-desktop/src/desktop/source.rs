//! What every page is handed, built in one place: the session owner until
//! #44's session type takes it. [`Mode`] holds what the window shows;
//! `Pilot::source` reads it with the shell's other fields and decides, in
//! one match, whether the window shows example data, a kubeconfig context
//! or a Talos cluster.

use std::sync::Arc;

use freshkube_core::cluster_overview::ClusterOverviewCollector;
use freshkube_core::cluster_source::{
    AppliedAccess, ClusterAccess, ClusterSource, Hold, LiveSource, TalosAccess,
};

use super::Pilot;
use super::kubernetes_only::KubernetesOnly;
use crate::backend::Target;
use crate::resources;
use crate::screens::{PageSource, ScreenSource};

/// What the window shows, as two answers: example data, with what it holds
/// back, or a live cluster, and Talos with Kubernetes or Kubernetes alone. They are independent:
/// example data is Kubernetes-only too after "Use Kubernetes only…" under
/// `--fixture`, or on the `kubernetes-only` debug page. The shell keeps one
/// in `Pilot.mode`; readers ask its questions, and the Kubernetes-only flow
/// changes it through its methods.
pub(super) struct Mode {
    /// Example data and what it never answers (`FRESHKUBE_FIXTURE_HOLD`),
    /// or `None` for a live cluster: a hold is example data's alone.
    example: Option<Hold>,
    /// Example data's clock: each refresh of the example cluster takes the
    /// next tick, so its readings move.
    tick: u64,
    kubernetes_only: Option<KubernetesOnly>,
}

impl Mode {
    /// Example data with its hold, or a live cluster, as the launch options
    /// chose; nothing changes it later.
    pub(super) fn new(example: Option<Hold>) -> Self {
        Self {
            example,
            tick: 0,
            kubernetes_only: None,
        }
    }

    /// Example data, which reads no cluster.
    pub(super) fn is_example(&self) -> bool {
        self.example.is_some()
    }

    /// Example data and its hold; `None` for a live cluster.
    pub(super) fn example(&self) -> Option<Hold> {
        self.example
    }

    /// Example data that never answers Talos, so the overview and the
    /// Kubernetes summary stay on their loading rows.
    pub(super) fn holds_talos(&self) -> bool {
        self.example.is_some_and(|hold| hold.talos)
    }

    /// Talos is part of the session: false when a kubeconfig is all it
    /// reads.
    pub(super) fn has_talos(&self) -> bool {
        self.kubernetes_only.is_none()
    }

    /// The kubeconfig a session without Talos reads, and its connection.
    pub(super) fn kubernetes_only(&self) -> Option<&KubernetesOnly> {
        self.kubernetes_only.as_ref()
    }

    pub(super) fn kubernetes_only_mut(&mut self) -> Option<&mut KubernetesOnly> {
        self.kubernetes_only.as_mut()
    }

    /// Reads `kube` without Talos from now on.
    pub(super) fn set_kubernetes_only(&mut self, kube: KubernetesOnly) {
        self.kubernetes_only = Some(kube);
    }

    /// Takes Talos back, returning the Kubernetes-only state it ends.
    pub(super) fn leave_kubernetes_only(&mut self) -> Option<KubernetesOnly> {
        self.kubernetes_only.take()
    }

    /// Advances example data's clock and returns the new tick.
    pub(super) fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// Turns example data on or off under a test; for the gate, switch,
    /// access, Kubernetes summary, dock and Kubernetes-only tests.
    /// Example data turned on starts holding nothing; turned off, its hold
    /// goes with it.
    #[cfg(test)]
    pub(super) fn set_example(&mut self, example: bool) {
        if example {
            self.example.get_or_insert(Hold::NONE);
        } else {
            self.example = None;
        }
    }

    /// Example data's hold, to change under a test; for the gate, switch,
    /// access, motion, Resources loading and Applications column tests,
    /// which all run on example data.
    #[cfg(test)]
    pub(super) fn hold_mut(&mut self) -> &mut Hold {
        self.example
            .as_mut()
            .expect("a hold is example data's; this test runs on a live cluster")
    }

    /// Example data's clock, for the menu and Nodes table tests.
    #[cfg(test)]
    pub(super) fn tick(&self) -> u64 {
        self.tick
    }
}

impl Pilot {
    /// What the window shows: example data or not, with Talos or not.
    pub(super) fn mode(&self) -> &Mode {
        &self.mode
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
        let (screen, kube, kube_identity) = match (mode.example(), mode.kubernetes_only()) {
            (Some(hold), _) => {
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
                        access: ClusterAccess::Example(hold),
                    });
                (self.screen_on(None), kube, id)
            }
            // Kubernetes-only mode reads no Talos overview, so no screen
            // has a target.
            (None, Some(kubernetes_only)) => {
                let access = kubernetes_only.access();
                let kube = access.map(|access| ClusterSource {
                    id: access.id(),
                    context: access.context().to_owned(),
                    access: ClusterAccess::Direct(access.clone()),
                });
                (None, kube, access.map(|access| access.id()))
            }
            (None, None) => {
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
            talos: mode.has_talos(),
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
