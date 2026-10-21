//! One user-confirmed write, retained independently of the pane's read jobs.
//! The delivery task holds this entity until Tokio accounts for the response.
use chrono::{DateTime, Utc};
use freshkube_core::resources::{ExternalSecret, Failure, FailureKind, refresh_external_secret};
use gpui_kit::{Context, SharedString, Task};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

use crate::ui::Tone;
use crate::{detail::DetailTarget, model::ResourceIdentity, screen::ClusterAccess};

gpui_kit::actions!(external_secrets, [RefreshExternalSecret]);

pub(crate) const OBSERVATION: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Submitting,
    Waiting,
    Finished,
}

pub(crate) struct Attempt {
    pub(crate) identity: ResourceIdentity,
    pub(crate) confirmed: DateTime<Utc>,
    before: Option<DateTime<Utc>>,
    deadline: Instant,
    pub(crate) phase: Phase,
    pub(crate) text: SharedString,
    pub(crate) tone: Tone,
}

pub(crate) struct ExternalRefresh {
    runtime: Handle,
    utc: Option<DateTime<Utc>>,
    epoch: Instant,
    last_seconds: Option<i64>,
    pub(crate) attempt: Option<Attempt>,
    timeout: Option<Task<()>>,
}

impl ExternalRefresh {
    pub(crate) fn new(runtime: Handle, cx: &Context<Self>) -> Self {
        Self {
            runtime,
            utc: None,
            epoch: cx.background_executor().now(),
            last_seconds: None,
            attempt: None,
            timeout: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_clock(&mut self, utc: DateTime<Utc>, cx: &Context<Self>) {
        self.utc = Some(utc);
        self.epoch = cx.background_executor().now();
    }

    pub(crate) fn now(&self, cx: &Context<Self>) -> DateTime<Utc> {
        self.utc.map_or_else(Utc::now, |utc| {
            utc + chrono::Duration::from_std(
                cx.background_executor().now().duration_since(self.epoch),
            )
            .unwrap_or_default()
        })
    }

    pub(crate) fn busy(&self) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|a| a.phase != Phase::Finished)
    }

    pub(crate) fn waiting(&self, identity: &ResourceIdentity) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|a| &a.identity == identity && a.phase == Phase::Waiting)
    }

    /// Called only after the confirmation revalidates the pane and captured target.
    pub(crate) fn submit(
        &mut self,
        target: DetailTarget,
        access: ClusterAccess,
        secret: &ExternalSecret,
        annotation: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        if self.busy() {
            return Err("A refresh is still being accounted for");
        }
        let confirmed = self.now(cx);
        let seconds = confirmed.timestamp();
        if self.last_seconds == Some(seconds) || annotation == Some(seconds.to_string().as_str()) {
            return Err(
                "The force-sync value already names this second. Wait for the next second and confirm again.",
            );
        }
        self.last_seconds = Some(seconds);
        self.timeout = None;
        self.attempt = Some(Attempt {
            identity: target.identity.clone(),
            confirmed,
            before: secret.refreshed,
            deadline: cx.background_executor().now() + OBSERVATION,
            phase: Phase::Submitting,
            text: "Submitting refresh…".into(),
            tone: Tone::Unknown,
        });
        cx.notify();
        if access.is_example() {
            let result = if target.identity.name == "access-denied" {
                Err(Failure::new(
                    FailureKind::Forbidden,
                    "Invented viewer cannot patch ExternalSecrets",
                ))
            } else {
                Ok(())
            };
            self.finish(result, cx);
            return Ok(());
        }
        // No OwnedJob, abort-on-hide guard, retry middleware or Operations ticket.
        let task = self.runtime.spawn(async move {
            let client = access.client().await?;
            refresh_external_secret(
                &client,
                &target.kind,
                &target.identity.namespace,
                &target.identity.name,
                seconds,
            )
            .await
        });
        let owner = cx.entity();
        cx.spawn(async move |_, cx| {
            let result = task
                .await
                .unwrap_or_else(|error| Err(Failure::new(FailureKind::Other, error.to_string())));
            owner.update(cx, |owner, cx| owner.finish(result, cx));
        })
        .detach();
        Ok(())
    }

    fn finish(&mut self, result: Result<(), Failure>, cx: &mut Context<Self>) {
        let Some(attempt) = self.attempt.as_mut() else {
            return;
        };
        if let Err(error) = result {
            attempt.phase = Phase::Finished;
            attempt.tone = Tone::Crit;
            attempt.text = match error.kind {
                FailureKind::Forbidden => format!("Not permitted to refresh: {}", error.message),
                FailureKind::Unauthorized | FailureKind::NotFound => format!("Refresh refused: {}", error.message),
                _ => format!("Refresh outcome unknown: {}. The annotation may have changed; no retry was sent.", error.message),
            }.into();
        } else {
            attempt.phase = Phase::Waiting;
            attempt.text = "Refresh requested; waiting for observed sync".into();
            let delay = attempt
                .deadline
                .saturating_duration_since(cx.background_executor().now());
            self.timeout = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                _ = this.update(cx, |owner, cx| {
                    owner.expire();
                    cx.notify();
                });
            }));
        }
        cx.notify();
    }

    fn expire(&mut self) {
        if let Some(attempt) = self.attempt.as_mut().filter(|a| a.phase == Phase::Waiting) {
            attempt.phase = Phase::Finished;
            attempt.tone = Tone::Warn;
            attempt.text = "Refresh requested; no resync observed within 30 seconds".into();
        }
    }

    pub(crate) fn observe(
        &mut self,
        identity: &ResourceIdentity,
        secret: &ExternalSecret,
        cx: &mut Context<Self>,
    ) {
        let Some(attempt) = self
            .attempt
            .as_ref()
            .filter(|a| &a.identity == identity && a.phase == Phase::Waiting)
        else {
            return;
        };
        if cx.background_executor().now() >= attempt.deadline {
            self.expire();
            cx.notify();
            return;
        }
        let Some(refreshed) = secret.refreshed.filter(|time| {
            *time > attempt.confirmed && attempt.before.is_none_or(|before| *time > before)
        }) else {
            return;
        };
        let failed = secret.conditions.iter().any(|c| c.status == "False");
        let ready = secret.conditions.iter().any(|c| c.status == "True");
        let causes = secret
            .conditions
            .iter()
            .map(|c| {
                [
                    format!("{} {}", c.kind, c.status),
                    c.reason.clone(),
                    c.message.clone(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ")
            })
            .collect::<Vec<_>>()
            .join("; ");
        let attempt = self.attempt.as_mut().unwrap();
        attempt.phase = Phase::Finished;
        attempt.tone = if failed {
            Tone::Crit
        } else if ready {
            Tone::Good
        } else {
            Tone::Unknown
        };
        attempt.text = format!(
            "Resync observed at {}. {}",
            refreshed.to_rfc3339(),
            if causes.is_empty() {
                "No sync condition reported"
            } else {
                &causes
            }
        )
        .into();
        self.timeout = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
