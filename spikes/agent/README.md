# Agent spike: an investigation agent the window follows

A throwaway crate with two backends feeding one dock model:

1. **Embedded:** [rpi](https://github.com/bigfish1913/pi-rust) (`rpi-ai` +
   `rpi-agent` 0.3.13, a Rust port of pi's provider and agent-loop layers)
   runs the loop inside the app on an API key.
2. **ACP:** the operator's own agent (Claude Code or Codex, on their
   subscription) runs as a child process, and reads the cluster only through
   our tools, served over MCP.

It is its own workspace, so the app's lockfile, tests and lints don't see it.

```sh
cd spikes/agent
cargo test                                   # offline: faux provider and a fake ACP agent
cargo run --bin acp -- --agent claude        # Claude Code on your login, canned cluster
cargo run --bin acp -- --agent codex         # Codex on your ChatGPT login
cargo run --bin acp -- --agent claude --abort-after-ms 18000 --trace trace.log
cargo run -- --faux                          # replay a scripted investigation
ANTHROPIC_API_KEY=… cargo run                # a real model on the canned cluster
cargo run -- --thinking high --abort-after-ms 4000
cargo run -- --steer-after-ms 3000 --steer "Check the database side first."
```

A live run sends only the canned scenario in `src/scenario.rs` to the
provider; no cluster is contacted. `FRESHKUBE_CLAUDE_DEBUG_FILE=<path>` keeps
Claude Code's own debug log.

## What it builds

- `scenario.rs`: a canned namespace with one realistic fault. An ExternalSecret
  rotated the app's database password, Reloader rolled the Deployment, and the
  new pod crash loops because the CloudNativePG role reads its password from a
  different Secret. The old pods keep working on connections they already had.
- `tools.rs`: four read-only tools (`list_objects`, `get_object`, `pod_logs`,
  `list_events`). Each reports a `Place`, which is where the window would go to
  show what was read. Secrets return key names only.
- `session.rs`: the embedding the app would own. It runs `run_agent_loop` on
  Tokio and sends updates over an unbounded channel. Steering and cancel come
  from outside, and dropping the session cancels it, like `OwnedJob`.
- `investigation.rs`: the dock's model. Both backends report the same
  `Event`s; the transcript holds thoughts, speech, steps (with state and place)
  and notes. The follower moves the main view, holds each view for at least
  1.5 s, and stops following when the operator navigates.
- `mcp.rs`: the same tools over MCP (rmcp 3.5, Streamable HTTP) on a loopback
  port, answering only the session's bearer token.
- `acp.rs`: the ACP client (`agent-client-protocol` 2.2). It spawns the
  agent, hands it the MCP endpoint, turns session updates into dock events,
  and answers permission requests. `bin/fake_acp_agent.rs` is a scripted agent
  for the offline tests in `tests/acp.rs`.

## Findings: embedded rpi

Checked offline by the tests; a live run on an API key is still to do.

| Question | Answer |
| --- | --- |
| Streams thinking, text and tool calls incrementally? | Yes. `ThinkingDelta`, `TextDelta` and `ToolCallDelta` arrive as the provider sends them. The Anthropic provider maps SSE frames as they arrive and asks for summarized adaptive thinking. |
| Tools as Rust code calling core directly? | Yes. `AgentTool` is an async trait with a cancellation token; a tool's error becomes a failed step the model sees. No sidecar, socket or TypeScript. |
| Cancellation? | The token reaches the provider, which selects on it while waiting for the next frame. A cancelled call ends with a `MessageEnd` whose stop reason is `Aborted`, and the loop still returns `Ok`, so the outcome has to be read from the message, not the result. Dropping the session cancels it. |
| Steering? | Works. A message queued mid-thought reaches the next model call, after the current turn's tools. It cannot interrupt a thought in progress; that takes cancel and a new prompt. |
| Fits the app's runtimes? | Tokio only, which the app already owns. Updates go to GPUI the same way cluster results do. |
| Dependency cost? | Small. About 12 crates the app doesn't already have: `reqwest`, `jsonschema` 0.18 and its helpers, `eventsource-stream`, `webpki-roots` and the three rpi crates. rustls, hyper and tokio are shared. |
| Offline testing? | Good. The `faux` provider scripts thinking, tool calls and pacing, and a step can be a closure that inspects the context. That is enough for headless UI tests of the dock. |

### Warts to work around

- **Use `run_agent_loop`, not `Agent`.** `Agent` publishes events on a 256-slot
  broadcast channel, and a slow receiver loses events (`Lagged`). Driving the
  loop with our own emitter avoids that and keeps steering in our hands.
