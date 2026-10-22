# The agent in the dock

The person's own agent investigates a cluster for them (issue [#874](https://github.com/skel84/freshkube/issues/874), part D1). It runs as a child process speaking ACP over stdio and reads only through Freshkube's four read tools. This page covers slice D1b (issue [#907](https://github.com/skel84/freshkube/issues/907)): the ACP client, the opencode profile and its lock-down, the version check and the scripted agent. The endpoint, the access gate and `agent_reads` are slice D1a, the dock tab is D1c and following on the page is D1d; they will add their parts below.

Code:

- `crates/freshkube-agent/src/`: `acp/` (`mod.rs` the session, `permission.rs` what is allowed, `updates.rs` session updates to events, `lines.rs` the line bound), `investigation.rs` (the transcript), `profile/` (`mod.rs` profiles, the locked environment and the launch, `opencode.rs` the lock-down, `locate.rs`, `version.rs`), `child.rs` (the process group and what ends it), `tools.rs` (the server and tool names), `scripted/` (the scripted agent), `bin/fake_acp_agent.rs` (that agent as a process).
- `crates/freshkube-agent/tests/`: `acp.rs` (in process), `process.rs`, `group.rs` and `environment.rs` (the scripted binary), `log.rs` (the token and the debug log), `support/` (the stub for the endpoint).

The crate has no GPUI and names nothing of desktop, `freshkube-ui` or the feature crates. Desktop does not depend on it yet, so the app starts no agent. The scripted agent and the MCP client it needs are behind the `scripted` feature, which the crate's own tests turn on from a dev-dependency; the app's build has neither.

## The ACP client

`AcpSession::start(link, endpoint)` takes a `Link` (both ends of the agent's stdio, from `Profile::spawn` or `scripted::in_process`) and an `Endpoint`, the MCP endpoint's URL and its token. It returns the session and a receiver of `investigation::Event`s.

- **Startup.** `initialize`, then `session/new` with one HTTP MCP server: the name `freshkube`, the URL, and an `Authorization: Bearer <token>` header. Startup has a 30 s bound (`STARTUP_TIMEOUT`); past it the turn ends as failed with "opencode didn't start" and the child is ended with the session.
- **The token** goes only into `session/new`, over the child's stdin. It is never in the argument list, the environment, a file, a log or `Debug` (`Endpoint`'s `Debug` prints `Token(..)`). The ACP library logs every message it sends at debug level, headers included, so the app's logger keeps `agent_client_protocol` and `rmcp` at info whatever its level (`log_filter` in `src/main.rs`, `QUIET_LOG_TARGETS`); `tests/log.rs` shows the token in a debug log without the directives and not with them. The client advertises no file system and no terminal, so the agent has none to ask for.
- **Prompts** run one after another in one session (`prompt`). `cancel` stops the running prompt, drops the ones waiting and gives the agent 5 s (`CANCEL_GRACE`) to answer; after that the child is ended and the turn ends as cancelled.
- **A crash** (the child exits or closes its output) while a prompt runs ends that turn as `Failed`, once; later calls return `Closed`.
- **Bounds on input.** A line from the agent is at most 4 MiB (the connection fails past it). Events wait in a queue of 1,024 and the agent is slowed to the reader's pace, so the reader must drain it. The transcript bounds are below.
- **A start needs a passing version check.** `Profile::spawn` takes the `Checked` that `Profile::check` gives, which only a supported or newer-than-checked version earns, and refuses one made on another binary. Another opencode major version may read its permissions under other keys and fall back to defaults that allow a shell.
- **Models.** `models()` gives the list the agent offers and the current one, and `set_model(id)` chooses one of them (an id the agent didn't offer is refused before anything is sent). A model is marked local when its provider is `ollama`, `lmstudio`, `llama.cpp`, `llamacpp` or `local`.

### Which tools are allowed

`acp/permission.rs` answers every permission request. A call is allowed only by its **name**, an exact match of server `freshkube` and one of the four tool names (`list_objects`, `get_object`, `pod_logs`, `list_events`; `tools.rs` copies the design's list until `agent_reads::TOOLS` exists): `freshkube_get_object` (how opencode names an MCP tool), or `mcp__freshkube__get_object` or `mcp.freshkube.get_object`, which other agents use.

- **Not the title and not the input.** In opencode 2.1 a shell call's title is its command, an update carries no name, and a call's input is the model's own text, so a shell whose input says `{server: "freshkube", tool: "get_object"}` is still a shell.
- **A refusal is final.** Once a call has read as not ours, no later update and no request makes it ours. A call with no name anywhere is refused, and so is an id nothing announced. A request that names something other than what the call announced is refused.
- **Everything else is refused,** with a transcript note "Refused: …" naming the call as it was announced: built-in tools, shell and exec, file reads, edits, web fetch and search, subagents, other MCP servers, and a name that only contains ours (`freshkube_get_object_extra`, `other_freshkube_get_object`).

The name is still the agent's own word. The boundary is the lock-down below, which leaves the agent no other tool; this handler is the second line. This replaces the spike's substring match.

## The transcript

`investigation::Investigation` holds what was asked, thoughts, speech, steps (their state and arguments, updated in place as a tool call is described) and notes. Everything the agent sends is bounded:

| Bound | Value | When it is reached |
| --- | --- | --- |
| Text in the transcript | 1 MiB, in at most 2,000 thought and speech entries | The oldest thought text is dropped first, then the oldest speech. An entry left with no text is removed, so the entries are bounded in memory too; `trimmed()` says that earlier text is gone, and a partly cut entry is marked `trimmed`. |
| Shown arguments of a step | 4 KiB | Cut on a character; `Args::is_cut` says so. |
| Steps | 1,000 | Later steps are refused with one note. |
| One chunk | 64 KiB | Cut. |
| Notes | 1,000 | Later ones are dropped. |

The 1 MiB covers text. Steps are bounded separately, at most 1,000 × 4 KiB of arguments. The follower that moves the page is not here; it comes with D1d.

## Profiles

A profile is a launch command, an environment, a lock-down and a supported version range. Every agent starts from a cleared environment, then gets:

- a minimal `PATH` (the system directories and the agent's own, so a script that starts `node` beside it runs), `HOME` and `LANG`;
- an empty temporary config home (`XDG_CONFIG_HOME`);
- `KUBECONFIG` and `TALOSCONFIG` pointing at empty temporary files;
- `TMPDIR` in the session's directory, and an empty temporary working directory.

The directories are one `Dirs` per session, removed when the process group is gone. No provider key, cloud credential or cluster credential is inherited. The child runs in its own process group; dropping the session sends SIGTERM to the group and SIGKILL after 2 s if anything is left.

- **Unix.** If the app dies without dropping the session, a small shell in a group of its own (started with the agent) sees the app's pid go and ends the agent's group, within a few seconds. `child::tests` kills a stand-in parent and checks the group goes.
- **Windows.** The child is created suspended, put in a job object that kills its processes when the job closes, and only then resumed, so nothing it starts is outside the job; if it can't be put in the job it is killed unstarted. The job closes with the app. `tests/group.rs` runs on Windows in CI and checks that a grandchild goes with the session.

### The opencode profile

The command is `opencode acp`. The configuration is handed inline in `OPENCODE_CONFIG_CONTENT`; `profile/opencode.rs` builds it (`config()`) and the environment (`environment()`). A guard inside a plugin is never counted as the boundary.

**What was read.** The source of opencode 2.1.3 (commit [`e965774`](https://github.com/anomalyco/opencode/tree/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b)), and for each switch below the same file at 2.1.0 ([`d30e990`](https://github.com/anomalyco/opencode/tree/d30e990c69efd8a954413a724b35c5a27462f320)), which is the floor of the range. Nothing here has been run against opencode; the live check is the lead's.

| Switch | Where opencode reads it |
| --- | --- |
| `acp` starts a private server | [`handlers/acp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/commands/handlers/acp.ts) calls `Standalone.start()` unconditionally and has no flag to attach to the shared service; [`standalone.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/services/standalone.ts) runs `serve --stdio --port 0` with a random `OPENCODE_PASSWORD`. The shared service is only reached by `resolve` in [`server-connection.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/services/server-connection.ts), which `acp` does not call. |
| The private server is on loopback | [`server-process.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/server-process.ts): the host defaults to `127.0.0.1`, and port `0` is an ephemeral port. It is a **listener on loopback**, behind that password, not no listener; it ends when its stdin closes. |
| No pairing or tunnel | `server-process.ts` starts the remote tunnel only in `service` mode, from the service configuration (`config.remote`). |
| `permissions`: deny all, allow our four | Ordered rules, the last match wins, no match asks ([`permission.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/permission.ts) `evaluate`). A tool whose last matching rule is `*` deny is not offered to the model ([`tool.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool.ts) `whollyDisabled`). An MCP tool's action is `<server>_<tool>` ([`tool/mcp.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool/mcp.ts) `name`). |
| Built-in tool names | `shell` (1.x called it `bash`), `read`, `grep`, `glob`, `webfetch`, `websearch`, `subagent` (1.x: `task`), `skill`, `todo`, `question`, `edit` (also what `write` and `patch` ask), and `opencode_list_mcp_resources` and `opencode_read_mcp_resource`, in [`tool/plugin/`](https://github.com/anomalyco/opencode/tree/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/tool/plugin). The profile denies each by name and everything else with `*`. |
| `agents.<name>.permissions` for every built-in agent, and `default_agent` | An agent's own rules come after the global ones in the order, so a source loaded after ours that sets them would win. The same list is written under each of `build`, `plan`, `general`, `explore`, `compaction`, `title` and `summary` ([`schema/config/agent.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/config/agent.ts), [`plugin/agent.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/plugin/agent.ts)). |
| `plugins`, `mcp`, `commands`, `skills`, `instructions`, `references`, `lsp`, `formatter`, `websearch`, `snapshots`, `update`, `share` | The config document in [`schema/config.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/schema/src/config.ts). The empty and false values are starting values, not removals: a source that loads after ours can still add to them. Our MCP server comes in `session/new`, which opencode accepts over HTTP ([`acp/service.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/acp/service.ts) `supportedMcpServers`). |
| `OPENCODE_CONFIG_CONTENT` | `server-process.ts` (`config.content`). |
| `OPENCODE_CONFIG_DIR`, `XDG_CONFIG_HOME` empty | The global config directory ([`util/global.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/util/src/global.ts), [`global-roots.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/util/src/global-roots.ts)). |
| `OPENCODE_CONFIG_PROJECT_DISABLE`, `OPENCODE_DISABLE_CLAUDE_CODE` | `server-process.ts` (`config.project`, `config.claude`). No project configuration, and no instructions or skills read from other coding tools' directories. `HOME` is empty and so is the working directory, so no instruction file is found beside them. |
| `OPENCODE_DISABLE_AUTOUPDATE` | [`services/updater.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/services/updater.ts); `update: "disable"` in the configuration as well. |
| `OPENCODE_DISABLE_FILEWATCHER`, `OPENCODE_DISABLE_MODELS_FETCH` | `server-process.ts` (`fs.filewatcher`, `models.fetch`). With the fetch off, the model list comes from the catalogue bundled in the binary ([`models-dev.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/models-dev.ts) `bundledSnapshot`), so no connection goes to the catalogue's host. |
| `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` | Logins, session history and the saved organisation config sources live in the database under `$XDG_DATA_HOME/opencode` ([`credential.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/credential.ts), [`wellknown.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/wellknown.ts) `wellknown:sources`). See the data modes below. |
| Admin-owned configs | The system directory (`/Library/Application Support/opencode`, `/etc/opencode`, `%ProgramData%\opencode`) and macOS managed preferences load after the inline config and win ([`config/discovery.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/core/src/config/discovery.ts)). The profile does not change them; an administrator who sets them owns what they allow. |

`profile/opencode.rs` carries the same table as `LOCKDOWN_SOURCES`, and a test fails if a variable the profile sets has no source in it.

#### Where opencode keeps its data

`Profile::opencode(binary, data)` takes a `Data`:

- **`Own(dir)`, the default.** A directory Freshkube owns (`opencode/` under the app's data directory, created with mode 0700). `XDG_DATA_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` are `data/`, `state/` and `cache/` inside it. The person signs in to their provider once inside it, and their own opencode history and saved organisation configs are never read or written.
- **`Shared`.** Their real directories, as opencode itself would use them: their logins are there, the investigations land in their opencode history, and any organisation config saved there loads. Which mode runs is `Profile::data()`; the setting lives in `agent.json` beside the grants (D1a), never in `freshkube.yaml`, and the dock's wording that the history and saved configs are shared comes with the dock tab (D1c).

An organisation config source (opencode's "wellknown" sources) is loaded after the inline configuration, fetches its URL when opencode starts and every ten minutes, and its per-agent rules win over ours. `Shared` therefore **refuses to start** if `organisation_sources` sees one: it looks in opencode's database files under the data directory (`opencode*.db*`) for the key the sources are saved under, `wellknown:sources`. It finds the key, not its value, so a source that was removed and left the key behind still refuses (the message says to use Freshkube's own directory). When a file can't be read or is past the 1 GiB scan limit it cannot tell, and the start goes ahead; the live check on an install with sources saved is what shows the sources load at all. `Own` never reads the real directory.

`opencode acp` has no `--standalone` flag (it is always standalone), so the profile passes none. `opencode models` and `opencode run` take it and would otherwise attach the shared service; the client never runs them.

**Models.** opencode reports them in ACP's session setup, as a `model` config option (category `model`, values `provider/model`; [`acp/config-option.ts`](https://github.com/anomalyco/opencode/blob/e965774fa0e2a8ffb27bf02827a2c3c6a8c0f45b/packages/cli/src/acp/config-option.ts)), and changes the model with `session/set_config_option`. The client uses that; `opencode models` is not run.

**Not confirmed.** Source reading cannot show these; the live check does: that `deny *` leaves the four tools offered, that the permission request names `freshkube_get_object` as read above, that the model list is not empty with the fetch off, and that nothing is read from the real home. Windows is written against the documented variables and not run.

### Locating opencode and the version check

A GUI app doesn't inherit a shell's `PATH`. `locate` takes a file the person chose first, then `/opt/homebrew/bin`, `/usr/local/bin`, `~/.opencode/bin` and `~/.local/bin`, then each directory of `PATH` (`Host::search_dirs`). A chosen file that is not a runnable file is skipped.

`Profile::check` (and `check_version`, which it uses) runs `<opencode> --version` with the locked environment and a 5 s timeout and reads the first word that is a version (`opencode v2.1.0` and `2.1.0` both read). It never installs, upgrades or runs `npx`, and stores no keys; signing in is `opencode auth login`, outside the app.

| State | When | Starts? |
| --- | --- | --- |
| `NotInstalled` | Nothing found, it doesn't run, exits with an error or times out. | No |
| `Unsupported { found, need }` | Below 2.1.0, 3.0.0 or above, or output that isn't a version. | No |
| `Newer { found, checked }` | Above `checked` (2.1.3, the version whose source was read; the live check replaces it) and below 3.0.0. A warning. | Yes |
| `Supported { found }` | 2.1.0 up to `checked`. | Yes |

## The scripted agent

`scripted::serve` is an ACP agent that follows a script and reads through the MCP endpoint it is handed, as the real agents do. It runs in process over a pipe (`scripted::in_process`, example code, not behind a feature, so the dock's UI tests and `--fixture` can use it) and as the `fake_acp_agent` binary for the tests that need a process; the app bundle never ships the binary. The prompt picks what it does: `slow` thinks until cancelled, `flood` sends 10 MiB of thoughts, `crash` drops the connection mid-prompt, `environment` reports its arguments, environment and directories, `grandchild` starts a process of its own, `tools` asks to run each of the calls an agent must be refused (a shell, a file read, an edit, a web fetch, another server's tool, a name that contains ours, a shell whose input or later update says it is ours, and a call with no name), `model` reports the model it uses, and anything else reads a Deployment through MCP and then asks to run a shell command. `--never-initialize` makes the binary never answer `initialize`. It reports two models by default, the second of them local.

## Tests

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
| `the_group_ends_when_the_parent_dies_without_ending_it` (Linux) | `child.rs` |

A stub MCP server in `tests/support/` stands in for the endpoint. The end-to-end test through the real endpoint lands with D1c.

## The live check

Not run yet. The lead runs it once, on a machine with opencode installed and signed in, with example data only (no cluster, kubeconfig or talosconfig), after both D1a and D1b have landed, through the `live` example (`cargo run -p freshkube-agent --example live`, which lands with whichever slice merges second):

1. Run the example; it starts an opencode session on the endpoint with the canned `shop`/`checkout` scenario and prints the transcript.
2. While it runs, list the sockets of the opencode process and its descendants (`lsof -a -i -P` over their pids). Expected: no listener except on 127.0.0.1, and connections only to the endpoint's loopback port and the model provider over HTTPS.
3. Prompt the agent to run a shell command, read `~/.kube/config` and fetch a URL. Expected: each is refused, with a note in the transcript, and nothing is read or run.
4. Record the version as the profile's `checked` (`SUPPORTED` in `profile/opencode.rs`) and post the results on the issue: the version, the sockets reduced to listen and remote kinds, and the refusals; no logins, account names or tokens.

The result goes here when it is in.
