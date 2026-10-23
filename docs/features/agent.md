# The agent in the dock

What an agent may read of a cluster, and the way it reaches those reads. This is slice D1a of the agent in the dock ([#874](https://github.com/skel84/freshkube/issues/874), [#906](https://github.com/skel84/freshkube/issues/906)): the app starts no endpoint and no agent yet, and desktop does not depend on the agent crate.

**Modules.** `crates/freshkube-core/src/agent_reads/` (`call.rs`, `gate.rs`, `read.rs`, `live.rs`, `example.rs`, `shape.rs`, `answer.rs`, `refusal.rs`, `target.rs`); `crates/freshkube-core/src/constants.rs` (`agent`); `crates/freshkube-agent/src/endpoint/`, `session.rs`, `saved.rs`; core's `resources/discovery.rs` (`resolve_kind_name`) and `write/audit.rs` (`AuditStore::named`).

**Proved by.** Core, `agent_reads/tests.rs`: `a_cluster_without_agent_access_refuses_before_any_request`, `turning_access_off_ends_a_read_in_flight`, `a_grant_for_another_context_reads_as_off`, `every_agent_read_is_a_get`, `secrets_are_read_metadata_only`, `managed_fields_and_last_applied_never_reach_the_answer`, `log_reads_are_bounded`, `lists_and_events_are_cut_at_their_limit`, `an_object_over_the_limit_is_too_large`, `call_parse_refuses_what_it_cannot_bound`, `failures_map_to_their_codes`, `example_objects_get_the_same_bounds_and_secret_rule`, `the_five_hundred_and_first_read_is_limit_reached`, `a_403_keeps_the_shared_client_and_a_401_or_a_dead_server_drops_it`, `events_are_read_page_by_page_to_find_the_newest`, `a_secrets_last_applied_copy_never_reaches_the_answer`. Agent, `crates/freshkube-agent/tests/`: `endpoint.rs` (`the_endpoint_listens_on_loopback_only`, `the_endpoint_refuses_without_the_session_token`, `the_token_dies_with_the_session`, `a_fake_agent_client_reads_through_the_endpoint`, `a_refused_read_gets_a_read_id_and_no_view`, `no_view_state_goes_on_the_wire`, `a_read_in_flight_when_the_endpoint_drops_sends_nothing`, `a_chunked_body_over_the_limit_is_refused_without_a_content_length`, `a_slow_header_or_body_is_dropped`), `saved.rs` (`set_access_saves_audits_and_gates_together`, a missing or unparsable file, a failed audit, `turning_access_off_fails_closed_even_when_the_save_or_the_audit_fails`) and `logs.rs` (`the_token_never_reaches_debug_or_logs`).

## The tools

Server `freshkube`, exactly four tools; `agent_reads::TOOLS` holds their names and `SERVER_NAME` the server's. An argument the tool does not take is refused. `cluster` is a cluster entry's id and defaults to the session's own.

| Tool | Arguments | Answer |
|------|-----------|--------|
| `list_objects` | `cluster?`, `kind`, `namespace?`, `limit?` 1–200 (100) | `{read, cluster, kind, items: [{namespace, name, uid, status, created}], cut}` |
| `get_object` | `cluster?`, `kind`, `namespace?`, `name` | `{read, cluster, kind, namespace, name, uid, object}` |
| `pod_logs` | `cluster?`, `namespace`, `pod`, `container?`, `previous?`, `tail_lines?` 1–500 (200), `since_seconds?` 1–86400 | `{read, cluster, namespace, pod, container, previous, lines, cut}` |
| `list_events` | `cluster?`, `namespace?`, `kind?`, `name?`, `since_seconds?` 1–86400 (3600), `limit?` 1–200 (100) | `{read, cluster, events: [{type, reason, object, message, count, last}], cut}` |

- `kind` takes what kubectl takes: a key (`deployments.apps`), a plural, a singular or Kind in any case, a short name (`deploy`), with an optional `.group`. `resolve_kind_name` resolves it: built-in kinds at once, then a key by `resolve_kind_key`, and only then a bare name in the groups discovery lists, at most 32 of them, read together. A kind the cluster does not serve (or a name two kinds share) is `not_served`. Deep links keep using the strict `resolve_kind_key`, which asks only for the key's own group.
- Names are DNS names: a namespace and a container are DNS labels, a name is a DNS subdomain.
- A success is `isError: false` with the JSON in one text block and as `structuredContent`. `cut` is `null` or `{by: items|lines|bytes, limit}`; a cut answer is still a success.
- `pod_logs` reads one pod container's `log` subresource. Without a `container`, the server chooses the pod's default and the answer's `container` is `null`.
- `list_events` reads up to five pages of 200 events (by involved Kind and name when given), because the API lists them in no order of time; it keeps those newer than `since_seconds`, sorts newest first and keeps `limit`. If the server had more than the pages read, `cut` is `{items, <events looked at>}`, so the agent knows the newest may be missing.

## Parsing

`Call::parse(tool, arguments)` is the only parser of what an agent sends. It yields `Call { cluster, request }`, `Request` being `List`, `Get`, `Logs` or `Events` with bounded fields. It refuses an unknown tool, an unknown argument, a wrong type, a value outside its range and a name that is not a DNS name, each as `invalid`. The message names the argument and never repeats the value; an unknown argument is named only if it is short and made of lower-case letters and underscores. `Call::parse_pairs` takes the arguments as pairs and also refuses a name given twice, which a JSON object cannot keep.

## Bounds

Asked of the server where the API allows (`limit`, `tailLines`, `sinceSeconds`, `limitBytes` of 64 KiB), and checked again in core on every answer, live or example. The numbers are in `freshkube_core::constants::agent`.

| Bound | Limit |
|-------|-------|
| Items in a list, events in an answer | 200 |
| Log lines, log bytes | 500, 64 KiB (the newest are kept) |
| One log line, one event message | cut at 2 KiB, 1 KiB |
| One object, as JSON | 64 KiB, else `too_large` |
| Any answer | 128 KiB (items or lines are dropped to fit) |
| One read | 20 s, then `timed_out` |
| Reads per session | 500, then `limit_reached` |

Every object loses `metadata.managedFields` and the last-applied annotation. **A Secret is read as metadata only**: `get_metadata` and `list_metadata_page` ask for `PartialObjectMetadata`, and core keeps only the API version, kind and metadata, so a value or a key name is never requested or kept. Example data goes through the same rule.

## Errors

`isError: true` with `{read, error: {code, message}}`; the message goes through the audit log's `scrub`. `FailureKind` maps onto the code in one match with no wildcard arm (`Refusal::from_failure`); a 404 while resolving a kind or listing one means the API is not served. The shared client is dropped (so the access builds it again) only after an unreachable server or a 401, decided from the `FailureKind` before it is mapped: a 403 is the person's role, routine under a `view` role, and keeps the client.

| Code | When |
|------|------|
| `invalid` | An argument that cannot be bounded |
| `unknown_cluster` | The session may not name that cluster (the app's `Targets` answered none) |
| `not_enabled` | Agent access is off for the cluster; nothing was sent |
| `refused` | 401 or 403 under the person's own access |
| `not_found` | The object, pod or container does not exist |
| `not_served` | The cluster does not serve the kind |
| `too_large` | An object over 64 KiB |
| `unreachable`, `timed_out`, `failed` | The cluster could not be reached, took too long, or failed otherwise |
| `limit_reached` | The session has used its 500 reads |

Every read, refused ones included, gets a read id (`r1`, `r2`, …). The `ViewState` never goes on the wire: the endpoint hands the app `ReadEvent {session, read, tool, cluster, view, outcome}`, where `view` is the Resources view of the answer's `Place` (the kind's key, namespace, object with its UID, and the Details, Events or Logs tab), checked by `ViewState::check`, and `None` for a refused read.

## The gate

`AgentAccess` (`agent_reads/gate.rs`) is in memory and cheap to clone. It holds grants as `(entry, context)` pairs and is **off for every cluster by default**.

- `read` takes a lease from the gate before it builds any client. With no grant it answers `not_enabled` and sends nothing.
- A grant counts only while the entry opens the context it was given for, so a context change reads as off.
- Turning a cluster off (`revoke`) ends its leases: a read in flight answers `not_enabled` and what it had read is dropped.
- A session's read budget lives in the gate handle the endpoint takes with `for_session()`, which shares the grants and counts its own reads. Only a read that took a lease is counted.

## The endpoint

`freshkube_agent::Endpoint` serves MCP Streamable HTTP at `http://127.0.0.1:<random port>/mcp`, one per session, and closes the port and ends its token when dropped.

- **401** without `Authorization: Bearer <token>` (compared in constant time); **403** for any `Origin` header or a `Host` other than `127.0.0.1:<port>`; **413** over 64 KiB, whether declared or chunked; **408** for a body that takes longer than `Timeouts::body` (10 s). A connection that is slow to send its headers (`Timeouts::header`, 10 s) is dropped. Each refusal is a bare status.
- **Dropping the endpoint ends everything:** the port closes, every connection is closed, and a read in flight is given up and publishes nothing: no answer, no log line and no `ReadEvent`.
- The token is 32 bytes of OS randomness (`session::Token`); `Debug` and `Display` print nothing of it and `reveal` is the one way to read it. It is never logged.
- The app implements `Targets` to resolve an optional entry id to an `AgentTarget { entry, context, source }`; the source is `Live(ClusterAccess)` or `Example(Arc<dyn ExampleObjects>)`.
- One `tracing` line per read: the short session id, the read id, the entry, the tool, the target and the code. No answer data, no token, no rejected value. A cluster id the session cannot read is `unknown` in the line and in `ReadEvent.cluster`, never the id the agent typed.

## `agent.json`

`agent.json` sits beside the preferences (`freshkube-prefs`, mode 0600, read to at most 1 MiB), not in `freshkube.yaml`: the setup file is what a person may share, and sending a cluster's data to their model provider is their own consent, as with `follows.json`. A grant is `{entry, context, since}`. A missing file grants nothing; an unreadable or unparsable one grants nothing and says why once.

`Access::set_access(entry, context, on, from)` is the one way to change a grant. It saves the file, appends `{at, entry, context, access, from}` to `<preferences>/audit/agent.jsonl` (mode 0600, rotated like the write audit) and updates the gate. Turning access **on** is all or none: if any step fails the error is returned, the file is put back and the gate is unchanged. Turning it **off** fails closed: the gate is turned off first, which ends the reads in flight, and then the file and the audit line are written; if either fails the error is returned but access stays off. Whether a context belongs to the workspace is the caller's to check.

## The client: slice D1b

The person's own agent investigates a cluster for them (issue [#874](https://github.com/skel84/freshkube/issues/874), part D1). It runs as a child process speaking ACP over stdio and reads only through Freshkube's four read tools. This part is slice D1b (issue [#907](https://github.com/skel84/freshkube/issues/907)): the ACP client, the opencode profile and its lock-down, the version check and the scripted agent. The endpoint, the access gate and `agent_reads` are slice D1a, the dock tab is D1c and following on the page is D1d; they will add their parts below.

Code:

- `crates/freshkube-agent/src/`: `acp/` (`mod.rs` the session, `permission.rs` what is allowed, `updates.rs` session updates to events, `lines.rs` the line bound), `investigation.rs` (the transcript), `profile/` (`mod.rs` profiles, the locked environment and the launch, `opencode.rs` the lock-down, `locate.rs`, `version.rs`), `child.rs` (the process group and what ends it), `tools.rs` (the server and tool names), `scripted/` (the scripted agent), `bin/fake_acp_agent.rs` (that agent as a process).
- `crates/freshkube-agent/tests/`: `acp.rs` (in process), `process.rs`, `group.rs` and `environment.rs` (the scripted binary), `log.rs` (the token and the debug log), `support/` (the stub for the endpoint).

The crate has no GPUI and names nothing of desktop, `freshkube-ui` or the feature crates. Desktop does not depend on it yet, so the app starts no agent. The scripted agent and the MCP client it needs are behind the `scripted` feature, which the crate's own tests turn on from a dev-dependency; the app's build has neither.

### The ACP client

`AcpSession::start(link, endpoint)` takes a `Link` (both ends of the agent's stdio, from `Profile::spawn` or `scripted::in_process`) and an `Endpoint`, the MCP endpoint's URL and its token. It returns the session and a receiver of `investigation::Event`s.

- **Startup.** `initialize`, then `session/new` with one HTTP MCP server: the name `freshkube`, the URL, and an `Authorization: Bearer <token>` header. Startup has a 30 s bound (`STARTUP_TIMEOUT`); past it the turn ends as failed with "opencode didn't start" and the child is ended with the session.
- **The token** goes only into `session/new`, over the child's stdin. It is never in the argument list, the environment, a file, a log or `Debug` (`Endpoint`'s `Debug` prints `Token(..)`). The ACP library logs every message it sends at debug level, headers included. The app's logger therefore drops the events of `agent_client_protocol*` and `rmcp` below info **after** the user's `RUST_LOG` is applied (`agent_libraries_at_info` in `src/main.rs`), since a more specific target such as `agent_client_protocol::jsonrpc=debug` beats a directive; the directives (`QUIET_LOG_TARGETS`) only save work. `tests/log.rs` shows the token in a debug log without the clamp, in a log with a specific target and the directives only, and not with the clamp. The client advertises no file system and no terminal, so the agent has none to ask for.
- **Prompts** run one after another in one session (`prompt`). `cancel` stops the running prompt, drops the ones waiting and gives the agent 5 s (`CANCEL_GRACE`) to answer; after that the child is ended and the turn ends as cancelled.
- **A crash** (the child exits or closes its output) while a prompt runs ends that turn as `Failed`, once; later calls return `Closed`.
- **Bounds on input.** A line from the agent is at most 4 MiB (the connection fails past it). Events wait in a queue of 1,024 and the agent is slowed to the reader's pace, so the reader must drain it. The transcript bounds are below.
- **A start needs a passing version check.** `Profile::spawn` takes the `Checked` that `Profile::check` gives, which only a supported or newer-than-checked version earns, and refuses one made on another binary. Another opencode major version may read its permissions under other keys and fall back to defaults that allow a shell. The proof is for one profile kind and range, so a generic command's proof is refused for opencode, and for the file that was run (its length, modification time, device and inode; on Windows the length and modification time only), so a binary replaced between the check and the start is refused until it is checked again.
- **Models.** `models()` gives the list the agent offers and the current one, and `set_model(id)` chooses one of them (an id the agent didn't offer is refused before anything is sent). A model is marked local when its provider is `ollama`, `lmstudio`, `llama.cpp`, `llamacpp` or `local`.

#### Which tools are allowed

`acp/permission.rs` answers every permission request. A call is allowed only by its **name**, an exact match of server `freshkube` and one of the four tool names (`list_objects`, `get_object`, `pod_logs`, `list_events`; `tools.rs` re-exports core's `agent_reads::TOOLS`, so the allow-list is the endpoint's own list): `freshkube_get_object` (how opencode names an MCP tool), or `mcp__freshkube__get_object` or `mcp.freshkube.get_object`, which other agents use.

- **Not the title and not the input.** In opencode 2.1 a shell call's title is its command, an update carries no name, and a call's input is the model's own text, so a shell whose input says `{server: "freshkube", tool: "get_object"}` is still a shell.
- **A refusal is final.** Once a call has read as not ours, no later update and no request makes it ours. A call with no name anywhere is refused, and so is an id nothing announced. A request that names something other than what the call announced is refused.
- **Everything else is refused,** with a transcript note "Refused: …" naming the call as it was announced: built-in tools, shell and exec, file reads, edits, web fetch and search, subagents, other MCP servers, and a name that only contains ours (`freshkube_get_object_extra`, `other_freshkube_get_object`).

The name is still the agent's own word. The boundary is the lock-down below, which leaves the agent no other tool; this handler is the second line. This replaces the spike's substring match.

### The transcript

`investigation::Investigation` holds what was asked, thoughts, speech, steps (their state and arguments, updated in place as a tool call is described) and notes. Everything the agent sends is bounded:

| Bound | Value | When it is reached |
| --- | --- | --- |
| Text in the transcript | 1 MiB, in at most 2,000 thought and speech entries | The oldest thought text is dropped first, then the oldest speech. An entry left with no text is removed, so the entries are bounded in memory too; `trimmed()` says that earlier text is gone, and a partly cut entry is marked `trimmed`. |
| Shown arguments of a step | 4 KiB | Cut on a character; `Args::is_cut` says so. |
| Steps | 1,000 | Later steps are refused with one note. |
| One chunk | 64 KiB | Cut. |
| Notes | 1,000 | Later ones are dropped. |

The 1 MiB covers text. Steps are bounded separately, at most 1,000 × 4 KiB of arguments. The follower that moves the page is not here; it comes with D1d.

### Profiles

A profile is a launch command, an environment, a lock-down and a supported version range. Every agent starts from a cleared environment, then gets:

- a minimal `PATH` (the system directories and the agent's own, so a script that starts `node` beside it runs), `HOME` and `LANG`;
- an empty temporary config home (`XDG_CONFIG_HOME`);
- `KUBECONFIG` and `TALOSCONFIG` pointing at empty temporary files;
- `TMPDIR` in the session's directory, and an empty temporary working directory.

The directories are one `Dirs` per session, removed when the process group is gone. No provider key, cloud credential or cluster credential is inherited. The child runs in its own process group; dropping the session sends SIGTERM to the group and SIGKILL after 2 s if anything is left.

- **Unix.** If the app dies without dropping the session, a small shell in a group of its own (started with the agent) sees the app's pid go and ends the agent's group, within a few seconds. `child::tests` kills a stand-in parent and checks the group goes.
- **Windows.** The child is created suspended, put in a job object that kills its processes when the job closes, and only then resumed, so nothing it starts is outside the job; if it can't be put in the job it is killed unstarted. The job closes with the app. `tests/group.rs` runs on Windows in CI and checks that a grandchild goes with the session.

#### The opencode profile

The command is `opencode acp`. The configuration is handed inline in `OPENCODE_CONFIG_CONTENT`; `profile/opencode.rs` builds it (`config()`) and the environment (`environment()`). A guard inside a plugin is never counted as the boundary.

**What was read.** The source of opencode 2.1.3 (commit [`e965774`](https://github.com/anomalyco/opencode/tree/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b)), and for each switch below the same file at 2.1.0 ([`d30e990`](https://github.com/anomalyco/opencode/tree/d30e990c69efd8a954413a724b35c5a27462f320)), which is the floor of the range. The table is from that reading; what a run of 2.1.3 showed is in [The live check](#the-live-check).

| Switch | Where opencode reads it |
| --- | --- |
| `acp` starts a private server | [`handlers/acp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/commands/handlers/acp.ts) calls `Standalone.start()` unconditionally and has no flag to attach to the shared service; [`standalone.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/services/standalone.ts) runs `serve --stdio --port 0` with a random `OPENCODE_PASSWORD`. The shared service is only reached by `resolve` in [`server-connection.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/services/server-connection.ts), which `acp` does not call. |
| The private server is on loopback | [`server-process.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/server-process.ts): the host defaults to `127.0.0.1`, and port `0` is an ephemeral port. It is a **listener on loopback**, behind that password, not no listener; it ends when its stdin closes. |
| No pairing or tunnel | `server-process.ts` starts the remote tunnel only in `service` mode, from the service configuration (`config.remote`). |
| `permissions`: deny all, allow our four | Ordered rules, the last match wins, no match asks ([`permission.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/permission.ts) `evaluate`). A tool whose last matching rule is `*` deny is not offered to the model ([`tool.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool.ts) `whollyDisabled`). An MCP tool's action is `<server>_<tool>` ([`tool/mcp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool/mcp.ts) `name`). |
| Built-in tool names | `shell` (1.x called it `bash`), `read`, `grep`, `glob`, `webfetch`, `websearch`, `subagent` (1.x: `task`), `skill`, `todo`, `question`, `edit` (also what `write` and `patch` ask), and `opencode_list_mcp_resources` and `opencode_read_mcp_resource`, in [`tool/plugin/`](https://github.com/anomalyco/opencode/tree/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool/plugin). The profile denies each by name and everything else with `*`. |
| `agents.<name>.permissions` for every built-in agent, and `default_agent` | An agent's own rules come after the global ones in the order, so a source loaded after ours that sets them would win. The same list is written under each of `build`, `plan`, `general`, `explore`, `compaction`, `title` and `summary` ([`schema/config/agent.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/config/agent.ts), [`plugin/agent.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/plugin/agent.ts)). |
| `commands`, `skills`, `instructions`, `references`, `lsp`, `formatter`, `websearch`, `snapshots`, `update`, `share` | The config document in [`schema/config.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/config.ts). The empty and false values are starting values, not removals: a source that loads after ours can still add to them. Our MCP server comes in `session/new`, which opencode accepts over HTTP ([`acp/service.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/acp/service.ts) `supportedMcpServers`); the inline document names no MCP server. |
| `plugins` lists one Freshkube-owned directory | **A change to the lock-down, made for [#946](https://github.com/skel84/freshkube/issues/946); see below.** |
| `OPENCODE_CONFIG_CONTENT` | `server-process.ts` (`config.content`). |
| `OPENCODE_CONFIG_DIR`, `XDG_CONFIG_HOME` empty | The global config directory ([`util/global.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/util/src/global.ts), [`global-roots.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/util/src/global-roots.ts)). |
| `OPENCODE_CONFIG_PROJECT_DISABLE`, `OPENCODE_DISABLE_CLAUDE_CODE` | `server-process.ts` (`config.project`, `config.claude`). No project configuration, and no instructions or skills read from other coding tools' directories. `HOME` is empty and so is the working directory, so no instruction file is found beside them. |
| `OPENCODE_DISABLE_AUTOUPDATE` | [`services/updater.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/services/updater.ts); `update: "disable"` in the configuration as well. |
| `OPENCODE_DISABLE_FILEWATCHER`, `OPENCODE_DISABLE_MODELS_FETCH` | `server-process.ts` (`fs.filewatcher`, `models.fetch`). With the fetch off, the model list comes from the catalogue bundled in the binary ([`models-dev.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/models-dev.ts) `bundledSnapshot`), so no connection goes to the catalogue's host. |
| `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` | Logins, session history and the saved organisation config sources live in the database under `$XDG_DATA_HOME/opencode` ([`credential.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/credential.ts), [`wellknown.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/wellknown.ts) `wellknown:sources`). See the data modes below. |
| Admin-owned configs | The system directory (`/Library/Application Support/opencode`, `/etc/opencode`, `%ProgramData%\opencode`) and macOS managed preferences load after the inline config and win ([`config/discovery.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/config/discovery.ts)). The profile does not change them; an administrator who sets them owns what they allow. |

`opencode debug config` and `opencode debug agents` go to the shared service and show the person's own configuration, so they don't show what an agent session uses; `opencode api --standalone config.get`, run with the session's environment, does.

**MCP tools and Code Mode.** opencode 2.1.3 offers an MCP server's tools to the model directly only when the server sets `codemode: false`; otherwise they are reachable only through Code Mode's `execute` ([`core/src/tool/mcp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool/mcp.ts) `options.codemode`, [`tool.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool.ts) `snapshot`: `direct` and `codeModeTools`). The profile's wildcard denies `execute`, so with Code Mode on the model is offered none of Freshkube's tools ([#946](https://github.com/skel84/freshkube/issues/946)). `execute` is not allowed in any form: it runs model-written code, and Code Mode also gives that code an unrestricted `fetch`.

A server added over ACP cannot say `codemode: false`, and no configuration reaches it. What opencode does, at `e965774`:

- `session/new`'s `mcpServers`: [`acp/service.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/acp/service.ts) `supportedMcpServers` only filters them, and [`acp/sessions.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/acp/sessions.ts) `mcpConfig` builds `{type, url, headers, oauth: false}`. It reads no `_meta` and no other field.
- A configuration entry of the same name does not merge: [`mcp/index.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/mcp/index.ts) rebuilds the server table from the servers added at run time (`initial`, `add`), and [`config/plugin/mcp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/config/plugin/mcp.ts) skips a configured server that is already there (`if (editor.get(name)) continue`). A server with its URL in the configuration and its token in `session/new` is therefore not a route: the added one wins whole. (The token never goes into the configuration in any case.)
- The configuration schema nests servers under `mcp.servers` and needs a transport for each ([`schema/config/mcp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/config/mcp.ts), [`schema/mcp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/mcp.ts)); any other key under `mcp` is read as a version 1 server ([`config/normalize.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/config/normalize.ts)). The profile's earlier `mcp.freshkube.codemode: false` was dropped there, which the live check found.
- There is no global flag, experimental key or environment variable for it. The built-in defaults ([`plugin/mcp-codemode-defaults.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/plugin/mcp-codemode-defaults.ts)) match only `https://executor.sh/<x>/mcp` and the command `executor mcp`.
- A plugin's `ctx.mcp.transform` ([`plugin/host.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/plugin/host.ts)) runs after the added server is in the table and can `update` it. This is the one route.

**The plugin, and what it changes in the lock-down.** `plugins` was `[]`. It now lists one directory, written by Freshkube, whose `server.js` is a fixed text (`plugin_source()` in `profile/opencode.rs`): it sets `codemode = false` on the server named `freshkube` and nothing else. It has no token, no environment access, no file or network access, no import, no tool and no hook, and a test fails if its text mentions any of them. Nothing else about the lock-down moves: the `*` deny and the per-tool denies stay, `execute` stays denied, the token goes only into `session/new`, and the model reaches the four tools only. The maintainer chose this route over an upstream change to `session/new`.

- *Where it lives.* In the session's own temporary directory (mode 0700, `Dirs::plugin()`), written at each launch and removed with the session, in both data modes. Never in the person's own folders, and `data: shared` writes nothing there. The files are mode 0400.
- *The check.* Before the agent starts, `stage_plugin` reads the files back and compares them byte for byte with the constants; the directory must hold those two files and nothing else (opencode takes the first of `server.ts`, `server.js` and the other extensions it finds), as plain files, not links. The files are created new (a file already there is not reused) and the directory itself must be a real directory, not a link. On a difference, a missing or an unreadable file, `Profile::spawn` returns an error, naming the plugin, and does not start the agent. The check is of launch only: opencode watches a configured plugin directory and loads the plugin again when it changes. After the check the directory is made read-only (mode 0500; `Dirs` puts 0700 back when the session ends so it can be removed). That guards against accident only, since a process running as the same user can change the mode back, or opencode itself.
- *The entry.* `plugins` holds the directory's absolute path. A path that is not absolute would be installed as a package ([`config/plugin/source.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/config/plugin/source.ts) `scan`); the test that parses the document refuses it. A plugin directory is the entry opencode wants: a configured file is refused there, and `server` is resolved in the directory ([`plugin/host.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/plugin/src/host.ts) `resolve`) as a module with a default export that has an `id` and a `setup` ([`plugin/module.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/plugin/module.ts)), so the directory also holds a `package.json` that makes the entry an ES module.
- *Other plugin sources.* This pull request does not change them. opencode also loads the `plugin` and `plugins` folders of its config directories ([`plugin/source-directory.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/plugin/source-directory.ts)). The global one follows `XDG_CONFIG_HOME`, which the profile points at an empty directory (the live check found `opencode debug` showing the person's own configuration, but those commands attach the shared service and are not what an agent session uses, [#945](https://github.com/skel84/freshkube/issues/945)); the project ones are off (`OPENCODE_CONFIG_PROJECT_DISABLE`, `OPENCODE_DISABLE_CLAUDE_CODE`); the administrators' system directories are theirs, as the table says. Any source that loads after ours can also remove our plugin with a `-<path>` entry; nothing here prevents that.
- *If opencode does not load it.* Nothing falls back to Code Mode and `execute` stays denied: the model is offered none of Freshkube's tools and nothing is widened. Freshkube cannot see that opencode failed to load a file it was given (the client reads only ACP), so the dock cannot yet say "the tools aren't offered" for it; the live check below is what shows it. A tab message for a session that ends without any MCP tool call is not built; it is tracked in [#967](https://github.com/skel84/freshkube/issues/967).

The tests: `the_document_has_the_shape_opencodes_schema_accepts` parses the profile's document with `profile/schema_copy.rs`, a copy of the pinned schema's rules for the keys the profile writes, so a key opencode drops fails ours (`the_codemode_only_entry_of_958_is_dropped` proves the copy catches the old entry); `the_plugin_is_written_and_reads_back`, `the_plugin_text_is_fixed_and_reaches_nothing`, `a_tampered_plugin_is_refused`, `a_missing_or_unreadable_plugin_is_refused` and `a_start_with_a_changed_plugin_never_starts_the_agent` (the last runs `spawn` with a binary that records that it ran) cover the file and the refusal; `the_opencode_profile_locks_down` asserts that `execute` and the built-ins stay denied. The live check repeats items 2 and 3 of pull request 912's check: opencode must refuse a shell, a file read and a fetch, and the first call is named `freshkube_get_object`.

`profile/opencode.rs` carries the same table as `LOCKDOWN_SOURCES`, and a test fails if a variable the profile sets has no source in it.

##### Where opencode keeps its data

`Profile::opencode(binary, data)` takes a `Data`:

- **`Own(dir)`, the default.** A directory Freshkube owns (`opencode/` under the app's data directory, created with mode 0700). `XDG_DATA_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` are `data/`, `state/` and `cache/` inside it. The person signs in to their provider once inside it, and their own opencode history and saved organisation configs are never read or written.
- **`Shared`.** Their real directories, as opencode itself would use them: their logins are there, the investigations land in their opencode history, and any organisation config saved there loads. Which mode runs is `Profile::data()`; the setting lives in `agent.json` beside the grants (D1a), never in `freshkube.yaml`, and the dock's wording that the history and saved configs are shared comes with the dock tab (D1c).

An organisation config source (opencode's "wellknown" sources) is loaded after the inline configuration, fetches its URL when opencode starts and every ten minutes, and its per-agent rules win over ours. Both modes therefore **refuse to start** if `organisation_sources` sees one (in `Own` mode a login to an organisation inside the owned directory saves its source there too), looking in the data directory the agent will use: it looks in opencode's database files under the data directory (`opencode*.db*`) for the key the sources are saved under, `wellknown:sources`. It finds the key, not its value, so a source that was removed and left the key behind still refuses (the message says to use Freshkube's own directory, or to remove it and sign in again). When a file can't be read or is past the 1 GiB scan limit it cannot tell, and that **refuses too**, with a message saying so: a key past the limit would load and could turn tools back on. `Own` never reads the real directory. The scan also reads opencode 1.x's login file, `auth.json`, since opencode 2 copies an organisation login (`"type": "wellknown"`) from it into the sources the first time it opens its database, after this scan; a login file that can't be read, or a data directory that can't be listed (anything but "not found"), is unknown and refuses. The scan reads up to 1 GiB on each start, so the dock tab (D1c) runs it off the UI thread. In `Shared` mode opencode reads `wellknown:sources` again every ten minutes, and the check at start does not see a source the person adds with their own opencode during a session; D1c can run the check again while the agent is running.

`opencode acp` has no `--standalone` flag (it is always standalone), so the profile passes none. `opencode models` and `opencode run` take it and would otherwise attach the shared service; the client never runs them.

**Models.** opencode reports them in ACP's session setup, as a `model` config option (category `model`, values `provider/model`; [`acp/config-option.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/acp/config-option.ts)), and changes the model with `session/set_config_option`. The client uses that; `opencode models` is not run.

**Not confirmed.** Source reading cannot show these; the live check does: that `deny *` leaves the four tools offered, that the permission request names `freshkube_get_object` as read above, that the model list is not empty with the fetch off, and that nothing is read from the real home. Windows is written against the documented variables and not run.

#### Locating opencode and the version check

A GUI app doesn't inherit a shell's `PATH`. `locate` takes a file the person chose first, then `/opt/homebrew/bin`, `/usr/local/bin`, `~/.opencode/bin` and `~/.local/bin`, then each directory of `PATH` (`Host::search_dirs`). A chosen file that is not a runnable file is skipped.

`Profile::check` (and `check_version`, which it uses) runs `<opencode> --version` with the locked environment and a 5 s timeout and reads the first word that is a version (`opencode v2.1.0` and `2.1.0` both read). It never installs, upgrades or runs `npx`, and stores no keys; signing in is `opencode auth login`, outside the app.

| State | When | Starts? |
| --- | --- | --- |
| `NotInstalled` | Nothing found, it doesn't run, exits with an error or times out. | No |
| `Unsupported { found, need }` | Below 2.1.0, 3.0.0 or above, or output that isn't a version. | No |
| `Newer { found, checked }` | Above `checked` (2.1.3, the version whose source was read; the live check replaces it) and below 3.0.0. A warning. | Yes |
| `Supported { found }` | 2.1.0 up to `checked`. | Yes |

### The scripted agent

`scripted::serve` is an ACP agent that follows a script and reads through the MCP endpoint it is handed, as the real agents do. It runs in process over a pipe (`scripted::in_process`; behind the `scripted` feature, which the dock's UI tests and `--fixture` turn on) and as the `fake_acp_agent` binary for the tests that need a process; the app bundle never ships the binary. The prompt picks what it does: `slow` thinks until cancelled, `flood` sends 10 MiB of thoughts, `crash` drops the connection mid-prompt, `environment` reports its arguments, environment and directories, `grandchild` starts a process of its own, `tools` asks to run each of the calls an agent must be refused (a shell, a file read, an edit, a web fetch, another server's tool, a name that contains ours, a shell whose input or later update says it is ours, and a call with no name), `model` reports the model it uses, and anything else reads a Deployment through MCP and then asks to run a shell command. `--never-initialize` makes the binary never answer `initialize`. It reports two models by default, the second of them local.

### Tests

In `crates/freshkube-agent`; none runs opencode.

| Test | Where |
| --- | --- |
| `only_freshkube_tools_are_allowed` | `tests/acp.rs`, with `acp/permission.rs` |
| `the_token_reaches_the_agent_only_in_session_new` | `tests/process.rs` |
| `the_child_gets_the_locked_environment` | `tests/environment.rs` |
| `the_opencode_profile_locks_down` | `profile/opencode.rs` |
| `version_states` | `profile/version.rs`, with the process tests beside it |
| `opencode_is_found_where_a_gui_app_can_find_it` | `profile/locate.rs` |
| `cancel_ends_the_prompt_within_500_ms` | `tests/acp.rs` (in process) and `tests/process.rs` |
| `dropping_the_session_ends_the_child_and_its_group` | `tests/process.rs` |
| `an_agent_that_never_initializes_fails_within_the_bound` | `tests/acp.rs`, on the test clock |
| `the_transcript_stays_bounded` | `investigation.rs`, and a 10 MiB flood in `tests/acp.rs` |
| `the_model_list_and_choice_reach_the_agent` | `tests/acp.rs` |
| `a_crash_mid_prompt_ends_as_failed` | `tests/acp.rs` and `tests/process.rs` |
| `a_shell_whose_input_says_it_is_ours_is_refused`, `a_call_that_read_as_not_ours_stays_not_ours`, `a_call_with_no_name_is_refused_whatever_its_title_says` | `acp/permission.rs`, and the `tools` prompt in `tests/acp.rs` |
| `only_a_passing_check_gives_the_proof_a_start_needs` | `profile/version.rs` |
| `the_token_does_not_reach_the_debug_log` | `tests/log.rs`, with `the_agent_libraries_stay_at_info_at_every_level` in `src/main.rs` |
| `alternating_chunks_keep_the_entry_count_bounded` | `investigation.rs` |
| `the_own_mode_keeps_every_xdg_directory_inside_the_owned_one`, `a_shared_start_is_refused_when_sources_are_saved` | `profile/opencode.rs` |
| `the_document_has_the_shape_opencodes_schema_accepts` (with `profile/schema_copy.rs`), `a_tampered_plugin_is_refused`, `a_missing_or_unreadable_plugin_is_refused`, `a_start_with_a_changed_plugin_never_starts_the_agent`, `the_plugin_text_is_fixed_and_reaches_nothing` | `profile/opencode.rs` |
| `the_group_ends_when_the_parent_dies_without_ending_it` (Linux) | `child.rs` |

A stub MCP server in `tests/support/` stands in for the endpoint. The end-to-end test through the real endpoint lands with D1c.

### The live check

First run by the lead on a Mac, at `0ed1df5`, with opencode 2.1.3 signed in to a model provider, an in-process stub for the endpoint and invented data (no cluster). **It failed**, and the live acceptance check is not counted as passed. Rows 2 and 3 were repeated on 23 October 2026 at `7b5f670a5adf2280a67689f5b20c02dcf136fdf3` (main after #958), using the same setup, shared provider login and `openai/gpt-6.1-sol`, with no cluster, kubeconfig or context. They still did not pass. Killing the harness again ended opencode and its private server, in 0.036 s:

| Check | Result |
| --- | --- |
| The profile's permissions and `default_agent` as opencode reads them | The private `opencode api --standalone config.get` matched the profile's document, all seven agents included. `opencode debug config` and `debug agents` attach the shared service and showed the person's own configuration; they are not what an agent session uses. |
| opencode refuses a shell, a file read and a fetch | Still not demonstrated on the repeat: prompted separately to run `printf 946-shell`, read `/etc/hosts` and fetch `https://example.com`, the model reported `shell`, `read` and `webfetch` unavailable. For each: permission request none, answer none; the ACP observer recorded zero permission exchanges. These are unavailable-tool responses, not observed permission refusals (#946). |
| The first MCP `tool_call.name` is `freshkube_get_object` | Failed on the repeat: the model reported an empty tool list, the ACP observer recorded zero `tool_call` notifications, and the stub received zero tool calls. `execute` was also reported unavailable. Although the profile supplied `mcp.freshkube.codemode: false`, the private server's parsed inline document contained no `mcp` field; opencode 2.1.3's [configuration schema](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/config/mcp.ts) expects `mcp.servers`, and its [server entries](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/mcp.ts) require a transport. #958 has not established that the four tools are offered directly (#946). |
| An organisation login, then a refused start | Skipped; it needs an organisation sign-in. |
| `opencode --version` loads no config and uses no network | Passed under a sandbox that denied both. |
| Project configuration is off | Passed: both `OPENCODE_CONFIG_PROJECT_DISABLE` and `OPENCODE_DISABLE_PROJECT_CONFIG` work on 2.1.3. |
| Killing the app ends opencode and its server | Passed: both processes were gone in 0.12 s. |
| Sockets | Passed for the observed processes: one listener, on 127.0.0.1, loopback connections to the endpoint, and one remote connection, to the model provider over HTTPS. |

`checked` stays 2.1.3.

**Next run, with the plugin.** In addition to the rows above, it must show: `config.get` lists the one plugin directory under `plugins`; the model is offered `freshkube_list_objects`, `freshkube_get_object`, `freshkube_pod_logs` and `freshkube_list_events` and nothing else, and the first call is named `freshkube_get_object`; `execute`, a shell, a file read and a fetch are still refused or unavailable; and the plugin directory is gone once the session ends. It cannot be run without a Mac and a sign-in.
