//! `cargo run --bin acp -- --agent claude` runs Claude Code (your login) over
//! ACP against the canned cluster; `--agent codex` runs Codex (your ChatGPT
//! login). Neither reaches a real cluster: their only tools are ours, over MCP.
//!
//! Options: --prompt <text>, --abort-after-ms <n>, --trace <file> (every raw
//! session update, one per line).

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use freshkube_agent_spike::acp::{AcpSession, AgentKind};
use freshkube_agent_spike::investigation::{Entry, Event, Investigation, Outcome};
use freshkube_agent_spike::scenario::Cluster;

const DEFAULT_PROMPT: &str =
    "checkout in namespace shop went degraded after its last rollout. Find out why.";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

#[tokio::main]
async fn main() {
    let mut agent = None;
    let mut prompt = DEFAULT_PROMPT.to_string();
    let mut abort_after = None;
    let mut trace_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .unwrap_or_else(|| panic!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--agent" => {
                agent = Some(match value().as_str() {
                    "claude" => AgentKind::Claude,
                    "codex" => AgentKind::Codex,
                    other => panic!("unknown agent {other}; use claude or codex"),
                })
            }
            "--prompt" => prompt = value(),
            "--abort-after-ms" => {
                abort_after = Some(Duration::from_millis(value().parse().expect("ms")))
            }
            "--trace" => trace_path = Some(value()),
            other => panic!("unknown option {other}"),
        }
    }
    let agent = agent.expect("--agent claude|codex");

    let trace = trace_path.map(|p| Mutex::new(std::fs::File::create(p).expect("trace file")));
    let started = Instant::now();
    let (session, mut rx) = AcpSession::start_with(
        agent,
        Arc::new(Cluster::checkout_crash()),
        prompt.clone(),
        move |update| {
            if let Some(file) = &trace {
                let ms = started.elapsed().as_millis();
                let _ = writeln!(file.lock().unwrap(), "{ms:>6} {update:?}");
            }
        },
    );
    println!("▶ {prompt}");

    let mut inv = Investigation::default();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let abort_at = abort_after.map(|d| started + d);
    let mut cancelled_at: Option<Instant> = None;
    let (mut first_thought, mut first_said, mut first_step) = (None, None, None);
    let (mut thought_chunks, mut moves) = (0usize, 0usize);
    let t = |at: Instant| format!("{:>6}", at.duration_since(started).as_millis());
    let mut out = std::io::stdout();

    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else { break };
                let now = Instant::now();
                let since = Some(now.duration_since(started));
                let ended = matches!(event, Event::Ended(_));
                match &event {
                    Event::Thought(chunk) => {
                        thought_chunks += 1;
                        if !matches!(inv.entries.last(), Some(Entry::Thought(_))) {
                            first_thought = first_thought.or(since);
                            let _ = write!(out, "\n{} 💭 {DIM}", t(now));
                        }
                        let _ = write!(out, "{}{RESET}{DIM}", chunk.replace('\n', " "));
                    }
                    Event::Said(chunk) => {
                        if !matches!(inv.entries.last(), Some(Entry::Said(_))) {
                            first_said = first_said.or(since);
                            let _ = write!(out, "{RESET}\n{} ▸ ", t(now));
                        }
                        let _ = write!(out, "{chunk}");
                    }
                    Event::StepStarted { tool, args, .. } => {
                        first_step = first_step.or(since);
                        let _ = write!(out, "{RESET}\n{} ⚙ {tool} {args}", t(now));
                    }
                    Event::StepDescribed { tool, args, .. } => {
                        let _ = write!(out, "{RESET}\n{}   ↳ {} {}", t(now), tool.clone().unwrap_or_default(), args.clone().unwrap_or_default());
                    }
                    Event::StepEnded { failed: true, .. } => {
                        let _ = write!(out, "{RESET}\n{} ✗ step failed", t(now));
                    }
                    Event::Note(note) => {
                        let _ = write!(out, "{RESET}\n{} ! {note}", t(now));
                    }
                    _ => {}
                }
                if let Some(place) = inv.record(event, now) {
                    moves += 1;
                    let _ = write!(out, "{RESET}\n{} ⇢ view: {place:?}", t(now));
                }
                let _ = out.flush();
                if ended { break }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                if let Some(place) = inv.tick(now) {
                    moves += 1;
                    println!("{RESET}\n{} ⇢ view: {place:?} (held)", t(now));
                }
                if abort_at.is_some_and(|at| now >= at) && cancelled_at.is_none() {
                    cancelled_at = Some(now);
                    println!("{RESET}\n{} ■ cancel", t(now));
                    session.cancel();
                }
            }
        }
    }

    let ms = |d: Option<Duration>| d.map_or("-".into(), |d| format!("{} ms", d.as_millis()));
    let steps: Vec<_> = inv
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Step { place, .. } => Some(place.is_some()),
            _ => None,
        })
        .collect();
    println!("{RESET}\n\n── outcome {:?}", inv.outcome);
    println!(
        "   first thought {}, first text {}, first tool {}",
        ms(first_thought),
        ms(first_said),
        ms(first_step)
    );
    println!(
        "   {} steps ({} matched to a read), {} unmatched reads, {} view moves, {} thought chunks",
        steps.len(),
        steps.iter().filter(|m| **m).count(),
        inv.unmatched_reads,
        moves,
        thought_chunks
    );
    println!(
        "   total {} ms, cancel → finished {}",
        started.elapsed().as_millis(),
        ms(cancelled_at.map(|at| at.elapsed()))
    );
    if inv.outcome == Outcome::Running {
        println!("   (finished without an outcome)");
    }
}
