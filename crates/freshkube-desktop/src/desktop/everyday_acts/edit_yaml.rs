//! Review of an edit of an object's YAML (#858). The pane asked, with core's
//! description of the edit; this takes the window's level and a client and
//! hands the write to `write_flow`, where core's `write` module checks the
//! level, runs the dry run, shows the diff and sends. The edit is a spec
//! change, so it is never sent without the dialog.

use super::*;
use crate::resources::EditYamlAct;

impl Pilot {
    pub(in crate::desktop) fn edit_yaml_act(
        &mut self,
        act: &EditYamlAct,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = self.everyday_where(&act.source, cx);
        let who = self.everyday_who(&act.source, &at, cx);
        if let Some(reason) = who.protection.refusal() {
            window.push_notification(format!("{reason} Nothing was sent."), cx);
            return;
        }
        if !claim(&act.identity, cx) {
            return;
        }
        let runtime = self.runtime.clone();
        // The cell of the entry the press fixed, as for every everyday action.
        let cell = entry_level_cell(&at.entry, who.protection, cx);
        if matches!(act.source.access, ClusterAccess::Example(_)) {
            release(&act.identity, cx);
            let runner = super::write_runner::example_runner(&runtime, who, cell);
            start(
                runner,
                runtime,
                act.action.clone(),
                &act.identity,
                window,
                cx,
            );
            return;
        }
        let Some(audit) = write_audit_store() else {
            release(&act.identity, cx);
            window.push_notification("The audit log can't be located, so nothing was sent.", cx);
            return;
        };
        let access = act.source.access.clone();
        let (job, receiver) = spawn_job(
            &runtime,
            READ,
            "The cluster did not answer in time. Nothing was sent.".to_owned(),
            async move { access.client().await.map_err(|failure| failure.message) },
        );
        let pressed = act.clone();
        cx.spawn_in(window, async move |this, cx| {
            let _job = job;
            let client = receiver
                .await
                .unwrap_or_else(|_| Err("Reading stopped unexpectedly.".to_owned()));
            _ = this.update_in(cx, |this, window, cx| {
                let what = pressed.identity.name.clone();
                release(&pressed.identity, cx);
                // The window may have moved to another cluster meanwhile.
                if this.everyday_where(&pressed.source, cx) != at {
                    window.push_notification(
                        format!(
                            "The cluster changed while {what} was prepared. Nothing was sent; ask again."
                        ),
                        cx,
                    );
                    return;
                }
                match client {
                    Ok(client) => {
                        let runner = Arc::new(Runner::new(client, audit, who).following(cell));
                        let runtime = this.runtime.clone();
                        start(runner, runtime, pressed.action, &pressed.identity, window, cx);
                    }
                    Err(why) => window.push_notification(
                        format!("Couldn't review the edit of {what}: {why} Nothing was sent."),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }
}
