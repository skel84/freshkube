//! `cargo run -- --faux` replays a scripted investigation offline.
//! `ANTHROPIC_API_KEY=… cargo run` runs a real model against the canned cluster.
//!
//! Options: --model <id>, --thinking <off|low|medium|high|xhigh|max>,
//! --prompt <text>, --abort-after-ms <n>, --steer-after-ms <n> --steer <text>.

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use freshkube_agent_spike::investigation::{Investigation, Outcome};
use freshkube_agent_spike::scenario::Cluster;
use freshkube_agent_spike::script;
use freshkube_agent_spike::session::{Backend, Session, Update};
use rpi_agent::{AgentEvent, AgentMessage};
use rpi_ai::Provider;
use rpi_ai::providers::anthropic::{AnthropicProvider, models};
use rpi_ai::types::{AssistantMessageEvent, ThinkingLevel, Usage};

const DEFAULT_PROMPT: &str =
    "checkout in namespace shop went degraded after its last rollout. Find out why.";

struct Options {
    faux: bool,
    model: String,
    thinking: ThinkingLevel,
    prompt: String,
    abort_after: Option<Duration>,
    steer_after: Option<Duration>,
    steer: Option<String>,
}

fn options() -> Options {
    let mut o = Options {
        faux: false,
        model: "claude-opus-5-5".into(),
        thinking: ThinkingLevel::Medium,
        prompt: DEFAULT_PROMPT.into(),
        abort_after: None,
        steer_after: None,
        steer: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .unwrap_or_else(|| panic!("{flag} needs a value"))
        };
        let ms = |v: String| Duration::from_millis(v.parse().expect("milliseconds"));
        match flag.as_str() {
            "--faux" => o.faux = true,
            "--model" => o.model = value(),
            "--prompt" => o.prompt = value(),
            "--abort-after-ms" => o.abort_after = Some(ms(value())),
            "--steer-after-ms" => o.steer_after = Some(ms(value())),
            "--steer" => o.steer = Some(value()),
            "--thinking" => {
                o.thinking = match value().as_str() {
                    "off" => ThinkingLevel::Off,
                    "low" => ThinkingLevel::Low,
                    "medium" => ThinkingLevel::Medium,
                    "high" => ThinkingLevel::High,
                    "xhigh" => ThinkingLevel::Xhigh,
                    "max" => ThinkingLevel::Max,
                    other => panic!("unknown thinking level {other}"),
                }
            }
            other => panic!("unknown option {other}"),
        }
    }
    o
}

fn backend(o: &Options) -> Backend {
    if o.faux {
        let provider = script::provider(Some(80.0));
        let model = provider.default_model().clone();
        return Backend::Provider {
            provider,
            model,
            thinking: o.thinking,
        };
    }
    let key = std::env::var("ANTHROPIC_API_KEY")
        .expect("set ANTHROPIC_API_KEY for a live run, or pass --faux");
    // rpi's catalogue stops at the Claude 5.0 ids; newer ones reuse the nearest entry.
    let mut model = if o.model.contains("sonnet") {
        models::claude_sonnet_5()
    } else {
        models::claude_opus_5()
    };
    model.id = o.model.clone();
    model.name = o.model.clone();
    let provider: Arc<dyn Provider> = Arc::new(AnthropicProvider::with_models_without_env_api_key(
        Some(key),
        reqwest::Client::new(),
        vec![model.clone()],
    ));
    Backend::Provider {
        provider,
        model,
        thinking: o.thinking,
    }
}

#[derive(Default)]
struct Metrics {
    first_thought: Option<Duration>,
    first_text: Option<Duration>,
    first_tool: Option<Duration>,
    tools: usize,
    failed_tools: usize,
    navigations: usize,
    deltas: usize,
    usage: Vec<Usage>,
    cancelled_at: Option<Instant>,
    cancel_latency: Option<Duration>,
}

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

