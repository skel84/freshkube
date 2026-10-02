//! The shell's Kubernetes summary shares the overview's cycle and identity.
use std::{sync::Arc, time::Instant};

use freshkube_core::kubernetes_summary::{KubernetesSummary, collect_kubernetes_summary};
use gpui_kit::{Context, Window};

use super::{Page, Pilot};
use crate::{
    backend,
    resources::{KubeAccess, example},
    screens::WorkloadData,
};

impl Pilot {
    pub(super) fn deliver_workloads(
        &self,
        data: Result<Arc<WorkloadData>, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, screen)) = self
            .screens
            .iter()
            .find(|(page, _)| *page == Page::Workloads)
        {
            screen.set_workloads(
                self.applied.context.as_deref().unwrap_or_default(),
                data,
                cx,
            );
        }
    }

    pub(super) fn refresh_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.kubernetes_summary.is_loading() {
            return;
        }
        let Some(source) = self.kube_source() else {
            return;
        };
        let request = self.kubernetes_summary.begin(self.applied.clone());
        if matches!(source.access, KubeAccess::Example) {
            let summary = example::summary(&source.context, chrono::Utc::now().timestamp());
            let health = WorkloadData::from_outcome(&summary.workloads);
            self.kubernetes_summary
                .apply(&request, Ok(Arc::new(summary)));
            self.summary_health = Some(health.clone());
            self.deliver_workloads(health, cx);
            cx.notify();
            return;
        }
        let epoch = self.epoch;
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            std::time::Duration::from_secs(60),
            "Reading the Kubernetes summary timed out".into(),
            async move {
                let started = Instant::now();
                let client = source.access.client().await?;
                let summary: KubernetesSummary = collect_kubernetes_summary(client).await;
                let health = WorkloadData::from_outcome(&summary.workloads);
                Ok((Arc::new(summary), health, started.elapsed()))
            },
        );
        self.summary_job = Some(job);
        self.summary_task = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = receiver
                .await
                .unwrap_or_else(|_| Err("The summary worker stopped".into()));
            _ = this.update_in(cx, |view, _, cx| {
                if view.epoch != epoch {
                    return;
                }
                let _span = crate::perf::span("summary.apply");
                view.summary_job = None;
                let result = match answer {
                    Ok((summary, health, duration)) => {
                        if let Some(error) = summary
                            .refresh_failure()
                            .filter(|_| view.kubernetes_summary.data().is_some())
                        {
                            view.kubernetes_summary
                                .apply(&request, Err(error.to_owned()));
                            view.deliver_workloads(Err(error.to_owned()), cx);
                            cx.notify();
                            return;
                        }
                        crate::perf::value("summary.tokio", duration.as_secs_f64() * 1000.);
                        view.summary_health = Some(health.clone());
                        view.deliver_workloads(health, cx);
                        Ok(summary)
                    }
                    Err(error) => {
                        view.deliver_workloads(Err(error.clone()), cx);
                        Err(error)
                    }
                };
                view.kubernetes_summary.apply(&request, result);
                cx.notify();
            });
        }));
    }
}
