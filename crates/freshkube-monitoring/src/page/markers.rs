//! The page's markers: deploys and node events, read from the cluster with
//! each generation while the page shows, merged with Talos boot times,
//! filtered by the header's toggles and the dashboard's namespace and node
//! variables, and handed to every panel. Example data has its own.
use std::rc::Rc;

use freshkube_core::monitoring::{
    ErrorKind, QueryError,
    markers::{self, Marker, MarkerKind, Markers},
    model::time::TimeWindow,
};
use gpui_kit::{Context, SharedString};
use talos_rs::TalosClient;

use super::board::Board;
use super::{MonitoringPage, Request};

/// A read this recent, reaching back as far, is used again.
const FRESH: i64 = 15;

/// Variable names that pick namespaces, and nodes.
const NAMESPACE_VARIABLES: [&str; 2] = ["namespace", "ns"];
const NODE_VARIABLES: [&str; 3] = ["node", "nodename", "kubernetes_node"];

/// The Talos nodes whose boot times mark reboots: each name and address.
#[derive(Clone)]
pub struct TalosNodes {
    pub client: TalosClient,
    pub nodes: Vec<(String, String)>,
}

impl TalosNodes {
    fn same(&self, other: &Self) -> bool {
        self.client.connection_id() == other.client.connection_id() && self.nodes == other.nodes
    }
}

/// Which markers the header's toggles show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkerToggle {
    Deploys,
    Nodes,
}

pub(super) struct MarkerState {
    kube: Vec<Marker>,
    talos: Vec<Marker>,
    /// Both, merged, oldest first.
    pub(super) all: Vec<Marker>,
    /// What couldn't be read, for the toggles' tooltip.
    pub(super) unavailable: Option<SharedString>,
    kube_request: Option<Request>,
    talos_request: Option<Request>,
    /// How far back the last read reached and when it was asked.
    read: Option<(i64, i64)>,
    pub(super) deploys: bool,
    pub(super) nodes: bool,
    /// What the panels show now.
    pub(super) shown: Rc<[Marker]>,
}

impl Default for MarkerState {
    fn default() -> Self {
        Self {
            kube: Vec::new(),
            talos: Vec::new(),
            all: Vec::new(),
            unavailable: None,
            kube_request: None,
            talos_request: None,
            read: None,
            deploys: true,
            nodes: true,
            shown: Rc::from([]),
        }
    }
}

impl MarkerState {
    /// Drops the reads in flight; showing again reads again.
    pub(super) fn hide(&mut self) {
        if self.kube_request.take().is_some() | self.talos_request.take().is_some() {
            self.read = None;
        }
    }

    /// Another cluster: nothing read so far applies. The toggles stay.
    pub(super) fn forget(&mut self) {
        *self = Self {
            deploys: self.deploys,
            nodes: self.nodes,
            ..Self::default()
        };
    }

    fn merged(&mut self) {
        let mut all = self.kube.clone();
        all.extend(self.talos.iter().cloned());
        self.all = markers::merge(all);
    }

    /// The markers the toggles and the dashboard's variables let through.
    fn filter(&self, board: Option<&Board>) -> Vec<Marker> {
        crate::probe::hit("monitoring-derive");
        let namespaces = board.and_then(|board| chosen(board, &NAMESPACE_VARIABLES));
        let nodes = board.and_then(|board| chosen(board, &NODE_VARIABLES));
        let among = |list: &Option<Vec<String>>, value: &Option<String>| match (list, value) {
            (None, _) => true,
            (Some(list), Some(value)) => list.contains(value),
            (Some(_), None) => false,
        };
        self.all
            .iter()
            .filter(|marker| match marker.kind {
                MarkerKind::Deploy => self.deploys && among(&namespaces, &marker.namespace),
                _ => self.nodes && among(&nodes, &marker.node),
            })
            .cloned()
            .collect()
    }
}