- **`StreamFn` is synchronous while providers are async.** Bridge it by
  spawning the provider and forwarding its stream (`session::stream_fn`), as
  rpi's own tests do.
- **The model catalogue lags.** It stops at the Claude 5.0 ids, so newer ids
  reuse the nearest entry's limits and prices, and reported cost is approximate.
  The app should own its model list.
- **The crate is young:** created in August 2026, one maintainer, version
  0.3.x. The loop and the Anthropic provider read well, with stated ordering
  invariants and many tests, but pin an exact version and keep it behind our
  own boundary.

### Following, from the scripted run

With a tool call every ~0.4 s and a 1.5 s dwell, the view went Deployment →
Secret → CNPG Cluster and skipped the pod list and the crash log in between. The
latest place wins, which keeps the view current, but the skipped reads were the
ones that mattered. Options for the real design:

- Keep "latest wins" for the view, and make every transcript step clickable,
  so a skipped place is one click away. That's what the spike does.
- Give evidence a priority: logs that explain a failure and failed reads get
  their full dwell even when newer places queue up.
- Let the agent mark what matters. A `show(place, why)` tool, or a citation in
  its answer, decides where the view stops; ordinary reads only update a
  breadcrumb.

## Findings: ACP with the operator's own agent

Run live on 2026-10-03 against the canned cluster, on the operator's logins.
Both agents found the real cause (the CNPG role reads its password from a
Secret that the rotation never touched) and named every object they read.

| | Claude Code (`claude-agent-acp` 0.85.1, Opus 5.5) | Codex (`codex-acp` 2.1.1) |
| --- | --- | --- |
| Total time | 42 s | 72 s |
| First tool call | 14 s | 24 s |
| Reads, matched to their step | 10 of 10 | 15 of 15 |
| View moves | 8 | 9 |
| Thinking streamed | Yes, summarized, ~95 chunks | No; short progress messages instead |
| Cancel → prompt answered | 15 ms | 24 ms |
| Parallel tool calls | Yes, 2–3 at a time | Yes, 3–4 at a time |
| Asks permission for our tools | No (pre-allowed) | Yes, every call |

What it takes:

- **Tools over MCP work for both.** The app keeps the tools, the place each
  read reports and the Secret redaction; the agent only calls them. Reads are
  matched to transcript steps by tool name, since an MCP call doesn't carry
  the agent's call id. With parallel calls that matched every time here, but
  two identical tools in flight could swap places.
- **Lock each agent down; the defaults are not safe.** Claude Code:
  `_meta.systemPrompt` replaces its prompt, `claudeCode.options.tools: []`
  removes its built-ins, `settingSources: []` skips the operator's settings,
  hooks and CLAUDE.md, `strictMcpConfig` skips their MCP servers, and
  `ENABLE_CLAUDEAI_MCP_SERVERS=false` is needed as well, because claude.ai
  connectors loaded despite `strictMcpConfig`. Codex: `CODEX_CONFIG` turns off
  shell, exec, apps, plugins, hooks, browser, web search and the rest, and
  `INITIAL_AGENT_MODE=read-only`. Both get an empty temporary workspace, and
  the permission handler refuses anything that isn't one of our tools.
- **Claude Code defers MCP tools behind its ToolSearch tool**, which `tools:
  []` removes, so the model saw no tools at all. Each tool now carries
  `_meta["anthropic/alwaysLoad"] = true`.
- **rmcp 3.5 and Claude Code disagree on MCP 2026-07-28.** Claude Code
  rejected `tools/list` without `ttlMs` and `cacheScope`, which rmcp leaves
  out unless set. Pin both versions and keep a live check per agent release.
- **Codex asks permission naming only the call's id.** The client remembers
  what each `tool_call` notification said and decides from that.
- **Tool-call input streams.** Claude Code announces a call with empty input
  and fills it in over several updates, so the dock updates a step in place
  (`StepDescribed`).
- **Startup is slow on first use.** `npx` fetched the adapters (~55 s the
  first time); later runs started in 3–10 s. The app should install a pinned
  adapter once, not run `npx` per session.
- **Steering isn't in ACP v1.** A new prompt after cancel is the only way to
  redirect; the embedded loop can queue a message for its next turn.

Open: Codex sent no reasoning summaries even with `model_reasoning_summary =
"detailed"`; whether that's the adapter, the model or the plan is unknown.

## Still to check live (embedded)

- How much summarized thinking arrives, and how early, for the models we'd offer.
- How long the time to first thought is; whether a thought arrives before every
  tool call or only some.
- Whether the model finds the cause, and how many reads and what cost it takes.
- How long cancelling takes against the real API, mid-thought and mid-tool.
