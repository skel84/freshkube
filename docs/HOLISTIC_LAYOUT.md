# One app for Talos and Kubernetes

Today the sidebar is a Talos app with Kubernetes added below it. Overview, Services, Logs and the NODE and CLUSTER sections read the Talos API. Kubernetes is a block of collapsible groups near the bottom. The Overview knows nothing about pods or workloads, and a TARGET picker in the title bar decides which node half the pages show. This file records what the code review found, the decisions taken with the user, and the design the build follows. The [roadmap](ROADMAP.md) tracks its progress.

The mockups are in `~/Downloads/freshkube-holistic-mockups/` and are not committed:
- `png/` holds 2× screenshots at 1280 × 880;
- `html/` holds the same screens as standalone pages.

Screen 00 is today. Screens 01–06 are the target: 01 Overview, 02 Nodes, 03 the node pane, 04 a pod's cross-links, 05 search, 06 Kubernetes-only. They show intent, not pixels. Where this file and a mockup differ, this file wins.

## What the code has today

Reviewed on 2 October 2026. Paths are under `crates/freshkube-desktop/src` (desktop) and `crates/freshkube-core/src` (core).

- **One target node.**
  - `Pilot.selected_node: Option<String>` is the TARGET picker's node.
  - `select_node` bumps the epoch, re-targets the log panel and the services snapshot, and pushes a new `ScreenSource` to every screen (`desktop/mod.rs`).
  - Processes, Storage, Network and Diagnostics inspect that node.
  - etcd, Workload health, Security, Lifecycle and Operations are cluster-wide. They still reload when the target changes, because `Target` includes the node.
  - Services and Logs show the target node only. The Overview's node cards and Lifecycle's "Target" button change it.
- **Talos already sees every node's services.** The Talos overview (`ClusterOverview`) holds service snapshots for all nodes. The sidebar's Services badge counts unhealthy services cluster-wide from it.
- **Nothing Kubernetes is read for a page that isn't showing.**
  - The Resources page watches one kind, through the Table API, only while it's visible.
  - Workload health (`screens/workloads.rs`) makes four one-shot typed lists on its own 15 s refresh (`collect_workloads` in core `workloads.rs`: deployments, statefulsets, daemonsets, pods). It reads through the Talos-derived client, so it is Talos mode only.
  - The shell's Talos overview, by contrast, refreshes every 15 s on every page.
- **Kubernetes and Talos nodes are already matched in core,** by name equal to the Talos hostname, with the InternalIP as the address (`cluster_overview.rs` `K8sNodeInfo`, `security_lifecycle.rs` `merge_node_roster`, `operations.rs` `node_matches_target`). The desktop has no joined view. `NodeSummary.responding` means Talos answered; it is not Kubernetes Ready.
- **The detail pane** (`resources/pane/`) shows one Overview for every kind: metadata, conditions, an "Owned by" line in plain text, labels, annotations and finalizers.
  - Pods add Logs, Shell and Ports tabs.
  - The pane is resizable, from 320 dp up to the list's minimum, and can't take the full width.
  - Nothing opens another object. `Pilot::open_kind` changes kind but can't select an object.
- **No selectors.** `watch_collection`, `list_table` and `watch_table` take a kind and a namespace only, so "pods on this node" can't be listed yet.
- **⌘K** is "Go to kind…" (`desktop/kind_switcher.rs`), over kinds only.
- **Example data** has pods, deployments, services, nodes, namespaces, secrets and certificates.
  - Pods name a node and a ReplicaSet owner, but there are no ReplicaSets, cluster-wide Events, PVCs or PVs.
  - The Kubernetes nodes have the same names and addresses as the Talos fixture's.

## Decisions

Taken with the user on 2 October 2026.

