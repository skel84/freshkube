//! The page owns its route subscription; ForwardList owns the forward.
use super::*;
use crate::results_forward::{Binding, Target};
use freshkube_delivery::entry::pool::ResultsRoute;
use gpui_kit::component::{Disableable, Sizable, button::Button};

impl ChangePage {
    /// Example reads stay synthetic; their owner hands over the same workspace
    /// and example access that the Changes list uses for its explicit action.
    pub(crate) fn configure_example_results(
        &mut self,
        runtime: &tokio::runtime::Handle,
        source: Option<&crate::resources::ClusterSource>,
        across: &crate::across::Across,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.fetch, Fetch::Example { .. }) {
            return;
        }
        if let Some(source) = source.filter(|source| source.access.is_example()) {
            let target = Target::of(&across.sources, across.mapping.open.as_deref(), source);
            self.results
                .configure(target, source, runtime.clone(), Self::results_changed, cx);
        } else {
            self.results = Binding::default();
        }
        self.detail = self.derive_detail();
        cx.notify();
    }

    pub(super) fn configure_results(&mut self, cx: &mut Context<Self>) {
        let changed = if let Fetch::Live {
            runtime,
            access,
            place,
            ..
        } = &self.fetch
        {
            let source = crate::resources::ClusterSource {
                id: place.cluster.clone(),
                context: place.label.clone(),
                access: access.clone(),
            };
            let target = Target::of(&place.sources, place.mapping.open.as_deref(), &source);
            self.results
                .configure(target, &source, runtime.clone(), Self::results_changed, cx)
        } else {
            // The example owner configures this binding; refreshing synthetic
            // data must preserve its shared forward session.
            return;
        };
        if changed {
            self.loader.reset();
            self.stale = None;
            self.hide_outcomes();
            self.show(self.fetch.unread());
        }
    }

    fn results_changed(&mut self, cx: &mut Context<Self>) {
        let before = (self.results.route(), self.results.epoch());
        self.results.sync(cx);
        if before != (self.results.route(), self.results.epoch()) {
            self.loader.retain_stale(self.results_identity());
            self.stale = None;
            if self.visible {
                self.read(None, cx);
            }
        }
        self.detail = self.derive_detail();
        cx.notify();
    }

    pub(super) fn results_route(&self) -> ResultsRoute {
        self.results.route()
    }

    pub(super) fn results_identity(&self) -> String {
        let identity = self.fetch.identity();
        let epoch = self.results.epoch();
        match epoch {
            Some(epoch) => format!("{identity}/forward/{}/{}", epoch.id, epoch.start),
            None => identity,
        }
    }

    /// The pruned example run cannot infer installation from its absence.
    /// Its configured forward supplies the next step; live evidence is kept.
    pub(super) fn results_example_banner(
        &self,
        key: &str,
    ) -> Option<(Tone, SharedString, SharedString)> {
        if key != "pr-run" || !matches!(self.fetch, Fetch::Example { .. }) {
            return None;
        }
        let words = self.results.words.as_ref()?;
        Some((Tone::Unknown, "Run pruned; Results forward configured.".into(),
            format!("Tekton pruned this run. {words}. This example keeps synthetic build evidence; no Results server is contacted.").into()))
    }

    pub(crate) fn results_action(&self, cx: &mut Context<Self>) -> Option<Button> {
        let hop = self.change.hop(self.selected.as_ref()?)?;
        let Shows::Hop(detail) = &hop.shows else {
            return None;
        };
        if detail.kind != "Builds" && detail.kind != "PipelineRun" {
            return None;
        }
        let session = self.results.session.as_ref()?.read(cx);
        let words = if self.loader.is_stale() {
            format!(
                "{} · Showing the last read while reading again",
                session.words
            )
            .into()
        } else {
            session.words.clone()
        };
        Some(
            Button::new("change-forward-results")
                .outline()
                .xsmall()
                .label(session.label.clone())
                .tooltip(words)
                .disabled(!session.enabled)
                .on_click(cx.listener(|this, _, _, cx| this.forward_results(cx))),
        )
    }

    fn forward_results(&mut self, cx: &mut Context<Self>) {
        let session = self.results.session.clone();
        let Some(session) = session else {
            return;
        };
        if let Some((entry, link)) = session.update(cx, |session, cx| session.act(cx)) {
            cx.emit(ChangeEvent::Switch(entry, Box::new(link)));
        }
    }
}
