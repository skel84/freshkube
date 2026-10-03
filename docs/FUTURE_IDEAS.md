# Future semantic engine and product ideas

This backlog holds the broader Kubernetes IDE direction. The tasks are proposals, not implemented capabilities. They follow the steps in [the roadmap](ROADMAP.md); activate one deliberately rather than because the roadmap's list ran out.

The long-term product should help answer three related questions: why an application is unhealthy, who controls its durable configuration, and what changed before the problem appeared. Rubick provides useful behavioral references, mapped in [the source guide](REFERENCES.md#rubick-concepts-to-revisit-later). Implement the concepts independently under the current source-reuse policy.

Keep proposed behavior, priorities, acceptance criteria, and the design constraints needed to make them reliable here. Preserve source observations and their architectural rationale in [the Rubick architecture notes](REFERENCES.md#rubick-architecture-lessons). Decide concrete Rust APIs, module layouts, and state ownership in the relevant feature's design document when implementation starts, following the pattern of [pod exec](POD_EXEC.md) and [port forwarding](PORT_FORWARD.md).

## Dependencies and recommended sequence

Every F task follows the roadmap's Kubernetes browsing steps. The table lists additional prerequisites. Completion checkboxes live in each task below; all are currently unchecked.

| Task | Additional prerequisites | Result |
| --- | --- | --- |
| F01 | None | Observation, evidence, and diagnostic contracts |
| F02 | F01 | Shared resource store and relationship indexes |
| F03 | F01, F02 | Consistent workload health and useful log targets |
| F04 | F01, F02 | Argo CD applications, ownership, and delivery relationships |
| F05 | F02, F03 | Request-specific traffic diagnosis |
| F06 | F03, F04 | Reviewed changes and observed outcomes |
| F07 | F01, F02 | Small integration contracts exercised by Argo CD and CloudNativePG |
| F08 | F03, F04, F07 | CloudNativePG database investigation |
| F09 | F02, F03 | Observed change history and replay |
| F10 | F02 | Simultaneous cluster sessions and bounded global search |
| F11 | F03, F04, F05 | Shared semantic queries for CLI and MCP |

The first semantic slice starts with F01 → F02 → F03 and Argo-first F04. Develop the minimum F07 contracts alongside these analyzers and CloudNativePG in F08. Traffic diagnosis in F05 can follow; completing the routing engine or a general plugin framework is not a prerequisite for the first integrations.

The first combined acceptance scenario is an Argo-managed CloudNativePG database: open its Application, follow the Cluster, identify a failed Backup or unhealthy instance, and reach the relevant events, pod logs, PVC, or node. Offer Talos inspection when that node has an established Talos connection. Keep each hop's evidence and any missing access visible. F04 and F08 must both work for this scenario; neither needs simultaneous background sessions in every destination cluster.

After that slice, prioritize Flux and cert-manager, then Prometheus and Loki for specific metrics and log workflows. The integration rollout below records their scope. F06 adds mutations through reviewed plans; F09 through F11 remain independently selectable once their prerequisites are ready. Selecting these priorities does not mark any implementation complete.

## F01 Model observations and evidence

A resource's diagnostic view should be a projection over observations. It should expose meaningful findings without forcing every row to load all related objects or hold a cloned full resource.

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
- [ ] Build typed analyzer inputs directly from structured API responses before display formatting. Retain the metadata and operator fields each query needs, including field-management evidence when requested and nonstandard top-level fields such as Cilium's `specs`. Keep Secret handling and display redaction explicit.
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

Ownership is a set of relationships. Kubernetes controller ownership, Helm association, GitOps delivery, field management, and source provenance can coexist on the same object. Start with Argo CD application inspection and its delivery relationships; add Flux's distinct source/reconciler workflow after the first integration slice.

- [ ] Represent typed relationships with evidence and confidence. Resolve controller chains by UID where available and preserve missing/unreadable links.
- [ ] Detect Flux, Argo CD, and Helm claims from metadata, then confirm them through the relevant controller inventory/tracking information when readable.
- [ ] Preserve cluster, API group, kind, namespace, name, and observed incarnation when resolving relationships. For Argo, account for the configured tracking method and installation identity; do not collapse same-named objects across groups, namespaces, or destinations.
- [ ] Distinguish historical association, claimed delivery, confirmed delivery, suspended reconciliation, and unknown behavior. A Helm label alone does not establish an active reconciler.
- [ ] Read controller-specific settings before predicting whether a live change will be restored. Field ownership is useful evidence but does not by itself prove a controller's next action.
- [ ] Offer navigation to the controlling resource, known repository/path/revision, and source file only when the evidence establishes that location.
- [ ] Batch owner resolution so a table column does not issue one API call per row.

Argo CD's initial scope uses the selected Kubernetes connection to read `Application`, `ApplicationSet`, and `AppProject` resources. These reads need no separate Argo API token.

- [ ] Add an Applications page showing sync and health separately, destination, sources, revisions, current/last operation, and the controller's resource-specific failure messages. Order failures for investigation without replacing Argo's reported states.
- [ ] Expose managed-resource inventory, ApplicationSet provenance, project repository/destination constraints, and reported deployment history with navigation to the underlying objects.
- [ ] Read `automated.enabled`, self-heal, retry policy, and operation outcomes before describing reconciliation behavior. An automated configuration alone does not prove retries are active or unlimited.
- [ ] Preserve multiple sources and distinguish requested, compared, and deployed revisions. Report times according to what the field establishes; a condition transition is not automatically the last successful apply.
- [ ] Link ordinary resource details back to their delivery Application and known source information. Confirm the relationship when possible and explain unresolved claims when inventory or the management cluster cannot be read.
- [ ] Map an Argo management connection to destination connections explicitly. Never open a same-named local object as a substitute for an object on another cluster. Initially, navigate by switching to the mapped connection; concurrent observation belongs to F10.
- [ ] Offer a verified/configured link to Argo's UI for detailed diffs. Native desired/live diff, sync, and rollback are separately scoped extensions with their own API/auth requirements; mutations also require F06.

Acceptance: an Application explains which managed resource needs attention and opens it in the correct cluster. A Deployment or CloudNativePG Cluster can expose its controller/delivery/field-management relationships together, and the UI explains where a durable change belongs without overstating what labels prove. Original fixtures cover disabled automated sync, bounded/exhausted retries, multiple sources, unavailable inventory, and identity collisions.

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
- [ ] Reuse the same plan and outcome model for future operator actions. CNPG backup/reload/restart can be added before fencing or hibernation; each needs operation-specific preconditions and an observed result. Namespace permission checks and API acceptance alone do not establish completion.

Acceptance: the user can inspect the proposed effect and known reconciliation risk, then distinguish an accepted request from a verified outcome. Competing updates and object recreation cannot silently redirect the operation.

- [ ] F06 accepted

Evidence: not started.

## F07 Establish integration contracts

Extract small contracts while implementing the Argo CD and CloudNativePG workflows, then extend them when another consumer needs an answer. Keep interpretation in `freshkube-core` and presentation in `freshkube-desktop`. A list of capability names is insufficient without typed queries and availability information.

- [ ] Separate API discovery, pure semantic analysis, external provider I/O, and action planning.
- [ ] Reuse served API discovery and the selected Kubernetes client for operator resources. Support namespace-scoped reads without requiring permission to list every CRD or resource cluster-wide. API presence, readable objects, and a healthy controller are separate observations.
- [ ] Define resource-scoped capabilities and states such as unsupported, not configured, unavailable, forbidden, and ready. Include provider identity and evidence freshness.
- [ ] Specify whether each query uses an explicitly selected provider or collects several providers' answers. Preserve overlapping delivery claims and conflicting evidence; registry order must not silently decide ownership or which cluster's metrics to show.
- [ ] Let an integration contribute a dedicated investigation page and relevant findings/relationships in ordinary resource details. Reuse resource navigation, events, log viewers, and evidence presentation across both surfaces.
- [ ] Bind every query and action to an explicit connection/session and resource scope. Validate that identity when applying results, including external-provider configuration and responses arriving after a context switch.
- [ ] Keep batching, cancellation, request budgets, credentials, and caches in the runtime. Reads follow visible consumers or an explicitly started session; sidebar badges must not start unrestricted polling. An analyzer should not start hidden per-row network calls.
- [ ] Extend the existing generic CRD browser with schema context and typed interpretation where supported. Preserve its printer columns, YAML, and raw conditions as useful inspection surfaces when a specialized interpretation is unavailable.
- [ ] Keep detected in-cluster APIs distinct from configured external endpoints. Prometheus/Loki connections need explicit endpoint or Service selection, connection-scoped credentials, and checks for relevant data as well as transport reachability. Reuse the port-forward session lifecycle when forwarding is chosen.
- [ ] Retain ordinary compiled Rust modules and a small registry. Generalize a capability only after real consumers establish its query, result, and absence behavior; third-party dynamic loading is outside the initial scope. Enforce generic consumers' separation from vendor implementations through Rust module visibility and narrowly scoped checks.

Acceptance: a concrete analyzer and resource-detail consumer exercise each introduced capability. The consumer can request an answer without importing vendor implementation code, while identifying which provider answered and why another capability is unavailable. Unsupported API versions, absent conditions, restricted namespaces, and stale/refused reads have original fixtures. The initial contract can ship before F08 is complete and expand as CloudNativePG needs it.

- [ ] F07 accepted

Evidence: not started.

## F08 Build the CloudNativePG investigation

CloudNativePG is the selected first operator. Current Rubick already provides database status, backups, poolers, and several actions. Freshkube's initial scope is a connected, read-only investigation using the `Cluster`, `Backup`, `ScheduledBackup`, and `Pooler` APIs plus their Kubernetes dependencies.

- [ ] Add a database overview and per-cluster inspection for instances, backups/schedules, storage, and poolers. Include operator health when its workloads can be read.
- [ ] Interpret current/target primary, instance readiness, failover versus switchover, fencing, and hibernation from documented operator fields. Preserve sentinel values and absent status without treating them as pod identities or health evidence.
- [ ] Show WAL archiving independently from instance readiness and backup completion, with the operator's reason and observation time. Healthy pods alone do not establish database durability or recoverability.
- [ ] Derive backup history from `Backup` objects: last completed backup, newest attempt, running/failed attempts, and associated schedules/suspension. A refused backup list means unknown coverage. Do not rely on deprecated Cluster backup-summary fields for plugin installations.
- [ ] Identify the configured backup method and relevant plugin configuration. Future Backup creation must explicitly use the selected plugin/object-store/snapshot method and its required fields; omitting `spec.method` can invoke the legacy default.
- [ ] Connect instances to pods, events, logs, Services, PVCs, and nodes using verified identities/references. Expose provisioning and attachment evidence and link to existing Talos inspection when a node mapping is established.
- [ ] Show each Pooler's cluster, role/type, mode, and declared instances. Source pod readiness from its workloads rather than inferring it from the requested count.
- [ ] Integrate F04 delivery information so the database links back to its Argo Application and known configuration source.
- [ ] Treat replication lag, storage fullness, and other measured database behavior as separately sourced metrics. Unknown measurements stay unknown when no provider answers.
- [ ] Exercise unsupported versions, missing conditions, failed archiving, failed/plugin backups, failover, fencing wildcards, hibernation, and partial RBAC with original fixtures.

Acceptance: investigate a failed backup or unhealthy instance from the database page to the underlying evidence and affected resources. Complete the combined Application → Cluster → Backup/Pod → storage/node investigation with F04 described above. Generic CRD inspection remains available at every step.

CNPG actions are a later extension through F06: backup, reload, restart, then separately scoped fence/unfence and hibernate/wake. Account for current operator state, unknown fencing data, concurrent changes, and the requested action's outcome. Switchover, restore, and a SQL console each require their own design; they are outside this initial scope.

- [ ] F08 accepted

Evidence: not started. Selected operator: CloudNativePG.

## F09 Retain observed change history

- [ ] Maintain a bounded history of relevant spec changes, revisions, condition transitions, and resource events during an investigation.
- [ ] Preserve timestamps, observation gaps, and clock limitations. Do not treat local receipt order as a complete causal ordering across controllers.
- [ ] Partition history by cluster and object incarnation so reusing a context or object name cannot attach an earlier resource's history to its replacement.
- [ ] Correlate rollout, readiness, restart, and source-revision changes while labeling inferred associations explicitly.
- [ ] Export a redacted investigation snapshot and replay it through the same semantic rules for offline debugging and regression coverage.
- [ ] Define retention, storage limits, and sensitive-field handling before enabling persistence.

Acceptance: an investigation can show what the app actually observed before a failure and where its history is incomplete. Replay preserves findings without reconnecting to the original cluster.

- [ ] F09 accepted

Evidence: not started.

## F10 Support simultaneous clusters and global search

- [ ] First offer scoped tabs that retain connection, namespace, selection, and navigation while ordinary inactive reads remain stopped. Treat this as a smaller delivery step; a saved tab is not yet a simultaneous live session.
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

## Integration rollout after CloudNativePG and Argo CD

These are proposed follow-on workflows, not additional prerequisites for F07 or F08. Promote each selected integration to its own implementation step with version coverage and acceptance evidence. Dedicated pages and contributions to ordinary resource details should share the same interpretation.

| Order | Integration | Initial scope and acceptance |
| --- | --- | --- |
| Next | Flux | Connect Git/OCI/Helm/Bucket sources to Kustomizations and HelmReleases, then to dependencies and managed resources where inventory is available. Explain a failed source behind an apparently ready reconciler, suspension, and exhausted retries; identify the established blocker and affected dependents. |
| Next | cert-manager | Follow Certificate → CertificateRequest → Order → Challenge and issuer references. Explain the failing domain/step, distinguish first issuance from renewal while an existing certificate remains valid, and connect expiry to the Secrets and ingress/route hosts that use it. |
| Then | Prometheus-compatible metrics | Add workload/node history and volume fullness to existing views. For Prometheus Operator resources, distinguish a monitor selecting nothing, a monitor no instance selects, and an actual scrape failure. Join PrometheusRule configuration with loaded/evaluating rules and pending/firing alerts so silence does not imply working alerting. |
| Then | Loki | Extend the existing log viewer with bounded historical pages for workloads and replaced pods. Preserve provider timestamps and pagination limits, and distinguish query failure or unsupported label mapping from a successful query with no matching lines. |

Implementation details that affect those conclusions:

- [ ] Flux keeps source fetching and reconciliation separate. Respect suspension, dependencies with full kind/namespace identity, observed generations where supplied, and both inline chart sources and `chartRef`. Do not use a Ready condition's transition time as the last apply time.
- [ ] Flux Kustomization inventory and Helm release evidence have different strengths. Preserve that distinction when resolving delivery. Read HelmRelease drift-detection mode, exclusions, and reported conditions before predicting correction; `warn` and `enabled` have different effects.
- [ ] cert-manager walks current issuance relationships by identity and handles multiple challenges. Keep missing/read-refused steps visible. A limited consumer scan can say which consumers were found, but cannot establish that a certificate is unused cluster-wide.
- [ ] Prometheus configuration reads use Kubernetes access; actual history, scrape targets, and rule evaluation require a configured compatible endpoint. Attribute results to the endpoint and cluster selection, preserve stale/missing series, and distinguish configuration from observed scraping/evaluation. Capability support may differ among compatible backends.
- [ ] Loki connections need explicit cluster/tenant scope and label mapping where the installation requires them. Namespace names alone cannot establish that a shared endpoint holds this cluster's logs. Keep historical results distinct from the live pod stream and explain matching limits for replaced workload pods.

Further integrations should follow the environment and a concrete investigation need:

| Candidate | Scope worth adding | Evidence limit to preserve |
| --- | --- | --- |
| Cilium | Policy validity, endpoint selection, identities, and links from affected pods; optional flow/drop evidence later. | Selection and validity do not establish effective enforcement or connectivity. Account for native NetworkPolicies, enforcement mode, ingress/egress direction, host policies, and supported `spec`/`specs` forms. |
| Traefik, Istio, ingress-nginx, cloud load-balancer controllers | Extend F05 with controller-specific routes, middleware, subsets, annotations, and shared load-balancer relationships. | Configuration, controller acceptance, published endpoints, and observed reachability remain separate evidence. |
| Scylla | Rack/member health, upgrade progress, repair/backup configuration, and node setup. | Declared tasks do not prove Manager execution or successful backups; operator actions require F06. |
| Cloud providers and Karpenter | Node pool, instance, zone, and spot metadata first; NodePool/NodeClaim provisioning investigation when needed. | Rubick's Karpenter adapter is label interpretation. Cloud management or autoscaler diagnosis requires additional APIs and its own contract. |
| KRO, Crossplane, Kargo | Resource composition, provisioning, or promotion workflows when one is used for development. | Define the operational question, supported APIs, credentials, and evidence before adding a catalog entry. |

## Other product follow-ups from the review

- [ ] Before packaging, expose connection diagnostics for exec/OIDC authentication, executable resolution, the app's effective search paths, and connection failure stages. Read and explain configuration without executing auth plugins merely to enumerate them; redact credentials and exported diagnostics. This belongs with the roadmap's packaging/auth work and does not depend on the semantic engine.
- [ ] After F03/F04, offer a small, manually pinned set of important workloads or operator resources per connection. Reuse readiness, delivery, and observation-state summaries; retain identity and namespace and bound active reads. Add traffic/change summaries only when F05/F09 supply them.
- [ ] Build original offline examples and regression fixtures for each accepted investigation, including failures and refused/stale reads. Reuse these for website/product demonstrations so an advertised explanation can be reproduced. Disposable-cluster reproduction manifests should accompany workflows needing live verification and remain separate from ordinary read-only checks.
