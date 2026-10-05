# Changelog

Freshkube's history starts at 0.2.0. The entries from 0.1.11 down are talos-pilot's.

## Unreleased

### Coroot

- **Applications uses the shared page and table:** namespace groups show their worst health and app count, with comfortable or compact rows, Columns, Show all and a legend. Categories start with applications only, falling back to all when that category is absent; All also includes newly discovered categories. The count follows the filters, which return the list to its first row. Healthy figures stay plain and healthy checks without a figure read “ok”; unknown reports keep their outlined circle and missing reports show “—”. Full captions and compact signal values keep their widths at each text size; failing upstreams show a count with their names in the tooltip. The namespace picker is searchable, tooltips name the app, and time ranges include local dates. Traces and Profiling show a failed first read with Retry.
- **Incidents is a table page like Pods:** a filter, Open and Resolved counts that filter the list, comfortable or compact rows, Columns, and "Showing n of m" with Show all in place of Previous and Next. The selected incident opens in a pane beside the list, or below it in a narrow window, with its objectives, Coroot's analysis and the affected applications. Rows stay selected by incident and application when Coroot answers again. Duration and Impact are hidden by default, since the pane's summary shows both; Columns shows them. Times are local, not UTC. A typed filter and the chosen count survive a reconnect. Example data now goes through the same page as live data, so the example-only mute, fix, edit and diff actions are gone; Incidents stays read-only.
- **Live Traces:** an application's latency and error heatmap, its latest requests, and the selected trace as a waterfall with each span's status, attributes and events. Click a cell to list that bucket's requests, or list the failed ones; choose OpenTelemetry or eBPF when Coroot has both.
- **Live Profiling:** an application's profile types, instances and flame graph, with zoom, search and a comparison with the window before that colours each frame by its change and lists the biggest increases.
- **A tidier Observability page:** the source, project and time picker sit in the page header, with one page Refresh. Coroot connection settings, Disconnect and the Kubernetes link open from the sidebar footer. The column uses the shell's usual rows, so only the open destination is highlighted, and Deployments shows only with example data. Application names take the table's spare width. Traces and Profiling pick an application by typing part of its name. The flame graph names only frames wide enough to read, outlines the selected one and shows its details above the graph.
- **Coroot's markup no longer shows:** report titles and messages lose HTML tags such as `<var>` and decode entities, keeping a plain `<` or `>` in text such as "latency < 500ms".
- **Traces is a table page like Pods:** example and live data now draw the same page. A filter over request name, service and trace id, the sources, All and Failed requests, Columns and the time range sit in the page header. Requests are one-line table rows with a status glyph, service, local start time and duration; a failed request's message is in its tooltip. The selected request's trace opens in a pane beside the list, or below it in a narrow window, and stays selected by trace and span when Coroot answers again. The heatmap's axis reads local time. The example-only error causes, Errors only, Clear selection and heatmap arrow keys are gone. Incident titles read as prose, and long text in both tables ends in "…" instead of clipping mid-word.

### Other changes

