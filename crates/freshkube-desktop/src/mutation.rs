//! Guard rails shared by every action that changes a cluster: one operation
//! at a time, confirmation against a preview that must still be current, an
//! audit trail, and a window that won't close while an operation runs.

use crate::palette::palette;
use crate::ui::{self, Tone, dp};
use freshkube_core::operations::{AuditLog, default_audit_path};
use freshkube_core::write::{LevelCell, Protection};
use freshkube_ui::form::{self, Action};
use gpui_kit::component::{Root, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

/// The single operation slot. Screens take it with [`Operations::begin`]
/// before changing anything; the shell shows what holds it and offers to
/// cancel.
#[derive(Default)]
pub(crate) struct Operations {
    running: Option<Running>,
    next_id: u64,
}

/// What currently holds the slot.
#[derive(Clone)]
pub(crate) struct Running {
    id: u64,
    pub(crate) label: SharedString,
    /// The latest progress step, for the status bar.
    pub(crate) step: Option<SharedString>,
    cancel: Arc<AtomicBool>,
    /// Dies with the ticket, so a task that ends without `finish` (a panic,
    /// a dropped future) can't hold the slot forever.
    alive: Weak<()>,
}

impl Running {
    pub(crate) fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Why [`Operations::begin`] gave no ticket.
pub(crate) enum Refused {
    /// Another operation holds the slot.
    Busy(Running),
    /// The cluster's level refuses writes.
    ReadOnly(&'static str),
}

/// Proof of holding the operation slot. Hand [`Self::cancellation`] to core
/// runners and return the ticket through [`Operations::finish`].
pub(crate) struct OperationTicket {
    id: u64,
    cancel: Arc<AtomicBool>,
    _alive: Arc<()>,
}

impl OperationTicket {
    /// A cooperative cancellation check for core runners.
    pub(crate) fn cancellation(&self) -> impl Fn() -> bool + Send + Sync + 'static {
        let cancel = self.cancel.clone();
        move || cancel.load(Ordering::SeqCst)
    }
}

struct GlobalOperations(Entity<Operations>);

impl Global for GlobalOperations {}

/// The protection level of the context the window shows, which the shell
/// keeps current (#852). A control that writes asks it, as it asks the
/// operation slot: one level at a time, like the one slot. It is a shared
/// cell, so a write's runner follows it and reads it again just before it
/// sends (#853).
pub(crate) struct GlobalProtection(LevelCell, std::collections::HashMap<String, LevelCell>);

impl Default for GlobalProtection {
    fn default() -> Self {
        Self(LevelCell::new(Protection::default()), Default::default())
    }
}

impl Global for GlobalProtection {}

/// Sets the level the window's context has.
pub(crate) fn set_protection(level: Protection, cx: &mut App) {
    cx.default_global::<GlobalProtection>().0.set(level);
}

/// Sets the level each workspace entry has, by its id, so a runner writing to
/// an entry other than the one the window shows follows a change made in
/// Settings too.
pub(crate) fn set_entry_levels(
    levels: impl IntoIterator<Item = (String, Protection)>,
    cx: &mut App,
) {
    let global = cx.default_global::<GlobalProtection>();
    for (entry, level) in levels {
        global
            .1
            .entry(entry)
            .and_modify(|cell| cell.set(level))
            .or_insert_with(|| LevelCell::new(level));
    }
}

/// The cell holding `entry`'s level, for a runner to follow. An entry not
/// seen yet starts at `level`.
pub(crate) fn entry_level_cell(entry: &str, level: Protection, cx: &mut App) -> LevelCell {
    let global = cx.default_global::<GlobalProtection>();
    let cell = global
        .1
        .entry(entry.to_owned())
        .or_insert_with(|| LevelCell::new(level));
    cell.set(level);
    cell.clone()
}

/// The window's level as a cell a runner can follow.
pub(crate) fn level_cell(cx: &mut App) -> LevelCell {
    cx.default_global::<GlobalProtection>().0.clone()
}

/// Why a write is refused on the context shown, if it is: the reason a
/// disabled control gives, and the guard that makes pressing it do nothing.
pub(crate) fn refusal(cx: &App) -> Option<&'static str> {
    cx.try_global::<GlobalProtection>()
        .and_then(|global| global.0.get().refusal())
}

impl Operations {
    /// The app-wide slot, created on first use.
    pub(crate) fn global(cx: &mut App) -> Entity<Operations> {
        if let Some(global) = cx.try_global::<GlobalOperations>() {
            return global.0.clone();
        }
        let entity = cx.new(|_| Operations::default());
        cx.set_global(GlobalOperations(entity.clone()));
        entity
    }

    /// What holds the slot, if anything. Never panics when nothing has used
    /// it yet.
    pub(crate) fn current(cx: &App) -> Option<Running> {
        cx.try_global::<GlobalOperations>()
            .and_then(|global| global.0.read(cx).running().cloned())
    }

    pub(crate) fn running(&self) -> Option<&Running> {
        self.running
            .as_ref()
            .filter(|running| running.alive.upgrade().is_some())
    }

    /// Takes the slot for a write to the cluster the window shows, or says
    /// why not: something else holds it, or the cluster is read-only. Every
    /// Talos write goes through here, so a control that forgot to ask the
    /// level still starts nothing (#852).
    pub(crate) fn begin(
        &mut self,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Result<OperationTicket, Refused> {
        if let Some(why) = refusal(cx) {
            return Err(Refused::ReadOnly(why));
        }
        self.take(label, cx).map_err(Refused::Busy)
    }

    /// Takes the slot for maintenance mode, which the level does not govern:
    /// it is not tied to a workspace entry, and it works on a node the
    /// maintainer reached over the Talos maintenance API. WRITE_MODEL.md says
    /// so; this is the one exemption from [`Self::begin`].
    pub(crate) fn begin_maintenance(
        &mut self,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Result<OperationTicket, Running> {
        self.take(label, cx)
    }

    /// Takes the slot, or returns what already holds it.
    fn take(
        &mut self,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Result<OperationTicket, Running> {
        if let Some(running) = self.running() {
            return Err(running.clone());
        }
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(());
        self.running = Some(Running {
            id: self.next_id,
            label: label.into(),
            step: None,
            cancel: cancel.clone(),
            alive: Arc::downgrade(&alive),
        });
        cx.notify();
        Ok(OperationTicket {
            id: self.next_id,
            cancel,
            _alive: alive,
        })
    }

    pub(crate) fn progress(
        &mut self,
        ticket: &OperationTicket,
        step: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(running) = self.running.as_mut().filter(|r| r.id == ticket.id) {
            running.step = Some(step.into());
            cx.notify();
        }
    }

    /// Asks the running operation to stop at its next checkpoint. Core
    /// runners never abandon a mutation halfway; they stop between steps.
    pub(crate) fn request_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(running) = self.running() {
            running.cancel.store(true, Ordering::SeqCst);
            cx.notify();
        }
    }

    pub(crate) fn finish(&mut self, ticket: OperationTicket, cx: &mut Context<Self>) {
        if self.running.as_ref().is_some_and(|r| r.id == ticket.id) {
            self.running = None;
            cx.notify();
        }
    }
}

/// The writes audit log (`<preferences>/audit/writes.jsonl`, #853): one for the
/// app, across contexts.
pub(crate) fn write_audit_store() -> Option<freshkube_core::write::AuditStore> {
    #[cfg(test)]
    if let Some(dir) = TEST_AUDIT.with(|dir| dir.borrow().clone()) {
        return Some(freshkube_core::write::AuditStore::new(dir));
    }
    crate::preferences_path()
        .and_then(|path| path.parent().map(|dir| dir.join("audit")))
        .map(freshkube_core::write::AuditStore::new)
}

#[cfg(test)]
thread_local! {
    /// Where a test's writes are audited: never the user's own folder.
    static TEST_AUDIT: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Audits this test's writes under `dir`.
#[cfg(test)]
pub(crate) fn audit_under(dir: std::path::PathBuf) {
    TEST_AUDIT.with(|slot| *slot.borrow_mut() = Some(dir));
}

/// The audit store for `context`, shared with the other frontends.
pub(crate) fn audit_log(context: &str) -> AuditLog {
    AuditLog::new(default_audit_path(context), context)
}

/// Whether the window may close now. While an operation runs it explains why
/// not and keeps the window open: closing would abandon a half-done change.
pub(crate) fn may_close(window: &mut Window, cx: &mut App) -> bool {
    let Some(running) = Operations::current(cx) else {
        return true;
    };
    let detail = format!(
        "{} is still running. Wait for it to finish, or cancel it from the status bar first.",
        running.label
    );
    // The answer doesn't matter; the prompt only explains the refusal.
    drop(window.prompt(
        PromptLevel::Warning,
        "An operation is in progress",
        Some(&detail),
        &["Keep running"],
        cx,
    ));
    false
}

/// Recomputes a preview's fingerprint; `None` once it can't be computed.
pub(crate) type Fingerprint = Rc<dyn Fn(&App) -> Option<u64>>;
pub(crate) type OnConfirm = Rc<dyn Fn(&mut Window, &mut App)>;

/// A confirmation for one mutation, showing the preview it was decided on.
pub(crate) struct Confirmation {
    pub(crate) title: SharedString,
    /// What will happen, in one or two sentences.
    pub(crate) summary: SharedString,
    /// Preview facts, e.g. ("Pods to evict", "12").
    pub(crate) facts: Vec<(SharedString, SharedString)>,
    pub(crate) warnings: Vec<SharedString>,
    pub(crate) confirm_label: SharedString,
    pub(crate) destructive: bool,
    /// Identifies the preview shown in the dialog.
    pub(crate) fingerprint: u64,
    /// The same preview's fingerprint now, or `None` once it can't be
    /// computed (target changed, data gone). Confirming needs a match: the
    /// user must have seen what will actually happen.
    pub(crate) current: Fingerprint,
    pub(crate) on_confirm: OnConfirm,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Blocked {
    Stale,
    Busy,
}

/// Opens a modal confirmation. Confirming is refused, with the reason shown
/// in the dialog, while another operation runs or once the preview is stale.
pub(crate) fn confirm(request: Confirmation, window: &mut Window, cx: &mut App) {
    let request = Rc::new(request);
    let blocked = Rc::new(Cell::new(None::<Blocked>));
    window.open_dialog(cx, move |dialog, window, cx| {
        let p = palette(cx);
        let reason = blocked.get();
        let ok = {
            let request = request.clone();
            let blocked = blocked.clone();
            let destructive = request.destructive;
            let action = Action::new(
                "confirm-ok",
                request.confirm_label.clone(),
                move |_, window, cx| {
                    let refusal = if Operations::current(cx).is_some() {
                        Some(Blocked::Busy)
                    } else if (request.current)(cx) != Some(request.fingerprint) {
                        Some(Blocked::Stale)
                    } else {
                        None
                    };
                    if refusal.is_some() {
                        blocked.set(refusal);
                        // The dialog renders inside the root view, which is
                        // cached until notified.
                        if let Some(Some(root)) = window.root::<Root>() {
                            root.update(cx, |_, cx| cx.notify());
                        }
                        return;
                    }
                    window.close_dialog(cx);
                    (request.on_confirm)(window, cx);
                },
            )
            .disabled(reason == Some(Blocked::Stale));
            if destructive { action.danger() } else { action }
        };
        let cancel = Action::cancel("confirm-cancel");
        let notice = reason.map(|reason| {
            let text = match reason {
                Blocked::Stale => {
                    "The cluster changed after this preview was taken. Close this and review the new preview before confirming."
                }
                Blocked::Busy => {
                    "Another operation is still running. Confirm again once it has finished."
                }
            };
            div()
                .id("confirm-blocked")
                .test_support()
                .role(Role::Alert)
                .aria_label(text)
                .child(ui::warning_banner(None, text, None, cx))
        });
        dialog
            .w(ui::dp_px(480., window))
            .overlay_closable(false)
            .title(request.title.clone())
            .child(
                v_flex()
                    .id("confirm-dialog")
                    .test_support()
                    .role(Role::Dialog)
                    .aria_label(request.title.clone())
                    .gap_3()
                    .text_size(dp(13.))
                    .child(request.summary.clone())
                    .when(!request.facts.is_empty(), |this| {
                        this.child(v_flex().gap_1p5().children(request.facts.iter().map(
                            |(label, value)| {
                                h_flex()
                                    .gap_3()
                                    .child(
                                        div()
                                            .w(dp(150.))
                                            .flex_none()
                                            .text_color(p.muted)
                                            .child(label.clone()),
                                    )
                                    .child(div().min_w_0().child(value.clone()))
                            },
                        )))
                    })
                    .children(request.warnings.iter().map(|warning| {
                        ui::tag(Tone::Warn, None, warning.clone(), cx)
                    }))
                    .children(notice),
            )
            .footer(form::actions(ok, cancel))
    });
}

/// Waits for the confirmation dialog to finish opening. Its animation runs on
/// the real clock, so a fixed sleep can end early on a loaded machine; poll
/// until the dialog has stopped moving instead.
#[cfg(test)]
pub(crate) fn settle_confirmation(cx: &mut gpui_kit::TestAppContext, handle: AnyWindowHandle) {
    use gpui_kit::test::TestWindowExt;
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut last, mut steady) = (None, 0);
    while steady < 2 {
        assert!(
            Instant::now() < deadline,
            "the confirmation dialog never settled"
        );
        std::thread::sleep(Duration::from_millis(50));
        cx.run_until_parked();
        let bounds = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .try_find("confirm-dialog")
                    .map(|dialog| dialog.bounds())
            })
            .unwrap();
        steady = if bounds.is_some() && bounds == last {
            steady + 1
        } else {
            0
        };
        last = bounds;
    }
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}
