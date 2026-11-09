//! Delete (#857): reads what the object owns, then asks what should happen to
//! it (propagation) only when that matters, and hands the described delete to
//! the write flow, which shows the impact, asks what the level wants, runs
//! the 5 s cancel window and sends through core's `write` module. Nothing here
//! builds a request or decides a level's requirement.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::*;
use crate::resources::model::ResourceIdentity;
use freshkube_core::write::delete::{self, Impact};
use freshkube_core::write::everyday::NotOffered;
use freshkube_core::write::{Propagation, Runner};
use freshkube_ui::form::{self, Action};
use gpui_kit::component::{
    Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use crate::palette::palette;
use crate::ui::dp;
use crate::write_flow::dismiss::OnDismiss;

impl Pilot {
    /// Delete on the selected row: read the object and what it owns, then
    /// describe the delete.
    pub(super) fn delete_act(
        &mut self,
        act: &WorkloadAct,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = self.everyday_where(&act.source, cx);
        let who = self.everyday_who(&act.source, &at, cx);
        if let Some(reason) = who.protection.refusal() {
            window.push_notification(format!("{reason} Nothing was sent."), cx);
            return;
        }
        if let Some(reason) = delete::refusal(&act.kind.group, &act.kind.kind) {
            window.push_notification(reason, cx);
            return;
        }
        if !claim(&act.identity, cx) {
            return;
        }
        let runtime = self.runtime.clone();
        let cell = entry_level_cell(&at.entry, who.protection, cx);
        if matches!(act.source.access, ClusterAccess::Example(_)) {
            let now = chrono::Utc::now().timestamp();
            let object = crate::resources::example::object_json(&act.identity, now)
                .unwrap_or_else(|| example_object(act));
            let row = row_of(act);
            let listed = vec![(
                "ReplicaSet",
                crate::resources::example::replicasets_of(&act.identity, now),
            )];
            let dependents = delete::dependents_from(
                &row.group,
                &row.kind,
                row.namespace.is_some(),
                &act.identity.uid,
                &listed,
            );
            let Some(impact) = Impact::new(&row, &object, dependents) else {
                release(&act.identity, cx);
                return;
            };
            let runner = super::super::write_runner::example_runner(&runtime, who, cell);
            self.begin_delete(runner, impact, &act.identity, window, cx);
            return;
        }
        let Some(audit) = write_audit_store() else {
            release(&act.identity, cx);
            window.push_notification("The audit log can't be located, so nothing was sent.", cx);
            return;
        };
        let (access, row) = (act.source.access.clone(), row_of(act));
        let (job, receiver) = spawn_job(
            &runtime,
            READ,
            "The cluster did not answer in time. Nothing was sent.".to_owned(),
            async move {
                let client = access.client().await.map_err(|failure| failure.message)?;
                let impact = Impact::read(&client, &row).await?;
                Ok::<_, String>((client, impact))
            },
        );
        let (identity, pressed) = (act.identity.clone(), act.clone());
        cx.spawn_in(window, async move |this, cx| {
            let _job = job;
            let read = receiver
                .await
                .unwrap_or_else(|_| Err("Reading stopped unexpectedly.".to_owned()));
            _ = this.update_in(cx, |this, window, cx| {
                let what = identity.name.clone();
                if this.everyday_where(&pressed.source, cx) != at {
                    release(&identity, cx);
                    window.push_notification(
                        format!(
                            "The cluster changed while {what} was read. Nothing was sent; ask again."
                        ),
                        cx,
                    );
                    return;
                }
                match read {
                    Ok((client, impact)) => {
                        let runner = Runner::new(client, audit, who).following(cell);
                        this.begin_delete(Arc::new(runner), impact, &identity, window, cx);
                    }
                    Err(why) => {
                        release(&identity, cx);
                        window.push_notification(
                            format!("Couldn't start Delete on {what}: {why} Nothing was sent."),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }

    /// Asks which propagation only when the object owns something (or that is
    /// not known); otherwise the default goes straight to the write flow.
    pub(super) fn begin_delete(
        &mut self,
        runner: Arc<Runner>,
        impact: Impact,
        identity: &ResourceIdentity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(why) = impact.refusal() {
            release(identity, cx);
            window.push_notification(why, cx);
            return;
        }
        if impact.choices().len() > 1 {
            open(
                runner,
                self.runtime.clone(),
                impact,
                identity.clone(),
                window,
                cx,
            );
            return;
        }
        match delete::delete(&impact, Propagation::Background) {
            Ok(action) => {
                release(identity, cx);
                start(runner, self.runtime.clone(), action, identity, window, cx);
            }
            Err(NotOffered(why)) => {
                release(identity, cx);
                window.push_notification(why, cx);
            }
        }
    }
}

struct DeleteDialog {
    runner: Arc<Runner>,
    runtime: Handle,
    impact: Impact,
    identity: ResourceIdentity,
    chosen: Propagation,
    error: Option<String>,
    /// The choice was handed on to the write flow, which holds the object now.
    handed_on: Rc<Cell<bool>>,
}

fn open(
    runner: Arc<Runner>,
    runtime: Handle,
    impact: Impact,
    identity: ResourceIdentity,
    window: &mut Window,
    cx: &mut App,
) {
    let handed_on = Rc::new(Cell::new(false));
    // However the picker ends without a choice, the object is let go of.
    let dismissed = OnDismiss::new(cx, {
        let (identity, handed_on) = (identity.clone(), handed_on.clone());
        move |cx| {
            if !handed_on.get() {
                release(&identity, cx);
            }
        }
    });
    let dialog = cx.new(|_| DeleteDialog {
        runner,
        runtime,
        impact,
        identity,
        chosen: Propagation::default(),
        error: None,
        handed_on,
    });
    window.open_dialog(cx, move |view, window, cx| {
        let _kept = &dismissed;
        let title = format!("Delete {}", dialog.read(cx).impact.target().label());
        let weak = dialog.downgrade();
        let actions = form::actions(
            Action::new("delete-review", "Review", move |_, window, cx| {
                _ = weak.update(cx, |dialog, cx| dialog.review(window, cx));
            }),
            Action::cancel("delete-cancel"),
        );
        freshkube_ui::dialog::frame(view, 560., title, dialog.clone(), actions, window)
    });
}

impl DeleteDialog {
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match delete::delete(&self.impact, self.chosen) {
            Ok(action) => {
                self.handed_on.set(true);
                release(&self.identity, cx);
                window.close_dialog(cx);
                start(
                    self.runner.clone(),
                    self.runtime.clone(),
                    action,
                    &self.identity,
                    window,
                    cx,
                );
            }
            Err(NotOffered(why)) => {
                self.error = Some(why.to_owned());
                cx.notify();
            }
        }
    }
}

impl Render for DeleteDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let weak = cx.entity().downgrade();
        v_flex()
            .id("delete-dialog")
            .test_support()
            .gap(dp(8.))
            .child(format!(
                "Owns: {}. Choose what happens to them; the next step shows each one.",
                self.impact.owns_summary()
            ))
            .children(self.impact.choices().into_iter().map(|policy| {
                let weak = weak.clone();
                Button::new(SharedString::from(format!(
                    "delete-policy-{}",
                    policy.key()
                )))
                .ghost()
                .small()
                .w_full()
                .justify_start()
                .label(format!("{}: {}", policy.label(), policy.effect()))
                .selected(self.chosen == policy)
                .on_click(move |_, _, cx| {
                    _ = weak.update(cx, |dialog, cx| {
                        dialog.chosen = policy;
                        dialog.error = None;
                        cx.notify();
                    });
                })
            }))
            .when_some(self.error.clone(), |this, why| {
                this.child(
                    div()
                        .id("delete-error")
                        .test_support()
                        .text_color(p.crit_ink)
                        .child(why),
                )
            })
    }
}