- **Nodes looks like Pods:** the same page header and table styling, with a text filter, source freshness, matching health glyph counts and problems first. Healthy rows fold while problems exist; comfortable/compact rows, a Columns menu and 12 px cards keep the retained node pane.
- **Nodes shows CPU and memory with Pods’ request/use meters** against node allocatable, retains stale metrics, and falls back to Talos memory when metrics-server is unavailable; load averages are an optional column.
- **New status glyphs:** every status is a round glyph whose inside tells the state: a dot for OK, a half-filled ring for a warning, a disc with a bar for critical, a dashed ring for pending and a ring with a plus where an integration is needed. A pod whose container crashed, errored or ran out of memory shows a small skull; one that can't start, such as an image that won't pull, keeps the critical disc. Both count as failing. Health and Overview's Needs attention show the skull for those pods too.
- **System services looks like Pods:** the same header, table and row heights, with comfortable and compact rows. Services group by health while any is unwell; the counts beside the filter (unhealthy, not reported, healthy) filter the list, and a refresh keeps your filter and place.
- Grafana text panels show readable text instead of raw Markdown or HTML: images are dropped, links keep their label.
- **Overview looks like the other pages:** the shared page frame and header, with the context as the title and the connection state, version drift, node list source and what the cluster runs on the line below. Needs attention is a card of compact rows grouped as Failing, Warning and Unknown, each with its glyph, kind, name, reason (in full in the tooltip) and actions; a node's pane shows its own rows the same way. Showing 8 of N and Show all replace the button; past fifty rows the bar says how many are left out. Rows from a stale source now come after current warnings, so the fifty kept are the current ones first.
- **Quieter column headers:** tables label their columns in sentence case (Name, Ready, Health check), not uppercase.
- **Pinned cells take their own clicks:** in a table scrolled sideways, a click, hover or tooltip on a pinned name or header no longer reaches the cell passing under it, so clicking the pinned Name header sorts by name alone.
- **Wide tables keep their bearings:** scrolled sideways, a table's group rows keep their label in view, and Pods, every other Resources kind, System services and Nodes keep each row's status and name at the left edge.
- **Overview's cards share Monitoring's card:** each figure takes its tone's colour above its detail, with a meter of the cluster's nodes (in large clusters, a run per state, problems first, so one failing node still shows) or Peak memory's bar in the card's colour. A card waiting for its first answer shows a skeleton, and one showing the last known answer has the stale mark with the reason. Tab reaches each card, and Enter or Space opens it.
- **Slimmer table headers:** a table's header row is 26 high, down from 30, closer to a desktop app's.
- **One row height:** every table draws 26-high rows, and the Density control is gone; a larger text size scales the rows for anyone who wants them bigger.
- **A table scrolls the way you swipe:** a plain wheel scrolls a table's rows and Shift+wheel or a sideways swipe scrolls its columns, without nudging the other way; a slightly diagonal trackpad swipe keeps to its main direction.
- **Tables without a card:** Pods, every other Resources kind, Nodes, System services and Applications draw their table straight on the page, with a hairline above and below, instead of inside a rounded card.

## 0.5.0 (2026-10-04)

### Metrics sources

- **Monitoring reads any Prometheus API:** besides Prometheus, discovery finds VictoriaMetrics (single-node and vmselect), Thanos Query and Mimir query frontends, with their ports and path prefixes. Each is confirmed with a query before use, so a missing prefix or refused token shows at once.
- **Settings → Metrics source**, per context: Automatic, one Service through the Kubernetes API (namespace, name, port, path prefix, HTTPS), or a URL read directly from your Mac with an optional bearer token. **Test** checks the form before you save it. The token is kept in the Keychain; `monitoring.json` only notes that one exists.

### Coroot

- **Read-only Incidents:** the project's latest incidents, each with its SLO objective, compliance and burn rates, root-cause analysis and suggested fixes as Coroot reports them, and links from propagated applications to their reports. Missing evidence is marked not reported, never invented. See [COROOT.md](docs/COROOT.md#read-only-incidents-slice).
- **Freshkube remembers the Coroot connection:** server, sign-in choice and last project, in `coroot.json` beside the preferences. **Remember key in Keychain** keeps the key in the system's credential store, sent only to its own server. The page reconnects when it first shows, and Disconnect forgets the key.
- **A visual pass:** Observability gets its own rail icon; chosen chips, toggles and segments are blue everywhere, Settings included; reports merge REST and MCP evidence with formatted units; the service map is layered with weighted, arrowed links; and an unconnected page shows one Connect Coroot state.

### Other changes

- On Linux and Windows, preferences move to the platform's folder: `~/.config/freshkube` and `%APPDATA%\Freshkube`. macOS keeps `~/Library/Application Support/Freshkube`.
- Lifecycle's Kubernetes version rules move into core, with no change in behaviour.

## 0.4.0 (2026-10-04)

### Live Coroot

