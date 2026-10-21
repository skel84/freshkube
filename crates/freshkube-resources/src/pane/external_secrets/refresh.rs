//! Confirmation and visible-only status observation, beside the read-side pane.
use super::super::{DetailPane, DocumentRead};
use crate::{detail::DetailTarget, external_refresh::Phase, screen::ClusterAccess};
use freshkube_core::resources::ExternalSecret;
use freshkube_ui::{form, ui};
use gpui_kit::base::ElementExt;
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::{cell::Cell, rc::Rc, time::Duration};

pub(in crate::pane) struct Confirmation {
    pane: WeakEntity<DetailPane>,
    target: DetailTarget,
    epoch: u64,
    context: SharedString,
    focus: FocusHandle,
    drawn: Rc<Cell<bool>>,
    error: Option<SharedString>,
    _subscription: Subscription,
}

impl Confirmation {
    pub(super) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.drawn.get() {
            return;
        }
        let Some(pane) = self.pane.upgrade() else {
            return;
        };
        let target = &self.target;
        let epoch = self.epoch;
        let result = pane.update(cx, |pane, cx| {
            let Some((secret, annotation)) = pane.refresh_inputs(target, epoch) else {
                return Err("The selection, access or object changed, or its document is not ready. Close this and review it again.");
            };
            let access = pane.access.clone().unwrap();
            pane.external_refresh.update(cx, |owner, cx| owner.submit(target.clone(), access, &secret, annotation.as_deref(), cx))
        });
        match result {
            Ok(()) => window.close_dialog(cx),
            Err(error) => {
                self.error = Some(error.into());
                cx.notify();
            }
        }
    }
}

impl Render for Confirmation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let drawn = self.drawn.clone();
        let owner = cx.entity().downgrade();
        let ready = self.pane.upgrade().is_some_and(|pane| {
            let pane = pane.read(cx);
            pane.refresh_inputs(&self.target, self.epoch).is_some()
                && !pane.external_refresh.read(cx).busy()
        });
        v_flex()
            .id("external-refresh-confirmation")
            .test_support()
            .role(Role::Dialog)
            .aria_label(format!("Refresh ExternalSecret in {}: {}", self.context, self.target.identity.address()))
            .key_context("ExternalSecretRefresh")
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &gpui_kit::base::actions::Cancel, window, cx| window.close_dialog(cx)))
            .gap(ui::dp(12.))
            .child(format!("Context: {}", self.context))
            .child(format!("ExternalSecret: {}", self.target.identity.address()))
            .child("Sets only this ExternalSecret’s force-sync annotation. External Secrets Operator may update its managed Secret. Freshkube never reads that Secret or provider credentials. The configured refresh policy still applies.")
            .when(!ready, |this| this.child("Waiting for a current document, or another refresh is still pending. A changed selection requires a new confirmation."))
            .children(self.error.clone().map(|text| div().id("external-refresh-blocked").test_support().role(Role::Alert).aria_label(text.clone()).child(ui::warning_banner(None, text, None, cx))))
            .child(form::actions(
                form::Action::new("external-refresh-confirm", "Refresh", move |_, window, cx| {
                    _ = owner.update(cx, |dialog, cx| dialog.confirm(window, cx));
                }).disabled(!ready),
                form::Action::cancel("external-refresh-cancel"),
            ))
            .on_prepaint(move |_, _, _| drawn.set(true))
    }
}

impl DetailPane {
    pub(crate) fn external_refresh_owner(
        &self,
    ) -> &Entity<crate::external_refresh::ExternalRefresh> {
        &self.external_refresh
    }

    pub(crate) fn external_refresh_busy(&self, cx: &App) -> bool {
        self.external_refresh.read(cx).busy()
    }

    fn refresh_inputs(
        &self,
        target: &DetailTarget,
        epoch: u64,
    ) -> Option<(ExternalSecret, Option<String>)> {
        if !self.active || self.refresh_epoch != epoch {
            return None;
        }
        let detail = self.detail.as_ref()?;
        if &detail.target != target
            || detail.read != DocumentRead::Loaded
            || target.identity.uid.is_empty()
        {
            return None;
        }
        let document = &detail.view.as_ref()?.document;
        if document.overview.deleting.is_some() || document.uid != target.identity.uid {
            return None;
        }
        let secret = document.overview.external_secret.clone()?;
        let token = document
            .overview
            .annotations
            .iter()
            .find(|(key, _)| key == "force-sync")
            .map(|(_, value)| value.clone());
        Some((secret, token))
    }

