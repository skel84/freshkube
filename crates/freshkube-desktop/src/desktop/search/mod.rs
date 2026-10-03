//! Command-K searches local destinations and bounded metadata-only object names.
use super::{Page, Pilot};
use crate::{
    backend::{self, OwnedJob},
    resources::{self, KubeAccess, KubeSource, model::ObjectRef},
    ui::dp,
};
use freshkube_core::resources::{ResourceKind, builtin};
use gpui_kit::{
    component::{
        WindowExt,
        command::{CommandGroup, CommandItem, CommandState},
    },
    *,
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio::runtime::Handle;
mod model;
mod reads;
mod view;
use model::{Destination, Entry, KINDS};

#[derive(Clone)]
struct Part {
    entries: Vec<Entry>,
    capped: bool,
}
pub(super) struct Search {
    runtime: Handle,
    command: Entity<CommandState>,
    source_id: Option<String>,
    local: Vec<Entry>,
    parts: BTreeMap<&'static str, Result<Part, String>>,
    groups: Vec<CommandGroup>,
    destinations: Vec<Vec<Option<Destination>>>,
    notes: Vec<(SharedString, SharedString)>,
    query: String,
    open: bool,
    sequence: u64,
    closed_at: Option<Instant>,
    jobs: Vec<(OwnedJob, Task<()>)>,
}
impl Search {
    pub(super) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            runtime,
            command: cx.new(|cx| CommandState::new(window, cx)),
            source_id: None,
            local: Vec::new(),
            parts: BTreeMap::new(),
            groups: Vec::new(),
            destinations: Vec::new(),
            notes: Vec::new(),
            query: String::new(),
            open: false,
            sequence: 0,
            closed_at: None,
            jobs: Vec::new(),
        }
    }
    #[cfg(any(debug_assertions, feature = "stress"))]
    pub(super) fn startup_query(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.query = query.into();
        self.rebuild();
        self.command
            .update(cx, |command, cx| command.set_query(query, window, cx));
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.jobs.clear();
        self.sequence = self.sequence.wrapping_add(1);
        self.closed_at = Some(cx.background_executor().now());
    }
}
impl Pilot {
    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let local = self.local_search_entries(cx);
        let source = self.kube_source();
        let search = self.search.clone();
        search.update(cx, |search, cx| search.begin(local, source, window, cx));
        let pilot = cx.entity().downgrade();
        let on_close = search.downgrade();
        window.open_dialog(cx, move |dialog, window, _| {
            let pilot = pilot.clone();
            let on_close = on_close.clone();
            let entity = search.clone();
            dialog
                .p_0()
                .w(crate::ui::dp_px(600., window)
                    .min(window.viewport_size().width - crate::ui::dp_px(32., window)))
                .close_button(false)
                .on_close(move |_, _, cx| {
                    _ = on_close.update(cx, |search, cx| search.close(cx));
                })
                .child(SearchView {
                    search: entity,
                    pilot,
                })
        });
        let command = self.search.read(cx).command.clone();
        command.update(cx, |state, cx| state.focus(window, cx));
    }
    fn search_destination(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match destination {
            Destination::Page(page) => self.navigate(page, window, cx),
            Destination::Kind(kind) => self.open_kind(kind, window, cx),
            Destination::Node(name) => self.unless_shell(window, cx, move |this, window, cx| {
                this.resources
                    .update(cx, |resources, cx| resources.close_for_link(cx));
                this.open_node_by_name(&name, super::nodes::NodeTab::Overview, window, cx)
            }),
            Destination::Object(kind, object) => {
                self.open_object(kind, object, resources::Tab::Overview, window, cx)
            }
        }
    }
}
use view::SearchView;
#[cfg(test)]
mod tests;
