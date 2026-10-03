//! The embedding the app would own: one investigation on Tokio, its updates on
//! an unbounded channel the UI drains, steering and cancellation from outside,
//! and cancellation on drop like `OwnedJob`.
//!
//! It drives `run_agent_loop` itself rather than `rpi_agent::Agent`, whose
//! events go through a 256-slot broadcast channel that drops events for a
//! receiver that falls behind.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rpi_agent::{
    AgentContext, AgentEmitter, AgentEvent, AgentLoopConfig, AgentMessage, StreamFn, run_agent_loop,
};
use rpi_ai::event_stream::create_assistant_message_event_stream;
use rpi_ai::provider::CacheRetention;
use rpi_ai::types::{ThinkingLevel, UserContent, UserMessage};
use rpi_ai::{Model, Provider};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use crate::scenario::Cluster;
use crate::tools::{self, Place};

pub const SYSTEM_PROMPT: &str = "\
You are the investigation assistant inside Freshkube, a desktop app for Kubernetes and Talos clusters. \
The operator watches the app follow your tool calls, so read the objects that matter rather than guessing.

Rules:
- Your tools are read-only. You cannot change the cluster; recommend changes for the operator to make.
- Prefer current state (spec, status, conditions) over logs. Use logs and events to explain, not to decide health.
- Distinguish what an object reports, what you conclude, and what you suspect.
- Name every object as Kind namespace/name.
- Finish with: Cause, Evidence (one line per object), and Suggested fix. Keep it short.";

pub enum Update {
    Agent(Box<AgentEvent>),
    Follow { tool_call_id: String, place: Place },
    Finished(Result<(), String>),
}

pub enum Backend {
    Provider {
        provider: Arc<dyn Provider>,
        model: Model,
        thinking: ThinkingLevel,
    },
}

pub struct Session {
    cancel: CancellationToken,
    steering: Arc<Mutex<Vec<AgentMessage>>>,
}

impl Session {
    pub fn start(
        backend: Backend,
        cluster: Arc<Cluster>,
        prompt: impl Into<String>,
    ) -> (Self, UnboundedReceiver<Update>) {
        let (tx, rx) = unbounded_channel();
        let cancel = CancellationToken::new();
        let steering: Arc<Mutex<Vec<AgentMessage>>> = Arc::default();

        let follow_tx = tx.clone();
        let follow: tools::Follow = Arc::new(move |id, place| {
            let _ = follow_tx.send(Update::Follow {
                tool_call_id: id.into(),
                place,
            });
        });
        let Backend::Provider {
            provider,
            model,
            thinking,
        } = backend;

        let context = AgentContext {
            system_prompt: SYSTEM_PROMPT.into(),
            messages: Vec::new(),
            tools: tools::all(cluster, follow),
        };
        let queue = Arc::clone(&steering);
        let config = AgentLoopConfig {
            model,
            convert_to_llm: rpi_agent::hooks::default_convert_to_llm_fn(),
            transform_context: None,
            get_api_key: None,
            should_stop_after_turn: None,
            prepare_next_turn: None,
            after_tool_results: None,
            get_steering_messages: Some(Arc::new(move || {
                let drained = std::mem::take(&mut *queue.lock().unwrap());
                Box::pin(async move { drained })
            })),
            get_follow_up_messages: None,
            before_tool_call: None,
            after_tool_call: None,
            tool_execution: Default::default(),
            thinking_level: thinking,
            api_key: None,
            timeout: Some(Duration::from_secs(120)),
            max_retries: Some(2),
            max_retry_delay: None,
            cache_retention: CacheRetention::Short,
            session_id: None,
            signal: cancel.clone(),
        };
        let emit: Arc<dyn AgentEmitter> = Arc::new(ChannelEmitter(tx.clone()));
        let prompt = vec![user(prompt.into())];

        tokio::spawn(async move {
            let outcome = run_agent_loop(prompt, context, config, emit, stream_fn(provider))
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(Update::Finished(outcome));
        });

        (Self { cancel, steering }, rx)
    }

    /// Delivered before the agent's next model call, after any tools running now.
    pub fn steer(&self, text: impl Into<String>) {
        self.steering.lock().unwrap().push(user(text.into()));
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn user(text: String) -> AgentMessage {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();
    AgentMessage::User(UserMessage::new(UserContent::Text(text), now))
}

struct ChannelEmitter(UnboundedSender<Update>);

impl AgentEmitter for ChannelEmitter {
    fn emit(&self, event: AgentEvent) -> futures::future::BoxFuture<'static, ()> {
        let _ = self.0.send(Update::Agent(Box::new(event)));
        Box::pin(async {})
    }

    fn try_emit(&self, event: AgentEvent) {
        let _ = self.0.send(Update::Agent(Box::new(event)));
    }
}

/// `StreamFn` returns its stream synchronously while providers are async, so
/// spawn the provider and forward its events, as rpi's own tests do.
fn stream_fn(provider: Arc<dyn Provider>) -> StreamFn {
    rpi_agent::stream_fn(move |model, ctx, opts| {
        let (mut producer, stream) = create_assistant_message_event_stream();
        let provider = Arc::clone(&provider);
        let (model, ctx, opts) = (model.clone(), ctx.clone(), opts.clone());
        tokio::spawn(async move {
            let mut upstream = provider.stream_simple(&model, &ctx, &opts).await;
            while let Some(event) = upstream.next().await {
                if !producer.push(event) {
                    break;
                }
            }
        });
        stream
    })
}
