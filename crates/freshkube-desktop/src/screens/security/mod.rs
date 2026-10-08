//! Security: the TUI's security view. Certificate expiry for the Talos and
//! Kubernetes PKI, the Talos RBAC role, and volume encryption on the target
//! node. Read-only.
//!
//! Every part of the audit is its own source. A part that didn't answer
//! shows as unknown (and is named in the partial notice), never as failed or
//! insecure.
use chrono::{DateTime, Duration, Utc};
use freshkube_core::HealthIndicator;
use freshkube_core::security_lifecycle::{
    CertificateAudit, CertificateExpiryStatus, CertificateSource, ClusterIdentity,
    EncryptionProvider, RbacAudit, SecurityAuditCollector, SecurityAuditSnapshot, SourceSnapshot,
    VolumeEncryptionAudit,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use freshkube_ui::inspector::{self, Inspector, InspectorSplit};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::status::{Part, Segment};
use freshkube_ui::table::TableState;

use super::{
    Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, TableLoading, failure_banner, field,
    first_read, gate, mono, partial_notice, refresh_control, segment, stat,
};
use crate::palette::palette;
use crate::ui::{self, Tone, dp};

const CONTEXT: &str = "TalosSecurity";
/// The page header's id prefix.
const PREFIX: &str = "security";

actions!(
    talos_security,
    [NextItem, PreviousItem, FirstItem, LastItem, CloseDetails]
);

mod audit;
mod example;
mod table;
#[cfg(test)]
mod tests;
mod view;

use audit::{Display, Item, identity_parts};
#[cfg(test)]
use audit::{Section, Verdict, items};
use example::example;

/// The selected row's key and its name, kept for when the row goes.
struct Selection {
    key: SharedString,
    name: String,
}

pub(crate) struct SecurityScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<SecurityAuditSnapshot>,
    /// The selected row, which survives a refresh that keeps it; one that
    /// goes keeps its name in the Inspector.
    selected: Option<Selection>,
    focus: FocusHandle,
    table: TableState,
    /// The table's columns for the derived rows, and their total width.
    columns: (Vec<table::Column>, f32),
    /// The Inspector's width beside the table, saved under `security`.
    split: InspectorSplit,
    /// The meta line's parts for the audit at a loader revision.
    status: Option<(u64, Segment)>,
    /// The audit's rows and counts at a loader revision.
    display: (u64, Display),
    /// The table's rows until the first answer.
    loading: TableLoading,
}

impl EventEmitter<ScreenEvent> for SecurityScreen {}

impl ScreenPanel for SecurityScreen {
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextItem, Some(CONTEXT)),
            KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
            KeyBinding::new("home", FirstItem, Some(CONTEXT)),
            KeyBinding::new("end", LastItem, Some(CONTEXT)),
            KeyBinding::new("escape", CloseDetails, Some(CONTEXT)),
        ]);
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            selected: None,
            focus: cx.focus_handle(),
            table: TableState::new(PREFIX),
            columns: table::columns(&[]),
            split: InspectorSplit::new(PREFIX, cx),
            status: None,
            display: (u64::MAX, Display::default()),
            loading: TableLoading::new(PREFIX, cx),
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.selected = None;
        }
        self.source = source;
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn status(&mut self) -> Option<&Segment> {
        self.sync_status();
        self.status.as_ref().map(|(_, line)| line)
    }

    fn loading_motion(&self, cx: &App) -> Option<Entity<freshkube_ui::table::LoadingMotion>> {
        self.loading.motion(self.first_read(cx))
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            self.loader
                .resolve(source.target.clone(), example(&source, Utc::now()));
            cx.notify();
            return;
        };
        let target = source.target.clone();
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "the security audit",
            async move {
                // A worker can't serve Kubernetes credentials, so pin that
                // part to a control plane; volumes still use the target node.
                let pinned = live.cluster.control_plane_ip();
                let client = pinned.as_deref().map_or_else(
                    || live.client.clone(),
                    |address| live.client.with_node(address),
                );
                let mut snapshot = SecurityAuditCollector::new(live.config_path.clone())
                    .collect_for_context(&client, &target.context, &target.address)
                    .await;
                if pinned.is_none() {
                    snapshot.talos_kubeconfig_certificates = SourceSnapshot::Unavailable {
                        reason:
                            "No control plane is known to serve the Talos kubeconfig; no ambient kubeconfig was audited"
                                .into(),
                    };
                }
                Ok(snapshot)
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

impl SecurityScreen {
    /// Whether the first answer is still to come: the table shows its
    /// loading rows.
    fn first_read(&self, cx: &App) -> bool {
        first_read(self.source.as_ref(), &self.loader, Scope::Cluster, cx)
    }

    /// The rows as last derived; [`Self::sync`] brings them up to date.
    fn items(&self) -> &[Item] {
        &self.display.1.items
    }

    /// Derives the rows and counts again when a new audit arrives or the
    /// old one goes.
    fn sync(&mut self) {
        let revision = self.loader.revision();
        if self.display.0 == revision {
            return;
        }
        let display = self.loader.data().map(Display::new).unwrap_or_default();
        self.columns = table::columns(&display.items);
        self.display = (revision, display);
    }

    /// The selected row, while it is listed.
    fn selected_item(&self) -> Option<&Item> {
        let key = &self.selected.as_ref()?.key;
        self.items().iter().find(|item| &item.key == key)
    }

    fn select(&mut self, key: SharedString, cx: &mut Context<Self>) {
        let Some(item) = self.items().iter().find(|item| item.key == key) else {
            return;
        };
        let name = item.name.clone();
        self.selected = Some(Selection { key, name });
        freshkube_ui::table::reveal(self, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Escape closes the Inspector by clearing the selection; with nothing
    /// selected it goes on to whatever handles it next.
    fn close_details(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.sync();
        if let Some(key) = freshkube_ui::table::step(self, delta, cx) {
            self.select(key, cx);
        }
    }

    /// Derives the status bar's line again when a new audit arrives.
    fn sync_status(&mut self) {
        let revision = self.loader.revision();
        if self.status.as_ref().is_some_and(|(at, _)| *at == revision) {
            return;
        }
        let parts = self
            .loader
            .data()
            .map(|snapshot| identity_parts(&snapshot.identity))
            .unwrap_or_default();
        let line = segment(
            self.source.as_ref(),
            &self.loader,
            parts.into_iter().map(Part::new),
        );
        self.status = Some((revision, line));
    }
}
