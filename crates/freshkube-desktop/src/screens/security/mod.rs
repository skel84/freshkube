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
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::status::{Part, Segment};

use super::{
    Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, content_width, failure_banner, field,
    gate, mono, panel, partial_notice, refresh_control, segment, stat,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};

const CONTEXT: &str = "TalosSecurity";
/// The page header's id prefix.
const PREFIX: &str = "security";
const ROW_HEIGHT: f32 = 34.;
/// Below this content width the details pane moves under the list.
const SIDE_DETAILS: f32 = 920.;
const LIST_MIN_HEIGHT: f32 = 240.;
const DETAILS_HEIGHT: f32 = 260.;

actions!(
    talos_security,
    [NextItem, PreviousItem, FirstItem, LastItem]
);

mod audit;
mod example;
#[cfg(test)]
mod tests;
mod view;

use audit::{Display, Item, Section, identity_parts};
#[cfg(test)]
use audit::{Verdict, items};
use example::example;

pub(crate) struct SecurityScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<SecurityAuditSnapshot>,
    /// Selection survives refreshes by key; the first row shows until one is chosen.
    selected: Option<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The meta line's parts for the audit at a loader revision.
    status: Option<(u64, Segment)>,
    /// The audit's rows and counts at a loader revision.
    display: (u64, Display),
}

impl EventEmitter<ScreenEvent> for SecurityScreen {}

impl ScreenPanel for SecurityScreen {
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextItem, Some(CONTEXT)),
            KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
            KeyBinding::new("home", FirstItem, Some(CONTEXT)),
            KeyBinding::new("end", LastItem, Some(CONTEXT)),
        ]);
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            selected: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            status: None,
            display: (u64::MAX, Display::default()),
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
        self.display = (revision, display);
    }

    /// Index of the selected row; the first row until the user picks one.
    fn selected_index(&self, items: &[Item]) -> Option<usize> {
        match &self.selected {
            Some(key) => items.iter().position(|item| &item.key == key),
            None => (!items.is_empty()).then_some(0),
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.sync();
        let items = self.items();
        if items.is_empty() {
            return;
        }
        let current = self.selected_index(&items).unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(items.len() - 1);
        self.selected = Some(items[next].key.clone());
        cx.notify();
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