- **Read-only Applications, service map and application reports** from an explicitly selected Coroot server and project. Live observations and fixture data use the same presentation, preserving Coroot's reported health and evidence, including healthy, unknown and absent signals.
- **Explicit connection settings:** enter an HTTP(S) URL and an API key or existing session cookie, or choose anonymous access. Credentials stay in memory; editing or clearing them cancels pending reads, and Disconnect forgets the connection.
- **Kubernetes links target the cluster you associate:** explicitly associate a Coroot cluster with the selected Freshkube access before opening its objects. Navigation uses the existing object resolver and running-shell confirmation.
- **Bounded reads and clear source states:** empty results, refused or failed requests, stale evidence, partial reports and unavailable capabilities remain distinct. Hiding Observability cancels its reads; selection survives reordering, and delayed responses cannot cross source, project, credential, time-range or application changes.
- **The first slice stays read-only.** Live incidents, deployment comparisons, CPU profiling, full report histories and missing trace detail remain follow-up work. Fixture mute, threshold and rollback controls retain their preview behavior. See the [supported evidence and limitations](docs/COROOT.md).

### Cluster reliability and interface

- **Access identity follows authenticated access and configuration changes.** Reloading credentials or replacing a configuration at the same path invalidates ordinary cached observations and delayed answers. Compatible retries and node selection retain the cluster access; explicit port forwards keep their captured connection until stopped.
- **Passive FPS readout in the status bar:** shows frame cadence during activity and a neutral state while idle, without requesting frames from its idle timer. The status bar adapts to narrow windows and larger text.

### Installation

