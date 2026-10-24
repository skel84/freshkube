//! The ACP client: the person's agent runs as a child process speaking ACP
//! over stdio, and reads the cluster only through Freshkube's MCP endpoint.
//!
//! The client takes the endpoint as a URL and a token. The token goes only
//! into `session/new`, over the child's stdin: never into the argument list,
//! the environment, a file, a log or `Debug`. The client advertises no file
//! system and no terminal, so the agent has none to ask for, and it refuses
//! every permission request that isn't one of Freshkube's four reads
//! ([`permission`]).
//!
//! Bounds: startup takes at most [`STARTUP_TIMEOUT`]; a cancelled prompt gets
//! [`CANCEL_GRACE`] to answer before the agent is ended; a line from the agent
//! is at most 4 MiB; the transcript bounds are in [`crate::investigation`].

mod lines;
mod permission;
mod updates;

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, HttpHeader, InitializeRequest, McpServer, McpServerHttp,
    NewSessionRequest, PromptRequest, RequestPermissionRequest, RequestPermissionResponse,
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigOptionValue,
    SessionConfigSelectOptions, SessionId, SessionNotification, SetSessionConfigOptionRequest,
    StopReason, TextContent,
};
use agent_client_protocol::{ByteStreams, Client, ConnectionTo};
use tokio::sync::mpsc::{
    Receiver, Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel,
};
use tokio::sync::watch;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use self::lines::{LineLimit, MAX_LINE_BYTES};
use self::permission::{Calls, decide};
use crate::child::{Guard, Link};
use crate::investigation::{Event, Outcome, clip};
use crate::tools::SERVER;

/// How long the agent has to initialize and open a session.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a cancelled prompt has to answer before the agent is ended.
pub const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// Events waiting for the reader. The agent is slowed to the reader's pace
/// rather than buffered without end, so the reader must drain them.
pub const EVENT_BUFFER: usize = 1024;

const MODEL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MODELS: usize = 1_000;
const MAX_MODEL_ID_BYTES: usize = 200;
const MAX_REASON_BYTES: usize = 300;

/// Providers whose models run on the person's machine.
const LOCAL_PROVIDERS: [&str; 5] = ["ollama", "lmstudio", "llama.cpp", "llamacpp", "local"];

/// Where Freshkube's tools are served, and the session's token for them.
#[derive(Clone)]
pub struct Endpoint {
    url: String,
    token: Token,
}

#[derive(Clone)]
struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Endpoint")
            .field("url", &self.url)
            .field("token", &self.token)
            .finish()
    }
}

impl Endpoint {
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: Token(token.into()),
        }
    }
}

/// The models the agent offers and the one it uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Models {
    pub choices: Vec<ModelChoice>,
    pub current: Option<String>,
    /// The saved model the agent didn't offer; the agent's default is in use.
    pub missing: Option<String>,
    /// The agent's own default model is one OpenCode may refuse outside its
    /// own app, and the person hasn't chosen it, so they pick one before the
    /// first question.
    pub must_pick: bool,
}

/// Models of OpenCode's own provider. The free ones work only inside
/// OpenCode's app, and opencode's ACP agent sends no cost or tier with a
/// model (`packages/cli/src/acp/config-option.ts`: `value` and `name` only),
/// so the paid ones can't be told apart. They are offered, marked, and never
/// the default.
pub fn is_app_only(id: &str) -> bool {
    id.split('/')
        .next()
        .is_some_and(|provider| provider.eq_ignore_ascii_case("opencode"))
}

/// An error's text without the session's token or an `Authorization` value.
fn redacted(text: &str, token: &str) -> String {
    let mut out = if token.is_empty() {
        text.to_string()
    } else {
        text.replace(token, "…")
    };
    for marker in ["Bearer ", "bearer ", "Authorization: ", "authorization: "] {
        while let Some(at) = out.find(marker) {
            let start = at + marker.len();
            let end = out[start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}'))
                .map_or(out.len(), |n| start + n);
            out.replace_range(at..end, "…");
        }
    }
    clip(&out, MAX_REASON_BYTES).to_string()
}