    pub(crate) fn request_external_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target().cloned().filter(|target| {
            target.kind.group == "external-secrets.io" && target.kind.plural == "externalsecrets"
        }) else {
            return;
        };
        if !self.active
            || self.external_refresh.read(cx).busy()
            || self
                .refresh_dialog
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some()
        {
            return;
        }
        let pane = cx.entity().downgrade();
        let epoch = self.refresh_epoch;
        let context = self.context.clone();
        cx.bind_keys([KeyBinding::new(
            "escape",
            gpui_kit::base::actions::Cancel,
            Some("ExternalSecretRefresh"),
        )]);
        let dialog = cx.new(|cx| {
            let subscription = cx.observe(&pane.upgrade().unwrap(), |_, _, cx| cx.notify());
            Confirmation {
                pane,
                target,
                epoch,
                context,
                focus: cx.focus_handle().tab_stop(true),
                drawn: Rc::default(),
                error: None,
                _subscription: subscription,
            }
        });
        self.refresh_dialog = Some(dialog.downgrade());
        let focus = dialog.read(cx).focus.clone();
        let weak = dialog.downgrade();
        window.open_dialog(cx, move |modal, window, _| {
            let weak = weak.clone();
            freshkube_ui::dialog::frame(
                modal,
                480.,
                "Refresh ExternalSecret?",
                dialog.clone(),
                div(),
                window,
            )
            .on_ok(move |_, window, cx| {
                _ = weak.update(cx, |dialog, cx| dialog.confirm(window, cx));
                false
            })
        });
        // Kit captures the old focus in open_dialog. Only now focus content.
        window.focus(&focus, cx);
        cx.defer_in(window, move |pane, window, cx| {
            if pane
                .refresh_dialog
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some()
            {
                window.focus(&focus, cx);
            }
        });
    }

    pub(in crate::pane) fn external_refresh_changed(&mut self, cx: &mut Context<Self>) {
        if self.active
            && let Some(detail) = self
                .detail
                .as_ref()
                .filter(|detail| detail.read == DocumentRead::Loaded)
            && let Some(secret) = detail
                .view
                .as_ref()
                .and_then(|v| v.document.overview.external_secret.as_ref())
        {
            let identity = &detail.target.identity;
            self.external_refresh
                .update(cx, |owner, cx| owner.observe(identity, secret, cx));
        }
        self.schedule_external_observation(cx);
        cx.notify();
    }

    pub(in crate::pane) fn schedule_external_observation(&mut self, cx: &mut Context<Self>) {
        let waiting = self
            .target_identity()
            .is_some_and(|id| self.external_refresh.read(cx).waiting(id));
        if !self.active
            || !waiting
            || self
                .detail
                .as_ref()
                .is_some_and(|d| d.read == DocumentRead::Deleted)
        {
            self.refresh_poll = None;
            return;
        }
        if self.refresh_poll.is_some() {
            return;
        }
        self.refresh_poll = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            _ = this.update(cx, |pane, cx| {
                pane.refresh_poll = None;
                if !pane.active {
                    return;
                }
                if pane.access.as_ref().is_some_and(ClusterAccess::is_example) {
                    pane.observe_example_refresh(cx);
                } else if pane.read_job.is_none() {
                    pane.read_document(cx);
                }
                pane.schedule_external_observation(cx);
            });
        }));
    }

    fn observe_example_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_ref() else {
            return;
        };
        if detail.target.identity.name == "old-refresh" {
            return;
        }
        let Some(view) = detail.view.as_ref() else {
            return;
        };
        let Some(attempt) = self.external_refresh.read(cx).attempt.as_ref() else {
            return;
        };
        if attempt.phase != Phase::Waiting {
            return;
        }
        let refreshed = attempt.confirmed + chrono::Duration::seconds(1);
        let mut object: serde_yaml::Value =
            serde_yaml::from_str(&view.document.yaml).expect("invented fixture YAML");
        object["status"]["refreshTime"] = refreshed.to_rfc3339().into();
        let document = freshkube_core::resources::object_from_yaml(
            &detail.target.kind,
            &serde_yaml::to_string(&object).unwrap(),
        )
        .expect("invented ExternalSecret");
        let identity = detail.target.identity.clone();
        self.finish_read(
            &identity,
            self.read_seq,
            Ok(crate::detail::DocumentView::new(document)),
            cx,
        );
    }
}
