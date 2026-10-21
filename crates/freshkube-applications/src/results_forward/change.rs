//! The page owns its route subscription; ForwardList owns the forward.
use super::*;
use crate::results_forward::{Binding, Target};
use freshkube_delivery::entry::pool::ResultsRoute;
use gpui_kit::component::{Disableable, Sizable, button::Button};

impl ChangePage {
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
            self.results = Binding::default();
            false
        };
        if changed {
            self.loader.reset();
            self.stale = None;
            self.hide_outcomes();
            self.show(self.fetch.unread());
        }
    }

    fn results_changed(&mut self, cx: &mut Context<Self>) {
        self.results.sync(cx);
        self.loader.reset();
        self.stale = None;
        self.hide_outcomes();
        self.show(self.fetch.unread());
        if self.visible {
            self.read(None, cx);
        }
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

    pub(crate) fn results_action(&self, cx: &mut Context<Self>) -> Option<Button> {
        let hop = self.change.hop(self.selected.as_ref()?)?;
        let Shows::Hop(detail) = &hop.shows else {
            return None;
        };
        if detail.kind != "Builds" && detail.kind != "PipelineRun" {
            return None;
        }
        let session = self.results.session.as_ref()?.read(cx);
        Some(
            Button::new("change-forward-results")
                .outline()
                .xsmall()
                .label(session.label.clone())
                .tooltip(session.words.clone())
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
