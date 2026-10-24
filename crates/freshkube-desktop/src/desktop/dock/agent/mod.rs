//! The Agent dock tab: the person's own agent investigates the cluster for
//! them, reading only through Freshkube's four read tools (docs/features/agent.md).
//!
//! **One owner for the session.** `AgentView` owns the Tokio job that holds
//! the endpoint and the ACP session (`run.rs`) and the GPUI task that
//! delivers what it reports. Both are user-started, like a dock log tab: a
//! session starts with the first question once access is on, reads while the
//! tab is open on any page, and ends when the tab closes (the job is dropped,
//! which closes the port, ends the token and ends the agent), the agent
//! exits, or the app quits. Every message carries the generation of the
//! session that sent it; one from a session that has ended publishes nothing.
//!
//! **Access is per cluster and off by default.** Nothing is read, and no
//! session starts, until the person turns it on from the egress card. Turning
//! it off is `Access::set_access`, which ends the reads in flight first and
//! then saves; a failed save leaves access off here and says the file may
//! turn it back on at the next start.

mod example;
mod follow;
mod host;
mod run;
mod setup;
mod targets;
mod trail;
mod view;

#[cfg(test)]
mod follow_tests;
#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use freshkube_agent::acp::Models;
use freshkube_agent::investigation::{Entry, Event, Investigation, Outcome};
use freshkube_agent::scripted::Script;
use freshkube_agent::tools::NotOffered;
use freshkube_core::job::OwnedJob;
use freshkube_core::workspace::Workspace;
use freshkube_ui::level::Mark;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;

use crate::agent_shape::AgentShape;
pub(crate) use follow::{AgentEvent, FollowStrip};
pub(crate) use host::AgentHost;
use targets::DockTargets;
pub(crate) use targets::Own;
use trail::Trail;

/// At most this many transcript entries are drawn, the newest; the
/// transcript itself is bounded by `freshkube-agent` (1 MiB of text, 1,000
/// steps), so this is a hard cap on what a frame draws.
const DRAWN_ENTRIES: usize = 300;
/// A question is cut here before it is sent.
const MAX_QUESTION_BYTES: usize = 4 * 1024;
/// The bytes of speech kept to read a citation split across two chunks.
const CITE_CARRY: usize = 8;

/// What the tab says above its body when something went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Notice {
    /// The session could not start, or the turn failed.
    Failed(String),
    /// An answered turn made no read and showed the agent was not offered
    /// Freshkube's tools (`freshkube_agent::tools::not_offered`).
    ToolsNotOffered(NotOffered),
    /// The saved model isn't offered any more; the agent's default is used.
    ModelMissing(String),
    /// Turning access on could not be saved: it stays off.
    AccessNotSaved(String),
    /// Turning access off could not be saved. It is off now. The file may
    /// still say on, so a note beside it keeps the cluster off at the next
    /// start; a retry, or the quit, writes the file.
    SaveFailed { error: String, remembered: bool },
    /// Turning access off was never saved, so at start the file's `on` was
    /// not trusted and the cluster began off.
    HeldOff,
}

/// A running session: dropping it ends everything.
struct Running {
    _job: OwnedJob,
    commands: UnboundedSender<run::Command>,
    _delivery: Task<()>,
}

/// The elapsed time of the turn, in an entity of its own so its tick draws
/// nothing else.
pub(crate) struct Elapsed {
    since: Option<Instant>,
    total: Duration,
    tick: Option<Task<()>>,
}

impl Elapsed {
    fn new() -> Self {
        Self {
            since: None,
            total: Duration::ZERO,
            tick: None,
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.since = Some(cx.background_executor().now());
        self.total = Duration::ZERO;
        self.tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        }));
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(since) = self.since.take() {
            self.total = cx.background_executor().now().duration_since(since);
        }
        self.tick = None;
        cx.notify();
    }

    pub(crate) fn seconds(&self, cx: &App) -> Option<u64> {
        match self.since {
            Some(since) => Some(
                cx.background_executor()
                    .now()
                    .duration_since(since)
                    .as_secs(),
            ),
            None => (!self.total.is_zero()).then_some(self.total.as_secs()),
        }
    }
}

