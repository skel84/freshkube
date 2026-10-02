# Reference codebases

This map records the local source reviewed on 2026-09-30, while Freshkube was a separate GPUI Kit prototype. Links point to sibling checkouts next to this repository (`../Kubeli`, `../rubick`, `../gpui-kit`). Findings below describe these revisions. Recheck relevant code when updating dependencies; performance and live-cluster behavior have not been established by this source review.

## Inspected revisions

| Checkout | Commit | Role |
| --- | --- | --- |
| Kubeli | `06a19153bf7f2d5d3340972780dceb88bf7a52cf` | Selected Rust implementation donor and existing workflow reference |
| Rubick | `030c6012bea9e00a934f6051add2f979f25f333d` | Future diagnostic behavior and architecture reference |
| GPUI Kit | `201b55a431fb1b82a6047e908de63913db3d4354` | Toolkit, component examples, and development guidance |

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
| Route diagnosis and per-parent conditions | [Route trace](../../rubick/src/lib/route-trace.ts), [Gateway model](../../rubick/src-tauri/src/resources/gateway.rs) | F05 |
| Published endpoints | [Endpoint interpretation](../../rubick/src-tauri/src/resources/published.rs) | F05 |
| Provider capability and availability | [Integration registry](../../rubick/src/integrations/registry.ts) | F07 |
| Operator-specific findings | [CNPG model](../../rubick/src/integrations/cloudnativepg/model.ts), [Cilium integration](../../rubick/src/integrations/cilium/index.ts) | F08 |
| Bounded multi-cluster search | [Search subsystem](../../rubick/src-tauri/src/search/mod.rs) | F10 |

Current Rubick already includes substantive Cilium and CloudNativePG behavior. A future claim of differentiation should identify a better investigation workflow or additional supported evidence, rather than merely adding those vendor names.

## Reuse policy

The checked-in [Kubeli license](../../Kubeli/LICENSE) is MIT. [GPUI Kit's manifest](../../gpui-kit/crates/kit/Cargo.toml) declares Apache-2.0. [Rubick's licensing history](../../rubick/LICENSE-HISTORY.md) records MIT releases through 3.1.0 and GPL-3.0-or-later afterward. This records source metadata, not a legal review of a future distribution.

Freshkube inherits talos-pilot's MIT license. When reusing Kubeli implementation, retain applicable copyright/license notices and record source revision, path, adaptations, and associated tests in a NOTICE file created with the first extraction. So far only Kubeli's sidebar grouping has been followed; no code has been taken.

Pod exec depends on `alacritty_terminal` (Apache-2.0) as a library; its licence text is in `licenses/Apache-2.0.txt`, listed in the root `NOTICE`. `gpui_xterm` (MIT) is read for its approach to drawing the grid, not copied ([POD_EXEC.md](POD_EXEC.md)).

Study current Rubick to understand behavior, then implement future features from Kubernetes/operator specifications and independently constructed fixtures. Do not copy its current implementation, tests, or documentation into the proposed permissive codebase. Reuse of an older MIT release requires verifying the exact source and its license; it is not part of the current plan.

## Primary specifications for future work

- [Kubernetes server-side apply](https://kubernetes.io/docs/reference/using-api/server-side-apply/) explains field ownership and conflict handling.
- [EndpointSlice conditions](https://kubernetes.io/docs/concepts/services-networking/endpoint-slices/#conditions) explain readiness, serving, and terminating behavior.
- [HTTPRoute](https://gateway-api.sigs.k8s.io/reference/api-types/httproute/) describes routing configuration and parent status.

Verify the API versions and operator documentation used by an actual implementation when starting its F task. A general model must not assume every installed CRD supports the same conditions or generation tracking.
