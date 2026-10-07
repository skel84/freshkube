//! A shell tab in the dock: a shell in one of a pod's running containers,
//! started only by an explicit action, the pane's Shell menu or the tab's
//! Start (docs/POD_EXEC.md). It is one of the two places the browsing pages
//! change the cluster. A session lives while its tab is open, whatever page
//! shows, and ends when the shell exits, the user ends it, or the tab
//! closes, which the dock asks about first (`unless_shell`). In example
//! mode a local shell answers instead.
//!
//! One terminal serves every session of the tab: a new session clears it,
//! so the remote side starts at the size the terminal already has.

use std::time::Duration;

use freshkube_core::resources::{
    ContainerRole, ContainerState, ExecEnd, ExecFailure, ExecFailureKind, ExecGuard, ExecInput,
    ExecOutput, ExecRequest, ExecSession, ExecSize, Failure, FailureKind, PodContainers,
    start_exec,
};
use gpui_kit::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::backend::{self, OwnedJob};
use crate::forwards;
use crate::logs::choice_label;
use crate::resources::model::ResourceIdentity;
use crate::resources::screen::KubeAccess;
use crate::terminal::{TerminalEvent, TerminalSize, TerminalView};
use crate::ui::Tone;

mod example;
#[cfg(test)]
mod tests;
mod view;

use example::ExampleShell;

/// How long starting may take: reading the pod, then the exec, each with
/// its own deadline in core.
const START_DEADLINE: Duration = Duration::from_secs(65);
/// How long quitting or closing the window waits for shells to end
/// politely (Control-C, then Control-D) before going ahead.
const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// Where the session stands.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ShellState {
    Idle,
    Connecting,
    Running,
    /// The session ended; the status says how.
    Ended,
    Failed,
}

/// One entry of the pane's Shell menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub(crate) name: String,
    pub(crate) role: ContainerRole,
    pub(crate) label: SharedString,
    /// Only a running container can take a shell.
    pub(crate) enabled: bool,
}

/// The Shell menu's entries for a pod's containers.
pub(crate) fn choices(containers: &PodContainers) -> Vec<Choice> {
    containers
        .containers
        .iter()
        .map(|container| Choice {
            name: container.name.clone(),
            role: container.role,
            label: format!("Start shell in {}", choice_label(container)).into(),
            enabled: matches!(container.state, ContainerState::Running(_)),
        })
        .collect()
}

/// What the controls say about the session, derived when it changes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Status {
    pub(super) tone: Tone,
    pub(super) tag: SharedString,
    pub(super) text: SharedString,
    /// Both, for assistive technology and tests.
    pub(super) label: SharedString,
}

/// What the dock hears from a shell tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ShellEvent {
    /// The terminal's leave shortcut: hand the keyboard back.
    Leave,
}

enum Session {
    Connecting {
        size: TerminalSize,
        _job: OwnedJob,
        _task: Task<()>,
    },
    Live {
        /// Dropped when the user ends the session, which closes stdin; the
        /// rest stays until the end arrives.
        input: Option<mpsc::UnboundedSender<ExecInput>>,
        _forward: OwnedJob,
        _delivery: Task<()>,
        /// Closed rather than dropped, so the shell ends politely.
        guard: ExecGuard,
    },
    Example(ExampleShell),
}

pub(crate) struct ShellView {
    runtime: Handle,
    access: Option<KubeAccess>,
    pod: Option<ResourceIdentity>,
    containers: PodContainers,
    /// Whether `containers` was read for this pod yet.
    known: bool,
    pub(super) container: Option<String>,
    pub(crate) state: ShellState,
    pub(super) status: Status,
    pub(super) terminal: Entity<TerminalView>,
    /// The terminal shows an earlier session, which a new one clears.
    used: bool,
    session: Option<Session>,
    /// Advances with each session; anything from an older one is dropped.
    seq: u64,
    /// The title the shell set, for its dock tab's tooltip.
    title: Option<SharedString>,
    /// Its controls and three lines, as last drawn.
    least: Pixels,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ShellEvent> for ShellView {}

/// Every shell view, so quitting can ask about a running session, and the
/// sessions still ending, so quitting can wait for them.
#[derive(Default)]
struct Shells {
    views: Vec<WeakEntity<ShellView>>,
    closing: Vec<JoinHandle<()>>,
}

impl Global for Shells {}

impl ShellView {
    pub(crate) fn new(runtime: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(|cx| TerminalView::new(window, cx));
        let subscriptions = vec![cx.subscribe(&terminal, |this, _, event, cx| {
            this.terminal_event(event, cx)
        })];
        let this = cx.entity().downgrade();
        let shells = cx.default_global::<Shells>();
        shells.views.retain(|shell| shell.upgrade().is_some());
        shells.views.push(this);
        let mut view = Self {
            runtime,
            access: None,
            pod: None,
            containers: PodContainers::default(),
            known: false,
            container: None,
            state: ShellState::Idle,
            status: Status {
                tone: Tone::Unknown,
                tag: SharedString::default(),
                text: SharedString::default(),
                label: SharedString::default(),
            },
            terminal,
            used: false,
            session: None,
            seq: 0,
            title: None,
            least: Pixels::ZERO,
            _subscriptions: subscriptions,
        };
        view.describe(None);
        view
    }

