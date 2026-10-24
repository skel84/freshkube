//! Roll back asks one thing first, the revision; everything after it is the
//! write flow's (`write_flow`), whose dialog shows the dry run's diff of the
//! pod template and asks for what the level wants.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use crate::resources::model::ResourceIdentity;
use freshkube_core::write::Runner;
use freshkube_core::write::everyday::{self, Revision, Revisions, Subject};
use freshkube_ui::form::{self, Action};
use gpui_kit::component::{
    Disableable, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use crate::palette::palette;
use crate::ui::dp;
use crate::write_flow::dismiss::OnDismiss;

pub(super) struct RollbackDialog {
    runner: Arc<Runner>,
    runtime: Handle,
    subject: Subject,
    identity: ResourceIdentity,
    chosen: Option<u64>,
    error: Option<String>,
    /// The revision was handed on to the write flow, which holds the object now.
    handed_on: Rc<Cell<bool>>,
}

/// One line of the picker: the revision, its images, when it was made.
fn line(revision: &Revision) -> String {
    let when = revision
        .created
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string());
    let images = if revision.images.is_empty() {
        "no image".to_owned()
    } else {
        revision.images.join(", ")
    };
    let mut parts = vec![format!("Revision {}", revision.number), images];
    parts.extend(when);
    if revision.current {
        parts.push("running now".to_owned());
    }
    parts.join(" · ")
}

pub(super) fn open(
    runner: Arc<Runner>,
    runtime: Handle,
    subject: Subject,
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
                super::release(&identity, cx);
            }
        }
    });
    let dialog = cx.new(|_| RollbackDialog {
        runner,
        runtime,
        subject,
        identity,
        chosen: None,
        error: None,
        handed_on,
    });
    window.open_dialog(cx, move |view, window, cx| {
        let _kept = &dismissed;
        let (title, none_chosen) = {
            let dialog = dialog.read(cx);
            (
                format!("Roll back {}", dialog.subject.target().label()),
                dialog.chosen.is_none(),
            )
        };
        let weak = dialog.downgrade();
        let actions = form::actions(
            Action::new("rollback-review", "Review", move |_, window, cx| {
                _ = weak.update(cx, |dialog, cx| dialog.review(window, cx));
            })
            .disabled(none_chosen),
            Action::cancel("rollback-cancel"),
        );
        freshkube_ui::dialog::frame(view, 560., title, dialog.clone(), actions, window)
    });
}

impl RollbackDialog {
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(number) = self.chosen else {
            return;
        };
        match everyday::rollback(&self.subject, number) {
            Ok(action) => {
                self.handed_on.set(true);
                super::release(&self.identity, cx);
                window.close_dialog(cx);
                super::start(
                    self.runner.clone(),
                    self.runtime.clone(),
                    action,
                    &self.identity,
                    window,
                    cx,
                );
            }
            Err(everyday::NotOffered(why)) => {
                self.error = Some(why.to_owned());
                cx.notify();
            }
        }
    }
}

impl Render for RollbackDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let revisions = match self.subject.revisions() {
            Revisions::Read(revisions) => revisions.clone(),
            _ => Vec::new(),
        };
        let weak = cx.entity().downgrade();
        v_flex()
            .id("rollback-dialog")
            .test_support()
            .gap(dp(8.))
            .child("Roll back to the pod template of:")
            .children(revisions.iter().map(|revision| {
                let number = revision.number;
                let weak = weak.clone();
                let button = Button::new(SharedString::from(format!("rollback-revision-{number}")))
                    .ghost()
                    .small()
                    .w_full()
                    .justify_start()
                    .label(line(revision))
                    .selected(self.chosen == Some(number));
                if revision.current {
                    // Where it is already: not a choice.
                    button.disabled(true)
                } else {
                    button.on_click(move |_, _, cx| {
                        _ = weak.update(cx, |dialog, cx| {
                            dialog.chosen = Some(number);
                            dialog.error = None;
                            cx.notify();
                        });
                    })
                }
            }))
            .when_some(self.error.clone(), |this, why| {
                this.child(
                    div()
                        .id("rollback-error")
                        .test_support()
                        .text_color(p.crit_ink)
                        .child(why),
                )
            })
    }
}
