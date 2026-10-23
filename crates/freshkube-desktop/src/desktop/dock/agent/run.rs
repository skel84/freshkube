//! One agent session on Tokio: the loopback endpoint, the agent's ACP
//! session, and a loop that carries the person's commands in and what
//! happens out. The task owns the endpoint and the session, so dropping its
//! `OwnedJob` closes the port, ends the token and ends the agent and its
//! process group. Nothing here touches GPUI: the view receives [`Message`]s
//! on its own executor and checks their generation before it publishes.

use std::sync::Arc;

use freshkube_agent::acp::{AcpSession, Endpoint as AcpEndpoint, Models};
use freshkube_agent::investigation::Event;
use freshkube_agent::profile::{Checked, Host, Profile};
use freshkube_agent::scripted::{self, Script};
use freshkube_agent::{Endpoint, ReadEvent, SessionId, Targets};
use freshkube_core::agent_reads::AgentAccess;
use freshkube_core::job::OwnedJob;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// What starts the agent: the scripted one in process (`--fixture`), or
/// opencode, with the proof its version passed.
pub(crate) enum Agent {
    Scripted(Script),
    Opencode { profile: Profile, checked: Checked },
}

pub(crate) struct Launch {
    /// The session's own entry.
    pub(crate) entry: String,
    pub(crate) gate: AgentAccess,
    pub(crate) targets: Arc<dyn Targets>,
    pub(crate) agent: Agent,
}

#[derive(Debug)]
pub(crate) enum Command {
    Prompt(String),
    Cancel,
    SetModel(String),
}

#[derive(Debug)]
pub(crate) enum Message {
    /// What the agent reports of the turn.
    Event(Event),
    /// A read the endpoint answered or refused.
    Read(ReadEvent),
    /// The models on offer, and the one in use.
    Models(Models),
    /// The endpoint's port, for a test to see it close.
    Listening(std::net::SocketAddr),
    /// The session could not start, or has ended (the agent exited).
    Ended(Option<String>),
}

/// Starts a session on `runtime`. Messages come back on the receiver; the
/// commands go in on the sender. Dropping the job ends everything.
pub(crate) fn start(
    runtime: &Handle,
    launch: Launch,
) -> (
    OwnedJob,
    UnboundedSender<Command>,
    UnboundedReceiver<Message>,
) {
    let (messages, receiver) = unbounded_channel();
    let (commands, command_rx) = unbounded_channel();
    let job = OwnedJob::new(runtime.spawn(run(launch, messages, command_rx)));
    (job, commands, receiver)
}

async fn run(
    launch: Launch,
    messages: UnboundedSender<Message>,
    mut commands: UnboundedReceiver<Command>,
) {
    let (read_tx, mut reads) = unbounded_channel();
    let endpoint = match Endpoint::start(
        SessionId::new(),
        &launch.entry,
        &launch.gate,
        launch.targets,
        read_tx,
    )
    .await
    {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let _ = messages.send(Message::Ended(Some(format!(
                "The read endpoint didn't start: {error}"
            ))));
            return;
        }
    };
    let _ = messages.send(Message::Listening(endpoint.addr()));
    let link = match launch.agent {
        Agent::Scripted(script) => scripted::in_process(script),
        Agent::Opencode { profile, checked } => match profile.spawn(&Host::current(), &checked) {
            Ok(link) => link,
            Err(error) => {
                let _ = messages.send(Message::Ended(Some(format!(
                    "opencode didn't start: {error}"
                ))));
                return;
            }
        },
    };
    let (session, mut events) = AcpSession::start(
        link,
        AcpEndpoint::new(endpoint.url(), endpoint.token().reveal()),
    );
    let mut models = session.models();
    loop {
        tokio::select! {
            Some(command) = commands.recv() => match command {
                Command::Prompt(text) => {
                    if session.prompt(text).is_err() {
                        break;
                    }
                }
                Command::Cancel => session.cancel(),
                Command::SetModel(id) => {
                    let _ = session.set_model(&id);
                }
            },
            event = events.recv() => {
                // The agent exited or closed its output: the session ends.
                let Some(event) = event else { break };
                if messages.send(Message::Event(event)).is_err() {
                    break;
                }
            }
            Some(read) = reads.recv() => {
                if messages.send(Message::Read(read)).is_err() {
                    break;
                }
            }
            Ok(()) = models.changed() => {
                let _ = messages.send(Message::Models(models.borrow().clone()));
            }
            else => break,
        }
    }
    drop(session);
    drop(endpoint);
    let _ = messages.send(Message::Ended(None));
}
