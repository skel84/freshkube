//! The selected ExternalSecret's cause, refresh facts and store status reads.
use super::*;
use crate::ui::{self, Tone};
use freshkube_core::resources::{
    ExternalSecret, MAX_STORES, StoreRead, read_external_secret_stores,
};
use gpui_kit::component::{h_flex, v_flex};

pub(in crate::pane) mod refresh;
#[cfg(test)]
mod refresh_tests;
#[cfg(test)]
mod tests;
mod view;

struct Line {
    id: SharedString,
    label: SharedString,
    status: SharedString,
    tone: Tone,
    detail: SharedString,
    accessible: SharedString,
}

impl Line {
    fn describe(&mut self) {
        self.accessible = [
            self.label.as_ref(),
            self.status.as_ref(),
            self.detail.as_ref(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
        .into();
    }
}

#[derive(Default)]
pub(super) struct ExternalView {
    secret: Option<ExternalSecret>,
    conditions: Vec<Line>,
    stores: Vec<Line>,
    previous: Vec<StoreRead>,
    refreshed: SharedString,
    interval: SharedString,
    policy: SharedString,
    note: Option<SharedString>,
    job: Option<Job>,
    seq: u64,
}

impl ExternalView {
    pub(super) fn reset(&mut self) {
        self.stop();
        *self = Self {
            seq: self.seq,
            ..Default::default()
        };
    }

    pub(super) fn stop(&mut self) {
        self.job = None;
        self.seq = self.seq.wrapping_add(1);
    }

    fn set_secret(&mut self, secret: ExternalSecret) {
        let changed = self
            .secret
            .as_ref()
            .is_none_or(|before| before.stores != secret.stores);
        if changed {
            self.previous.clear();
        }
        self.conditions = secret
            .conditions
            .iter()
            .map(|condition| Line {
                accessible: SharedString::default(),
                id: format!("external-condition-{}", condition.kind).into(),
                label: condition.kind.clone().into(),
                status: condition.status.clone().into(),
                tone: super::overview::condition_tone(&condition.kind, &condition.status),
                detail: [condition.reason.as_str(), condition.message.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ")
                    .into(),
            })
            .collect();
        // Failures lead, retaining the operator's own wording.
        self.conditions
            .sort_by_key(|line| if line.tone == Tone::Crit { 0 } else { 1 });
        self.refreshed = secret
            .refreshed
            .map(local_time)
            .unwrap_or_else(|| "Not refreshed yet".into())
            .into();
        self.interval = if secret.interval_default {
            format!("{} (default)", secret.interval).into()
        } else {
            secret.interval.clone().into()
        };
        self.policy = if secret.policy_default {
            format!("{} (default)", secret.policy).into()
        } else {
            secret.policy.clone().into()
        };
        let mut notes = Vec::new();
        if secret.unsupported_sources {
            notes.push("Some sources cannot be inspected here; their health is unknown".to_owned());
        }
        if secret.stores_omitted > 0 {
            notes.push(format!(
                "{} more stores not read (limit {MAX_STORES})",
                secret.stores_omitted
            ));
        }
        self.note = (!notes.is_empty()).then(|| notes.join(". ").into());
        if changed {
            self.stores = secret
                .stores
                .iter()
                .map(|store| Line {
                    accessible: SharedString::default(),
                    id: format!(
                        "external-store-{}-{}-{}",
                        store.kind,
                        store.namespace.as_deref().unwrap_or_default(),
                        store.name
                    )
                    .into(),
                    label: format!(
                        "{} {}{}",
                        store.kind,
                        store
                            .namespace
                            .as_ref()
                            .map(|ns| format!("{ns}/"))
                            .unwrap_or_default(),
                        store.name
                    )
                    .into(),
                    status: "Reading".into(),
                    tone: Tone::Unknown,
                    detail: "Reading the store's own Ready condition".into(),
                })
                .collect();
        }
        for line in self.conditions.iter_mut().chain(&mut self.stores) {
            line.describe();
        }
        self.secret = Some(secret);
    }

    fn finish(&mut self, reads: Vec<StoreRead>) {
        for (line, read) in self.stores.iter_mut().zip(&reads) {
            let previous = self
                .previous
                .iter()
                .find(|old| old.store == read.store)
                .and_then(|old| old.status.as_ref().ok());
            let (status, error) = match &read.status {
                Ok(status) => (Some(status), None),
                Err(error) => (previous, Some(error)),
            };
            if let Some(ready) = status.and_then(|status| status.ready.as_ref()) {
                line.status = format!(
                    "Ready {}{}",
                    ready.status,
                    if error.is_some() { " · stale" } else { "" }
                )
                .into();
                let reported_tone = super::overview::condition_tone("Ready", &ready.status);
                line.tone = if error.is_some() && reported_tone != Tone::Crit {
                    Tone::Warn
                } else {
                    reported_tone
                };
                line.detail = [ready.reason.as_str(), ready.message.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ")
                    .into();
            } else {
                line.status = "Unknown".into();
                line.tone = Tone::Unknown;
                line.detail = "No Ready condition reported".into();
            }
            if let Some(error) = error {
                let state = match error.kind {
                    FailureKind::Forbidden => "Not permitted to read store status",
                    FailureKind::NotFound if read.api_absent => "Store API not served",
                    FailureKind::NotFound => "Store not found",
                    _ => "Could not read store status",
                };
                line.detail = format!(
                    "{}{}: {}",
                    if previous.is_some() && !line.detail.is_empty() {
                        format!("Last known: {}. ", line.detail)
                    } else {
                        String::new()
                    },
                    state,
                    error.message
                )
                .into();
            }
        }
        for line in &mut self.stores {
            line.describe();
        }
        for read in reads {
            if read.status.is_ok() {
                self.previous.retain(|old| old.store != read.store);
                self.previous.push(read);
            }
        }
    }
}

impl DetailPane {
    pub(super) fn start_external_stores(&mut self, cx: &mut Context<Self>) {
        self.external.stop();
        let Some(secret) = self
            .detail
            .as_ref()
            .and_then(|d| d.view.as_ref())
            .and_then(|v| v.document.overview.external_secret.clone())
        else {
            self.external.reset();
            return;
        };
        self.external.set_secret(secret.clone());
        if !self.active {
            return;
        }
        let Some(access) = self.access.clone() else {
            return;
        };
        let identity = self.detail.as_ref().unwrap().target.identity.clone();
        let seq = self.external.seq;
        if let ClusterAccess::Example(_) = access {
            let reads = crate::example::external_secrets::stores(&secret);
            self.finish_external_stores(&identity, seq, reads, cx);
            return;
        }
        let requested = secret.stores.clone();
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            READ_DEADLINE,
            "Store status read timed out".to_owned(),
            async move {
                match access.client().await {
                    Ok(client) => Ok(read_external_secret_stores(&client, &secret).await),
                    Err(error) => Ok(secret
                        .stores
                        .into_iter()
                        .map(|store| {
                            StoreRead::new(
                                store,
                                Err(Failure::new(FailureKind::Other, error.message.clone())),
                            )
                        })
                        .collect()),
                }
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(reads)) => reads,
                Ok(Err(message)) => requested
                    .into_iter()
                    .map(|store| {
                        StoreRead::new(
                            store,
                            Err(Failure::new(FailureKind::Timeout, message.clone())),
                        )
                    })
                    .collect(),
                Err(_) => return,
            };
            _ = this.update(cx, |pane, cx| {
                pane.finish_external_stores(&identity, seq, result, cx)
            });
        });
        self.external.job = Some((job, task));
    }

    fn finish_external_stores(
        &mut self,
        identity: &ResourceIdentity,
        seq: u64,
        reads: Vec<StoreRead>,
        cx: &mut Context<Self>,
    ) {
        if !self.active
            || seq != self.external.seq
            || self.target_identity() != Some(identity)
            || self
                .detail
                .as_ref()
                .is_some_and(|detail| detail.read == DocumentRead::Deleted)
        {
            return;
        }
        self.external.job = None;
        self.external.finish(reads);
        cx.notify();
    }
}
