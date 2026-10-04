# Reference codebases

This map records the local source reviewed on 2026-09-30, while Freshkube was a separate GPUI Kit prototype. Links point to sibling checkouts next to this repository (`../Kubeli`, `../rubick`, `../gpui-kit`). Findings below describe these revisions. Recheck relevant code when updating dependencies; performance and live-cluster behavior have not been established by this source review.

The integration review was expanded on 2026-10-02 against the same Rubick revision. Its findings inform the Argo CD/CloudNativePG first slice and the [integration rollout](FUTURE_IDEAS.md#integration-rollout-after-cloudnativepg-and-argo-cd).

## Inspected revisions

| Checkout | Commit | Role |
| --- | --- | --- |
| Kubeli | `06a19153bf7f2d5d3340972780dceb88bf7a52cf` | Selected Rust implementation donor and existing workflow reference |
| Rubick | `030c6012bea9e00a934f6051add2f979f25f333d` | Future diagnostic behavior and architecture reference |
| GPUI Kit | `201b55a431fb1b82a6047e908de63913db3d4354` | Toolkit, component examples, and development guidance |
| [corust / coroot-rs](https://github.com/skel84/corust) | `c35f7138f208a5b84aae28c0ccbfe776f9d58e15` | Reviewed dependency for [Coroot integration](COROOT.md), pinned by git revision (MIT OR Apache-2.0). Includes signal preservation (PR #3), bounded response bodies (PR #4) and Applications/map collection-shape validation (PR #5). Freshkube uses typed application, map and report reads; richer native-view models remain in corust #2. Validation and live-check status are recorded in COROOT.md. |

## GPUI Kit starting points

| Need | Local source |
| --- | --- |
| Application guidance | [Kit skill](../../gpui-kit/skills/gpui-kit/SKILL.md) and [design skill](../../gpui-kit/skills/gpui-kit-design-guides/SKILL.md) |
| Minimal bootstrap | [Hello world](../../gpui-kit/examples/hello_world/src/main.rs) |
| Dependency compatibility | [Workspace manifest](../../gpui-kit/Cargo.toml) and [umbrella manifest](../../gpui-kit/crates/kit/Cargo.toml) |
| Tables | [Data table story](../../gpui-kit/crates/story/src/stories/data_table_story.rs) and [table documentation](../../gpui-kit/website/component/data-table.md) |
| Workspace panes | [Dock example](../../gpui-kit/examples/dock/src/main.rs) and [dock documentation](../../gpui-kit/website/component/dock.md) |
| Editor | [Editor example](../../gpui-kit/examples/editor/src/main.rs) and [editor documentation](../../gpui-kit/website/component/editor.md) |
| Command palette | [Command story](../../gpui-kit/crates/story/src/stories/command_story.rs) |
| Async ownership | [Async reference](../../gpui-kit/skills/gpui-kit/references/gpui/async.md) |
| UI verification | [Testing guide](../../gpui-kit/crates/kit/TESTING.md) and [UI test documentation](../../gpui-kit/website/docs/test.md) |

The inspected Kit manifest declares version 0.7.0 and the workspace pins matching `gpui-pre` packages. Freshkube builds against the published 0.7.0 crates; the checkout is for its guides and examples. Check signatures against the registry source, as [AGENTS.md](../AGENTS.md#learning-the-api) describes.

## Kubeli extraction map

| Operation | Source and reusable concerns | Adaptation needed |
| --- | --- | --- |
| Kubeconfig and connection | [Configuration](../../Kubeli/src-tauri/src/k8s/config.rs), [client](../../Kubeli/src-tauri/src/k8s/client.rs) | Remove desktop command/state coupling; retain explicit context and timeout behavior; inspect OIDC dependencies |
| Pod list and detail | [Resource commands](../../Kubeli/src-tauri/src/commands/resources.rs) | Extract only required operations and mappers from the large command module |
| Watches | [Watch commands](../../Kubeli/src-tauri/src/commands/watch.rs) | Retain cancellation and initial/resync behavior; replace Tauri emission with bounded transport |
| Logs | [Log commands](../../Kubeli/src-tauri/src/commands/logs.rs) | Retain container selection, previous logs, batching, and terminal events; define retention in the native view |
| Exec | [Shell commands](../../Kubeli/src-tauri/src/commands/shell.rs) | Separate pod exec from node-debug workflows; preserve byte ordering, input, resize, and cleanup. Report Connected only after exec succeeds, send the first terminal size, and read the exit status ([POD_EXEC.md](POD_EXEC.md#references)) |
| Errors | [Structured error mapping](../../Kubeli/src-tauri/src/error.rs) | Keep failure categories available to the UI rather than reducing every error to a string |
| Port forwarding | [Forwarding commands](../../Kubeli/src-tauri/src/commands/portforward.rs) | Separate session ownership from selected cluster. Keep binding before reporting a port, and Service targetPort resolution; avoid ignoring the port's error channel and `SO_REUSEADDR` binds ([PORT_FORWARD.md](PORT_FORWARD.md#references)) |
| Metrics | [Metrics commands](../../Kubeli/src-tauri/src/commands/metrics.rs) | Later feature; metrics API access does not provide historical provider queries |

Important scope corrections from the review:

- The [client manager](../../Kubeli/src-tauri/src/k8s/client.rs) holds one active client/context pair. [Cluster switching](../../Kubeli/src-tauri/src/commands/clusters/commands.rs) stops watches, logs, and shells; forwards have a different lifetime.
- Derived pod display status exists in [TypeScript](../../Kubeli/src/lib/utils/pod-status.ts). [Graph health](../../Kubeli/src-tauri/src/commands/graph.rs) and [MCP health](../../Kubeli/src-tauri/src/mcp/tools.rs) use separate rules.
- Pod summaries omit some metadata needed for future relationships. Do not assume they can reconstruct a full Kubernetes object.
- [Helm commands](../../Kubeli/src-tauri/src/commands/helm.rs) inspect releases and their history. The function named `uninstall_helm_release` forgets release records while leaving deployed resources in place.
- YAML apply in the resource command module uses forced server-side apply. Do not adopt that default for the future reviewed change workflow.
- Kubeli's [frontend dependencies](../../Kubeli/package.json) include Monaco, xterm, and React Flow. Their functionality does not transfer by reusing the Rust backend.
- MCP has its own fetching and interpretation paths. A future shared semantic API requires deliberate consolidation.

## Rubick concepts to revisit later

| Concept | Source to study | Future task |
| --- | --- | --- |
| Known versus missing versus unreadable | [Relationship types](../../rubick/src-tauri/src/resources/connections.rs) | F01 |
| Batched watches and subscription startup | [Watch subsystem](../../rubick/src-tauri/src/watch/mod.rs) | F02 |
| Shared reads for related objects | [Connection snapshots](../../rubick/src-tauri/src/commands/connections/snapshot.rs) | F02 |
| Container-aware display status | [Pod display](../../rubick/src-tauri/src/resources/types/pod_display.rs) | F03 |
| Claimed and confirmed delivery relationships | [Delivery model](../../rubick/src/integrations/gitops.ts) | F04 |
| Argo application diagnosis and ownership | [Application model](../../rubick/src/integrations/argocd/model.ts), [ownership resolver](../../rubick/src/integrations/argocd/owner.ts) | F04 |
| Route diagnosis and per-parent conditions | [Route trace](../../rubick/src/lib/route-trace.ts), [Gateway model](../../rubick/src-tauri/src/resources/gateway.rs) | F05 |
| Published endpoints | [Endpoint interpretation](../../rubick/src-tauri/src/resources/published.rs) | F05 |
| Provider capability and availability | [Integration registry](../../rubick/src/integrations/registry.ts) | F07 |
| Database findings and operator actions | [CNPG model](../../rubick/src/integrations/cloudnativepg/model.ts), [actions](../../rubick/src/integrations/cloudnativepg/actions.ts) | F08; actions through F06 |
| Source and reconciler dependencies | [Flux model](../../rubick/src/integrations/flux/model.ts), [delivery resolver](../../rubick/src/integrations/flux/owner.ts) | Integration rollout after F04/F08 |
| Certificate issuance and consumer relationships | [cert-manager model](../../rubick/src/integrations/cert-manager/model.ts), [consumer lookup](../../rubick/src/integrations/cert-manager/serves.ts) | Integration rollout |
| Monitor and alert-rule diagnosis | [Monitor model](../../rubick/src/integrations/prometheus/monitors/model.ts), [alert model](../../rubick/src/integrations/prometheus/alerts/model.ts) | Integration rollout |
| Persistent workload logs | [Loki queries](../../rubick/src/integrations/loki/queries.ts), [history client](../../rubick/src/integrations/loki/client.ts) | Integration rollout |
| Policy selection and its limits | [Cilium coverage](../../rubick/src/integrations/cilium/coverage.ts) | Environment-dependent integration |
| Bounded multi-cluster search | [Search subsystem](../../rubick/src-tauri/src/search/mod.rs) | F10 |

Current Rubick already includes substantive Cilium and CloudNativePG behavior. A future claim of differentiation should identify a better investigation workflow or additional supported evidence, rather than merely adding those vendor names.

Interpretation details to recheck against operator specifications: Argo's parser treats any `automated` object as enabled, its ownership matching drops API group, and source attribution uses the first source. CNPG's backup action omits the method/plugin configuration. Flux's HelmRelease delivery prediction does not inspect drift-detection mode. Cilium's selection-based coverage does not establish actual enforcement. These are limitations of the inspected implementation to address independently, not behavior to reproduce.

## Rubick architecture lessons

These are observations about the inspected revision and recommendations for Freshkube, not an implemented Freshkube architecture. [FUTURE_IDEAS.md](FUTURE_IDEAS.md) owns feature priorities and acceptance criteria; these notes preserve why particular boundaries are useful and where the reference has limits. Source inspection establishes the mechanisms below, not their performance on our workloads.

1. **Use capabilities to keep ordinary screens independent of vendors.** Rubick declares typed operations such as delivery lookup and historical usage in its [registry](../../rubick/src/integrations/registry.ts). A compiled list supplies implementations; `defineVendor` checks the declaration rather than loading a plugin. Its [lint configuration](../../rubick/eslint.config.js) rejects imports into vendor folders from generic consumers. Freshkube can use a small set of typed core queries and private implementations, with GPUI page construction kept in the desktop crate. Choose trait/enum shapes only when the first consumers need them.

2. **Provider selection is part of a query's meaning.** The [dispatcher](../../rubick/src/integrations/index.ts) offers both one-provider and many-provider access. Delivery consumers may need answers from Argo and Flux together; returning the first provider would lose ownership claims. Configured-provider lookup also returns the first applicable entry, including an unreachable one. Freshkube should define selection or aggregation explicitly for each query and retain who supplied every answer. Registration order is not an appropriate policy for resolving conflicting ownership or choosing a metrics source.

3. **A dedicated page and a resource-detail contribution serve different needs.** A Rubick vendor can declare a page, custom-resource columns, summary facts, and capabilities independently. Page metadata supplies navigation and links to the fuller explanation. Its [CNPG declaration](../../rubick/src/integrations/cloudnativepg/index.ts) and [Argo declaration](../../rubick/src/integrations/argocd/index.ts) show different combinations. Feature-local `data`, `model`, `page`, `report`, and `actions` files separate several concerns, although some model helpers also contain presentation wording. Freshkube should share core interpretation between specific GPUI views and later reports/headless queries. This does not require a universal renderer or one large integration trait.

4. **Interpret complete inputs before reducing them for display.** Rubick's [custom-resource transport model](../../rubick/src-tauri/src/commands/crds/types.rs) and [conversion](../../rubick/src-tauri/src/commands/crds/convert.rs) retain selected metadata, `spec`, and `status`. Cilium's legal top-level `specs` field does not survive that shape, so its [policy analyzer](../../rubick/src/integrations/cilium/model.ts) must decline some conclusions. Conversely, [pod display](../../rubick/src-tauri/src/resources/types/pod_display.rs) is derived in Rust where the required input is available. Freshkube has no Rust-to-TypeScript boundary: keep domain interpretation in core and derive typed results from appropriately retained API inputs. Display YAML and table cells should not become the source for later analysis.

5. **Missing knowledge is part of the result contract.** Rubick's [relationship types](../../rubick/src-tauri/src/resources/connections.rs) distinguish existence from whether a reference was checked, while [namespace snapshots](../../rubick/src-tauri/src/commands/connections/snapshot.rs) carry failures separately for independent resource lists. Its richer capability state distinguishes an unconfigured provider from a configured one that cannot answer; the simpler `useCapability` helper discards that distinction. Freshkube should preserve typed read failures, scope coverage, provider identity, and freshness through joins and summaries. A failed optional read should leave the known part of an investigation usable without manufacturing an empty result.

6. **Make read cost and lifetime shared responsibilities.** Rubick's [snapshot cache](../../rubick/src-tauri/src/commands/connections/snapshot.rs) coalesces concurrent namespace reads, briefly reuses successful results, and removes snapshots containing failed reads after their shared request. Its [live-query hook](../../rubick/src/hooks/useLiveQuery.ts) centralizes refresh policy; lint prevents individual callers from adding arbitrary polling intervals. These are separate mechanisms, and neither establishes an atomic cluster snapshot. Also inspect individual adapters: [Argo data](../../rubick/src/integrations/argocd/data.ts) uses `useQuery` with a stale time, while [CNPG data](../../rubick/src/integrations/cloudnativepg/data.ts) uses live queries for changing cluster/backup state. Freshkube should own subscriptions and request sharing in a scoped runtime, expose staleness, and verify resumption and cancellation per consumer. Reuse policies and timings need measurement rather than transplantation.

7. **Stream startup and resynchronization need explicit ordering.** Rubick's [watch subsystem](../../rubick/src-tauri/src/watch/mod.rs) waits until the frontend listener is installed before starting the producer, then delivers batches into its query cache. That prevents initial events from being lost across the IPC boundary. Freshkube can use Rust channels directly, while preserving the underlying invariants: establish the receiver before production, account for initial synchronization and resync, bound queued work, and reject updates for a superseded session. Adopt those guarantees without recreating Tauri's event machinery.

8. **Keep durable targets separate from transient connections.** Rubick's [scope tabs](../../rubick/src/stores/scopeTabStore.ts) retain navigation for inactive tabs while its ordinary backend connection remains singular. Several [configured integration operations](../../rubick/src/integrations/registry.ts) consult the global current context. These choices do not supply simultaneous independent cluster sessions. Its [forwarded integrations](../../rubick/src/integrations/forwarded.ts) do preserve a useful distinction: the Service is the selected target, and the pod is resolved again when a tunnel is established. Freshkube should bind queries/configuration/actions to explicit sessions and preserve the selected Service/provider separately from the current pod or local URL.

9. **Plan requests and observe effects separately.** Rubick's [search planner](../../rubick/src-tauri/src/search/plan.rs) computes scope and limits without I/O, making request cost reviewable before execution. Its [CNPG actions](../../rubick/src/integrations/cloudnativepg/actions.ts) separate operation construction from the page, but the [page](../../rubick/src/integrations/cloudnativepg/page.tsx) reports success after the write returns and invalidates queries; that is not a dedicated reconciliation-outcome workflow. Freshkube should apply pure planning to reads and mutations, then retain explicit execution/outcome state. F06 adds concurrency checks and distinguishes request acceptance from controller convergence or reversal.

The registration story also has a boundary: the TypeScript vendor list is accompanied by a Rust [detection marker table](../../rubick/src-tauri/src/integrations/mod.rs), and new capabilities require consumer support. Adding a folder does not automatically supply detection, backend access, or UI behavior. In Freshkube, keep API/discovery descriptors with the core integration and presentation bindings in the desktop layer, and test their agreement when the feature is introduced.

Open implementation choices include the exact query/result types, registry representation, cache ownership and retention, scheduling rules, and GPUI entity boundaries. Resolve them in the first feature design using the current [coding guides](../../gpui-kit/website/docs/coding-guides.md) and measured workloads. Preserve the constraints and failure cases above without treating Rubick's file layout, cache durations, or frontend state model as a specification to port.

## Reuse policy

The checked-in [Kubeli license](../../Kubeli/LICENSE) is MIT. [GPUI Kit's manifest](../../gpui-kit/crates/kit/Cargo.toml) declares Apache-2.0. [Rubick's licensing history](../../rubick/LICENSE-HISTORY.md) records MIT releases through 3.1.0 and GPL-3.0-or-later afterward. This records source metadata, not a legal review of a future distribution.

Freshkube inherits talos-pilot's MIT license. When reusing Kubeli implementation, retain applicable copyright/license notices and record source revision, path, adaptations, and associated tests in a NOTICE file created with the first extraction. So far only Kubeli's sidebar grouping has been followed; no code has been taken.

Pod exec depends on `alacritty_terminal` (Apache-2.0) as a library; its licence text is in `licenses/Apache-2.0.txt`, listed in the root `NOTICE`. `gpui_xterm` (MIT) is read for its approach to drawing the grid, not copied ([POD_EXEC.md](POD_EXEC.md)).

Study current Rubick to understand behavior, then implement future features from Kubernetes/operator specifications and independently constructed fixtures. Do not copy its current implementation, tests, or documentation into the proposed permissive codebase. Reuse of an older MIT release requires verifying the exact source and its license; it is not part of the current plan.

## Primary specifications for future work

- [Kubernetes server-side apply](https://kubernetes.io/docs/reference/using-api/server-side-apply/) explains field ownership and conflict handling.
- [EndpointSlice conditions](https://kubernetes.io/docs/concepts/services-networking/endpoint-slices/#conditions) explain readiness, serving, and terminating behavior.
- [HTTPRoute](https://gateway-api.sigs.k8s.io/reference/api-types/httproute/) describes routing configuration and parent status.
- [Argo automated sync](https://argo-cd.readthedocs.io/en/stable/user-guide/auto_sync/) describes enablement, self-heal, and retry behavior; [resource tracking](https://argo-cd.readthedocs.io/en/stable/user-guide/resource_tracking/) describes tracking methods and installation identity.
- [CloudNativePG's API reference](https://github.com/cloudnative-pg/cloudnative-pg/blob/main/docs/src/cloudnative-pg.v1.md) and [backup documentation](https://github.com/cloudnative-pg/cloudnative-pg/blob/main/docs/src/backup.md) describe backup methods and plugin configuration. Select versioned specifications when implementing against an installed operator.
- [Flux HelmRelease drift detection](https://fluxcd.io/flux/components/helm/helmreleases/#drift-detection) distinguishes detection from correction and documents exclusions.
- [Cilium policy enforcement](https://docs.cilium.io/en/stable/security/policy/intro/) explains agent modes and per-direction behavior beyond selector matching.

Verify the API versions and operator documentation used by an actual implementation when starting its F task. A general model must not assume every installed CRD supports the same conditions or generation tracking.