/// The connection to the agent failed after it started.
fn connection_failure(text: &str, token: &str) -> Failure {
    Failure {
        starting: false,
        why: redacted(text, token),
    }
}

/// The agent's own default is an OpenCode model, and none is chosen.
const PICK_A_MODEL: &str = "Pick a model first. The agent's default model is one of OpenCode's, and its free models work only inside OpenCode.";

/// Added to a failure of a turn that ran on an OpenCode model: a free one is
/// refused outside OpenCode's app, and the error says so in the provider's
/// words, if at all.
const APP_ONLY_HINT: &str = "OpenCode's free models only work inside OpenCode. Pick another model in the model picker, or one of OpenCode's you pay for.";

/// What a turn says when the agent gave neither an answer nor a message.
const EMPTY_TURN: &str =
    "The agent ended the turn without an answer. Check that the chosen model is signed in.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    /// What `set_model` takes; opencode's is `provider/model`.
    pub id: String,
    pub name: String,
    /// Runs on the person's machine, so nothing leaves it.
    pub local: bool,
    /// One of OpenCode's own models: a free one is refused outside OpenCode's
    /// app. See [`is_app_only`].
    pub app_only: bool,
}

/// The agent's model option, as session setup reports it.
struct ModelOption {
    config_id: String,
    models: Models,
}

impl ModelOption {
    fn find(options: &[SessionConfigOption]) -> Option<Self> {
        let option = options
            .iter()
            .find(|o| o.category == Some(SessionConfigOptionCategory::Model))?;
        let SessionConfigKind::Select(select) = &option.kind else {
            return None;
        };
        let flat: Vec<_> = match &select.options {
            SessionConfigSelectOptions::Ungrouped(options) => options.iter().collect(),
            SessionConfigSelectOptions::Grouped(groups) => {
                groups.iter().flat_map(|g| g.options.iter()).collect()
            }
            _ => Vec::new(),
        };
        let choices = flat
            .into_iter()
            .take(MAX_MODELS)
            .map(|choice| {
                let id = clip(&choice.value.to_string(), MAX_MODEL_ID_BYTES).to_string();
                let provider = id
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                ModelChoice {
                    name: clip(&choice.name, MAX_MODEL_ID_BYTES).to_string(),
                    local: LOCAL_PROVIDERS.contains(&provider.as_str()),
                    app_only: is_app_only(&id),
                    id,
                }
            })
            .collect();
        let current = clip(&select.current_value.to_string(), MAX_MODEL_ID_BYTES).to_string();
        Some(Self {
            config_id: option.id.to_string(),
            models: Models {
                choices,
                must_pick: is_app_only(&current),
                current: Some(current),
                missing: None,
            },
        })
    }
}

/// The session has ended; nothing more can be asked of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

/// The model asked for isn't one the agent offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownModel;

enum Command {
    Prompt(String),
    Cancel,
    SetModel(String),
}

/// One agent session. Several prompts can run in it, one after another;
/// dropping it ends the agent and everything it started.
pub struct AcpSession {
    commands: UnboundedSender<Command>,
    models: watch::Receiver<Models>,
    guard: Guard,
}

impl AcpSession {
    /// Starts the session on the current Tokio runtime. What happens comes
    /// back as events; a session that can't start, or that dies in a prompt,
    /// ends that turn as failed.
    pub fn start(link: Link, endpoint: Endpoint) -> (Self, Receiver<Event>) {
        Self::start_with(link, endpoint, None)
    }

