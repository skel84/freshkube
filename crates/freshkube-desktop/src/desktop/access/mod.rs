//! Access replacement is separate from a node selection and a summary generation.
use super::*;
use crate::{
    backend::{CollectedOverview, ObservedNodes},
    state::Request,
};

impl Pilot {
    pub(super) fn observed_nodes(&self) -> Option<ObservedNodes> {
        self.summary_session.as_ref()?;
        Some(ObservedNodes {
            access: self.access?,
            nodes: self
                .kubernetes_summary
                .data()
                .map(|summary| summary.nodes.clone())
                .unwrap_or_else(|| {
                    freshkube_core::kubernetes_summary::Part::Failed(
                        "Waiting for shared Nodes".into(),
                    )
                }),
        })
    }

    pub(super) fn overview_received(
        &mut self,
        epoch: u64,
        request: Request<AppliedConfig>,
        result: Result<CollectedOverview, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.epoch || !self.overview.is_loading() || !self.overview.is_current(&request)
        {
            return;
        }
        self.overview_job = None;
        match result {
            Ok(collected) => {
                let changed = self
                    .access_configuration
                    .is_some_and(|old| old != collected.configuration)
                    || collected
                        .access
                        .is_some_and(|new| self.access.is_some_and(|old| old != new));
                if changed && !resources::shell::running_anywhere(cx).is_empty() {
                    // Cancel leaves the original explicit session in place. Mark
                    // ordinary data stale, and ask once per replacement; manual
                    // Refresh allows another confirmation after a cancellation.
                    self.overview.apply(&request, Err("Access changed; refresh to reconnect after confirming the running shell can end".into()));
                    self.rebuild_joined_nodes();
                    let replacement = (collected.configuration, collected.access);
                    if self.prompted_access != Some(replacement) {
                        self.prompted_access = Some(replacement);
                        self.unless_shell(window, cx, move |view, window, cx| {
                            if view.epoch == epoch && view.overview.is_current(&request) {
                                view.install_overview(request, collected, true, window, cx);
                            }
                        });
                    }
                } else {
                    self.install_overview(request, collected, changed, window, cx);
                }
            }
            Err(error) => {
                if self.overview.apply(&request, Err(error)) {
                    self.rebuild_joined_nodes();
                }
            }
        }
        cx.notify();
    }

    fn install_overview(
        &mut self,
        mut request: Request<AppliedConfig>,
        collected: CollectedOverview,
        changed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if changed {
            self.invalidate_target(window, cx);
            request = self.overview.begin(self.applied.clone());
        }
        self.access_configuration = Some(collected.configuration);
        if collected.access.is_some() {
            self.access = collected.access;
        }
        if self.overview.apply(&request, collected.cluster) {
            let fresh = !self.overview.is_stale() && self.overview.error().is_none();
            if fresh {
                self.overview_succeeded();
            }
            self.sync_nodes(window, cx);
            if fresh {
                self.refresh_services(window, cx);
                self.ensure_summary(window, cx);
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
