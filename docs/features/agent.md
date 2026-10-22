# The agent's reads

What an agent may read of a cluster, and the way it reaches those reads. This is slice D1a of the agent in the dock ([#874](https://github.com/skel84/freshkube/issues/874), [#906](https://github.com/skel84/freshkube/issues/906)): the app starts no endpoint and no agent yet, and desktop does not depend on the agent crate.

**Modules.** `crates/freshkube-core/src/agent_reads/` (`call.rs`, `gate.rs`, `read.rs`, `live.rs`, `example.rs`, `shape.rs`, `answer.rs`, `refusal.rs`, `target.rs`); `crates/freshkube-core/src/constants.rs` (`agent`); `crates/freshkube-agent/src/endpoint/`, `session.rs`, `saved.rs`; core's `resources/discovery.rs` (`resolve_kind_key`) and `write/audit.rs` (`AuditStore::named`).

**Proved by.** Core, `agent_reads/tests.rs`: `a_cluster_without_agent_access_refuses_before_any_request`, `turning_access_off_ends_a_read_in_flight`, `a_grant_for_another_context_reads_as_off`, `every_agent_read_is_a_get`, `secrets_are_read_metadata_only`, `managed_fields_and_last_applied_never_reach_the_answer`, `log_reads_are_bounded`, `lists_and_events_are_cut_at_their_limit`, `an_object_over_the_limit_is_too_large`, `call_parse_refuses_what_it_cannot_bound`, `failures_map_to_their_codes`, `example_objects_get_the_same_bounds_and_secret_rule`, `the_five_hundred_and_first_read_is_limit_reached`. Agent, `crates/freshkube-agent/tests/`: `endpoint.rs` (`the_endpoint_listens_on_loopback_only`, `the_endpoint_refuses_without_the_session_token`, `the_token_dies_with_the_session`, `a_fake_agent_client_reads_through_the_endpoint`, `a_refused_read_gets_a_read_id_and_no_view`, `no_view_state_goes_on_the_wire`), `saved.rs` (`set_access_saves_audits_and_gates_together`, a missing or unparsable file, a failed audit) and `logs.rs` (`the_token_never_reaches_debug_or_logs`).

## The tools

Server `freshkube`, exactly four tools; `agent_reads::TOOLS` holds their names and `SERVER_NAME` the server's. An argument the tool does not take is refused. `cluster` is a cluster entry's id and defaults to the session's own.

| Tool | Arguments | Answer |
|------|-----------|--------|
| `list_objects` | `cluster?`, `kind`, `namespace?`, `limit?` 1–200 (100) | `{read, cluster, kind, items: [{namespace, name, uid, status, created}], cut}` |
| `get_object` | `cluster?`, `kind`, `namespace?`, `name` | `{read, cluster, kind, namespace, name, uid, object}` |
| `pod_logs` | `cluster?`, `namespace`, `pod`, `container?`, `previous?`, `tail_lines?` 1–500 (200), `since_seconds?` 1–86400 | `{read, cluster, namespace, pod, container, previous, lines, cut}` |
| `list_events` | `cluster?`, `namespace?`, `kind?`, `name?`, `since_seconds?` 1–86400 (3600), `limit?` 1–200 (100) | `{read, cluster, events: [{type, reason, object, message, count, last}], cut}` |

- `kind` takes what kubectl takes: a key (`deployments.apps`), a plural, a singular or Kind in any case, a short name (`deploy`), with an optional `.group`. `resolve_kind_key` resolves it against discovery; a kind the cluster does not serve (or a name two kinds share) is `not_served`.
- Names are DNS names: a namespace and a container are DNS labels, a name is a DNS subdomain.
- A success is `isError: false` with the JSON in one text block and as `structuredContent`. `cut` is `null` or `{by: items|lines|bytes, limit}`; a cut answer is still a success.
- `pod_logs` reads one pod container's `log` subresource. Without a `container`, the server chooses the pod's default and the answer's `container` is `null`.
- `list_events` asks the server for up to 200 events (by involved Kind and name when given), keeps those newer than `since_seconds`, sorts newest first and keeps `limit`.

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

`isError: true` with `{read, error: {code, message}}`; the message goes through the audit log's `scrub`. `FailureKind` maps onto the code in one match with no wildcard arm (`Refusal::from_failure`); a 404 while resolving a kind or listing one means the API is not served.

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

- **401** without `Authorization: Bearer <token>` (compared in constant time); **403** for any `Origin` header or a `Host` other than `127.0.0.1:<port>`; **413** over 64 KiB. Each is a bare status.
- The token is 32 bytes of OS randomness (`session::Token`); `Debug` and `Display` print nothing of it and `reveal` is the one way to read it. It is never logged.
- The app implements `Targets` to resolve an optional entry id to an `AgentTarget { entry, context, source }`; the source is `Live(ClusterAccess)` or `Example(Arc<dyn ExampleObjects>)`.
- One `tracing` line per read: the short session id, the read id, the entry, the tool, the target and the code. No answer data, no token, no rejected value.

## `agent.json`

`agent.json` sits beside the preferences (`freshkube-prefs`, mode 0600, read to at most 1 MiB), not in `freshkube.yaml`: the setup file is what a person may share, and sending a cluster's data to their model provider is their own consent, as with `follows.json`. A grant is `{entry, context, since}`. A missing file grants nothing; an unreadable or unparsable one grants nothing and says why once.

`Access::set_access(entry, context, on, from)` is the one way to change a grant. It saves the file, appends `{at, entry, context, access, from}` to `<preferences>/audit/agent.jsonl` (mode 0600, rotated like the write audit) and updates the gate. If any step fails the error is returned, the file is put back and the gate is unchanged. Whether a context belongs to the workspace is the caller's to check.