    /// Like [`AcpSession::start`], and applies `model` right after
    /// `session/new`, before any prompt. A model the agent doesn't offer is
    /// left out and named in [`Models::missing`].
    pub fn start_with(
        link: Link,
        endpoint: Endpoint,
        model: Option<String>,
    ) -> (Self, Receiver<Event>) {
        let (events, rx) = channel(EVENT_BUFFER);
        let (commands, command_rx) = unbounded_channel();
        let (models_tx, models) = watch::channel(Models::default());
        let guard = link.guard.clone();
        let turn = Arc::new(AtomicBool::new(false));
        let run = Run {
            endpoint,
            events: events.clone(),
            commands: command_rx,
            models: models_tx,
            guard: guard.clone(),
            turn: Arc::clone(&turn),
            output: Arc::new(AtomicBool::new(false)),
            model,
            name: link.name,
        };
        tokio::spawn(async move {
            let name = run.name;
            let ended = run.guard.clone();
            let failure = run.run(link).await;
            // An agent the session ended itself says nothing more.
            if let Some(reason) = failure.filter(|_| !ended.is_ended()) {
                // Startup failed, or the agent died: a turn that was running
                // ends as failed. Anything else ended quietly.
                if reason.starting || turn.swap(false, Ordering::SeqCst) {
                    let _ = events
                        .send(Event::Ended(Outcome::failed(reason.text(name))))
                        .await;
                }
            }
        });
        (
            Self {
                commands,
                models,
                guard,
            },
            rx,
        )
    }

    /// Asks the agent something. Prompts run one after another.
    pub fn prompt(&self, text: impl Into<String>) -> Result<(), Closed> {
        self.commands
            .send(Command::Prompt(text.into()))
            .map_err(|_| Closed)
    }

    /// Stops the running prompt and drops the ones waiting. The agent has
    /// [`CANCEL_GRACE`] to answer; after that it is ended.
    pub fn cancel(&self) {
        let _ = self.commands.send(Command::Cancel);
    }

    /// Chooses one of the models the agent offered.
    pub fn set_model(&self, id: &str) -> Result<(), UnknownModel> {
        if !self.models.borrow().choices.iter().any(|m| m.id == id) {
            return Err(UnknownModel);
        }
        let _ = self.commands.send(Command::SetModel(id.to_string()));
        Ok(())
    }

    /// The models on offer; it changes when the choice does.
    pub fn models(&self) -> watch::Receiver<Models> {
        self.models.clone()
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        self.guard.end();
    }
}

/// Why a session ended before it was asked to.
struct Failure {
    starting: bool,
    why: String,
}

impl Failure {
    fn text(&self, name: &str) -> String {
        if self.starting {
            format!("{name} didn't start")
        } else {
            format!("{name} stopped: {}", self.why)
        }
    }
}

struct Run {
    endpoint: Endpoint,
    events: Sender<Event>,
    commands: UnboundedReceiver<Command>,
    models: watch::Sender<Models>,
    guard: Guard,
    turn: Arc<AtomicBool>,
    /// The turn showed text or a tool call.
    output: Arc<AtomicBool>,
    /// The model to apply before the first prompt.
    model: Option<String>,
    name: &'static str,
}

enum Exit {
    /// The session was dropped, or its agent was ended.
    Closed,
}