#[tokio::main]
async fn main() {
    let o = options();
    let cluster = Arc::new(Cluster::checkout_crash());
    let started = Instant::now();
    let (session, mut rx) = Session::start(backend(&o), cluster, o.prompt.clone());
    println!(
        "{DIM}model {}  thinking {:?}{RESET}\n▶ {}",
        if o.faux { "faux" } else { &o.model },
        o.thinking,
        o.prompt
    );

    let mut inv = Investigation::default();
    let mut m = Metrics::default();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let abort_at = o.abort_after.map(|d| tokio::time::Instant::now() + d);
    let steer_at = o.steer_after.map(|d| tokio::time::Instant::now() + d);
    let mut steered = false;
    let t = |at: Instant| format!("{:>6}", at.duration_since(started).as_millis());

    loop {
        tokio::select! {
            update = rx.recv() => {
                let Some(update) = update else { break };
                let now = Instant::now();
                let finished = matches!(update, Update::Finished(_));
                if finished && let Some(at) = m.cancelled_at {
                    m.cancel_latency = Some(now.duration_since(at));
                }
                print_update(&update, &t(now), &mut m, started, now);
                if let Some(place) = inv.apply(update, now) {
                    m.navigations += 1;
                    println!("\n{} ⇢ view: {place:?}", t(now));
                }
                if finished { break }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                if let Some(place) = inv.tick(now) {
                    m.navigations += 1;
                    println!("\n{} ⇢ view: {place:?} (held)", t(now));
                }
                if abort_at.is_some_and(|at| tokio::time::Instant::now() >= at) && m.cancelled_at.is_none() {
                    m.cancelled_at = Some(now);
                    println!("\n{} ■ cancel", t(now));
                    session.cancel();
                }
                if !steered && steer_at.is_some_and(|at| tokio::time::Instant::now() >= at) {
                    steered = true;
                    let text = o.steer.clone().unwrap_or_else(|| "Also check the database side.".into());
                    println!("\n{} ✎ steer: {text}", t(now));
                    session.steer(text);
                }
            }
        }
    }

    let ms = |d: Option<Duration>| d.map_or("-".into(), |d| format!("{} ms", d.as_millis()));
    let cost: f64 = m.usage.iter().map(|u| u.cost.total).sum();
    let input: i64 = m
        .usage
        .iter()
        .map(|u| u.input + u.cache_read + u.cache_write)
        .sum();
    let output: i64 = m.usage.iter().map(|u| u.output).sum();
    println!("\n\n── outcome {:?}", inv.outcome);
    println!(
        "   first thought {}, first text {}, first tool {}",
        ms(m.first_thought),
        ms(m.first_text),
        ms(m.first_tool)
    );
    println!(
        "   {} tool calls ({} failed), {} view moves, {} stream deltas",
        m.tools, m.failed_tools, m.navigations, m.deltas
    );
    println!(
        "   {} model calls, {input} input / {output} output tokens, ${cost:.4}",
        m.usage.len()
    );
    println!(
        "   total {} ms, cancel → finished {}",
        started.elapsed().as_millis(),
        ms(m.cancel_latency)
    );
    if inv.outcome == Outcome::Running {
        println!("   (finished without an answer)");
    }
}

fn print_update(update: &Update, t: &str, m: &mut Metrics, started: Instant, now: Instant) {
    let since = Some(now.duration_since(started));
    let mut out = std::io::stdout().lock();
    match update {
        Update::Agent(event) => match event.as_ref() {
            AgentEvent::MessageUpdate {
                assistant_message_event: e,
                ..
            } => match e {
                AssistantMessageEvent::ThinkingStart { .. } => {
                    m.first_thought = m.first_thought.or(since);
                    let _ = write!(out, "\n{t} 💭 {DIM}");
                }
                AssistantMessageEvent::ThinkingDelta { delta, .. } => {
                    m.deltas += 1;
                    let _ = write!(out, "{}", delta.replace('\n', " "));
                }
                AssistantMessageEvent::ThinkingEnd { .. } => {
                    let _ = write!(out, "{RESET}");
                }
                AssistantMessageEvent::TextStart { .. } => {
                    m.first_text = m.first_text.or(since);
                    let _ = write!(out, "\n{t} ▸ ");
                }
                AssistantMessageEvent::TextDelta { delta, .. } => {
                    m.deltas += 1;
                    let _ = write!(out, "{delta}");
                }
                AssistantMessageEvent::Error { error, .. } => {
                    let _ = write!(
                        out,
                        "\n{t} ✗ {:?} {}",
                        error.stop_reason,
                        error.error_message.clone().unwrap_or_default()
                    );
                }
                _ => {}
            },
            AgentEvent::MessageEnd {
                message: AgentMessage::Assistant(message),
            } => {
                m.usage.push(message.usage.clone());
            }
            AgentEvent::ToolExecutionStart {
                tool_name, args, ..
            } => {
                m.first_tool = m.first_tool.or(since);
                m.tools += 1;
                let _ = write!(out, "\n{t} ⚙ {tool_name} {args}");
            }
            AgentEvent::ToolExecutionEnd {
                is_error: true,
                result,
                ..
            } => {
                m.failed_tools += 1;
                let text: Vec<String> = result.content.iter().map(|c| format!("{c:?}")).collect();
                let _ = write!(out, "\n{t} ✗ tool failed: {}", text.join(" "));
            }
            AgentEvent::RetryScheduled { attempt, error, .. } => {
                let _ = write!(out, "\n{t} ↻ retry {attempt}: {error}");
            }
            _ => {}
        },
        Update::Follow { .. } => {}
        Update::Finished(Err(e)) => {
            let _ = write!(out, "\n{t} ✗ {e}");
        }
        Update::Finished(Ok(())) => {}
    }
    let _ = out.flush();
}