- Install with Homebrew: `brew install --cask skel84/tap/freshkube`. The cask in [skel84/homebrew-tap](https://github.com/skel84/homebrew-tap) follows published releases, pre-releases included, and links the `freshkube` command.

## 0.3.0 (2026-10-04)

### Fog interface

- **A calmer, denser interface:** grey content, a blue-slate frame, Figtree for the UI and IBM Plex Mono for resource names and numbers. Status keeps a consistent glyph and colour; CPU and memory use compact bullet meters with explicit request, limit and stale-data states.
- **Collapsible contextual navigation:** Command-B toggles the sidebar between its full list and an icon strip. The choice is remembered, narrow windows collapse automatically, and Workloads shows namespace Pod counts.
- **Pods open on problems:** cause groups, status filters, comfortable/compact density, a Columns menu and bulk copy controls. Readiness and restarts share one cell; node and owner references stay quiet until hovered. The toolbar and detail layout adapt to small windows and larger text.
- **Pod detail and charts follow Fog:** the cause stays first, Runs on adds a node memory bullet meter, and Prometheus panels use the new chart palette with neutral value text.

### Observability prototype

- **Seven interactive Coroot-inspired screens in fixture mode:** Applications, service map, application reports, incidents, deployment spec comparisons, CPU profiling and traces. Open them with `freshkube --fixture`.
- **Fictional data only:** real connections show Integration required. Threshold edits and incident muting stay in the example session; fixes and rollback show previews. Live Coroot integration is planned separately ([integration plan](docs/COROOT.md)).

### Cluster reliability and builds

- **Kubernetes summaries stay current through session-owned watches**, with independent source availability and shared observations for Overview, Health and Lifecycle. Namespace Pod counts feed the new sidebar. The remaining CPU cost under large watch bursts is tracked in [#22](https://github.com/skel84/freshkube/issues/22).
- **etcd reports remaining failure tolerance:** a cluster with 2 of 3 or 3 of 5 members available has no additional failure margin and shows a warning. Overview, etcd and operation checks share the same quorum calculation.
- **Shared node-health and Operations policy**, with regression coverage for unavailable sources, per-node safety gates, cancellation, deadlines and partial failures.
- **Talos protobuf code is generated in Cargo's build directory**, so native and CI builds leave the source checkout clean. Source builds still require `protoc`.

## 0.2.1 (2026-10-03)

### Monitoring

- **Dashboards from the cluster's Prometheus**, drawn natively in the Console look: timeseries, stats, gauges, bar lists and tables. Freshkube finds Prometheus through the Kubernetes API's service proxy and remembers the Service per context. It sends GET requests only, and reads nothing while the page is hidden.
- **Built-in Cluster and Workloads dashboards**, then your own folder of Grafana JSON (Settings → Dashboards folder). Variables, a time picker and auto-refresh work as in Grafana. Only panels in or near view are asked.
- **One cursor across every chart** on the page. Hovering a legend entry fades the other series, and a panel's title shows its PromQL after interpolation, with Copy. With many series, the readout shows the highest values that fit the panel.
- **Deploy and node markers** on every timeseries: a Deployment's new ReplicaSet, and nodes turning NotReady, rebooting or Ready again.
- **CPU and memory over the last hour** near the top of a pod's Overview and in the node pane, from the Prometheus the Monitoring page found.
- **Crowded dashboards stay fast.** Dozens of series past the six colours draw as one grey line and area, rather than one GPU pass per series.

## 0.2.0 (2026-10-03)

The first release of **Freshkube**, a native macOS app for Talos Linux and Kubernetes clusters. It began as a fork of [talos-pilot](https://github.com/Handfish/talos-pilot), whose terminal UI stays upstream. Freshkube reads by default. It changes a cluster only through Talos operations behind their own confirmation, and through a pod shell or port forward that you start yourself.

### Cluster

- **Overview** shows cluster cards and an Attention list of what needs a look (node problems, services, pods, workloads, claims and etcd alarms), each linked to the object, its logs or its node. Health reads the same snapshot.
- **Nodes** joins Kubernetes and Talos into one table or set of cards. A node's pane holds its Talos screens, service logs, Events, YAML and the pods running on it.
- **Navigation:** the sidebar groups Cluster, Resources and Control plane (etcd, System services, Security, Lifecycle and Operations). The context switcher sits above it; Command-1 to Command-9 open the numbered rows, and Control-Tab switches to the last other kind.
- **Search everything** (Command-K) finds pages, kinds, nodes and objects by name. It reads only metadata, capped per kind, and never asks for Secret data.

### Kubernetes resources

- **Browsing:** the Resources page lists any kind through the server's table view, kept current by a watch, with a namespace picker and filter. In Talos mode it uses the cluster's Kubernetes connection from Talos.
- **Kubernetes-only mode:** without a talosconfig, or with `--kubernetes-only`, Freshkube browses Kubernetes alone. The sidebar lists the kubeconfig's contexts and connects to its current one, or to the one `--kube-context` names. A named context that is missing is reported, never replaced.
- **Detail pane:** the selected object's Overview, YAML and Events, beside the list or below it in a narrow window. It reads the object again when the list sees a new version, at most once a second, and says when a read is refused, failed or stale, or the object was deleted or recreated. Secret values stay hidden: you reveal, copy or hide one key at a time, and Copy YAML always copies the hidden form.
- **Cross-links:** a pod's Overview links its Services, owners, node and recent warnings. Every link opens through the same guard, which keeps a matching namespace and asks before ending a running shell.
- **Custom resources:** the sidebar lists the cluster's API groups and their kinds when opened. Custom kinds open like built-in ones, with the server's columns. A group that is refused or unreachable says so, with Retry, rather than looking empty.
- **Pod logs:** a Logs tab with a container picker, tail length, timestamps, Stop and Resume from the last line, and the previous instance on request. Restarts and reconnects show as marker rows.
- **Pod shell:** a Shell tab starts a shell in a running container, only when you press Start. While it runs, anything that would end it asks first.
- **Port forwarding:** a Ports tab on pods, Services and workloads forwards a declared or typed port to a local one. Forwards run until you stop them, across pages and contexts, and the status bar lists them all with Copy address, Open in browser, Stop and Start again.

### App

- **The keyboard** goes from context to namespace, kind, object and details. Enter opens, Escape steps back one level at a time, and Command-F and Command-G find in whatever has focus.
- **Text size:** Command-= and Command-- step through 12–20 px, and the whole layout scales with it. Settings has the same choice.
- **Choose a talosconfig in Finder:** open the app, pick a talosconfig and a context, and both are remembered as a path and a name. Credentials are never copied.
- **Performance:** a first list of 50,000 pods costs the main thread 1 ms; watch bursts apply at most ten times a second; logs keep up at 10,000 lines a second.

### Changes from talos-pilot

- The GPUI Kit desktop app is the only interface. The terminal UI, the egui GUI and the Dioxus prototype were removed.
- Crates renamed: `talos-pilot-core` → `freshkube-core`, `talos-pilot-gpui` → `freshkube-desktop`. The binary is `freshkube`, the `--ui` option is gone, and `--fixture` opens the app with example data.
- Operations audit files move from `~/.talos-pilot/` to `~/.freshkube/`. Debug pages open with `FRESHKUBE_PAGE` (was `TALOS_PILOT_GPUI_PAGE`).

### Packaging

- macOS only, for Apple silicon and Intel, as `.app` bundles for macOS 15 or later. They are ad-hoc signed, not notarized, so macOS asks you to allow the first open. Linux and Windows wait until GPUI's builds there are verified. Homebrew publishing to the upstream tap was removed.
- Pull requests run formatting, Clippy and the tests, and merges to `main` also build both bundles. A version tag drafts a GitHub pre-release from those same bundles without building again ([docs/MACOS_PACKAGING.md](docs/MACOS_PACKAGING.md#releases)).


## 0.1.11

Reliability release: node enumeration no longer depends on the Talos discovery service, and Kubernetes views always target the launched cluster.

### Bug Fixes

**Worker nodes missing when cluster discovery is disabled ([#25](https://github.com/Handfish/talos-pilot/issues/25), [#22](https://github.com/Handfish/talos-pilot/issues/22))**

0.1.10 fixed workers vanishing when `--config` broke context resolution, but they still disappeared whenever Talos cluster discovery returns no members. This is increasingly common: Sidero has disabled the Kubernetes discovery registry by default, leaving only the external `discovery.talos.dev` service registry — which hardened/airgapped clusters routinely turn off — so `talosctl get members` comes back empty as the norm, not the exception.

talos-pilot built its node roster from `talosctl get members`, and when that's empty it fell back to etcd membership — which only ever contains control-plane nodes — so workers dropped out (leaving the "cluster discovery returned no members" warning). It now falls back to the **Kubernetes API**, which knows every node (control plane and worker) independently of Talos discovery. The kubeconfig for that lookup is fetched directly from a control-plane node over the Talos API (certificate auth), so it always targets the correct cluster and never an unrelated one an ambient `KUBECONFIG` might point at. Displayed nodes still come from successful Talos queries, so unreachable/bogus IPs self-filter.

The same fallback is applied everywhere the app enumerates nodes independently of the main list:

- **Cluster node list** — workers appear again (and everything derived from it: rolling operations, per-node logs/processes/network/storage/operations).
- **Lifecycle (versions / config drift)** — the version-skew and config-drift table now covers all nodes, not just the endpoint the base client targets.

Reproduced on a discovery-disabled Docker cluster (1 control plane + 2 workers): before, both views showed the control plane only; after, all three nodes appear. Verified end-to-end + regression tests added.

**Lifecycle "Config" and "Time Sync" columns were always blank**

The Lifecycle node table showed `-` for every node's config version and time-sync status. Two causes: `talosctl get machineconfig` returns multiple YAML documents (the `persistent` and `v1alpha1` configs), but the parser tried to decode the whole blob as a single document and always errored; and per-node time-sync info was only fetched for the endpoint. Both are fixed — machineconfig parsing now splits documents and prefers the canonical `v1alpha1` config (tolerating a numeric `version`), and version + time-sync are queried per node — so the table is fully populated for control-plane and worker nodes alike. Regression tests added.

**Kubernetes views could target the wrong cluster when `KUBECONFIG` was set**

The workloads view and the drain/cordon operations build a Kubernetes client that preferred an ambient `KUBECONFIG` over the launched Talos context. If your shell's `KUBECONFIG` pointed at a *different* cluster than `--context`, read views silently showed the wrong cluster's data and — more seriously — drain/cordon could act on the wrong nodes.

These clients now pin to the launched cluster: the kubeconfig is fetched from a control-plane node over the Talos API, so it always identifies the cluster you asked for. An ambient `KUBECONFIG` is still honored when it names the **same** cluster (matched by cluster CA, so a VIP/load-balancer endpoint isn't mistaken for a different cluster) — preserving your configured, possibly more reachable endpoint — but is ignored when it names a different one. When it's ignored, a wrapped warning on the cluster view explains why, so nothing is silently overridden. Verified end-to-end against a cluster with a mismatched `KUBECONFIG`; regression tests added.

## 0.1.10

Reliability release focused on connectivity, node discovery, and node operations.

### Bug Fixes

**Multi-endpoint failover — app no longer hangs silently ([#27](https://github.com/Handfish/talos-pilot/issues/27))**

talos-pilot now fails over across endpoints like `talosctl`: it dials all of a context's endpoints concurrently and uses the first that actually connects (bounded by a 10s timeout), so a dead first endpoint is skipped instead of hanging the session. The old "Successfully connected" log was just tonic's lazy channel, not a real handshake — hence the ~21s wait before the first RPC failed. Startup no longer blocks on a blank screen: there's now a cancellable "Connecting…" screen (q/Esc/Ctrl-C), and an all-unreachable cluster shows a clear Connection Error panel. Verified on a multi-endpoint cluster with the dead endpoint first — connects in ~2ms where it used to hang, so the reorder-endpoints workaround is no longer needed. (commit 9cab34f)

**Lifecycle/Versions view now fetches data ([#26](https://github.com/Handfish/talos-pilot/issues/26))**

The Lifecycle view fetched the Talos version first and treated it as a hard gate: on failure it cleared its own error and returned early, skipping the etcd/K8s/pod checks entirely (hence zero follow-up requests and the silent "unknown" fields). Now a version-fetch failure just logs a warning and continues; the other checks fire regardless, and you only get an error banner if the whole refresh comes back empty. Regression tests added. (commit 1ce1e01)

**Worker nodes no longer missing ([#25](https://github.com/Handfish/talos-pilot/issues/25), [#22](https://github.com/Handfish/talos-pilot/issues/22))**

talos-pilot fetches the full node list (workers included) via `talosctl get members`, but it wasn't passing `--talosconfig`, so if you ran with `--config <path>` talosctl fell back to the default `~/.talos/config`, couldn't find your context, and the list silently collapsed to just the control-plane nodes. It now passes your config through correctly. Node roles are now determined from the discovery `machineType` (not inferred from the etcd service), and if worker discovery genuinely can't run (discovery service disabled, or `talosctl` not on PATH) you'll now see a visible warning instead of a silently short list. Verified end-to-end + tests added. (commit fd5080f)

**"No K8s client available" when rebooting a node ([#23](https://github.com/Handfish/talos-pilot/issues/23))**

When you open node operations, talos-pilot builds a Kubernetes client to run the pre-drain safety checks. It was only trying your environment kubeconfig and then a VIP fallback — and since we build kube-rs without the exec/OIDC auth plugins, a kubeconfig that `kubectl` accepts (e.g. EKS/OIDC) could fail to load, leaving no client, so the reboot bailed out with the generic "No K8s client available". It now fetches the kubeconfig directly from a control-plane node over the Talos API (certificate auth, always available), so it no longer depends on your local kubeconfig's auth method. And if a client genuinely can't be created, you'll now see the specific reason instead of the generic message. Verified that drain/reboot safety checks load even with no usable local kubeconfig. (commit 49814c7)

## 0.1.9

See the git history for changes prior to 0.1.10.