    /// Fresh handles for the same connection.
    pub(crate) fn set_access(&mut self, access: KubeAccess) {
        self.access = Some(access);
    }

    /// Shows the shell of `pod`, or of none. Another pod ends the session
    /// and starts over; the page asked first.
    pub(crate) fn show_pod(
        &mut self,
        pod: Option<ResourceIdentity>,
        access: Option<KubeAccess>,
        cx: &mut Context<Self>,
    ) {
        if access.is_some() {
            self.access = access;
        }
        if self.pod == pod {
            return;
        }
        self.drop_session(cx);
        self.pod = pod;
        self.containers = PodContainers::default();
        self.known = false;
        self.container = None;
        self.state = ShellState::Idle;
        self.title = None;
        if std::mem::take(&mut self.used) {
            self.terminal.update(cx, |terminal, cx| terminal.reset(cx));
        }
        self.describe(None);
        cx.notify();
    }

    /// The UID of a pod asked for by name, as a restored tab's, once it is
    /// found: a session starts only on a pod known by its UID.
    pub(crate) fn settle_uid(&mut self, uid: &str) {
        if let Some(pod) = self.pod.as_mut()
            && pod.uid.is_empty()
        {
            pod.uid = uid.to_owned();
        }
    }

    /// The pod's containers as last read. Without a chosen container the
    /// first read picks the default, or the first running one when the
    /// default isn't.
    pub(crate) fn set_containers(&mut self, containers: PodContainers, cx: &mut Context<Self>) {
        if self.pod.is_none() || (self.known && self.containers == containers) {
            return;
        }
        if self.container.is_none() {
            let running = |name: &str| {
                containers
                    .get(name)
                    .is_some_and(|container| matches!(container.state, ContainerState::Running(_)))
            };
            self.container = containers
                .default
                .clone()
                .filter(|name| running(name))
                .or_else(|| {
                    containers
                        .containers
                        .iter()
                        .find(|container| running(&container.name))
                        .map(|container| container.name.clone())
                })
                .or_else(|| containers.default.clone());
        }
        self.known = true;
        self.containers = containers;
        if self.state == ShellState::Idle {
            self.describe(None);
        }
        cx.notify();
    }

    /// Whether a session runs or is starting: what the page asks about
    /// before ending it.
    pub(crate) fn running(&self) -> bool {
        matches!(self.state, ShellState::Connecting | ShellState::Running)
    }

    /// The pod whose shell runs, for "End the shell in ⟨pod⟩?".
    pub(crate) fn running_pod(&self) -> Option<SharedString> {
        self.running()
            .then(|| self.pod.as_ref().map(|pod| pod.name.clone().into()))
            .flatten()
    }

