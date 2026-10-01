# Future semantic engine and product ideas

This backlog holds the broader Kubernetes IDE direction. The tasks are proposals, not implemented capabilities. They follow the steps in [the roadmap](ROADMAP.md); activate one deliberately rather than because the roadmap's list ran out.

The long-term product should help answer three related questions: why an application is unhealthy, who controls its durable configuration, and what changed before the problem appeared. Rubick provides useful behavioral references, mapped in [the source guide](REFERENCES.md#rubick-concepts-to-revisit-later). Implement the concepts independently under the current source-reuse policy.

## Dependencies and recommended sequence

Every F task follows the roadmap's Kubernetes browsing steps. The table lists additional prerequisites. Completion checkboxes live in each task below; all are currently unchecked.

| Task | Additional prerequisites | Result |
| --- | --- | --- |
| F01 | None | Observation, evidence, and diagnostic contracts |
| F02 | F01 | Shared resource store and relationship indexes |
| F03 | F01, F02 | Consistent workload health and useful log targets |
| F04 | F01, F02 | Ownership, delivery, and reconciliation relationships |
| F05 | F02, F03 | Request-specific traffic diagnosis |
| F06 | F03, F04 | Reviewed changes and observed outcomes |
| F07 | F03, F04, F05 | Integration contracts based on real analyzers |
| F08 | F07 | One deep operator investigation |
| F09 | F02, F03 | Observed change history and replay |
| F10 | F02 | Simultaneous cluster sessions and bounded global search |
| F11 | F03, F04, F05 | Shared semantic queries for CLI and MCP |

The first semantic slice is F01 → F02 → F03, followed by F04 and F05. Complete a route-to-container-to-delivery-owner investigation before expanding the integration catalog. F06 adds mutation deliberately; F09 through F11 can be selected independently once their prerequisites are ready.

## F01 Model observations and evidence

The proposed `ResourceView` should be a projection over observations. It should expose meaningful findings without forcing every row to load all related objects or hold a cloned full resource.

- [ ] Define resource identity separately from request API version and observed object incarnation.
- [ ] Separate operational state from observation state: health/progress/suspension is distinct from loading, stale, forbidden, unsupported, or unavailable data.
- [ ] Define evidence references containing object identity, resource version, relevant field paths/values, and observation time. Keep controller-reported facts, derived conclusions, and hypotheses distinguishable.
- [ ] Represent multiple findings with severity, evidence, affected resources, and available next actions. Avoid a single free-form reason string as the complete diagnostic model.
- [ ] Keep transport, localization, and GPUI rendering outside the pure interpretation model. Pass an explicit clock where time affects a rule.
- [ ] Construct original tests for unknown versus absent, stale previously healthy observations, and unsupported condition conventions.

Acceptance: a finding can explain its evidence without a GPUI runtime or cluster connection. An unreadable collection cannot produce a confident "none exist" conclusion. Freshness limitations survive summarization into a table badge.

- [ ] F01 accepted

Evidence: not started.

## F02 Build the shared resource store

Extend the resource store behind the Resources page only where the semantic slice needs more than its table rows. Start with ordinary in-memory maps and indexes rather than a graph database.

- [ ] Introduce shared list/watch subscriptions keyed by connection, credentials/session, group/version/resource, scope, and selectors. Record read coverage and initial synchronization state.
- [ ] Preserve full or suitably retained observations for selected diagnostic inputs. Define memory budgets, eviction, redaction, and on-demand loading rather than caching every kind and Secret indiscriminately.
- [ ] Index owner references, labels/selectors, named references, and Service/EndpointSlice associations. Update affected indexes on deletion and object recreation.
- [ ] Invalidate derived results based on changed inputs. Avoid recomputing or refetching the entire resource graph for a one-pod update.
- [ ] Handle reconnect, incomplete reads, unsupported APIs, permission changes, and watch resync explicitly. Coalesce compatible subscribers without erasing coverage differences.
- [ ] Preserve per-input evidence and times; do not imply that observations from different kinds form an atomic cluster snapshot or compare unrelated resource versions numerically.

Acceptance: table, inspector, and a diagnostic query share observations. Opening multiple consumers does not multiply equivalent watches. Incomplete knowledge remains visible, and watch replay produces a reproducible final state.

- [ ] F02 accepted

Evidence: not started.

## F03 Derive consistent workload health

Replace the separate display rules with shared analyzers, starting with pods and Deployments. Preserve raw phase and conditions for inspection.

- [ ] Derive pod findings from application/init container states, sidecar semantics, readiness, scheduling conditions, termination, and deletion state.
- [ ] Distinguish intentional completion or scale-to-zero from failure, and ongoing rollout progress from an established failure.
- [ ] Use controller generation observations where the API actually supplies them. Do not assume every CRD reports `observedGeneration` or the same Ready condition.
- [ ] Use restart history and recent observations carefully. A lifetime restart count alone does not establish a current failure; exit code 137 alone does not establish OOM as the cause.
- [ ] Produce a recommended log target with an explanation: failing init container, restarting application container, or previous instance when supported by evidence. Preserve manual choice.
- [ ] Use identical findings in list, inspector, search summaries, and later headless queries.

Acceptance: the same observed input has the same health interpretation everywhere. Tests cover successful Jobs, restartable init sidecars, stale conditions, readiness gates, and incomplete container status.

- [ ] F03 accepted

Evidence: not started.

## F04 Explain ownership and reconciliation

Ownership is a set of relationships. Kubernetes controller ownership, Helm association, GitOps delivery, field management, and source provenance can coexist on the same object.

- [ ] Represent typed relationships with evidence and confidence. Resolve controller chains by UID where available and preserve missing/unreadable links.
- [ ] Detect Flux, Argo CD, and Helm claims from metadata, then confirm them through the relevant controller inventory/tracking information when readable.
- [ ] Distinguish historical association, claimed delivery, confirmed delivery, suspended reconciliation, and unknown behavior. A Helm label alone does not establish an active reconciler.
- [ ] Read controller-specific settings before predicting whether a live change will be restored. Field ownership is useful evidence but does not by itself prove a controller's next action.
- [ ] Offer navigation to the controlling resource, known repository/path/revision, and source file only when the evidence establishes that location.
- [ ] Batch owner resolution so a table column does not issue one API call per row.

Acceptance: a Deployment can expose its controller/delivery/field-management relationships together, and the UI explains where a durable change belongs without overstating what labels prove.

- [ ] F04 accepted

Evidence: not started.

## F05 Diagnose a traffic path

Begin with Service endpoints and one HTTPRoute workflow. Treat routing configuration and workload ownership as related branches. EndpointSlices describe destinations; they are not a literal network appliance in the packet path.

- [ ] Resolve Service ports, named target ports, EndpointSlices, and referenced pods. Handle multiple slices, external endpoints, missing target references, and unsupported backends explicitly.
- [ ] Diagnose a specific hostname/protocol/port/path where applicable. Read route attachment status per parent/listener/controller and the observed generation.
- [ ] Add GatewayClass, listener, allowed-route, backend reference, and cross-namespace authorization evidence. Discover served API versions rather than assuming every route kind is installed.
- [ ] Interpret endpoint `ready`, `serving`, and `terminating` together with Service behavior such as `publishNotReadyAddresses`.
- [ ] Represent partial traffic failure and weighted backends. A broken branch must not automatically mean every request fails.
- [ ] Show what configuration implies, what controllers report, and what an optional probe observed as separate evidence. A port-forward probe does not verify the public ingress path.
- [ ] Extend to GRPCRoute, TCPRoute, TLSRoute, UDPRoute, and Ingress after the HTTPRoute contract is demonstrated, giving each protocol its own relevant checks.

Acceptance: a user can investigate an HTTP request, identify the earliest established problem or knowledge gap, open affected pod logs, and follow the ownership branch. Refused reads and untested reachability never become green results by default.

- [ ] F05 accepted

Evidence: not started.

## F06 Plan changes and observe their outcomes

Add mutations only after the app can explain the selected target and its known reconciliation relationships. The API accepting a change is distinct from the desired outcome occurring.

- [ ] Introduce a change plan with explicit cluster/object identity, current observation, intended field changes, relevant ownership evidence, and expected outcome.
- [ ] Show a diff, perform supported server-side dry-run validation, and preserve field conflicts for review. Do not inherit Kubeli's unconditional forced-apply default.
- [ ] Use operation-appropriate concurrency checks, including object UID and resource-version preconditions where supported. Rebuild an obsolete plan rather than silently applying it to a replacement object.
- [ ] Separate ordinary apply from explicitly chosen ownership takeover. Do not promise that dry-run predicts controller behavior or every runtime outcome.
- [ ] Observe the requested rollout/reconciliation and report convergence, reversal, failure, timeout, or loss of visibility.
- [ ] Begin with one bounded operation such as a controlled Deployment field change. Add wider actions after that path is proven on disposable workloads.

Acceptance: the user can inspect the proposed effect and known reconciliation risk, then distinguish an accepted request from a verified outcome. Competing updates and object recreation cannot silently redirect the operation.

- [ ] F06 accepted

Evidence: not started.

## F07 Establish integration contracts

Extract contracts from the concrete health, delivery, and traffic analyzers. A list of capability names is insufficient without typed queries and availability information.

- [ ] Separate API discovery, pure semantic analysis, external provider I/O, and action planning.
- [ ] Define resource-scoped capabilities and states such as unsupported, not configured, unavailable, forbidden, and ready. Include provider identity and evidence freshness.
- [ ] Keep batching, cancellation, request budgets, credentials, and caches in the runtime. An analyzer should not start hidden per-row network calls.
- [ ] Add a generic CRD inspector using discovery, schemas, printer columns, and conditions with explicit limits on interpretation.
- [ ] Introduce Prometheus-compatible history and persistent-log providers only when a concrete workflow uses them. Keep configured endpoints distinct from detected in-cluster APIs.
- [ ] Retain ordinary compiled Rust modules until multiple implementations demonstrate a stable plugin boundary.

Acceptance: a consumer can request a supported capability without importing vendor implementation code, while identifying which provider answered and why another capability is unavailable.

- [ ] F07 accepted

Evidence: not started.

## F08 Build one deep operator workflow

Choose one operator based on the environment used for development. Current Rubick already has Cilium and CloudNativePG functionality; the goal is a coherent investigation with stronger evidence or useful connections.

- [ ] For CloudNativePG, investigate primary/replica state, readiness, archiving/backup status, and storage dependencies using documented operator evidence. Avoid inferring database durability solely from healthy pods.
- [ ] Alternatively, for Cilium, join policy validity, selected endpoints, and coverage. Add separately sourced runtime flow/drop evidence if available; static policy selection alone does not prove connectivity.
- [ ] Reuse the same findings, resource navigation, log targets, and evidence presentation as core resources.
- [ ] Cover unsupported versions, absent conditions, limited RBAC, and unavailable external evidence with original fixtures.
- [ ] Record which operational question the workflow answers that a generic CRD table cannot.

Acceptance: complete one operator investigation end to end before adding another vendor. KRO, Crossplane, Kargo, Karpenter, and Talos remain candidates requiring their own contracts, API/credential assessment, and concrete use case.

- [ ] F08 accepted

Evidence: not started. Selected operator: undecided.

## F09 Retain observed change history

- [ ] Maintain a bounded history of relevant spec changes, revisions, condition transitions, and resource events during an investigation.
- [ ] Preserve timestamps, observation gaps, and clock limitations. Do not treat local receipt order as a complete causal ordering across controllers.
- [ ] Correlate rollout, readiness, restart, and source-revision changes while labeling inferred associations explicitly.
- [ ] Export a redacted investigation snapshot and replay it through the same semantic rules for offline debugging and regression coverage.
- [ ] Define retention, storage limits, and sensitive-field handling before enabling persistence.

Acceptance: an investigation can show what the app actually observed before a failure and where its history is incomplete. Replay preserves findings without reconnecting to the original cluster.

- [ ] F09 accepted

Evidence: not started.

## F10 Support simultaneous clusters and global search

- [ ] Replace the one-active-session policy with an explicit session registry. Bind tabs, subscriptions, credentials, and actions to their session rather than a global selected context.
- [ ] Return indexed cached results immediately, then perform bounded discovery/search in selected clusters.
- [ ] Cancel superseded searches and display independent cluster completion, failure, and coverage. An unavailable cluster is not a zero-result cluster.
- [ ] Share compatible watches and set connection/query/memory budgets so opening a workspace cannot watch every kind in every cluster accidentally.
- [ ] Test same-named objects and contexts, reconnects, credential changes, and late responses across sessions.

Acceptance: two cluster workspaces remain correct while the user switches focus, and global search identifies the clusters and scopes it actually searched.

- [ ] F10 accepted

Evidence: not started.

## F11 Expose shared explanations through CLI and MCP

- [ ] Add read-only semantic queries such as explain resource health, trace a request path, and inspect reconciliation relationships.
- [ ] Reuse the same analyzers and evidence contracts as the GUI. Do not create a separate health heuristic in the headless adapter.
- [ ] Make cluster, namespace, query scope, and coverage explicit. Bound output and provide references or pagination for large evidence sets.
- [ ] Verify that equivalent observations produce equivalent findings across GUI and headless callers. Disclose different observation times or sessions when answers differ.
- [ ] Consider mutation access only after F06, as a separately scoped workflow with the same change-plan semantics.

Acceptance: a human or tool consumer can obtain the same explainable findings without a GPUI runtime. Natural-language summaries may elaborate those findings but must not silently replace deterministic health or traffic verdicts.

- [ ] F11 accepted

Evidence: not started.
