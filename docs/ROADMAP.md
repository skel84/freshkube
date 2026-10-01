# Roadmap

What has landed, what comes next and in what order. Update this file in the same commit as the work it describes: move a step to Done with its commit, and keep open checks honest.

## Ground rules

These hold for every step until a later one deliberately changes them.

- **Read-only Kubernetes.** The app lists, watches, gets and streams logs. Changing the cluster waits for a reviewed change workflow ([F06](FUTURE_IDEAS.md#f06-plan-changes-and-observe-their-outcomes)). Talos node operations stay on the Operations page behind their own preflight and confirmation.
- **The chosen context only.** Connect to the context the user named, or the one the kubeconfig marks as current. When a named context is missing, report it; never swap in another.
- **Identity, not position.** Rows and documents are identified by connection, resource, namespace, name and UID. A response for another selection or object incarnation is dropped.
- **Honest states.** Loading, loaded, refused, failed and stale are distinct. An unreadable collection is never shown as empty, and failed refreshes keep the previous data marked stale.
- **Source reuse.** Code taken from Kubeli (MIT) keeps its notice and is recorded in a NOTICE file. Rubick is studied for behavior, not copied. See [the reuse policy](REFERENCES.md#reuse-policy).

## Done

| Step | What landed | Commit |
| --- | --- | --- |
| Foundation | talos-pilot fork renamed to Freshkube; the GPUI Kit app is the only frontend | `c878fa9` |
| 1. Core | Read-only listing and watching of any kind through the server-side Table API (kube 0.98), checked against a live cluster | `86294f9` |
| 2a. Browse | KUBERNETES sidebar groups in Kubeli's order; one Resources page with a live list and watch, namespace picker and filter; Talos mode reads through the Talos-derived Kubernetes client | `361b9d5` |
| 2b. Kubernetes only | Works without a talosconfig: kubeconfig contexts in the sidebar, `--kubernetes-only`, `--kube-context`, one shared connection attempt per context, Talos pages ask for a talosconfig | `3bfb4d4` |

## Next: finish Kubernetes browsing

### Step 3. Detail pane

Overview, YAML and Events for the selected object, beside the list and stacked below it in a narrow window.

- Fetch the selected object's full representation on demand. Never rebuild it from a table row.
- Drop late responses for another selection or UID. Show a deleted or recreated object as such instead of silently switching documents.
- Show loading, refused and failed states, keeping a previous document visible and marked stale.
- Read-only YAML viewer with copy and search. Editing waits for F06. A large object's YAML runs to thousands of lines, so it renders as a long list ([LONG_LISTS.md](LONG_LISTS.md)): `uniform_list` for unwrapped lines, measured rows if it wraps.
- Events for the object, matched by its UID, newest first.
- Decide before Secrets get a YAML view: is data hidden until revealed explicitly?

Done when a selected object's documents follow it through updates, deletion and recreation, and a slow response can't show another object's YAML.

### Step 4. Custom resources

- A Custom Resources group, with one entry per API group, discovered when expanded.
- CRD kinds list through the same Table path, so their printer columns come from the server.
- Report removed APIs, unserved versions and refused discovery per group; never as an empty group.

### Step 5. Pod logs

- Choose a container, including init containers. Offer follow, tail length, timestamps and the previous instance, chosen explicitly and never substituted silently.
- Bounded retention and a virtualized view with search and copy. Following must not pull the view away from where the user scrolled or selected.
- Cancel the stream when the pane closes, the scope changes or the app quits. Tell a normal end from a failure, and make reconnecting visible.
- Reuse the Talos log panel's virtualized list and hidden-batch coalescing where they fit.
- First apply the wrapped-resize change in [LONG_LISTS.md](LONG_LISTS.md#wrapped-rows-during-a-resize) to that shared view, with its test, so both Talos and pod logs resize smoothly. Measured in talos-pilot's frontend, it took a wrapped resize step from 115 ms to 44 ms at 5,000 lines.

### Open checks

- Talos mode's Kubernetes pages against the live cluster. Only Kubernetes-only mode has been checked live so far.
- A visual pass of Kubernetes-only mode on a live cluster. The live check ran with the screen locked, so it covered data, not drawing.
- Launched as an app bundle, the app couldn't reach a LAN cluster that the terminal-launched binary reached. This is probably macOS Local Network privacy. Confirm before packaging, and add `NSLocalNetworkUsageDescription`.

## Then: a daily-use workflow

These come from the GPUI evaluation plan (G09–G12) and the follow-ups it listed.

- **Workflow pass.** Context → namespace → object → detail → logs, by keyboard and pointer, with consistent commands, states and focus restoration. Check light and dark themes, larger text and the minimum window size.
- **Pod exec.** Settle feasibility first: pick a terminal emulator and renderer, check its license and test it with synthetic byte streams. Then add exec on an explicitly chosen pod and container. Node-debug pods stay out.
- **Performance and reliability.** Reproducible workloads (large tables, watch bursts, log floods) measured on release builds, since debug builds draw about 14× slower ([K12](GPUI_FRICTION.md#k12-unoptimised-builds-draw-about-14-slower)). Record demonstrated bottlenecks and fix those in scope.
- **GPUI Kit evaluation.** A short report from the [friction log](GPUI_FRICTION.md): what the toolkit served well, what it cost, and whether to continue, contribute upstream or limit scope.
- **Follow-ups**, each promoted to a step with acceptance criteria when it enters scope:
  - port forwarding with an explicit session lifetime;
  - current CPU and memory from the metrics API;
  - YAML schema validation;
  - packaging, signing and kubeconfig auth plugins (exec and OIDC);
  - Linux and Windows builds.

## Later: understanding the cluster

[FUTURE_IDEAS.md](FUTURE_IDEAS.md) holds the longer-term backlog. It aims at three questions: why an application is unhealthy, who controls its configuration, and what changed before it broke.

| Task | Topic |
| --- | --- |
| F01 | Observations and evidence |
| F02 | Shared resource store |
| F03 | Consistent workload health |
| F04 | Ownership and reconciliation |
| F05 | Traffic paths |
| F06 | Planned changes |
| F07 | Integration contracts |
| F08 | One deep operator workflow |
| F09 | Change history |
| F10 | Several clusters at once |
| F11 | CLI and MCP explanations |

The first semantic slice is F01 → F02 → F03, then F04 and F05. F06 is the first task that changes the cluster.

## History

The GPUI Kit evaluation began in a separate prototype. It is an unpublished local checkout, archived as `freshkube-prototype`. It built a fixture-only pod table, docked workspace, inspector and YAML editor (G01–G04, H01), then moved into this repository. Its plan from G05 on is folded into this roadmap. Its backlog and source notes now live in [FUTURE_IDEAS.md](FUTURE_IDEAS.md) and [REFERENCES.md](REFERENCES.md).