impl Render for Elapsed {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = crate::palette::palette(cx);
        let text = self
            .seconds(cx)
            .map(|s| format!("{}:{:02}", s / 60, s % 60))
            .unwrap_or_default();
        div()
            .id("agent-elapsed")
            .test_support()
            .text_color(p.muted)
            .child(text)
    }
}

pub(crate) struct AgentView {
    runtime: Handle,
    host: AgentHost,
    /// The session's own cluster: fixed when the tab opens, shown in the
    /// toolbar, never moved by a context switch. `None` when the cluster
    /// the tab opened on is not an entry of the setup, so it can't be granted.
    own: Option<Own>,
    /// "⟨entry⟩ · ⟨context⟩".
    cluster: SharedString,
    level: Mark,
    example: bool,
    shape: AgentShape,
    script: Option<Script>,
    setup: setup::Setup,
    found: Option<setup::Found>,
    chosen: Option<std::path::PathBuf>,
    check: Option<(OwnedJob, Task<()>)>,
    investigation: Investigation,
    trail: Trail,
    models: Models,
    /// The model the person chose: saved in `agent.json`, and kept here for
    /// the session when the save fails.
    model: Option<String>,
    input: Entity<InputState>,
    elapsed: Entity<Elapsed>,
    session: Option<Running>,
    generation: u64,
    /// The question waiting for the egress card's "Turn on".
    pending: Option<String>,
    working: bool,
    notice: Option<Notice>,
    /// Presses Stop as soon as a step starts (the `stopped` shape).
    stop_at_first_step: bool,
    shared_shape: bool,
    /// The sign-in command the Signed out card shows, made once the data
    /// directories are ready, or the error line saying why they could not be
    /// made.
    sign_in: Option<Result<String, String>>,
    /// The endpoint's port while the session runs.
    #[cfg_attr(not(test), allow(dead_code))]
    listening: Option<std::net::SocketAddr>,
    focus: FocusHandle,
    /// The tail of the speech so far, so a citation split across two chunks
    /// is still read without scanning the whole answer again.
    cite_carry: String,
    /// What each failed step of Freshkube's own reads stands for, by its
    /// entry's index: refused, or a read that broke. Derived when a step or a
    /// read arrives, by the order of the turn's steps and reads.
    step_marks: std::collections::HashMap<usize, trail::Mark>,
    /// Which read the page shows, when, and the strip that says so; `None`
    /// for a tab that isn't an entry of the setup, which reads nothing.
    follow: Option<follow::Follow>,
    #[cfg(test)]
    targets: Option<std::sync::Arc<dyn freshkube_agent::Targets>>,
    /// The text of each transcript entry as shown, kept when it changes so a
    /// frame copies none of it.
    texts: Vec<SharedString>,
    /// The least height the dock keeps for this tab: the toolbar and a few
    /// rows.
    least: Pixels,
    _subscriptions: Vec<Subscription>,
}

/// The least the dock gives an Agent tab, in dp.
const LEAST_HEIGHT: f32 = 180.;

