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
| Logs resize | With wrapping on, a resize lays out only the log lines on screen and re-measures the rest once the width holds ([LONG_LISTS.md](LONG_LISTS.md#wrapped-rows-during-a-resize)) | `decdefa` |
| 3. Detail pane | Overview, YAML and Events for the selected object, beside the list or below it in a narrow window. Reads on demand and follows the list's resourceVersion at most once a second; drops late answers; shows refused, failed, stale, deleted and recreated. YAML is a `uniform_list` with search, line selection and copy. Events match by UID. Secret values stay hidden until one key is revealed | `28bf866`, `f8546ef` |
| 4. Custom resources | A Custom Resources sidebar section with one entry per API group, discovered when opened, and each group's kinds when the group opens; custom kinds list through the same Table path, with the server's printer columns. A refused, failed or listless group says why with Retry, never as an empty group; a version answering 404 shows as not served. A page kind the server stops serving says so and has its group discovered again. Checked live: 58 groups, a custom kind's rows, and an unserved version | `09fb6fd`, `c8547cb`, `7bdad9c`, `8e728bf` |
| 5. Pod logs | A Logs tab in a pod's detail pane, on the shared log view. A container picker grouped into init, app and ephemeral containers; tail (100 to all), timestamps, Stop and Resume from the last line, and the previous instance as an explicit choice. A crash loop is offered, never substituted. Marker rows note restarts and reconnects, and copy and search skip them. Waiting, empty, ended, reconnecting and failed each say so. The stream lives while the pod stays open on any tab and is cancelled when another object opens, the pane closes, the scope changes, the page hides or the app quits. Checked live: a 500-line tail in batches, a previous instance read to its end, an ended container, a waiting one, and cancellation on close | `c8fb3ab`, `d316d13`, `d16bd54`, `eec2418` |
| Workflow pass | Context → namespace → kind → object → detail by keyboard as by pointer. The Resources page keeps the keyboard in every state, and a new context, the namespace picker (N) and the filter all hand it back to the list. Enter opens a row and moves to its details; Escape steps back one level: search, pane, filter, then closes the pane. Command-Shift-[ and ] switch detail tabs from the list or the pane, and the tabs take ← and →. Command-F finds in what has focus, Command-G and F3 step through matches, Command-A selects the newest log lines. Control-Tab skips pages that can't load in Kubernetes-only mode. Command-K goes to any kind, built-in or discovered. Retry reads the same everywhere, and tooltips and labels name the keys. Checked visually in light and dark at full and minimum size | `bcff0c4`, `97b8e58`, `2c9c278`, `53a053c`, `994d9bc`, `bf04e23`, `f6c28d0`, `6f0d497` |

## Next: finish Kubernetes browsing

Steps 1 to 5 are done. What remains here is checking them against the live cluster.

### Open checks

- Talos mode's Kubernetes pages against the live cluster. Only Kubernetes-only mode has been checked live so far.
- A CRD removed or unserved while the app runs is covered by UI tests only, since a read-only check can't make one. Checked live (2 October): the cluster has 18 versions marked `served: false` across 13 CRDs (old Cluster API, external-secrets, NATS JetStream and Kyverno versions). Legacy `/apis` discovery leaves them out, and each of those CRDs is offered at the version it serves, which is right: asking for an unserved version answers 404. Earlier, asking for a version that doesn't exist (`FRESHKUBE_VERSION=v0` in `examples/probe.rs`) showed a 404 group version and page kind as not served, with the group discovered again.
- The Custom Resources section hasn't been seen drawn against the live cluster. Checked visually (2 October): Kubernetes-only mode live, light and dark, with Talos pages dimmed, nodes and deployments listed, the Overview pane, and the namespace picker opened with N. The Logs tab was checked with example data only, since live log text is never captured.
- At larger text (20 px) the Kit controls grow and the app's fixed-size text doesn't. Checked visually (2 October): with example data, the workflow pass's focus states, tab keys and Command-K palette draw correctly in light and dark, and at the minimum window size the stacked pane takes two thirds of the height and its Logs tab shows lines (`20ec82c`).
- Pod logs reconnecting after a dropped connection, and a restart while following, are covered by core and UI tests only. The live check saw streaming, ended, waiting and previous-instance logs.
- The detail pane's recreated state (same name, new UID) is covered by UI tests only; no object on the cluster is recreated on its own. Checked live (2 October): with a node Lease open, 4 renewals in 50 seconds gave exactly 4 reads, and the 400 list batches about other leases gave none. An Event open when the API server expired it turned Deleted at once, kept its last-read document and was not read again.
- Launched as an app bundle, the app couldn't reach a LAN cluster that the terminal-launched binary reached. This is probably macOS Local Network privacy. Confirm before packaging, and add `NSLocalNetworkUsageDescription`.

## Then: a daily-use workflow

These come from the GPUI evaluation plan (G09–G12) and the follow-ups it listed.

- **Pod exec.** Settle feasibility first: pick a terminal emulator and renderer, check its license and test it with synthetic byte streams. Then add exec on an explicitly chosen pod and container. Node-debug pods stay out.
- **Performance and reliability.** Reproducible workloads (large tables, watch bursts, log floods) measured on release builds, since debug builds draw about 14× slower ([K12](GPUI_FRICTION.md#k12-unoptimised-builds-draw-about-14-slower)). Record demonstrated bottlenecks and fix those in scope.
- **GPUI Kit evaluation.** A short report from the [friction log](GPUI_FRICTION.md): what the toolkit served well, what it cost, and whether to continue, contribute upstream or limit scope.
- **Follow-ups**, each promoted to a step with acceptance criteria when it enters scope:
  - logs of several containers, or of a workload's pods, in one view: the same `LogView` with several sources and its source filter;
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