    /// The title the shell set, which its dock tab's tooltip shows.
    pub(crate) fn title(&self) -> Option<&SharedString> {
        self.title.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn pod(&self) -> Option<&ResourceIdentity> {
        self.pod.as_ref()
    }

    /// The container the shell runs in.
    pub(crate) fn container(&self) -> Option<&str> {
        self.container.as_deref()
    }

    /// The least height the tab needs: its controls and three lines.
    pub(crate) fn least_height(&self) -> Pixels {
        self.least
    }

    /// The terminal's handle once it shows a session: what takes the
    /// keyboard when its tab is selected.
    pub(crate) fn focus_target(&self, cx: &App) -> Option<FocusHandle> {
        (self.state != ShellState::Idle).then(|| self.terminal.focus_handle(cx))
    }

    fn chosen_running(&self) -> bool {
        self.container
            .as_deref()
            .and_then(|name| self.containers.get(name))
            .is_some_and(|container| matches!(container.state, ContainerState::Running(_)))
    }

    /// Whether Start can start a session now.
    pub(crate) fn can_start(&self) -> bool {
        self.pod.as_ref().is_some_and(|pod| !pod.uid.is_empty())
            && self.access.is_some()
            && !self.running()
            && self.chosen_running()
    }

    /// The container the tab's shell runs in, chosen before any session.
    pub(crate) fn choose_container(&mut self, name: String, cx: &mut Context<Self>) {
        if self.running() || self.container.as_ref() == Some(&name) {
            return;
        }
        self.container = Some(name);
        if self.state == ShellState::Idle {
            self.describe(None);
        }
        cx.notify();
    }

    /// Starts a session in the chosen container, with a clear terminal that
    /// takes the keyboard.
    pub(crate) fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_start() {
            return;
        }
        let (Some(pod), Some(container), Some(access)) = (
            self.pod.clone(),
            self.container.clone(),
            self.access.clone(),
        ) else {
            return;
        };
        self.drop_session(cx);
        if std::mem::replace(&mut self.used, true) {
            self.terminal.update(cx, |terminal, cx| terminal.reset(cx));
        }
        self.title = None;
        window.focus(&self.terminal.focus_handle(cx), cx);
        let size = self.terminal.read(cx).size();
        let seq = self.seq;
        if let KubeAccess::Example = access {
            let shell = ExampleShell::new(&pod.name, &container, size);
            let greeting = shell.start();
            self.session = Some(Session::Example(shell));
            self.state = ShellState::Running;
            self.describe(None);
            self.terminal
                .update(cx, |terminal, cx| terminal.feed(&greeting, cx));
            cx.notify();
            return;
        }
        let request = ExecRequest {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            uid: pod.uid.clone(),
            container,
            size: ExecSize {
                columns: size.columns,
                rows: size.rows,
            },
        };
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            START_DEADLINE,
            format!("No answer within {} seconds", START_DEADLINE.as_secs()),
            async move {
                let client = match access.client().await {
                    Ok(client) => client,
                    Err(error) => {
                        access.forget();
                        return Ok(Err(Failure::new(FailureKind::Other, error).into()));
                    }
                };
                let result = start_exec(client, request).await;
                if let Err(ExecFailure {
                    kind: ExecFailureKind::Request(kind),
                    ..
                }) = &result
                    && !kind.is_permanent()
                {
                    access.forget();
                }
                Ok(result)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(result)) => result,
                Ok(Err(message)) => Err(Failure::new(FailureKind::Timeout, message).into()),
                Err(_) => return,
            };
            _ = this.update(cx, |shell, cx| shell.connected(seq, result, cx));
        });
        self.session = Some(Session::Connecting {
            size,
            _job: job,
            _task: task,
        });
        self.state = ShellState::Connecting;
        self.describe(None);
        cx.notify();
    }

    /// The exec was accepted or refused.
    fn connected(
        &mut self,
        seq: u64,
        result: Result<ExecSession, ExecFailure>,
        cx: &mut Context<Self>,
    ) {
        let Some(Session::Connecting { size, .. }) = &self.session else {
            return;
        };
        if seq != self.seq {
            return;
        }
        let requested = *size;
        let ExecSession {
            input,
            mut output,
            guard,
        } = match result {
            Ok(session) => session,
            Err(failure) => return self.fail(failure, cx),
        };
        // Keys go out in order through one task, however fast they come.
        let (sender, mut queue) = mpsc::unbounded_channel();
        let forward = OwnedJob::new(self.runtime.spawn(async move {
            while let Some(message) = queue.recv().await {
                if input.send(message).await.is_err() {
                    break;
                }
            }
        }));
        let delivery = cx.spawn(async move |this, cx| {
            while let Some(output) = output.recv().await {
                if this
                    .update(cx, |shell, cx| shell.apply(seq, output, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        // The terminal may have changed size while connecting.
        let size = self.terminal.read(cx).size();
        if size != requested {
            _ = sender.send(ExecInput::Resize(ExecSize {
                columns: size.columns,
                rows: size.rows,
            }));
        }
        self.session = Some(Session::Live {
            input: Some(sender),
            _forward: forward,
            _delivery: delivery,
            guard,
        });
        self.state = ShellState::Running;
        self.describe(None);
        cx.notify();
    }

    fn apply(&mut self, seq: u64, output: ExecOutput, cx: &mut Context<Self>) {
        if seq != self.seq {
            return;
        }
        match output {
            ExecOutput::Bytes(bytes) => self
                .terminal
                .update(cx, |terminal, cx| terminal.feed(&bytes, cx)),
            ExecOutput::End(end) => self.finish(end, cx),
        }
    }

    /// The session ended. Its screen stays until a new one starts.
    fn finish(&mut self, end: ExecEnd, cx: &mut Context<Self>) {
        self.drop_session(cx);
        self.terminal.update(cx, |terminal, cx| terminal.end(cx));
        if self.state == ShellState::Ended {
            // The user ended it, and has been told.
            return;
        }
        let text = match end {
            ExecEnd::Exited(0) => "The shell exited.".to_owned(),
            ExecEnd::Exited(code) => format!("The shell exited with code {code}."),
            ExecEnd::PodGone(reason) => format!("{reason}."),
            ExecEnd::Disconnected(failure) => format!("The connection was lost · {failure}"),
            ExecEnd::Closed => "You ended the shell.".to_owned(),
            ExecEnd::Failed(failure) => return self.fail(failure, cx),
        };
        self.state = ShellState::Ended;
        self.describe(Some(text));
        cx.notify();
    }

    fn fail(&mut self, failure: ExecFailure, cx: &mut Context<Self>) {
        self.drop_session(cx);
        self.terminal.update(cx, |terminal, cx| terminal.end(cx));
        self.state = ShellState::Failed;
        let tag = match failure.kind {
            ExecFailureKind::Request(FailureKind::Forbidden | FailureKind::Unauthorized) => {
                "Not permitted"
            }
            ExecFailureKind::Request(FailureKind::NotFound) => "Not found",
            ExecFailureKind::Request(FailureKind::Unreachable | FailureKind::Timeout) => {
                "Unreachable"
            }
            ExecFailureKind::Request(_) => "Failed",
            ExecFailureKind::NotRunning => "Not running",
            ExecFailureKind::NoShell => "No shell",
        };
        self.describe(Some(failure.to_string()));
        self.status.tag = tag.into();
        self.status.label = format!("{tag}: {}", self.status.text).into();
        cx.notify();
    }

    /// End: a starting session stops at once; a running one closes its
    /// input, and its last output still shows.
    pub(crate) fn end(&mut self, cx: &mut Context<Self>) {
        match self.state {
            ShellState::Connecting => {
                self.drop_session(cx);
                self.state = ShellState::Idle;
                self.describe(None);
            }
            ShellState::Running => {
                match &mut self.session {
                    Some(Session::Live { input, .. }) => drop(input.take()),
                    _ => self.drop_session(cx),
                }
                self.terminal.update(cx, |terminal, cx| terminal.end(cx));
                self.state = ShellState::Ended;
                self.describe(Some("You ended the shell.".to_owned()));
            }
            _ => return,
        }
        cx.notify();
    }

    /// Lets the session go. A live one ends politely in the background:
    /// closing the connection alone would leave the shell, and whatever
    /// runs in it, running in the container.
    fn drop_session(&mut self, cx: &mut App) {
        if let Some(Session::Live { guard, .. }) = self.session.take() {
            let closing = guard.close();
            let shells = cx.default_global::<Shells>();
            shells.closing.retain(|task| !task.is_finished());
            shells.closing.push(closing);
        }
        self.seq += 1;
    }

    /// Ends the session for good, as its tab or the window closes or the
    /// app quits.
    pub(crate) fn close_session(&mut self, cx: &mut Context<Self>) {
        match self.state {
            ShellState::Connecting => {
                self.drop_session(cx);
                self.state = ShellState::Idle;
                self.describe(None);
            }
            ShellState::Running => {
                self.drop_session(cx);
                self.terminal.update(cx, |terminal, cx| terminal.end(cx));
                self.state = ShellState::Ended;
                self.describe(Some("You ended the shell.".to_owned()));
            }
            _ => return,
        }
        cx.notify();
    }

    fn terminal_event(&mut self, event: &TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::Output(bytes) => self.send(bytes, cx),
            TerminalEvent::Resize(size) => match &mut self.session {
                Some(Session::Live {
                    input: Some(input), ..
                }) => {
                    _ = input.send(ExecInput::Resize(ExecSize {
                        columns: size.columns,
                        rows: size.rows,
                    }));
                }
                Some(Session::Example(shell)) => {
                    let bytes = shell.resize(*size);
                    if !bytes.is_empty() {
                        self.terminal
                            .update(cx, |terminal, cx| terminal.feed(&bytes, cx));
                    }
                }
                _ => {}
            },
            TerminalEvent::Title(title) => {
                self.title = title.clone();
                cx.notify();
            }
            TerminalEvent::Leave => cx.emit(ShellEvent::Leave),
        }
    }

    /// Keys, pastes and the terminal's replies, for the shell. Nothing
    /// goes anywhere once the session ended.
    fn send(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        match &mut self.session {
            Some(Session::Live {
                input: Some(input), ..
            }) => {
                _ = input.send(ExecInput::Bytes(bytes.to_vec()));
            }
            Some(Session::Example(shell)) => {
                let reply = shell.input(bytes);
                if !reply.bytes.is_empty() {
                    self.terminal
                        .update(cx, |terminal, cx| terminal.feed(&reply.bytes, cx));
                }
                if let Some(code) = reply.exit {
                    self.finish(ExecEnd::Exited(code), cx);
                }
            }
            _ => {}
        }
    }

    /// Derives what the controls say; `ended` is how the session ended.
    fn describe(&mut self, ended: Option<String>) {
        let container = self.container.clone().unwrap_or_default();
        let (tone, tag, text) = match &self.state {
            ShellState::Idle if self.pod.is_some() && !self.known => {
                (Tone::Unknown, "Reading", String::new())
            }
            ShellState::Idle if container.is_empty() => {
                (Tone::Unknown, "", "The pod has no containers.".to_owned())
            }
            ShellState::Idle if !self.chosen_running() => (
                Tone::Warn,
                "Not running",
                format!("{container} isn't running, so it can't take a shell."),
            ),
            ShellState::Idle => (
                Tone::Unknown,
                "",
                format!("Start runs a shell in {container}, as kubectl exec does."),
            ),
            ShellState::Connecting => (
                Tone::Unknown,
                "Connecting",
                format!("Starting a shell in {container}"),
            ),
            ShellState::Running => (
                Tone::Good,
                "Running",
                format!(
                    "{container} · {} leaves the terminal",
                    crate::terminal::leave_shortcut_label()
                ),
            ),
            ShellState::Ended => (Tone::Unknown, "Ended", ended.unwrap_or_default()),
            ShellState::Failed => (Tone::Crit, "Failed", ended.unwrap_or_default()),
        };
        let label = match (tag, text.as_str()) {
            ("", text) => text.to_owned(),
            (tag, "") => tag.to_owned(),
            (tag, text) => format!("{tag}: {text}"),
        };
        self.status = Status {
            tone,
            tag: tag.into(),
            text: text.into(),
            label: label.into(),
        };
    }
}

/// The pod of each shell running anywhere in the app.
pub(crate) fn running_anywhere(cx: &App) -> Vec<SharedString> {
    let Some(shells) = cx.try_global::<Shells>() else {
        return Vec::new();
    };
    shells
        .views
        .iter()
        .filter_map(WeakEntity::upgrade)
        .filter_map(|shell| shell.read(cx).running_pod())
        .collect()
}

/// "End the shell in ⟨pod⟩?" or "End 3 shells?", with its detail and the
/// agreeing answer.
fn ending_question(pods: &[SharedString]) -> (String, &'static str, &'static str) {
    match pods {
        [pod] => (
            format!("End the shell in {pod}?"),
            SHELL_DETAIL,
            "End the shell",
        ),
        pods => (
            format!("End {} shells?", pods.len()),
            SHELLS_DETAIL,
            "End the shells",
        ),
    }
}

const SHELL_DETAIL: &str = "Freshkube sends Control-C, then Control-D, to stop what runs and end \
                            the shell. A program that ignores them, such as an open editor, \
                            keeps running in the pod.";
const SHELLS_DETAIL: &str = "Freshkube sends Control-C, then Control-D, to stop what runs and \
                             end each shell. A program that ignores them, such as an open \
                             editor, keeps running in its pod.";

/// Asks to end the shells in `pods`; resolves to whether the user agreed.
fn ask(
    pods: &[SharedString],
    window: &mut Window,
    cx: &mut App,
) -> impl Future<Output = bool> + use<> {
    let (question, detail, answer) = ending_question(pods);
    let answer = window.prompt(
        PromptLevel::Warning,
        &question,
        Some(detail),
        &[answer, "Cancel"],
        cx,
    );
    async move { answer.await == Ok(0) }
}

/// Runs `then` at once when no shell runs (`pods` is empty), or once the
/// user agrees to end the shells in `pods`. Cancel leaves everything as it
/// was.
pub(crate) fn unless_shell<V: 'static>(
    view: &mut V,
    pods: Vec<SharedString>,
    window: &mut Window,
    cx: &mut Context<V>,
    then: impl FnOnce(&mut V, &mut Window, &mut Context<V>) + 'static,
) {
    if pods.is_empty() {
        return then(view, window, cx);
    }
    let agreed = ask(&pods, window, cx);
    cx.spawn_in(window, async move |this, cx| {
        if agreed.await {
            _ = this.update_in(cx, then);
        }
    })
    .detach();
}

/// Whether the window may close now. With a shell or a forward running it
/// asks first, in one question, and `close` runs once the user agrees and
/// every shell has ended and every forward stopped.
pub(crate) fn may_close(
    window: &mut Window,
    cx: &mut App,
    close: impl FnOnce(&mut Window, &mut App) + 'static,
) -> bool {
    let pods = running_anywhere(cx);
    let forwards = forwards::running(cx);
    if pods.is_empty() && forwards == 0 {
        return true;
    }
    let agreed = ask_closing(&pods, forwards, window, cx);
    window
        .spawn(cx, async move |cx| {
            if !agreed.await {
                return;
            }
            if let Ok((ending, stopping)) = cx.update(|_, cx| (end_all(cx), forwards::stop_all(cx)))
            {
                futures::future::join(ending, stopping).await;
            }
            _ = cx.update(close);
        })
        .detach();
    false
}

/// Asks before quitting or closing the window: "End the shell in ⟨pod⟩?",
/// "Stop 2 forwards?" or both in one. Resolves to whether the user agreed.
fn ask_closing(
    pods: &[SharedString],
    forwards: usize,
    window: &mut Window,
    cx: &mut App,
) -> impl Future<Output = bool> + use<> {
    let (question, detail, answer) = closing_question(pods, forwards);
    let answer = window.prompt(
        PromptLevel::Warning,
        &question,
        Some(&detail),
        &[answer.as_str(), "Cancel"],
        cx,
    );
    async move { answer.await == Ok(0) }
}

/// The question, its detail and the agreeing answer.
fn closing_question(pods: &[SharedString], forwards: usize) -> (String, String, String) {
    let stop = match forwards {
        1 => "stop 1 forward".to_owned(),
        count => format!("stop {count} forwards"),
    };
    let forward_detail = "Stopping closes each local port and every connection through it.";
    if pods.is_empty() {
        return (
            format!("{}?", capitalized(&stop)),
            forward_detail.to_owned(),
            "Stop".to_owned(),
        );
    }
    let (question, detail, answer) = ending_question(pods);
    if forwards == 0 {
        return (question, detail.to_owned(), answer.to_owned());
    }
    (
        format!("{} and {stop}?", question.trim_end_matches('?')),
        format!("{detail} {forward_detail}"),
        "End and stop".to_owned(),
    )
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Ends every session, and resolves once they have ended politely or
/// [`CLOSE_GRACE`] has passed.
fn end_all(cx: &mut App) -> impl Future<Output = ()> + use<> {
    let shells: Vec<_> = cx
        .default_global::<Shells>()
        .views
        .iter()
        .filter_map(WeakEntity::upgrade)
        .collect();
    for shell in shells {
        shell.update(cx, |shell, cx| shell.close_session(cx));
    }
    let closing = std::mem::take(&mut cx.default_global::<Shells>().closing);
    let grace = cx.background_executor().timer(CLOSE_GRACE);
    async move {
        futures::future::select(futures::future::join_all(closing), grace).await;
    }
}