/// The values a dashboard's variable named like `names` picks, or None
/// when it has none or picks all.
fn chosen(board: &Board, names: &[&str]) -> Option<Vec<String>> {
    let variables = board.variables.as_ref()?;
    let (_, value) = variables
        .context_values()
        .into_iter()
        .find(|(name, _)| names.contains(&name.to_lowercase().as_str()))?;
    let values: Vec<String> = value.split(" + ").map(str::to_owned).collect();
    let all = values
        .iter()
        .any(|value| matches!(value.as_str(), "" | "All" | "$__all" | ".*" | ".+"));
    (!all).then_some(values)
}

impl MonitoringPage {
    /// The Talos nodes from the shell, when it has a Talos connection.
    pub fn set_talos(&mut self, talos: Option<TalosNodes>, cx: &mut Context<Self>) {
        let same = match (&self.talos, &talos) {
            (Some(old), Some(new)) => old.same(new),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.talos = talos;
        self.markers.talos.clear();
        self.markers.talos_request = None;
        self.markers.read = None;
        self.markers.merged();
        self.push_markers(cx);
    }

    /// Reads the markers over `window` unless a recent read covers it.
    pub(super) fn read_markers(&mut self, window: TimeWindow, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let now = (self.now)();
        let since = window.end - window.span as i64;
        if self.example() {
            self.markers.kube = markers::example_markers(now);
            self.markers.merged();
            self.push_markers(cx);
            return;
        }
        let fresh = self
            .markers
            .read
            .is_some_and(|(reached, at)| reached <= since && now - at < FRESH);
        if fresh || self.markers.kube_request.is_some() {
            self.push_markers(cx);
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let access = source.access.clone();
        self.markers.kube_request = Some(self.run(
            async move {
                let client = access
                    .client()
                    .await
                    .map_err(|message| QueryError::new(ErrorKind::Unavailable, message))?;
                Ok(markers::read_markers(client, since).await)
            },
            cx,
            move |this, result, cx| this.markers_read(since, now, result, cx),
        ));
        if let Some(talos) = self.talos.clone() {
            self.markers.talos_request = Some(self.run(
                async move { Ok(markers::boot_markers(talos.client, talos.nodes, since).await) },
                cx,
                |this, result, cx| this.boots_read(result, cx),
            ));
        }
        self.push_markers(cx);
    }

    fn markers_read(
        &mut self,
        since: i64,
        asked: i64,
        result: Result<Markers, QueryError>,
        cx: &mut Context<Self>,
    ) {
        self.markers.kube_request = None;
        match result {
            Ok(read) => {
                self.markers.kube = read.markers;
                self.markers.unavailable =
                    (!read.unavailable.is_empty()).then(|| read.unavailable.join("\n").into());
                self.markers.read = Some((since, asked));
            }
            Err(error) => self.markers.unavailable = Some(error.message.into()),
        }
        self.markers.merged();
        self.push_markers(cx);
        cx.notify();
    }

    fn boots_read(
        &mut self,
        result: Result<(Vec<Marker>, usize), QueryError>,
        cx: &mut Context<Self>,
    ) {
        self.markers.talos_request = None;
        if let Ok((boots, _)) = result {
            self.markers.talos = boots;
            self.markers.merged();
            self.push_markers(cx);
        }
    }

    /// Hands every panel the markers to draw, when they changed.
    pub(super) fn push_markers(&mut self, cx: &mut Context<Self>) {
        let shown = self.markers.filter(self.board.as_ref());
        if *self.markers.shown != *shown {
            self.markers.shown = shown.into();
        }
        let Some(board) = &self.board else {
            return;
        };
        for slot in &board.slots {
            let shown = self.markers.shown.clone();
            slot.view
                .update(cx, |panel, cx| panel.set_markers(shown, cx));
        }
    }

    pub(super) fn toggle_markers(&mut self, toggle: MarkerToggle, cx: &mut Context<Self>) {
        match toggle {
            MarkerToggle::Deploys => self.markers.deploys = !self.markers.deploys,
            MarkerToggle::Nodes => self.markers.nodes = !self.markers.nodes,
        }
        self.push_markers(cx);
        cx.notify();
    }
}