- **Talos is "Control plane" plus the node pane.** No section of the sidebar is about Talos as such. Machines are reached through Nodes, and cluster-wide Talos pages sit under Control plane.
- **Sidebar**, top to bottom:
  - the context switcher;
  - Search everything;
  - **Cluster**: Overview, Nodes, Namespaces, Events;
  - **Resources**: Workloads (Health, then today's kinds), Networking, Configuration, Storage, Access Control, Administration, Custom Resources;
  - **Control plane**: etcd, System services, Security, Lifecycle, Operations;
  - Settings.
- **The context switcher replaces the Contexts list.**
  - It shows the context's name, a state dot and a detail line ("Talos + Kubernetes · 6 nodes").
  - It opens a list of the contexts. ⌥↑ and ⌥↓ keep working.
  - Names can be long. The switcher truncates in the middle so both ends stay readable, and shows the full name in its tooltip and in the open list.
- **Node pages are per machine, in a node pane.** Processes, Storage, Network, Diagnostics, the node's system services and its logs become tabs of a pane on the Nodes page.
  - The pane can expand to the full content width and back.
  - The NODE section and the TARGET picker go. The selected node is the target.
- **System services are also cluster-wide,** as Control plane › System services: one table of every node's services, problems first. Restarting a service stays in the node pane.
- **Names.**
  - Talos "Services" become "System services". The plain words (Services, Storage, Network) are kept for Kubernetes and for the node pane's tabs.
  - "Workload health" becomes Workloads › Health.
- **A Kubernetes summary refreshes on every page,** with the Talos overview on the shell's 15 s cycle. It feeds the Overview, the sidebar's counts and badges, the Nodes join, the pod pane's "Runs on" and Health.
  - It reads from the API server's cache (`resourceVersion=0`).
  - A stress check at 20,000 pods gates it.
  - This is a deliberate exception to "hidden pages never contact the cluster". Lists and screens still load only when shown.
- **Search everything (⌘K) lists names when it opens.** It runs one metadata-only list per kind in a fixed set, capped at 2,000 each, and keeps them until the dialog closes. Secret values are never fetched.
- **Keys ⌘1–⌘9:** Overview, Nodes, Namespaces, Events, Health, etcd, System services, Security, Lifecycle.
- **Kubernetes-only mode hides Control plane** instead of dimming it, with one line in its place: "System services, processes, etcd and node logs come from Talos. Add a talosconfig". The Overview, Nodes, Health and search work from Kubernetes alone.
- **The status bar names both layers**: "Connected to prod-fra · Talos v1.13.2 · Kubernetes v1.32.3", then forwards and the last success, as today.

## Design

### Pages

| Today | Becomes |
| --- | --- |
| Overview | Cluster › Overview, with Kubernetes and Talos cards and one Needs attention list |
| Resources › Cluster › Nodes | Cluster › Nodes, a new page (`Page::Nodes`) joining both sides |
| Resources › Cluster › Namespaces, Events | Cluster › Namespaces and Events: the same Resources lists, with their rows moved up |
| Resources › Cluster › Leases | Resources › Administration, last |
| Services | Node pane › System services (one node, with Restart), plus Control plane › System services (all nodes) |
| Logs | Node pane › Logs |
| Processes, Storage, Network, Diagnostics | Node pane tabs |
| etcd, Security, Lifecycle | Control plane, unchanged inside |
| Workload health | Resources › Workloads › Health (`Page::Health`), the same screen |
| Operations | Control plane › Operations, unchanged inside |
| Contexts list | The context switcher |
| TARGET picker | Gone. The node pane's node is the target |
| ⌘K "Go to kind…" | Search everything |

- **`Page`** becomes Overview, Nodes, Health, Resources, Etcd, SystemServices, Security, Lifecycle and Operations.
  - Namespaces and Events are sidebar rows that open `Page::Resources` on their kind. A row is active when that kind is shown.
  - Opening the `nodes` kind from anywhere (`open_builtin`, ⌘K, a link) goes to `Page::Nodes`.
- **The Workloads group** starts with Health, a page row, then keeps today's kinds in today's order. ReplicaSets stay; the mockup left them out by mistake. The Cluster group of `navigation::NAVIGATION` goes.
- **Stable ids.**
  - Pages: `nav-overview`, `nav-nodes`, `nav-health`, `nav-etcd`, `nav-system-services`, `nav-security`, `nav-lifecycle`, `nav-operations`.
  - Kinds keep `nav-k8s-<key>`, so Namespaces is `nav-k8s-namespaces`, and groups keep `nav-k8s-group-<slug>`.
  - The switcher is `context-switcher`, its rows keep `("context", ix)`, and search is `search-everything`.
- **Counts and badges** come from the two summaries:
  - Nodes: the joined rows.
  - Pods: the total.
  - Events: warnings in the last hour (warn).
  - Health: unhealthy workloads (crit).
  - System services: unhealthy services (crit).

  While a summary is stale, its numbers are dimmed. Badges are computed when a summary lands, never in render.
- **`FRESHKUBE_PAGE`** takes the new slugs, plus `node-logs` for the node pane's Logs tab on the first responding node. `bin/stress.rs` uses `node-logs` for the log flood.

### The Kubernetes summary

Core gets `kubernetes_summary.rs`. It provides `collect_kubernetes_summary(client) -> KubernetesSummary`, which reads concurrently on Tokio with a 15 s timeout.

- **Reads.**
  - The server version.
  - Typed lists of nodes, pods, deployments, statefulsets, daemonsets, namespaces, persistentvolumeclaims and persistentvolumes.
  - Events with `type=Warning`.
  - Every list uses `ListParams::default().match_any()`, which is `resourceVersion=0` and is served from the API server's cache. `collect_workloads` moves to the same parameters and becomes the summary's workloads part.
- **Each part has its own state:** loaded, refused (403, saying which list) or failed. A refused events list doesn't blank the pods card. A failed refresh keeps the previous summary, marked stale.
- **It keeps derived data, not objects.** Typed lists are dropped once read.
  - **Nodes:** name, Ready (status, reason, message, since), unschedulable, roles from labels, InternalIP and ExternalIP, kubelet version, capacity, pressure conditions, and the number of pods on it.
  - **Pods:** counts by phase, and the issues `collect_workloads` already classifies (CrashLoopBackOff, image pull, Pending, high restarts). Each issue keeps its namespace, name, node, restarts and since. Pods on a NotReady node are counted too.
  - **Workloads:** today's `WorkloadSnapshot`.
  - **Events:** warnings whose last time is within the hour, counted by reason, plus the newest per involved object.
  - **Storage:** Pending claims with their age and the reason from their newest warning event, the number bound, and the PVs Available.
  - **Namespaces:** the count.
  - Every list of issues is capped at 200, newest kept.
- **The desktop holds it on `Pilot`** next to the Talos overview. It refreshes in `refresh`, on the same cycle and with the same epoch, so a late answer for another context is dropped.
  - In Talos mode it reads through `LiveSource::kubernetes()`, in Kubernetes-only mode through the context's `DirectAccess`.
  - Display rows are built on Tokio, as first lists are today, so the main thread only swaps them in.
- **Health reads the summary's workloads** instead of making its own lists. So it also works in Kubernetes-only mode, and the two never disagree.
- **Cost gate.** `scripts/stress.sh` gains a `summary` workload: the synthetic API serves 20,000 pods, 2,000 deployments and 5,000 warning events. `docs/PERFORMANCE.md` records the time on Tokio, the main-thread time per refresh (budget 16 ms) and the memory. If it misses the budget, the summary refreshes only while Overview, Nodes or Health shows, the badges keep their last value marked stale, and PERFORMANCE.md says why.

### Nodes and the node pane

`desktop/nodes/` holds the page, its join and its pane.

- **The join** is a presentation function, run when either summary lands. It produces `NodeRow`s, keyed by `NodeKey { kubernetes: Option<String>, talos: Option<String> }`.
  - A Talos node joins the Kubernetes node with the same name. Failing that, it joins the one whose InternalIP or ExternalIP equals its address. Each node joins at most once.
  - A Talos node with no partner shows "Not in Kubernetes" in the Kubernetes column. One example is a control plane before bootstrap.
  - A Kubernetes node with no partner shows "No Talos data". Its pane has only the Kubernetes tabs.
  - When one side can't be read at all, every row says so in that side's column ("Talos unavailable", "Kubernetes unavailable"). Rows still come from the other side.
  - The row's id is `node-<name>`, using the Kubernetes name when there is one.
  - **Order:** control planes first, then by name. The order is fixed when the rows are built.
  - **Tone:** the worst of Kubernetes NotReady (crit), no Talos response (crit), an unhealthy system service (warn) and memory at 90% or more (warn). Cordoned shows as a muted tag.
- **Columns** (screen 02): Name, Role, Kubernetes (Ready, NotReady or a dash), Talos (version or "No response"), Load, Memory, Pods, System services.
  - The Cards/Table toggle moves here from today's Overview. Its cards keep the load sparkline and memory bar, and their buttons become Open.
  - In Kubernetes-only mode the Talos columns are hidden.
- **Selecting a node** opens the pane (screen 03). With a Talos side, it calls `select_node_by_name`, so today's target plumbing stays. The selected node is the target for every screen.
  - Closing the pane keeps the target.
  - The first responding node stays the default target, as `sync_nodes` does today.
- **Layout.**
  - With no pane open, the table fills the page.
  - With a pane open, the list narrows to a compact column (name, status dot and a short note such as "cp", "92 %", "1 svc" or "NotReady") and the pane takes the rest. They sit in an `h_resizable`: list at least 240 dp, pane at least 480 dp.
  - Below the split width (900 dp), the pane replaces the list and shows a back button.
- **Expanded.** An Expand button in the pane's header, or Command-Shift-Return, hides the list. Collapsing brings it back with the same selection and scroll.
- **Header:**
  - the node's name and its Kubernetes Ready tag, or the Talos state when there's no Kubernetes side;
  - chips for role, address, Talos version, kubelet version, and cores and memory;
  - Expand and Close.
- **Tabs.** The pane remembers its tab when the node changes, if the new node has it.
  - **Overview** is new and makes no reads. From both summaries it shows:
    - Kubernetes conditions, taints and cordon, capacity and addresses;
    - the Talos version, response, load sparkline, memory, etcd membership and service health counts;
    - the node's rows from Needs attention.
  - **Pods N** is a Resources list of pods across all namespaces, with the field selector `spec.nodeName=<name>`. It has its own filter and no pane of its own: Enter opens the pod on the Resources page (see [Opening objects](#opening-objects)).
    - Core's `watch_collection`, `list_table` and `watch_table` gain an optional field selector, carried in the watch's identity. Example data filters its pods by node.
  - **System services** is today's `desktop/services.rs` for this node, with Restart and Open logs.
    - The "unhealthy on another node · Show it" notice goes; Control plane covers it.
    - When the kubelet is selected, its detail adds "Pods on this node · 31", which goes to the Pods tab.
    - The tab carries a dot when a service is unhealthy.
  - **Processes, Storage, Network and Diagnostics** are today's screen panels. `ScreenPanel` gains an embedded mode that leaves out the "on node · address" header line, since the pane's header says it. Their gates ("No node selected", "isn't responding") stay.
  - **Logs** is today's log panel for the target node. It keeps collecting in the background, as now.
    - The tab carries the green dot while collecting.
    - The status bar shows the logs line while this tab is visible.
  - **More ▾** opens Events and YAML for the Kubernetes Node, reusing the resource pane's events and YAML views. If they're too tied to `DetailPane`, embed a `DetailPane` for the Node limited to those two tabs.
  - A node with no Talos side has Overview, Pods, Events and YAML. A node with no Kubernetes side has Overview and the Talos tabs.
- **Keys.**
  - ↑ and ↓ move through the list (today's `TalosNodes` context), and Enter opens the pane.
  - Command-Shift-[ and ] switch tabs, and ← and → do on the tab strip.
  - Escape steps back: out of the tab, then closes the pane.
- **Screen events.**
  - `ScreenEvent::SelectNode` opens that node's pane.
  - `OpenLogsOn { node, service }` opens its Logs tab with that service (`logs.open_service`).
  - `OpenLogs` does the same on the target node.
  - Lifecycle's "Target ⟨node⟩" button becomes "Open node".

### Control plane › System services

`desktop/system_services.rs`, built from the Talos overview's per-node service snapshots. It makes no new reads.

- **One `uniform_list` row per node and service:** Node, Service, State, Health and the health message. The overview's snapshots carry no service events, so the last event stays in the node pane.
  - Order: unhealthy first, then not reported, then by node and service. The order is fixed when the overview lands.
  - Filters: text, health (All, Unhealthy, Not reported, as today's) and a node picker.
- **Actions:**
  - Logs opens the node's pane on the Logs tab with that service.
  - Open node opens it on the System services tab with the service selected.
  - Restart is not offered here.
- The sidebar row carries the unhealthy count that the Services row has today.

### Overview

Screen 01. It keeps today's header extras: the version drift tag, the roster pill and the discovery and kubeconfig warnings. Its node cards move to Nodes.

- **Header:** the context, Connected, and "Talos v1.13.2 · Kubernetes v1.32.3 · 6 nodes · 140 pods · 31 namespaces".
- **Eight cards,** four to a row, two to a row below 900 dp. Each opens its target. A card whose part was refused or failed says so ("Can't list pods: forbidden") instead of a number.

| Card | Shows | Opens |
| --- | --- | --- |
| Nodes (`tile-nodes`) | Ready of the joined rows, a segment per row in its tone, control planes and workers | Nodes |
| Control plane (`tile-etcd`) | etcd quorum and tolerance as today, then "Kubernetes API answering" or why not | etcd |
| Workloads (`tile-workloads`) | Unhealthy count, the first one ("payments/worker · 0 of 1 available"), healthy and progressing | Health |
| Pods (`tile-pods`) | Total; CrashLoopBackOff, Pending and on a NotReady node, each when not zero | Pods, filtered to the first issue's status |
| System services (`tile-services`) | As today | System services, Unhealthy |
| Peak memory (`tile-memory`) | As today | That node's pane, Processes |
| Events (`tile-events`) | Warnings in the last hour, the top reasons | Events, filtered to "Warning" |
| Storage (`tile-storage`) | Pending claims, the first one, bound, PVs available | PersistentVolumeClaims, filtered to "Pending" |

- **Filtered lists** use the existing filter box: `ResourcesScreen::set_filter(text)` sets it and focuses the list.
- **Needs attention** is one list from both sides, built by `presentation::attention` when either summary lands.
  - **Order:** crit, then warn, then longest-standing, then name.
  - **One row per subject.** A node whose Talos API is silent and whose Kubernetes Node is NotReady is one row: "Talos API not answering · Kubernetes NotReady for 4 min".
  - **Subjects:** nodes, pods, workloads, system services, claims, and etcd (no quorum, an alarm).
  - **Each row:** a tone dot, the kind, the name in mono, the reason, and actions.
    - Open: the object's list and pane, or the node pane.
    - Logs: a pod's Logs tab, or a service's node Logs tab.
    - Open node: for a service.
  - **Size and ids:** eight rows show, then "Show all N" up to 50. Row ids are `attention-<kind>-<namespace or node>-<name>`.

### Opening objects

`Pilot::open_object(kind, ObjectRef { namespace, name, uid }, tab)` is the one way any link reaches an object. Cards, Needs attention, the node pane's Pods tab, the pod pane's links and search all use it.

1. It shows the kind, as `open_kind` does.
2. It keeps the namespace picker if it's on All or on the object's namespace; otherwise it switches to All.
3. It clears a filter that would hide the row.
4. It selects the row once the list has it and opens the pane on the given tab. If the list loads without the row, it opens the pane by identity anyway, since the pane reads the object itself.

A running shell's pin is respected, because opening goes through `open_now` and `unless_shell` as today.

### The pod pane's Overview

Screen 04. These sections come first, for pods only; the generic summary follows unchanged.

- **Runs on:**
  - the node, linked to its pane;
  - its Kubernetes Ready;
  - any Talos problem on it from the shell's summaries (no response, an unhealthy service, high memory). An unhealthy service links to the node pane's System services tab.

  The pane gets the joined node rows as a shared snapshot that updates with the summaries. It makes no reads.
- **Controlled by:** the owner references as links.
  - For a ReplicaSet, one read of it when the pane opens adds its own controller: "ReplicaSet … → Deployment …".
  - The generic "Owned by" line becomes links for every kind.
- **Selected by:** the Services in the pod's namespace whose selector matches its labels. One typed list of that namespace's Services is made when the pane opens. Each Service has Forward, which opens its pane on the Ports tab.
- **Containers,** from the `PodContainers` core already returns: name, image, state and restarts, with Logs, plus Previous instance when the container has restarted.
- **Recent events:** the newest three warnings from the events the pane already watches, and "All events", which goes to the tab.

### Search everything

`desktop/search/` replaces `kind_switcher.rs`, in a Kit dialog opened by ⌘K or the sidebar button.

- **What it searches.**
  - Pages and sidebar rows, kinds as today, and nodes from the joined rows. These are local, with no reads.
  - Objects. When the dialog opens, it makes one `list_metadata` per kind, at most 2,000 each, all namespaces, concurrently with a 5 s timeout. The kinds are pods, deployments, statefulsets, daemonsets, services, ingresses, configmaps, secrets and namespaces.
    - Results appear as each kind answers. A refused kind says "Can't list secrets", and a capped one says "First 2,000 shown".
    - Metadata lists carry no Secret data, so values are never fetched.
    - The names are kept until the dialog closes. A reopen within 30 s reuses them.
- **Matching** is on name and namespace/name. Results are grouped (pages, nodes, pods, workloads, networking, config, kinds), eight to a group, with "N more".
- **Enter opens:** a page, a node's pane, or an object through `open_object`.
- **Example data** answers from the example store.

### Context switcher

- **The trigger:**
  - the F mark;
  - the state dot (connected, connecting, couldn't connect);
  - the name in mono;
  - a detail line: "Talos + Kubernetes · 6 nodes", "Kubernetes · 3 nodes", "Connecting…" or "Couldn't connect".
- **Middle truncation.** The name is mono, so the number of characters that fit is the trigger's width divided by the mono advance at the current text size.
  - Keep the first half and the last half around "…": `talos-production-…-baremetal-b7`.
  - Compute it when the contexts, the selection or the text size change, never in render.
  - The tooltip has the full name.
- **The list** is a popover with today's context rows. Full names wrap to a second line instead of truncating. The ⌥↑ ⌥↓ hint is at its top.
- **The app's version** moves into the Settings popover.

### Title bar and status bar

- **Breadcrumb:** context / page. On Nodes with a pane it reads context / Nodes / node / tab, and on Resources, context / kind. The Nodes crumb closes the pane.
- **Without the TARGET picker,** the title bar keeps the theme toggle and the refresh ring. Its accessible label names the context, page and node.
- **Status bar:** "Connected to ⟨context⟩ · Talos ⟨version or range⟩ · Kubernetes ⟨version⟩". Kubernetes-only mode leaves out Talos. Everything else stays.

### Kubernetes-only mode

- **Sidebar:** Control plane is replaced by the one-line hint; its "Add a talosconfig" link opens Settings. ⌘6–⌘9 do nothing.
- **The app starts on Overview,** not Resources. Overview has the Nodes, Workloads, Pods and Events cards (screen 06), and Needs attention holds only Kubernetes subjects.
- Nodes, the node pane's Kubernetes tabs, Health and search all work.
- The "Needs a talosconfig" empty state stays for any Talos view reached some other way.

### Keys

- **⌘1–⌘9:** Overview, Nodes, Namespaces, Events, Health, etcd, System services, Security, Lifecycle. The sidebar draws their keycaps.
- **⌃Tab and ⌃⇧Tab** step through the sidebar's fixed rows: Overview, Nodes, Namespaces, Events, Health, Kinds, etcd, System services, Security, Lifecycle, Operations.
  - Kinds is the last kind shown other than Namespaces and Events, Pods at first.
  - Kubernetes-only mode skips Control plane.
- **Unchanged:** ⌥↑ and ⌥↓ change context, and ⌘K opens search.

### Example data

- **New kinds,** consistent with what exists:
  - ReplicaSets owned by the Deployments and owning the pods;
  - cluster-wide Events built from today's per-object events;
  - PersistentVolumeClaims, with `batch/report-data` Pending because StorageClass "fast" isn't found;
  - a few PersistentVolumes.

  The Kubernetes summary has an example built from the same store, so cards, the sidebar and lists agree.
- **A fourth context with a long name,** `talos-production-frankfurt-equinix-fr5-baremetal-b7`, shaped like homelab, for truncation. Tests that count or cycle contexts change with it.

### What stays

Lists, the detail pane, shells, forwards and their questions, custom resources, operations and its preflight, text size and themes are unchanged, except where this file names a change. A shell or forward keeps running across every new navigation path.

## Build order

Each step lands with its tests, the gates in AGENTS.md, and a Done row in the roadmap. When a step changes how things are found, AGENTS.md and README.md change in the same commit.

1. **The Kubernetes summary and example data.**
   - `kubernetes_summary.rs` with unit tests on JSON fixtures: Ready and its since, pod issues, pods on NotReady nodes, warnings within the hour, Pending claims, a refused and a failed part, caps.
   - The refresh on `Pilot` with its epoch.
   - Health reads it.
   - The new example kinds and the long-named context.
   - The stress workload, with its numbers in PERFORMANCE.md, and AGENTS.md's rule about hidden pages amended.
   - **Accepted when** Health shows the same numbers as before from the summary, and a UI test shows a refused events list leaving the other parts loaded.
2. **Nodes and the node pane.**
   - Field selectors in core.
   - `Page::Nodes` with the join, table and cards.
   - The pane with every tab, embedded screens, Expand and the narrow layout.
   - For now, a Nodes row at the top of today's sidebar; the old pages stay until step 3.
   - **Accepted when** UI tests show:
     - the joined rows for prod-fra, including talos-wk-fra1-02 (NotReady, kubelet unhealthy) and talos-wk-fra1-03 (no response);
     - a Kubernetes-only node;
     - Pods listing only that node's pods;
     - Expand and collapse keeping the selection;
     - tabs kept across nodes;
     - the target following the pane.
3. **Sidebar and navigation.**
   - The new sections and `Page` list, Health under Workloads, Namespaces and Events rows, and Leases moved.
   - Control plane › System services.
   - The context switcher with middle truncation, the breadcrumb, the status bar text, and the version in Settings.
   - ⌘1–⌘9 and ⌃Tab.
   - Screen events routed to the node pane.
   - Services, Logs, the NODE pages and the TARGET picker removed.
   - Kubernetes-only mode's hint and starting page, and `FRESHKUBE_PAGE`.
   - **Accepted when:**
     - the tests in `desktop/tests.rs` that name `nav-*` ids, the TARGET picker, ⌘ keys or page order are moved to the new places, not deleted;
     - a test shows the long name truncated in the middle at text sizes 14 and 20, with the full name in the list;
     - a test shows every old page's view reachable from its new place.
4. **Overview.**
   - The cards and Needs attention.
   - `open_object` and `set_filter`.
   - The Kubernetes-only Overview.
   - **Accepted when** UI tests open each card's target with its filter, and each attention row's actions, from example data.
5. **Cross-links.**
   - The pod Overview's Runs on, Controlled by, Selected by, Containers and Recent events.
   - Owner links for every kind.
   - The kubelet's "Pods on this node".
   - **Accepted when** UI tests follow pod → node pane, pod → ReplicaSet → Deployment, and pod → Service → Ports, with a running shell asking before the pane moves.
6. **Search everything.**
   - The dialog, local results and metadata lists.
   - **Accepted when** UI tests find a page, a node, a pod and a Secret by name from example data, a refused kind says so, and no Secret data is read.
7. **Docs and visual check.**
   - AGENTS.md's sidebar and page descriptions, README.md, and this file's state.
   - Screenshots from `--fixture`, in light and dark, at text sizes 14 and 20, and at the 760 × 560 minimum, compared with the mockups.
   - The user then checks it against the live cluster. A live check runs only with their go-ahead.

## Left to the build

These have a default, so they don't block a step. Change one only by updating this file in the same commit.

- **Expand's key** is Command-Shift-Return. If a terminal or text field needs it, pick another and say so.
- **No uptime chip** in the node pane header unless Talos data the app already reads carries the boot time.
- **No Command-Enter in search.** Enter opens, focusing the pane for objects.

- **The batch example** gains a Deployment for `report`, so its pod → ReplicaSet → Deployment chain resolves in the same example store.

- **Nodes starts in Table view.** It matches screen 02; the Cards toggle keeps the load history and memory bars in a virtualized grid.
- **Node Events and YAML** use an embedded `DetailPane` with its own header and tab strip hidden; reads run only on those tabs.

- **Context rows** use a 360 dp popover with wrapped full names. Clicking a node tab focuses its content; the tab strip remains in the keyboard order for arrow navigation.

- **Narrow node headers** keep chips and tabs in horizontally scrolling rows, so the inspection view retains height at the minimum window size and at 20 px. Keyboard tab changes reveal the active button.

- **Attention ages** use the Ready transition for nodes and the creation time for Pending claims. Pod creation time orders issues when the API has no issue transition; services and workloads with no transition time follow dated subjects, then sort by name. It does not label a pod’s age as the duration of its failure.
- **etcd attention** merges quorum and deduplicated alarms into one cluster subject. The existing Talos overview cycle reads alarms alongside status; an unavailable alarm response stays unknown. Node attention retains up to fifty rows for each node independently of the cluster list’s fifty-row cap.
- **Workload card counts** call Pending workloads progressing; Degraded and Failing count as unhealthy. Their first problem follows the core health order.