impl Run {
    async fn run(self, link: Link) -> Option<Failure> {
        let Link {
            reader,
            writer,
            workspace,
            ..
        } = link;
        let token = self.endpoint.token.0.clone();
        let calls = Arc::new(Calls::default());
        let seen = Arc::clone(&calls);
        let updates = self.events.clone();
        let output = Arc::clone(&self.output);
        let refusals = self.events.clone();
        let transport = ByteStreams::new(
            writer.compat_write(),
            LineLimit::new(reader, MAX_LINE_BYTES).compat(),
        );
        let result = Client
            .builder()
            .name("freshkube")
            .on_receive_notification(
                async move |notification: SessionNotification, _cx| {
                    seen.note(&notification.update);
                    for event in updates::events(notification.update) {
                        if matches!(&event, Event::Said(text) if !text.trim().is_empty())
                            || matches!(event, Event::StepStarted { .. })
                        {
                            output.store(true, Ordering::SeqCst);
                        }
                        let _ = updates.send(event).await;
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, _cx| {
                    let (outcome, refused) = decide(&request, &calls);
                    if let Some(what) = refused {
                        let name = calls.name_of(&request);
                        let _ = refusals.send(Event::Refused { name }).await;
                        let _ = refusals.send(Event::Note(format!("Refused: {what}"))).await;
                    }
                    responder.respond(RequestPermissionResponse::new(outcome))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_with(
                transport,
                async move |cx: ConnectionTo<agent_client_protocol::Agent>| {
                    Ok(self.session(cx, workspace).await)
                },
            )
            .await;
        match result {
            Ok(Ok(Exit::Closed)) => None,
            Ok(Err(failure)) => Some(failure),
            Err(error) => Some(connection_failure(&error.to_string(), &token)),
        }
    }

    async fn session(
        mut self,
        cx: ConnectionTo<agent_client_protocol::Agent>,
        workspace: std::path::PathBuf,
    ) -> Result<Exit, Failure> {
        fn starting<E>(_: E) -> Failure {
            Failure {
                starting: true,
                why: String::new(),
            }
        }
        let open = async {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            let server =
                McpServerHttp::new(SERVER, &self.endpoint.url).headers(vec![HttpHeader::new(
                    "Authorization",
                    format!("Bearer {}", self.endpoint.token.0),
                )]);
            cx.send_request(
                NewSessionRequest::new(&workspace).mcp_servers(vec![McpServer::Http(server)]),
            )
            .block_task()
            .await
        };
        let created = tokio::time::timeout(STARTUP_TIMEOUT, open)
            .await
            .map_err(starting)?
            .map_err(starting)?;
        let mut option = created
            .config_options
            .as_deref()
            .and_then(ModelOption::find);
        if let Some(option) = &option {
            self.models.send_replace(option.models.clone());
        }
        let session = created.session_id;
        if let Some(wanted) = self.model.take() {
            self.apply_saved(&cx, &session, &mut option, wanted).await;
        }

        let mut pending = VecDeque::new();
        loop {
            let command = match pending.pop_front() {
                Some(command) => command,
                None => match self.commands.recv().await {
                    Some(command) => command,
                    None => return Ok(Exit::Closed),
                },
            };
            match command {
                Command::Cancel => pending.clear(),
                Command::SetModel(id) => self.set_model(&cx, &session, &mut option, id).await,
                Command::Prompt(text) if option.as_ref().is_some_and(|o| o.models.must_pick) => {
                    let _ = self.events.send(Event::Prompt(text)).await;
                    self.turn.store(true, Ordering::SeqCst);
                    self.finish(Outcome::failed(PICK_A_MODEL)).await;
                }
                Command::Prompt(text) => {
                    if !self.prompt(&cx, &session, text, &mut pending).await {
                        return Ok(Exit::Closed);
                    }
                }
            }
        }
    }

    /// Applies the saved model, and finishes before the first prompt is read.
    async fn apply_saved(
        &self,
        cx: &ConnectionTo<agent_client_protocol::Agent>,
        session: &SessionId,
        option: &mut Option<ModelOption>,
        wanted: String,
    ) {
        let Some(current) = option else { return };
        if current.models.current.as_deref() == Some(wanted.as_str()) {
            current.models.must_pick = false;
            self.models.send_replace(current.models.clone());
            return;
        }
        if !current.models.choices.iter().any(|m| m.id == wanted) {
            current.models.missing = Some(wanted);
            self.models.send_replace(current.models.clone());
            return;
        }
        self.set_model(cx, session, option, wanted).await;
    }

    async fn set_model(
        &self,
        cx: &ConnectionTo<agent_client_protocol::Agent>,
        session: &SessionId,
        option: &mut Option<ModelOption>,
        id: String,
    ) {
        let Some(current) = option else {
            let _ = self
                .events
                .send(Event::Note(
                    "This agent doesn't offer a choice of model".into(),
                ))
                .await;
            return;
        };
        let request = SetSessionConfigOptionRequest::new(
            session.clone(),
            current.config_id.clone(),
            SessionConfigOptionValue::value_id(id),
        );
        match tokio::time::timeout(MODEL_TIMEOUT, cx.send_request(request).block_task()).await {
            Ok(Ok(response)) => {
                if let Some(mut updated) = ModelOption::find(&response.config_options) {
                    // The person chose this one, even if it is OpenCode's.
                    updated.models.must_pick = false;
                    self.models.send_replace(updated.models.clone());
                    *current = updated;
                }
            }
            _ => {
                let _ = self
                    .events
                    .send(Event::Note("The model wasn't changed".into()))
                    .await;
            }
        }
    }

    /// Runs one prompt. False when the session can't go on: the agent died,
    /// was ended, or the session was dropped.
    async fn prompt(
        &mut self,
        cx: &ConnectionTo<agent_client_protocol::Agent>,
        session: &SessionId,
        text: String,
        pending: &mut VecDeque<Command>,
    ) -> bool {
        let _ = self.events.send(Event::Prompt(text.clone())).await;
        self.turn.store(true, Ordering::SeqCst);
        self.output.store(false, Ordering::SeqCst);
        let answer = cx
            .send_request(PromptRequest::new(
                session.clone(),
                vec![ContentBlock::Text(TextContent::new(text))],
            ))
            .block_task();
        tokio::pin!(answer);
        let mut grace: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
        let response = loop {
            tokio::select! {
                response = &mut answer => break response,
                command = self.commands.recv(), if grace.is_none() => match command {
                    Some(Command::Cancel) => {
                        pending.clear();
                        let _ = cx.send_notification(CancelNotification::new(session.clone()));
                        grace = Some(Box::pin(tokio::time::sleep(CANCEL_GRACE)));
                    }
                    Some(other) => pending.push_back(other),
                    None => return false,
                },
                () = async { grace.as_mut().expect("guarded").await }, if grace.is_some() => {
                    self.guard.end();
                    self.finish(Outcome::Cancelled).await;
                    return false;
                }
            }
        };
        match response {
            Ok(response) => {
                self.finish(match response.stop_reason {
                    StopReason::EndTurn if !self.output.load(Ordering::SeqCst) => {
                        Outcome::failed(EMPTY_TURN)
                    }
                    StopReason::EndTurn => Outcome::Answered,
                    StopReason::Cancelled => Outcome::Cancelled,
                    other => Outcome::failed(format!("stopped: {other:?}")),
                })
                .await;
                true
            }
            Err(error) => {
                let why = redacted(&error.to_string(), &self.endpoint.token.0);
                let hint = if self.on_app_only_model() {
                    format!(" {APP_ONLY_HINT}")
                } else {
                    String::new()
                };
                self.finish(Outcome::failed(format!(
                    "{} stopped: {why}{hint}",
                    self.name
                )))
                .await;
                false
            }
        }
    }

    /// The model in use is one of OpenCode's.
    fn on_app_only_model(&self) -> bool {
        self.models
            .borrow()
            .current
            .as_deref()
            .is_some_and(is_app_only)
    }

    /// Ends the turn once, whoever gets there first.
    async fn finish(&self, outcome: Outcome) {
        if self.turn.swap(false, Ordering::SeqCst) {
            let _ = self.events.send(Event::Ended(outcome)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_failure_carries_neither_the_token_nor_an_authorization_value() {
        let failure = connection_failure(
            "transport closed: Authorization: Bearer secret-token, retry with secret-token",
            "secret-token",
        );
        assert!(!failure.starting);
        assert!(!failure.why.contains("secret-token"), "{}", failure.why);
        assert!(!failure.why.contains("Bearer secret"), "{}", failure.why);
    }

    #[test]
    fn only_opencodes_own_provider_is_app_only() {
        assert!(is_app_only("opencode/big-pickle"));
        assert!(is_app_only("OpenCode/x"));
        assert!(!is_app_only("opencode-zen/x"));
        assert!(!is_app_only("hosted/large"));
    }
}
