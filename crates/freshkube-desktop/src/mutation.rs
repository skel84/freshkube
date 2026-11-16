//! Guard rails shared by every action that changes a cluster: one operation
//! at a time, confirmation against a preview that must still be current, an
//! audit trail, and a window that won't close while an operation runs.

use crate::palette::palette;
use crate::ui::{self, Tone, dp};
use freshkube_core::operations::{AuditLog, default_audit_path};
use freshkube_core::write::{LevelCell, Protection};
use freshkube_ui::form::{self, Action};
use gpui_kit::component::{
    Root, Sizable, WindowExt, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
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
pub(crate) struct GlobalProtection(
    LevelCell,
    std::collections::HashMap<String, LevelCell>,
    Option<(
        freshkube_core::workspace::Workspace,
        freshkube_core::workspace::SourceRecovery,
    )>,
    /// The id of the workspace entry the window is on, `None` with no entry
    /// active. What a protected action on several objects asks to be typed.
    Option<String>,
    /// The setup file the levels come from, `None` for example data and a
    /// window with no preferences: a write's send step reads it again
    /// ([`send_guard`]).
    Option<std::path::PathBuf>,
    /// A level a send found in the setup file that is stricter than the one
    /// the app holds, kept until the app reads the file itself: the header
    /// and every write follow it ([`raise_level`]).
    Option<Protection>,
    /// Whether the entries the window works under came from that file (and
    /// not, for a legacy `workspace.json` user, from the migration), so an
    /// entry subject is only read from a file that lists it.
    bool,
    /// The kubeconfig context the window shows (what its level is held for
    /// when no entry is active), published with the level so the name a send
    /// reads from the setup file and the level it holds are about the same
    /// context. Not the Talos cluster's name, which can differ.
    Option<String>,
);

impl Default for GlobalProtection {
    fn default() -> Self {
        Self(
            LevelCell::new(Protection::default()),
            Default::default(),
            None,
            None,
            None,
            None,
            false,
            None,
        )
    }
}

impl Global for GlobalProtection {}

/// Set when a send found a stricter level in the setup file: the shell
/// listens and draws it (`LevelRaised` carries no data; the level is
/// [`level_floor`]).
pub(crate) struct LevelRaised;

impl Global for LevelRaised {}

/// Sets the level the window's context has.
pub(crate) fn set_protection(level: Protection, cx: &mut App) {
    cx.default_global::<GlobalProtection>().0.set(level);
}

/// Sets the id of the workspace entry the window is on.
pub(crate) fn set_active_entry(entry: Option<String>, cx: &mut App) {
    cx.default_global::<GlobalProtection>().3 = entry;
}

/// Sets the setup file the levels are read from, for a write's send step to
/// read again (#1055), and whether the entries came from it. `None` has no
/// file to read, as for example data.
pub(crate) fn set_setup_file(file: Option<std::path::PathBuf>, from_file: bool, cx: &mut App) {
    let global = cx.default_global::<GlobalProtection>();
    global.4 = file;
    global.6 = from_file;
}

/// Sets the kubeconfig context the window shows, with its level.
pub(crate) fn set_window_context(context: String, cx: &mut App) {
    cx.default_global::<GlobalProtection>().7 = Some(context);
}

/// What a write reads again from the setup file at its send step, for the
/// identity the dialog is for: the active entry when the setup file lists
/// the entries, else the kubeconfig context the window shows, as the shell
/// published it (a legacy `workspace.json` user has no file to name an entry
/// in), else `context`, the caller's. Taken when the dialog opens, so the re-read is against the same identity however the
/// window moves after.
///
/// `None` only for example data with no setup file behind it, where nothing
/// is sent. A live window with no setup file gets a guard that can't say, so
/// its send refuses: no guard never means "send without reading".
pub(crate) fn send_guard(
    context: &str,
    example: bool,
    cx: &App,
) -> Option<freshkube_core::write::SetupLevels> {
    use freshkube_core::write::{SetupLevels, Subject};
    let global = cx.try_global::<GlobalProtection>();
    let Some(file) = global.and_then(|global| global.4.clone()) else {
        return (!example).then(|| SetupLevels::unavailable(Subject::Context(context.to_owned())));
    };
    let window = global
        .and_then(|global| global.7.clone())
        .unwrap_or_else(|| context.to_owned());
    let subject = match global.map(|global| (&global.3, global.6)) {
        Some((Some(entry), true)) => Subject::Entry(entry.clone()),
        _ => Subject::Context(window),
    };
    Some(SetupLevels::new(file, subject))
}

/// The notice when the window went away while the setup file was read: a
/// failed update is a refusal, never "go on".
pub(crate) const WINDOW_GONE: &str =
    "The window closed while the setup file was read; review again.";

/// How long the setup file may take to read before a send refuses: it is a
/// few kilobytes, so a read that takes this long is not going to answer.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Reads the setup file again off the UI thread and decides the level a write
/// is sent under: the stricter of `held` and the file's, refusing when the
/// file got stricter, can't say, or doesn't answer within a deadline.
/// `Ok(held)` with no guard, which [`send_guard`] gives only to example data.
pub(crate) async fn level_at_send(
    guard: Option<freshkube_core::write::SetupLevels>,
    held: Protection,
    executor: &gpui_kit::BackgroundExecutor,
) -> Result<Protection, freshkube_core::write::SendRefusal> {
    use futures::FutureExt;
    let Some(guard) = guard else {
        return Ok(held);
    };
    let read = executor.spawn(async move { guard.read() }).fuse();
    let deadline = executor.timer(READ_DEADLINE).fuse();
    futures::pin_mut!(read, deadline);
    let fresh = futures::select_biased! {
        fresh = read => fresh,
        _ = deadline => Err(freshkube_core::write::Unreadable::NotReadable),
    };
    freshkube_core::write::at_send(held, fresh)
}

/// The notice for a refused send. A stricter level is also kept
/// ([`raise_level`]), so the header and the next dialog show it: the app's own
/// read of the file stays pending while a dialog is open.
pub(crate) fn refused_text(refusal: freshkube_core::write::SendRefusal, cx: &mut App) -> String {
    if let freshkube_core::write::SendRefusal::Stricter(level) = refusal {
        raise_level(level, cx);
    }
    refusal.text()
}

/// Makes the window's level at least `level` until the app reads the setup
/// file itself: the cell every write follows is stricter at once, and the
/// shell draws the header under it.
pub(crate) fn raise_level(level: Protection, cx: &mut App) {
    let global = cx.default_global::<GlobalProtection>();
    let stricter = global.0.get().stricter(level);
    global.0.set(stricter);
    global.5 = Some(global.5.map_or(level, |floor| floor.stricter(level)));
    cx.set_global(LevelRaised);
}

/// The level a send found in the file that the app has not read yet.
pub(crate) fn level_floor(cx: &App) -> Option<Protection> {
    cx.try_global::<GlobalProtection>()
        .and_then(|global| global.5)
}

/// The app read the setup file itself, so what it holds is authoritative.
pub(crate) fn clear_level_floor(cx: &mut App) {
    if let Some(global) = cx.try_global::<GlobalProtection>()
        && global.5.is_some()
    {
        cx.default_global::<GlobalProtection>().5 = None;
    }
}

/// Why a send that waited on the setup file must not go on, when the window
/// moved while it read: the cluster became read-only or stricter in the app.
/// `held` is the level the dialog was confirmed under.
pub(crate) fn moved_during_read(held: Protection, cx: &App) -> Option<String> {
    if let Some(why) = refusal(cx) {
        return Some(why.to_owned());
    }
    let now = level(cx);
    (now < held).then(|| freshkube_core::write::SendRefusal::Stricter(now).text())
}

/// The id of the workspace entry the window is on, if one is active.
pub(crate) fn active_entry(cx: &App) -> Option<String> {
    cx.try_global::<GlobalProtection>()
        .and_then(|global| global.3.clone())
}

/// The level of the context the window shows, as it is now.
pub(crate) fn level(cx: &App) -> Protection {
    cx.try_global::<GlobalProtection>()
        .map_or_else(Protection::default, |global| global.0.get())
}

/// Sets the level each workspace entry has, by its id, so a runner writing to
/// an entry other than the one the window shows follows a change made in
/// Settings too.
pub(crate) fn set_entry_levels(
    levels: impl IntoIterator<Item = (String, Protection)>,
    cx: &mut App,
) {
    let global = cx.default_global::<GlobalProtection>();
    let mut listed = std::collections::HashSet::new();
    for (entry, level) in levels {
        listed.insert(entry.clone());
        global
            .1
            .entry(entry)
            .and_modify(|cell| cell.set(level))
            .or_insert_with(|| LevelCell::new(level));
    }
    // An entry the file no longer lists has no level for a lookup to find
    // (`entry_level`); a runner already holding its cell keeps it.
    global.1.retain(|entry, _| listed.contains(entry));
}

/// Publishes the setup the levels come from, so a context no entry names
/// is read under it live (`context_level`), as the window's own is.
pub(crate) fn set_workspace_levels(
    workspace: &freshkube_core::workspace::Workspace,
    recovery: &freshkube_core::workspace::SourceRecovery,
    cx: &mut App,
) {
    cx.default_global::<GlobalProtection>().2 = Some((workspace.clone(), recovery.clone()));
}

/// Whether any setup has been published: until one has, no level is known
/// and no entry can be called gone.
pub(crate) fn setup_read(cx: &App) -> bool {
    cx.try_global::<GlobalProtection>()
        .is_some_and(|global| global.2.is_some())
}

/// The level of the kubeconfig context `context` under the setup as it is
/// now: the stricter of the entries naming it, guarded for one no entry
/// names, or what a skipped row locked it to. `None` before any setup was
/// published, which blocks a shell (#878).
pub(crate) fn context_level(context: &str, cx: &App) -> Option<Protection> {
    let (workspace, recovery) = cx.try_global::<GlobalProtection>()?.2.as_ref()?;
    Some(workspace.level_of(context, recovery))
}

/// The setup as it is now: the entries an agent session may be pointed at,
/// and the kubeconfig their contexts are read from. `None` before any setup
/// was published.
pub(crate) fn workspace(cx: &App) -> Option<freshkube_core::workspace::Workspace> {
    cx.try_global::<GlobalProtection>()?
        .2
        .as_ref()
        .map(|(workspace, _)| workspace.clone())
}

/// The level of the workspace entry `entry` as it is now, or `None` when no
/// entry of that id is listed. A dock shell's gate reads this and nothing
/// else (#878); it never creates a cell, so a gone entry stays gone.
pub(crate) fn entry_level(entry: &str, cx: &App) -> Option<Protection> {
    cx.try_global::<GlobalProtection>()
        .and_then(|global| global.1.get(entry))
        .map(LevelCell::get)
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
    /// A name the person must type before Confirm is enabled (a protected
    /// cluster, #852), or `None` for the operation's own confirmation.
    pub(crate) type_name: Option<TypeName>,
}

/// The name a confirmation asks to be typed, and where the dialog leaves
/// what was typed for `on_confirm` to read.
#[derive(Clone)]
pub(crate) struct TypeName {
    pub(crate) expected: SharedString,
    /// Several objects are typed once by the entry's id, and the dialog says so.
    pub(crate) entry_id: bool,
    pub(crate) typed: Rc<std::cell::RefCell<String>>,
}

impl TypeName {
    pub(crate) fn new(expected: impl Into<SharedString>, entry_id: bool) -> Self {
        Self {
            expected: expected.into(),
            entry_id,
            typed: Rc::default(),
        }
    }

    fn placeholder(&self) -> &'static str {
        if self.entry_id {
            "Type the entry's id"
        } else {
            "Type the name"
        }
    }

    fn prompt(&self) -> String {
        if self.entry_id {
            format!(
                "Type the cluster entry's id, {}, to confirm.",
                self.expected
            )
        } else {
            format!("Type {} to confirm.", self.expected)
        }
    }
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
    let input = request.type_name.as_ref().map(|name| {
        let placeholder = name.placeholder();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let typed = name.typed.clone();
        typed.borrow_mut().clear();
        window
            .subscribe(&input, cx, move |input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    *typed.borrow_mut() = input.read(cx).value().to_string();
                    // The dialog renders inside the root view, which is
                    // cached until notified.
                    if let Some(Some(root)) = window.root::<Root>() {
                        root.update(cx, |_, cx| cx.notify());
                    }
                }
            })
            .detach();
        input
    });
    // Enter on a typed dialog presses Confirm, which checks the name, and
    // never closes the dialog by itself.
    let enter: Rc<std::cell::RefCell<Option<OnConfirm>>> = Rc::default();
    let scroll = gpui_kit::ScrollHandle::new();
    let focus = input.clone();
    freshkube_ui::dialog::open(window, cx, move |dialog, window, cx| {
        let p = palette(cx);
        let reason = blocked.get();
        let ok = {
            let request = request.clone();
            let blocked = blocked.clone();
            let destructive = request.destructive;
            let input = input.clone();
            let name_missing =
                request
                    .type_name
                    .as_ref()
                    .zip(input.as_ref())
                    .is_some_and(|(name, input)| {
                        input.read(cx).value().as_ref() != name.expected.as_ref()
                    });
            let confirm_label = request.confirm_label.clone();
            let press: OnConfirm = Rc::new(move |window, cx| {
                if let (Some(name), Some(input)) = (&request.type_name, &input) {
                    // Confirm is disabled until the name matches; this
                    // keeps a stray key from getting past it.
                    if input.read(cx).value().as_ref() != name.expected.as_ref() {
                        return;
                    }
                }
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
                // What the person typed, as it is now: the change event
                // that fills the sink can lag a key pressed with it.
                if let (Some(name), Some(input)) = (&request.type_name, &input) {
                    *name.typed.borrow_mut() = input.read(cx).value().to_string();
                }
                window.close_dialog(cx);
                (request.on_confirm)(window, cx);
            });
            let on_enter = press.clone();
            *enter.borrow_mut() = Some(on_enter);
            let action = Action::new("confirm-ok", confirm_label, move |_, window, cx| {
                press(window, cx)
            })
            .disabled(reason == Some(Blocked::Stale) || name_missing);
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
        let typed_field = request
            .type_name
            .as_ref()
            .zip(input.as_ref())
            .map(|(name, input)| {
                let prompt = name.prompt();
                v_flex()
                    .gap(dp(4.))
                    .child(
                        div()
                            .id("confirm-type-prompt")
                            .test_support()
                            .aria_label(prompt.clone())
                            .child(prompt),
                    )
                    .child(Input::new(input).small().id("confirm-type-name"))
            });
        let enter = enter.clone();
        let body = v_flex()
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
            .children(
                request
                    .warnings
                    .iter()
                    .map(|warning| ui::tag(Tone::Warn, None, warning.clone(), cx)),
            )
            .children(notice);
        // The name sits with the button it unlocks, outside the body that a
        // short window cuts off.
        let actions = v_flex()
            .gap(dp(12.))
            .children(typed_field)
            .child(form::actions(ok, cancel));
        // The body scrolls inside the room under the title bar's inset, so a
        // tall review (two-line target, options, warnings, the typed name)
        // is never pushed up under the window buttons.
        freshkube_ui::dialog::frame_scrolled(
            dialog,
            480.,
            request.title.clone(),
            body,
            actions,
            &scroll,
            window,
        )
        .when(request.type_name.is_some(), |dialog| {
            dialog.on_ok(move |_, window, cx| {
                let press = enter.borrow().clone();
                if let Some(press) = press {
                    press(window, cx);
                }
                // Confirm closes the dialog itself; a wrong name leaves it open.
                false
            })
        })
    });
    // The name is the one thing a protected dialog wants: focus it after the
    // dialog opens, since Kit remembers what had focus before.
    if let Some(input) = focus {
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }
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

/// A setup file a test edits while a dialog is open (#1055): the entry `prod`
/// at a level, in a fresh temporary directory, published as the file a write's
/// send step reads again.
#[cfg(test)]
pub(crate) struct TestSetup {
    dir: tempfile::TempDir,
}

#[cfg(test)]
impl TestSetup {
    /// Writes the file with `prod` at `level`, makes `prod` the window's
    /// entry and publishes the file.
    pub(crate) fn new(level: Protection, cx: &mut gpui_kit::TestAppContext) -> Self {
        let setup = Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        };
        setup.level(level);
        let file = setup.file();
        cx.update(|cx| {
            set_active_entry(Some("prod".into()), cx);
            set_setup_file(Some(file), true, cx);
        });
        setup
    }

    /// A window with no entry active, whose shown context is `window_context`
    /// (the Talos cluster's name is something else), and a setup file that
    /// lists an entry on that context at `level`.
    pub(crate) fn by_context(
        window_context: &str,
        level: Protection,
        cx: &mut gpui_kit::TestAppContext,
    ) -> Self {
        let setup = Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        };
        let text = format!(
            "version: 1\nclusters:\n  - id: other\n    role: core\n    context: {window_context}\n    protection: {}\n",
            level.key()
        );
        std::fs::write(setup.file(), text).expect("the setup file is written");
        let file = setup.file();
        let window_context = window_context.to_owned();
        cx.update(|cx| {
            set_active_entry(None, cx);
            set_setup_file(Some(file), true, cx);
            set_window_context(window_context, cx);
        });
        setup
    }

    /// A legacy `workspace.json` user: the window works under an entry the
    /// migration made, and there is no `freshkube.yaml` to list it.
    pub(crate) fn legacy(cx: &mut gpui_kit::TestAppContext) -> Self {
        let setup = Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        };
        let file = setup.file();
        cx.update(|cx| {
            set_active_entry(Some("prod".into()), cx);
            set_setup_file(Some(file), false, cx);
        });
        setup
    }

    pub(crate) fn file(&self) -> std::path::PathBuf {
        self.dir.path().join("freshkube.yaml")
    }

    /// Edits the file, as an editor outside the app would.
    pub(crate) fn level(&self, level: Protection) {
        let text = format!(
            "version: 1\nclusters:\n  - id: prod\n    role: core\n    context: prod-fra\n    protection: {}\n",
            level.key()
        );
        std::fs::write(self.file(), text).expect("the setup file is written");
    }

    /// The file is gone.
    pub(crate) fn remove(&self) {
        std::fs::remove_file(self.file()).expect("the setup file is removed");
    }

    /// The file no longer parses.
    pub(crate) fn garble(&self) {
        std::fs::write(self.file(), "clusters: [[[").expect("the setup file is written");
    }

    /// The file no longer lists `prod`.
    pub(crate) fn drop_entry(&self) {
        std::fs::write(
            self.file(),
            "version: 1\nclusters:\n  - id: other\n    role: core\n    context: other-ctx\n",
        )
        .expect("the setup file is written");
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{
        Protection, TestSetup, clear_level_floor, level, level_floor, moved_during_read,
        raise_level, send_guard, set_protection, set_setup_file,
    };
    use freshkube_core::write::{Subject, Unreadable};
    use gpui_kit::TestAppContext;

    /// A live window with no setup file can't say, so its send refuses; only
    /// example data with no file is sent without reading.
    #[gpui_kit::test]
    fn safety_g28_a_live_send_with_no_setup_file_refuses_and_only_example_data_goes_unread(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            set_setup_file(None, false, cx);
            let live = send_guard("prod-fra", false, cx).expect("a live send always has a guard");
            assert_eq!(live.read(), Err(Unreadable::NotReadable));
            assert!(send_guard("prod-fra", true, cx).is_none());
        });
    }

    /// A legacy `workspace.json` user has an entry from the migration and no
    /// file: the guard reads the context, which is guarded, not a refusal.
    #[gpui_kit::test]
    fn a_legacy_user_is_read_by_context_not_refused_for_a_missing_file(cx: &mut TestAppContext) {
        let setup = TestSetup::legacy(cx);
        cx.update(|cx| {
            // No context was published here, so the caller's name is the one used.
            let guard = send_guard("callers-context", false, cx).unwrap();
            assert_eq!(guard.subject(), &Subject::Context("callers-context".into()));
            assert_eq!(guard.read(), Ok(Protection::Guarded));
        });
        drop(setup);
    }

    /// With the entries read from the file, the guard names the entry.
    #[gpui_kit::test]
    fn an_entry_listed_in_the_file_is_read_by_its_id(cx: &mut TestAppContext) {
        let _setup = TestSetup::new(Protection::Protected, cx);
        cx.update(|cx| {
            let guard = send_guard("prod-fra", false, cx).unwrap();
            assert_eq!(guard.subject(), &Subject::Entry("prod".into()));
            assert_eq!(guard.read(), Ok(Protection::Protected));
        });
    }

    /// A stricter level a send found is kept until the app reads the file,
    /// and every write follows the cell at once.
    #[gpui_kit::test]
    fn a_raised_level_is_kept_until_cleared_and_never_loosens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            set_protection(Protection::Guarded, cx);
            raise_level(Protection::Protected, cx);
            assert_eq!(level(cx), Protection::Protected);
            assert_eq!(level_floor(cx), Some(Protection::Protected));
            raise_level(Protection::Open, cx);
            assert_eq!(
                level(cx),
                Protection::Protected,
                "a looser one changes nothing"
            );
            assert_eq!(
                moved_during_read(Protection::Guarded, cx).as_deref(),
                Some("This cluster is now protected; review again.")
            );
            clear_level_floor(cx);
            assert_eq!(level_floor(cx), None);
        });
    }

    /// The Context subject takes its name from what the shell published, so the
    /// name read and the level held are about the same context, not from the
    /// Talos cluster's name the caller passes.
    #[gpui_kit::test]
    fn the_context_subject_is_the_windows_not_the_talos_clusters_name(cx: &mut TestAppContext) {
        let _setup = TestSetup::by_context("display-ctx", Protection::ReadOnly, cx);
        cx.update(|cx| {
            let guard = send_guard("talos-cluster", false, cx).unwrap();
            assert_eq!(guard.subject(), &Subject::Context("display-ctx".into()));
            assert_eq!(guard.read(), Ok(Protection::ReadOnly));
        });
    }
}