impl AgentView {
    pub(crate) fn new(
        runtime: Handle,
        own: Option<Own>,
        level: Mark,
        shape: AgentShape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = AgentHost::get(cx);
        let example = own.as_ref().is_some_and(|own| {
            matches!(
                own.access,
                freshkube_core::cluster_source::ClusterAccess::Example(_)
            )
        });
        let cluster: SharedString = match &own {
            Some(own) if own.entry != own.context => format!("{} · {}", own.entry, own.context),
            Some(own) => own.entry.clone(),
            None => String::new(),
        }
        .into();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask about this cluster"));
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.ask(window, cx);
                    }
                }),
            ];
        let mut subscriptions = subscriptions;
        // Quitting ends the agent now, rather than leaving it to the process
        // group's watcher.
        subscriptions.push(cx.on_app_quit(|this, cx| {
            this.end(cx);
            async {}
        }));
        let mut view = Self {
            runtime,
            host,
            own,
            cluster,
            level,
            example,
            shape,
            script: None,
            setup: if example {
                setup::example(shape)
            } else {
                setup::Setup::Checking
            },
            found: None,
            chosen: None,
            check: None,
            investigation: Investigation::default(),
            trail: Trail::default(),
            models: Models::default(),
            model: None,
            input,
            elapsed: cx.new(|_| Elapsed::new()),
            session: None,
            generation: 0,
            pending: None,
            working: false,
            notice: None,
            stop_at_first_step: false,
            shared_shape: shape == AgentShape::Shared,
            sign_in: None,
            listening: None,
            focus: cx.focus_handle(),
            cite_carry: String::new(),
            step_marks: Default::default(),
            follow: None,
            #[cfg(test)]
            targets: None,
            texts: Vec::new(),
            least: crate::ui::dp_px(LEAST_HEIGHT, window),
            _subscriptions: subscriptions,
        };
        view.follow = view
            .own
            .as_ref()
            .map(|own| follow::Follow::new(&own.entry, cx.entity().downgrade(), cx));
        view.chosen = view.host.access.opencode_path();
        view.model = view.host.access.model();
        if view
            .own
            .as_ref()
            .is_some_and(|own| view.host.access.held_off().contains(&own.entry))
            && !view.access_on()
        {
            view.notice = Some(Notice::HeldOff);
        }
        if example {
            if matches!(view.setup, setup::Setup::SignedOut { .. }) {
                view.prepare_sign_in();
            }
            view.apply_shape(cx);
        } else {
            view.check_again(cx);
        }
        view
    }

    /// The example shapes that start somewhere past the setup.
    fn apply_shape(&mut self, cx: &mut Context<Self>) {
        let shape = self.shape;
        if shape.grants() {
            self.turn_on_access("egress card", cx);
        }
        if shape == AgentShape::SaveFailed {
            // The file can't be written: a directory sits where it should be.
            let path = self.host.access.file_path().to_path_buf();
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::create_dir_all(&path);
            self.turn_off_access(cx);
        }
        self.stop_at_first_step = shape == AgentShape::Stopped;
        if shape.asks() {
            self.submit(example::QUESTION.to_owned(), cx);
        }
    }

    pub(crate) fn least_height(&self) -> Pixels {
        self.least
    }

    pub(crate) fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// The level tag of the cluster this tab belongs to, as the dock last
    /// read it.
    pub(crate) fn set_level(&mut self, level: Mark, cx: &mut Context<Self>) {
        if self.level != level {
            self.level = level;
            cx.notify();
        }
    }

    /// Looks for opencode again (or, for an example, does nothing).
    pub(crate) fn check_again(&mut self, cx: &mut Context<Self>) {
        if self.example {
            return;
        }
        self.setup = setup::Setup::Checking;
        let chosen = self.chosen.clone();
        let data = self.data();
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let job = OwnedJob::new(self.runtime.spawn(async move {
            let _ = sender.send(setup::check(chosen, data).await);
        }));
        let delivery = cx.spawn(async move |this, cx| {
            if let Ok((setup, found)) = (&mut receiver).await {
                _ = this.update(cx, |view, cx| {
                    view.setup = setup;
                    view.found = found;
                    cx.notify();
                });
            }
        });
        self.check = Some((job, delivery));
        cx.notify();
    }

    /// "Choose a file": the person picks an opencode; the path is kept in
    /// `agent.json` and tried first.
    pub(crate) fn choose_file(&mut self, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use opencode".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = selection.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            _ = this.update(cx, |view, cx| view.use_file(path, cx));
        })
        .detach();
    }

    pub(crate) fn use_file(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if let Err(error) = self.host.access.set_opencode_path(Some(path.clone())) {
            self.notice = Some(Notice::Failed(format!(
                "The chosen file couldn't be kept in agent.json: {error}"
            )));
        }
        self.chosen = Some(path);
        self.check_again(cx);
    }

    fn data(&self) -> freshkube_agent::profile::Data {
        if self.data_shared() {
            freshkube_agent::profile::Data::Shared
        } else {
            freshkube_agent::profile::Data::Own(self.host.own_data())
        }
    }

    /// Makes the data directories private, then the sign-in command for the
    /// data mode the session runs in, built from the same directories its
    /// environment gets. A failure to prepare is kept instead, as the
    /// card's error line, and no command is shown.
    fn prepare_sign_in(&mut self) {
        let data = self.data();
        let host = freshkube_agent::profile::Host::current();
        self.sign_in = Some(
            data.prepare(&host)
                .map(|()| data.sign_in_command(&host))
                .map_err(|error| {
                    format!("Freshkube couldn't make its opencode data directory private: {error}")
                }),
        );
    }

    /// The sign-in command, or why there is none; None until signed out.
    pub(crate) fn sign_in(&self) -> Option<&Result<String, String>> {
        self.sign_in.as_ref()
    }

    /// Whether opencode runs on the person's own data (`data: shared` in
    /// `agent.json`).
    pub(crate) fn data_shared(&self) -> bool {
        self.shared_shape || self.host.access.data_mode() == freshkube_agent::DataMode::Shared
    }

    /// Whether agent access is on for this tab's cluster.
    pub(crate) fn access_on(&self) -> bool {
        self.own
            .as_ref()
            .is_some_and(|own| self.host.gate.is_on(&own.entry, &own.context))
    }

    /// Reads the question box and asks it.
    pub(crate) fn ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_owned();
        if text.is_empty() || self.working {
            return;
        }
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.submit(text, cx);
    }

    /// Asks `question`. With access off nothing starts: the egress card
    /// shows and the question waits for "Turn on".
    pub(crate) fn submit(&mut self, mut question: String, cx: &mut Context<Self>) {
        if !self.setup.can_start() || self.own.is_none() || self.working {
            return;
        }
        if question.len() > MAX_QUESTION_BYTES {
            let mut end = MAX_QUESTION_BYTES;
            while !question.is_char_boundary(end) {
                end -= 1;
            }
            question.truncate(end);
        }
        self.notice = self
            .notice
            .take()
            .filter(|notice| matches!(notice, Notice::SaveFailed { .. } | Notice::HeldOff));
        if !self.access_on() {
            self.pending = Some(question);
            cx.notify();
            return;
        }
        self.send(question, cx);
    }

    fn send(&mut self, question: String, cx: &mut Context<Self>) {
        if self.session.is_none() && !self.start_session(cx) {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let _ = session.commands.send(run::Command::Prompt(question));
        self.working = true;
        self.elapsed.update(cx, |elapsed, cx| elapsed.start(cx));
        cx.notify();
    }

    /// The egress card's "Turn on".
    pub(crate) fn turn_on(&mut self, cx: &mut Context<Self>) {
        if self.turn_on_access("egress card", cx)
            && let Some(question) = self.pending.take()
        {
            self.send(question, cx);
        }
    }

    /// "Not now" on the egress card: nothing was read, nothing starts.
    pub(crate) fn decline(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }

    fn turn_on_access(&mut self, from: &str, cx: &mut Context<Self>) -> bool {
        let Some(own) = &self.own else {
            return false;
        };
        match self
            .host
            .access
            .set_access(&own.entry, &own.context, true, from)
        {
            Ok(()) => {
                self.notice = None;
                cx.notify();
                true
            }
            Err(error) => {
                self.notice = Some(Notice::AccessNotSaved(error.to_string()));
                cx.notify();
                false
            }
        }
    }

    /// Turns agent access off for this cluster. The gate ends the reads in
    /// flight at once; the save follows, and if it fails the tab says so.
    pub(crate) fn turn_off_access(&mut self, cx: &mut Context<Self>) {
        let Some(own) = &self.own else {
            return;
        };
        match self
            .host
            .access
            .set_access(&own.entry, &own.context, false, "agent tab")
        {
            Ok(()) => {
                if matches!(self.notice, Some(Notice::SaveFailed { .. })) {
                    self.notice = None;
                }
            }
            Err(error) => self.notice = Some(self.save_failed(error)),
        }
        cx.notify();
    }

    fn save_failed(&self, error: std::io::Error) -> Notice {
        Notice::SaveFailed {
            error: error.to_string(),
            remembered: self.host.access.revocation_remembered(),
        }
    }

    /// Writes `agent.json` again after a failed save of turning access off.
    pub(crate) fn retry_save(&mut self, cx: &mut Context<Self>) {
        match self.host.access.retry_save() {
            Ok(()) => self.notice = None,
            Err(error) => self.notice = Some(self.save_failed(error)),
        }
        cx.notify();
    }

    /// Stop: cancels the prompt. The session stays for the next question.
    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            let _ = session.commands.send(run::Command::Cancel);
        }
        cx.notify();
    }

    /// Chooses one of the models the agent offered.
    pub(crate) fn choose_model(&mut self, id: String, cx: &mut Context<Self>) {
        self.model = Some(id.clone());
        // A failed save keeps the choice for this session, quietly.
        if let Err(error) = self.host.access.set_model(Some(id.clone())) {
            tracing::debug!("the chosen agent model could not be saved: {error}");
        }
        if let Some(session) = &self.session {
            let _ = session.commands.send(run::Command::SetModel(id));
        }
        cx.notify();
    }

    /// Ends the session now, as closing the tab does: the endpoint's port
    /// closes, its token ends and the agent and its group are gone.
    pub(crate) fn end(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.session = None;
        // The page is handed back, so the followed log tab goes with it.
        self.hand_back(cx);
        cx.emit(AgentEvent::Ended);
        self.follow = None;
        self.listening = None;
        self.check = None;
        self.working = false;
        self.pending = None;
        self.elapsed.update(cx, |elapsed, cx| elapsed.stop(cx));
    }

    /// Targets a test put in place of the dock's own.
    #[cfg(test)]
    fn injected_targets(&self) -> Option<std::sync::Arc<dyn freshkube_agent::Targets>> {
        self.targets.clone()
    }

    #[cfg(not(test))]
    fn injected_targets(&self) -> Option<std::sync::Arc<dyn freshkube_agent::Targets>> {
        None
    }

    fn start_session(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(own) = self.own.clone() else {
            return false;
        };
        let workspace = crate::mutation::workspace(cx);
        let agent = if self.example {
            let other = other_entry(workspace.as_ref(), &own.entry);
            run::Agent::Scripted(
                self.script
                    .clone()
                    .unwrap_or_else(|| example::script(&own.context, other.as_deref(), self.shape)),
            )
        } else {
            match &self.found {
                Some(found) => run::Agent::Opencode {
                    profile: found.profile.clone(),
                    checked: found.checked.clone(),
                },
                None => return false,
            }
        };
        let launch = run::Launch {
            entry: own.entry.clone(),
            gate: self.host.gate.clone(),
            targets: self
                .injected_targets()
                .unwrap_or_else(|| DockTargets::new(&own, workspace.as_ref()).shared()),
            agent,
            model: self.model.clone(),
        };
        let (job, commands, mut receiver) = run::start(&self.runtime, launch);
        self.generation += 1;
        let generation = self.generation;
        let delivery = cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.recv().await {
                if this
                    .update(cx, |view, cx| view.apply(generation, message, cx))
                    .is_err()
                {
                    return;
                }
            }
        });
        self.session = Some(Running {
            _job: job,
            commands,
            _delivery: delivery,
        });
        true
    }

    /// Publishes what the session reported, unless it came from a session
    /// that has since ended or been replaced.
    fn apply(&mut self, generation: u64, message: run::Message, cx: &mut Context<Self>) {
        if generation != self.generation || self.session.is_none() {
            return;
        }
        match message {
            run::Message::Event(event) => self.record(event, cx),
            run::Message::Read(read) => {
                if let Some(own) = &self.own {
                    self.trail.read(&read, &own.entry);
                }
                self.follow_read(&read, cx);
            }
            run::Message::Models(models) => {
                match &models.missing {
                    Some(id) if self.notice.is_none() => {
                        self.notice = Some(Notice::ModelMissing(id.clone()));
                    }
                    None if matches!(self.notice, Some(Notice::ModelMissing(_))) => {
                        self.notice = None;
                    }
                    _ => {}
                }
                // Only OpenCode's own models: show how to sign in elsewhere.
                if !models.choices.is_empty()
                    && models.choices.iter().all(|choice| choice.app_only)
                    && self.sign_in.is_none()
                {
                    self.prepare_sign_in();
                }
                self.models = models;
            }
            run::Message::Listening(addr) => self.listening = Some(addr),
            run::Message::ToolsNotOffered(why) => self.tools_not_offered(why),
            run::Message::Ended(reason) => self.session_ended(reason, cx),
        }
        self.refresh_steps();
        cx.notify();
    }

    /// Shows that the tools weren't offered unless another notice is up. The
    /// session tells it at each answered turn that shows it, so it comes up
    /// after the other one clears.
    fn tools_not_offered(&mut self, why: NotOffered) {
        self.notice.get_or_insert(Notice::ToolsNotOffered(why));
    }

    /// The session is over (the agent exited, or it could not start): the
    /// page is handed back, so no pending move and no followed log tab
    /// outlive it.
    fn session_ended(&mut self, reason: Option<String>, cx: &mut Context<Self>) {
        self.session = None;
        self.listening = None;
        self.working = false;
        self.elapsed.update(cx, |elapsed, cx| elapsed.stop(cx));
        self.hand_back(cx);
        cx.emit(AgentEvent::Ended);
        if let Some(reason) = reason {
            self.notice = Some(Notice::Failed(reason));
        }
    }

    /// Says what each failed step of Freshkube's own reads stands for. A
    /// turn's steps of ours and its reads come in the same order, so the
    /// k-th step is the k-th read; a failed step with no read has failed for
    /// another reason and stays Failed.
    fn refresh_steps(&mut self) {
        if self.investigation.trimmed() {
            // Indices moved when text was trimmed off the front.
            self.step_marks.clear();
        }
        let entries = self.investigation.entries();
        let Some(start) = entries
            .iter()
            .rposition(|entry| matches!(entry, Entry::Asked { .. }))
        else {
            return;
        };
        let reads: Vec<trail::Mark> = self
            .trail
            .rows()
            .iter()
            .filter(|row| row.number > 0)
            .map(|row| row.mark)
            .collect();
        let mut next = 0;
        for (ix, entry) in entries.iter().enumerate().skip(start) {
            let Entry::Step(step) = entry else {
                continue;
            };
            if !is_ours(&step.tool) {
                continue;
            }
            if step.state == freshkube_agent::investigation::StepState::Failed
                && let Some(
                    mark
                    @ (trail::Mark::Refused | trail::Mark::NotFound | trail::Mark::CouldntRead),
                ) = reads.get(next)
            {
                self.step_marks.insert(ix, *mark);
            }
            next += 1;
        }
    }

    fn record(&mut self, event: Event, cx: &mut Context<Self>) {
        match &event {
            Event::Note(note) => {
                if let Some(what) = note.strip_prefix("Refused: ") {
                    self.trail.refused_tool(what);
                }
            }
            Event::StepStarted { .. } if self.stop_at_first_step => {
                self.stop_at_first_step = false;
                self.stop(cx);
            }
            Event::Ended(outcome) => {
                self.working = false;
                self.elapsed.update(cx, |elapsed, cx| elapsed.stop(cx));
                if let Outcome::Failed(reason) = outcome {
                    if signed_out(reason) {
                        if let setup::Setup::Ready { version } = &self.setup {
                            self.setup = setup::Setup::SignedOut {
                                version: version.clone(),
                            };
                            self.prepare_sign_in();
                        }
                    } else {
                        self.notice = Some(Notice::Failed(reason.clone()));
                    }
                }
            }
            _ => {}
        }
        let chunk = match &event {
            Event::Said(chunk) => Some(chunk.clone()),
            _ => None,
        };
        if matches!(event, Event::Prompt(_)) {
            self.trail.clear();
            self.cite_carry.clear();
            self.follow_new_turn(cx);
        }
        self.investigation.record(event);
        self.refresh_texts();
        if let Some(chunk) = chunk {
            // Only the new chunk and the tail before it: a citation is at
            // most nine bytes, so eight carry a split one.
            let scan = format!("{}{chunk}", self.cite_carry);
            self.trail.cite(&scan);
            self.follow_cited(&scan, cx);
            let mut from = scan.len().saturating_sub(CITE_CARRY);
            while !scan.is_char_boundary(from) {
                from += 1;
            }
            self.cite_carry = scan[from..].to_owned();
        }
    }

    /// Keeps `texts` equal to the transcript's entries: the last one is
    /// rebuilt, a new one added, and everything rebuilt only when text was
    /// trimmed off the front.
    fn refresh_texts(&mut self) {
        let entries = self.investigation.entries();
        let shown = |entry: &Entry| -> SharedString {
            match entry {
                Entry::Asked { text, .. } | Entry::Note(text) => text.clone().into(),
                Entry::Thought(text) | Entry::Said(text) => text.text.clone().into(),
                Entry::Step(_) => SharedString::default(),
            }
        };
        let len = self.texts.len();
        if self.investigation.trimmed() || entries.len() < len || entries.len() > len + 1 {
            self.texts = entries.iter().map(shown).collect();
        } else {
            if entries.len() == len + 1 {
                self.texts.push(SharedString::default());
            }
            if let (Some(entry), Some(last)) = (entries.last(), self.texts.last_mut()) {
                *last = shown(entry);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn end_session_for_test(&mut self, cx: &mut Context<Self>) {
        self.session_ended(None, cx);
    }

    #[cfg(test)]
    pub(crate) fn with_targets(&mut self, targets: std::sync::Arc<dyn freshkube_agent::Targets>) {
        self.targets = Some(targets);
    }

    #[cfg(test)]
    pub(crate) fn with_script(&mut self, script: Script) {
        self.script = Some(script);
    }

    /// What tests read of the session.
    #[cfg(test)]
    pub(crate) fn outcome(&self) -> Outcome {
        self.investigation.outcome()
    }

    #[cfg(test)]
    pub(crate) fn steps(&self) -> Vec<(String, freshkube_agent::investigation::StepState)> {
        self.investigation
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Step(step) => Some((step.id.clone(), step.state)),
                _ => None,
            })
            .collect()
    }

    /// Each question asked, and whether its turn failed.
    #[cfg(test)]
    pub(crate) fn questions(&self) -> Vec<(String, bool)> {
        self.investigation
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Asked { text, failed } => Some((text.clone(), *failed)),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn said(&self) -> String {
        self.investigation
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Said(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn trail_rows(&self) -> Vec<trail::Row> {
        self.trail.rows().to_vec()
    }

    #[cfg(test)]
    pub(crate) fn models(&self) -> Models {
        self.models.clone()
    }

    #[cfg(test)]
    pub(crate) fn has_session(&self) -> bool {
        self.session.is_some()
    }

    #[cfg(test)]
    pub(crate) fn listening(&self) -> Option<std::net::SocketAddr> {
        self.listening
    }

    #[cfg(test)]
    pub(crate) fn waiting_on_access(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(test)]
    pub(crate) fn is_working(&self) -> bool {
        self.working
    }

    #[cfg(test)]
    pub(crate) fn notice(&self) -> Option<Notice> {
        self.notice.clone()
    }

    #[cfg(test)]
    pub(crate) fn host(&self) -> AgentHost {
        self.host.clone()
    }
}

/// Whether a step's tool is one of Freshkube's four reads, under the names
/// the agents use for an MCP tool.
fn is_ours(tool: &str) -> bool {
    use freshkube_agent::tools::{SERVER, TOOLS};
    TOOLS.iter().any(|name| {
        [
            format!("{SERVER}_{name}"),
            format!("mcp__{SERVER}__{name}"),
            format!("mcp.{SERVER}.{name}"),
        ]
        .iter()
        .any(|ours| ours == tool)
    })
}

/// An entry of the setup other than `own`, for the example's read on another
/// cluster.
fn other_entry(workspace: Option<&Workspace>, own: &str) -> Option<String> {
    workspace?
        .clusters
        .iter()
        .find(|entry| entry.id != own)
        .map(|entry| entry.id.clone())
}

/// Whether a failure reads as opencode not being signed in to a provider.
fn signed_out(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    [
        "auth",
        "sign in",
        "sign-in",
        "log in",
        "login",
        "credential",
    ]
    .iter()
    .any(|word| reason.contains(word))
}

impl Focusable for AgentView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
