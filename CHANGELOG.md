# Changelog

Freshkube's history starts at 0.2.0. The entries from 0.1.11 down are talos-pilot's.

## Unreleased

## 0.8.0 (2026-10-07)

### Coroot

- **Coroot logs:** the Logs histogram and a pattern's chart draw each severity
  in its own colour, errors and fatal in red and warnings in amber, instead of
  the next colours in turn.

### Other changes

- **Log errors look like errors:** an ERROR line in any log takes the
  error colour, coral, for its level and its stripe, so it no longer reads
  as less severe than a WARN line beside it. Error lines still don't count
  toward any health check.
- **Node logs' service picker:** the service pills take at most two rows, and
  a "+N" button lists every service with its collect and show toggles. In a
  short window no pill row shows: "3 of 12 services" sits on the collection
  row and opens the same list, so the controls leave more room for lines.
- Pod, workload and node log rows show their times in your local time, as the charts, Coroot's logs and the status bar do; Kubernetes and Talos write theirs in UTC. Copy still copies each line as it was written, and restart and gap notes read on the same clock as their rows.
- **One loading state for tables:** Resources, Nodes and System services
  show their header over skeleton rows at the row height until their
  first answer, in place of a card of bars. System services waits for
  the Talos overview instead of saying no node has reported, and says
  why when it can't read the services.
- **Resources watch:** the mapping of watch events and the 16 ms window that
  batches them move into their own unit, with tests for a burst, an idle gap, the
  batch cap and a relist after an expired watch. Nothing changes on screen.
- **Delivery:** the delivery spike parses what Argo CD reports about
  reconciling an Application: auto-sync and self-heal, the retry limit and
  the operation's retry count and phase, single or multiple sources, the
  requested, compared and deployed revisions, each managed object's sync and
  health, and the Application's conditions. A missing inventory is unknown,
  not empty, and configuration alone never claims a retry is running.
- **Delivery join split:** `delivery/join.rs` is now a `join/` module of
  build, Kargo, Argo CD, workload and render parts. No behaviour change.
- **Delivery links say where their evidence came from:** each link of a
  delivery trail now lists the observations behind it, with the cluster,
  object, UID, resource version, field and whether it was reported or
  declared, and a trail's text prints one `from …` line for each. What the
  join itself compared is marked as derived and named by its rule. A link
  from a source that couldn't be read lists none. A few confirmed links
  rest on a declared field alone on one side (a revision parameter, Chains'
  annotation, a Rollout's spec pin); their evidence says so, and #387 will
  decide their confidence, which this change leaves as it was. The read time
  comes from the caller.
- A Kubernetes-only UI test waits for the kubeconfig to be read before it
  checks the files Settings lists, so it no longer fails on a busy machine.
- **Log search:** Next and Previous step through every matching line, so a
  stack trace that names the search on several lines shows each of them in
  turn, even in a row taller than the list, before moving on to the next row.
  Such a row marks its matching lines rather than the whole row, the current
  one more strongly. The match count and the status line count lines.
- **Log dock:** logs open in a full-width dock under the page, on every
  page, one tab per pod or workload, instead of the detail pane's Logs tab.
  A tab keeps reading while it is open, whatever the page shows; the dock
  minimizes to its tabs, fits to the window, drags to any height, and comes
  back with its tabs for the same context. Control-. and Control-, switch
  tabs, Shift-Escape minimizes it, and a middle-click closes a tab.
- **Log toolbar:** a log's controls fit one row: the pod's container and tail
  pickers, Previous and Timestamps as toggles, Stop, a Levels menu with counts,
  search, Wrap, Follow and Copy. Narrow windows and large text show icons and
  wrap to two rows. A crash-looping pod's waiting reason and last exit share one
  line, which ends by pointing at Previous, so a 760 × 560 window keeps three
  whole log lines. A pod's pane has one Logs button, which opens the container
  at fault and names it in its tooltip.
- **Shells in the dock:** a pod's Shell menu starts a shell in the container
  you pick, in a dock tab of its own that keeps running whatever page or
  object shows; navigating no longer asks to end it. Closing a running
  shell's tab asks first, and a shell tab saved with the dock comes back
  idle, starting nothing until Start. The dock keeps at most eight shell
  tabs; a ninth replaces the oldest idle one, or asks before ending the
  oldest running shell. The compact Levels button keeps its
  chevron at large text sizes, and a crash loop's log note starts with
  "Previous shows the last run" and wraps in its tooltip.
- **Workload logs:** a container whose log is refused is read again only when
  the workload's pods change or on Retry, not every 30 seconds, and other
  failed streams spread their retries so they don't all read again at once.
- **Details and YAML:** the details drawer has two tabs instead of five.
  Details is one page you scroll: the overview, with a failing pod's
  cause first, then its ports and its events, under an index that jumps
  to each part and marks the one you're reading. YAML has a tab of its
  own, and going to it and back keeps your place. A button beside the
  object's name copies the name. Details lists an object's newest 25
  events, and Show all lists the rest.
- **Details in a drawer:** a Resources row now opens its details in a
  drawer over the right of the list, under the page's toolbar, instead of
  a split that squeezed the table. It opens about 725 points wide, leaves
  the list at least 280, and remembers the width you drag its left edge
  to. Another row swaps what it shows, and a click beside the rows closes
  it; Logs and Shell close it too, since they open in the dock. In a
  narrow window it takes the whole list.
- **Pods:** a Containers column after the name draws a small square per
  container, init containers first and dimmed, coloured by what each
  does: running, restarted, not ready, waiting, failing or exited. Its
  tooltip names each container with its state, reason and restarts, and
  the column sorts the pods with a failing container first.
- **Pods:** restarts have a column of their own, sortable, next to the
  pod's CPU and memory, and the columns follow Freelens's order: Name,
  Ready, CPU, Memory, Restarts, Owner, Node, Age. Ready sorts the least
  ready pods first. A group's buttons now follow its count, so the
  details drawer no longer hides them, and the Healthy group no longer
  offers Collapse while a filter keeps every healthy pod in sight.
- **Pods:** every pod row has a Logs button after its name, faint until
  you point at the row, that opens the pod's logs in the dock without
  selecting the row or opening its details.
- **Delivery join follow-ups:** a build read discovers Tekton's API once, not
  once per PipelineRun, and one build whose TaskRuns are refused leaves the
  other builds of the commit whole. An error page that isn't the API server's
  JSON loses the bare host names and addresses it names, as `host:port`
  already did ([#292](https://github.com/skel84/freshkube/issues/292)).
- **Kubernetes summary:** the node summary drops an unused pods parameter, and
  the volumes, namespaces and version parts gain tests for never listed, failed and
  stale. Nothing changes on screen.
- **Download logs to a file:** every log's toolbar has a Download menu
  that saves the visible lines or every retained line, as Copy copies
  them, to a file you name, suggested in ~/Downloads. A failed save says
  why and leaves no partial file. A workload's lines, copied or saved,
  now start with the pod and container that wrote them. When the toolbar
  shows icons alone, a pod's status tag shows its glyph, with its words
  in the tooltip.
- **Workload logs pick a pod:** a workload's log tab has a Pod select,
  All pods or one pod, which narrows its lines and container chips to
  that pod while every container reads on, and a Timestamps toggle like a
  pod tab's. A pod that leaves keeps its pick, marked "(gone)", and a pod
  past the 20-container cap says it isn't read.
- **Error pages redacted further:** an error page that isn't the API server's
  `Status` also loses wildcard and suffix domains (`*.apps.cluster.example`,
  `.svc.cluster.local`), host names whose last label has a digit, IPv6
  addresses inside markup and fields with their zones, and credentials and
  email addresses whole. A proxy's JSON no longer passes for the API server's
  error, and the message is cut to 4 KiB
  ([#305](https://github.com/skel84/freshkube/issues/305)).
- **Status messages redacted further:** a Kubernetes or object status message
  loses credentials and email addresses whole (`user:pass@`, base64 and
  percent-encoded userinfo, `git@host:org/…` with its owner), IPv6 addresses
  wherever they stand, and IPv4 addresses in fields with their port or prefix
  (`ip=192.0.2.1`, `10.0.0.0/24`). It keeps the API groups and versions it
  names, and is cut to 4 KiB after redaction. Error pages lose the same, and
  host names between dashes, and their 4 KiB cap now holds on the final
  message ([#311](https://github.com/skel84/freshkube/issues/311)).
- **FPS meter:** a followed log no longer reads red. The meter counts frames
  only while the app draws back to back, such as when scrolling, and frames
  that take longer than 33 ms; sparse, quick frames read idle.
- **Host names out of Status messages:** a Kubernetes or object status message
  now also loses the host names Go's errors write: after `lookup` (`lookup
  git.example.com on …: no such host`) and `address`, and in an x509 "valid
  for" list, its `not …` and its `none matched …`. A webhook's quoted name
  goes too (`failed calling webhook "…"`, `admission webhook "…"`). API group
  and resource names, such as `pipelineruns.tekton.dev` or `(get
  widgets.example.com)`, stay
  ([#314](https://github.com/skel84/freshkube/issues/314)).
- **Log search:** in the dark theme the current match now stands out
  clearly from the other matches, in logs and in the YAML pane's search.
  Enter in a log's search field steps to the next match and Shift-Enter
  to the previous one, and the keyboard stays in the field, as in a
  browser.
- **Log Copy and Download:** Copy no longer builds its text on every frame to
  decide whether it's enabled; any selection enables it, and a selection it can't
  copy says why. Download's temporary file is named `.<name>.freshkube-<pid>.part`
  and is removed on every failed save.
- **Nodes keys:** Enter on a focused button in a node's inspector presses it, as it
  does elsewhere; Enter in the Nodes filter hands the keyboard to the list without
  opening a node, as the Resources filter does; and a log's match count keeps a
  gap from the search field's focus ring.
- **Proxy and local clusters:** a kubeconfig server on loopback (`127.0.0.1`,
  `::1`, `localhost`) is reached directly when `HTTPS_PROXY` is set, as kubectl
  does, instead of failing to make a client; a kubeconfig's own `proxy-url` is
  still used.
- **Delivery join destinations:** an Argo CD Application sent to `in-cluster`
  or `https://kubernetes.default.svc` now reaches the Rollout and pods when Argo
  CD runs in the environment cluster. Any other cluster name maps with
  `--known-as-name alias=name`, and when a name isn't mapped the join says how
  to map it. A Rollout's pods come from its ReplicaSet at the promoted digest,
  never from an older one beside it. The output now shows whether a Promotion
  was automatic or by hand, the commit it pushed, why a Stage is unhealthy, the
  Freight image's OCI revision and the run's result names, and it ends with a
  summary line ([#330](https://github.com/skel84/freshkube/issues/330)).
- Nodes: Escape steps back the way it does on Resources. In the filter it clears the text, then leaves. In the node pane it hands the keyboard to the table and leaves the pane open on its tab. On the table it clears the filter, then closes the pane.
- Kubeconfig connections now work through an `http://` proxy, taken from `HTTPS_PROXY` or a kubeconfig's `proxy-url`, as kubectl does. `NO_PROXY` is still honoured. An `https` API server is reached through a `CONNECT` tunnel, so TLS still runs to the server, and any `user:password` in the proxy's URL goes to the proxy alone. A proxy that refuses the connection is now named in the error, such as `unsuccessful tunnel (HTTP/1.1 407 …)`. `socks5://` and `https://` proxies are still refused, with an error that says so.
- Kubeconfig connections now honour `NO_PROXY` (and `no_proxy`) as kubectl does: hosts and their subdomains, `.domain`, IP addresses, CIDR blocks, `host:port` and `*`. A server it names connects directly instead of through `HTTPS_PROXY`. When a proxy Freshkube can't use stops a connection, the error now names the proxy, without any credentials in its URL, and says how to bypass it, instead of only "Couldn't make a client".
- **Delivery:** a Promotion whose commit and push steps record the same
  commit now lists that commit once.
- Nodes: with a node's pane open, the table now follows Filter nodes and the health groups, as it does without the pane, so its rows agree with the counts above it. A filter that hides the open node leaves its pane open.
- Nodes: Enter on a node, a click on one of its pane's tabs, or a click in the pane moves the keyboard into the pane, as Enter does on Resources, so Escape first hands the keyboard back to the table and only a second Escape closes the pane. A tab whose screen takes no keys gives the keyboard to the tab strip, even from the filter.
- **Chart legends:** five or six series with short names, such as the Logs
  histogram's severities, list inline, so a short chart no longer hides the fifth
  under its legend's edge.
- **Charts in the light theme:** series, bars, thresholds and log
  severities draw in darker inks of the same hues; series and threshold
  lines reach at least 3:1 on the white card, so the Logs histogram's warning bars no longer read as pale
  tan. In the dark theme only bars change: they draw at full opacity, so
  blue and grey bars stand out as clearly as lines.
- Processes: the find field now takes every letter you type. Before, t, d, y and z ran the list's shortcuts instead, so typing `etcd` gave `ec`. Escape in the field clears it, then hands the keyboard back to the list; it no longer resets the zombie or disk-wait filter, which Escape on the list still clears.
- **Heatmaps in the light theme:** an empty cell now reads pale and the busiest cell darkest, where the light theme had them the other way round. Its steps mirror the dark theme's, and the Traces caption reads "stronger = more" instead of "brighter = more", which held only in dark. Flame graph colours stay the same in both themes, and a chart's hover readout takes a new theme at once, without waiting for the pointer to move.
- **Linux compile and lint can be required:** platform checks run it as its own
  job, "Compile and lint (Linux)", so a change to docs alone still reports it
  as skipped instead of leaving the merge waiting. Windows stays advisory.
- **Windows:** a port forward no longer binds over another program listening on
  the same port, the automatic port steps past ports Windows reserves, and a
  directory chosen as talosconfig says it is not a regular file.
- **Windows tests in CI:** platform checks run the whole workspace test suite on
  Windows in a separate advisory job, alongside Linux's.
- **Talos node facts:** what Talos reports about each node (its address,
  role, etcd membership, whether it answers, memory and services) is worked
  out in the core library, with the same rules, and gains tests for missing
  and partial answers. Nothing changes on screen.
- **Smaller Talos client:** the `talos-rs` crate drops the client methods, talosctl
  wrappers, display helpers, generated protos and TLS helpers nothing called, and
  four dependencies with them. The processes view formats memory sizes with the
  shared formatter, so a size of 1 TiB or more now reads as TB instead of GB.
- **Core API:** freshkube-core no longer exports code nothing used, including
  the `AsyncState` loading container and several unused helpers.
- **Tooling cleanup:** the unmaintained `dirs-next` dependency is gone (the home
  folder now comes from `std::env::home_dir`, the Downloads folder from `dirs`;
  on Windows the home folder is read from `USERPROFILE` first), four one-off
  test-cluster scripts are deleted, and the stress binary has one query parser.
- **Less hand-built Kubernetes plumbing:** single-object metadata reads use
  kube's `get_metadata`, a Service's label selector uses kube's `Selector`,
  and the GET reads, pod lookups and base-URL checks share one helper each.
  Requests use the same paths, queries and Accept headers.
- Maintenance mode's form keeps its field captions short, with their notes under the fields, so none runs out of its card; in a narrow window or at a large text size the form fills the page's width above the workflow.
- Maintenance mode's page uses the shared page header and padding, like the other pages.
- **Nodes' healthy fold:** H shows or folds the healthy nodes, and a
  right-click on a node offers Open and the fold with their keys. The
  healthy group keeps a chevron in place of its Expand/Collapse button.
- **Actions on the selected row:** a right-click on a Resources row opens
  its actions (Open and Mark on every kind; for pods also Logs, Select
  all in its group, Open its node, Expand or Collapse the healthy pods),
  each with its key. New keys: Shift-X marks the selected pod's group,
  O opens its node, H shows or folds the healthy pods. The group rows
  lose their Select all and Open node buttons to the menu; the healthy
  group keeps its chevron. A row's menu acts only on the row it was
  opened for: if the list changes under it, it does nothing.
- **System services' actions on the selection:** the Logs and Open node
  buttons leave the rows for the toolbar. Select a service with a click or
  the arrows, then use the toolbar, a right-click, L for its logs, or O or
  Enter for its node.
- **Tables:** the code that draws a table's line is split into smaller
  parts, making room for actions on the selection. No behaviour change.
- **The design guide matches the app:** every page now draws with the shared
  components, so DESIGN.md, the screen guide in AGENTS.md and the roadmap
  describe that. The style check's allowlist is empty, so any page that draws
  its own title, table, glyphs or corners fails CI.
- Monitoring and Observability take the cluster connection through a small
  core type, `ClusterSource`, instead of the Resources page's own, so
  Monitoring can later move into a crate of its own. Nothing changes in
  the app.

## 0.7.0 (2026-10-07)

### Coroot

- A service map connection's evidence reads in its own units, such as 12 rps, 3 ms and 2.4 KB/s, instead of three fixed decimals.

### Other changes

- **Delivery chain spike:** core gains a read-only join from one commit or pull
  request to the pods running its image, through Tekton builds, Kargo Freight,
  Promotions and Stages, Argo CD Applications and Argo Rollouts, joined only on
  commit SHA and image digest, with each link marked confirmed, claimed or
  unknown. A `delivery_spike` example runs it against named contexts; setups
  that differ are described by settings with no defaults.
- Log search shows the matched line within a long multi-line message, such as a stack trace, instead of the message's middle.
- In a short pane a log's controls and a crash-looping pod's restart banner show whole; the panel scrolls on to the lines instead of cutting the banner.
- Selecting a node's system service, by click or arrow key, scrolls its details into view when they sit under the list.
- A node's System services, Processes, Storage, Network, Diagnostics and Logs tabs no longer repeat the tab's name and the node above their own controls, so the list or log starts higher in the inspector.
- **Reduced motion:** Freshkube follows the system's reduce-motion setting on
  macOS, Windows and Linux, and its loading bars stand still under it. The
  developer workbench adds stories for a shared table loading state and a
  change flash on live rows ([motion](docs/DESIGN.md#motion)).
- **Kubernetes summary internals:** the summary has one derivation, from the
  watch-backed session; the unused one-shot collector is gone. Nothing changes
  on screen.
- **IPv6 nodes:** requests for a node addressed by IPv6, bare or as
  `[address]:port`, reach that node; the address was cut at its first colon
  before. A restart, configuration apply, reboot or shutdown with no node
  left to target, for example only `localhost`, is refused instead of
  reaching whichever machine the endpoint is.
- **Workload logs:** a Deployment, StatefulSet, DaemonSet, ReplicaSet or Job
  has a Logs tab that follows every pod it runs in one log view, lines
  interleaved by time and tagged with their pod. Pods are picked up as they
  start and dropped as they go; a container's lines can be hidden from its
  chip, and at most 20 containers are read, the newest pods first.
- **IPv6 nodes, continued:** Storage, the etcd status of each member, the
  control-plane lookup and the overview's version fallback read an IPv6 node
  address whole instead of cutting it at its first colon. Overview and
  Lifecycle discovery and the KubeSpan check handle IPv6 endpoints too.
- A detail pane too narrow for its tabs now scrolls them sideways: a cut end fades under an arrow that pages to the tabs beyond it, and the tab you're on, or move to with the keyboard, is always in view.
- Stacked bar charts, such as an application's log histogram, now fill each column instead of a thin slice of it, and charts of whole counts label only whole numbers on their axis.
- **Linux and Windows preview builds:** every push to `main` packages a Linux
  tarball and a Windows zip and checks that each opens a window, on a machine that
  didn't build it ([packaging](docs/PACKAGING.md)).
- **Linux and Windows in releases:** a release draft attaches the Linux and Windows
  preview builds of its commit when they passed CI, and warns at the top of the
  notes when one is missing. [First-run reports](docs/FIRST_RUN.md) record when a
  platform turns supported.
- Monitoring's variables and Annotations toggles now sit in the page header's second row, sized like the other header controls, and auto-refresh says "Auto-refresh off" or its interval. Tables inside a card no longer show square corners at the card's rounded bottom.
- Monitoring's dashboards and the pod and node CPU and memory history now send their Prometheus reads through one shared path, so both time out and report failures the same way.
- Workloads rows are identified by the namespace, workload or pod they show rather than by their place in the list, so a selection or a test follows the same item through a refresh or a filter.
- A narrow Workloads list scrolls sideways with each row's glyph and name kept at its left edge, as Nodes does; in a narrow window or at a large text size a long name truncates, and its tooltip and the details hold all of it.

## 0.6.0 (2026-10-06)

### Coroot

- **Application charts:** each report on the Application page draws Coroot's
  charts and heatmaps with Monitoring's chart: stacked and column charts, limits
  as dashed lines, rollouts as deploy markers, and chart groups with a picker that
  opens on Coroot's featured chart.
- **Narrow Application page:** at a narrow window or a large text size, charts
  take the whole row inside the page's margin, and a check's long condition
  wraps inside the Checks card.
- **Application page:** an application opens on Coroot's own view of it: a map of
  its clients, instances and dependencies with links coloured by status, Coroot's
  report tabs with their glyphs, each report's checks with their conditions and
  thresholds, and its tables. The Tracing and Profiling reports show the Traces
  and Profiling views in place.
- **Application page as Coroot shows it:** the evidence card is gone, along with
  its REST and MCP report reads, so the page shows Coroot's own view and nothing
  more. CPU delay and throttled time now read in seconds, as in 2 ms.
- **Coroot logs read:** core reads an application's messages and patterns
  from Coroot, at the severity Coroot stored, and refreshes without adding a
  message twice. The Application page's Logs tab will draw them.
- **Coroot logs tab:** the Application page's Logs report shows Coroot's
  histogram and its messages, at the severity Coroot stored, or its patterns.
  Choose the origin, messages or patterns and a limit; search, filter by level
  and copy as on a pod's logs. A refresh adds only newer messages and marks
  where some may be missing.

### Other changes

- The stress run's Monitoring workload now opens its dashboard and asks its Prometheus, so it measures Monitoring; a run that never draws a panel fails instead of printing only the app's numbers.
- A tooltip no longer appears over another node card or table row when a scroll moves its own row away from a still pointer.
- On Linux, a stopped port forward's local port is free again at once, instead of staying taken for up to a minute.
- A narrow Monitoring chart's readout, such as a pod's last-hour CPU, stays inside the plot: it no longer covers the value axis or spills its "top N of M" past its box.
- Moving the pointer over a Monitoring chart redraws only its crosshair and readout, not the chart's card, legend and plot, so a large dashboard follows the pointer more smoothly.
- Moving the pointer over a Monitoring chart redraws only that chart: the page draws the crosshair on the others, so a large dashboard keeps up with the pointer.
- A large Monitoring dashboard draws only the panels in or near view, so it stops drawing once those have answered, and scrolling to a panel draws it in place without moving the page.
- **Log messages no longer lose their start:** a line such as
  `cart-db: connection refused` shows whole in pod, node and Coroot logs. Only
  a Talos service's own prefix (`main.go:94: `, `udevd[812]: `) and a leading
  level word the row already shows are left out.
- **Log notes on the rows' clock:** a Coroot log's "Some messages may be
  missing" note shows its time in local time, as the messages around it do,
  instead of in UTC.
- A mouse wheel or trackpad over a log pauses Follow only when the log really scrolls up from its newest line; a wheel down at the end, or a sideways one, keeps following.
- Nodes' cards share the page's width evenly, a lone card in the last row as wide as the rest; the cards and the pane's node list are drawn with a shared virtualised card grid.
- **Diagnostics as a table:** a node's Diagnostics tab lists its checks in the same edge-to-edge
  table as Pods, grouped by category, each group marked with its worst status and counting its
  checks and problems. Passing, Warnings, Failing and Unknown become chips in the header that
  filter to that status; P still shows only the problems. The CNI and the addons move into the
  line under the header, and a long result no longer runs out of the table: it ends with "…",
  and hovering the row or opening the check shows all of it.
- etcd and Security show their context, quorum or audit sources and update time in the status bar, not under their header.
- Tables show how many rows are selected or showing in their footer, beside the legend, instead of in a blue strip above the rows: Pods, Nodes, Applications, Incidents, Traces and Monitoring's table panels.
- The detail pane on Resources is an inspector, not a card: its tabs are 28 high, it runs to the window's edge beside the list or under it, and the width you drag it to is remembered.
- Incidents and Traces show the selection in an inspector beside a bare table, instead of in a card: drag the hairline between them to resize it, and each page remembers its width.
- **Lifecycle takes the shared header and fills the status bar:** the page opens with the same 38
  dp header as the others, with Refresh as an icon. The Talos and kubelet versions, the etcd
  pre-check and the alert count move from the row of stats into the status bar, where an unsafe
  etcd pre-check and alerts to review show as warnings. The bar's tooltip gives the etcd verdict and
  the alerts in words, and the alerts keep their card on the page.
- Maintenance mode lists the install disks in the shared table, with a column each for the device, ID, size, model and serial. The disk is still chosen only with its Select install disk button.
- Maintenance mode tags an optical drive, and an optical drive's read-only flag, in a neutral tone rather than a warning, as Storage does: they are read-only by design.
- **Meter tooltips:** Pods and Nodes word their CPU and memory meters the same way,
  and the tooltip now says why a bar stands out: above request, or at least 85% of
  its limit or the node's allocatable.
- Monitoring's legend entries and series swatches, and the chart cursor's swatches, use the design's 3 px data-mark radius.
- **Network takes the toolbar header:** a node's Network tab opens with the same 38 dp header as
  Pods and Processes. Interfaces / Connections / Listeners / KubeSpan, the connection filter, the
  interface being followed and the connection states sit in it and move into the "…" menu when the
  pane is narrow. Throughput, errors, drops and connection counts move into the line below the
  header with the update time. The table runs edge to edge and fills the pane, with the selection's
  details beside or below it; notices, KubeSpan's status and a node that isn't responding show
  under the header, and packet capture under the table.
- **Operations takes the shared header:** the page opens with the same header as the others, with
  Refresh as an icon. Its context, when it last read the audit log and whether it shows example data
  move to the status bar.
- **Processes takes the toolbar header:** a node's Processes tab opens with the same 38 dp header as
  Pods and etcd. The filter, Tree and the All / Running / Disk wait / Zombie switch sit in it and
  move into the "…" menu when the pane is narrow, as does the subtree's button. CPU, memory, load
  and the process counts move into the line below the header with the update time. The table runs
  edge to edge and fills the pane, with the selection's details beside or below it; a node that
  isn't responding, or a read that failed, shows its message under the header.
- Screens can fill the status bar's page segment, which the next change uses for etcd and Security; nothing changes on screen yet.
- **Security takes the toolbar header:** the Security page opens with the same 38 dp header as Pods
  and etcd, with Refresh as an icon. The context, the Talos endpoints and the volume target move
  into the line below the header with the update time. The certificate, RBAC and volume summary
  and the audit list sit under it as before; a target that isn't responding, or an audit that
  failed, shows its message under the header.
- A status bar too narrow for a page's whole line now drops the refresh time and counts of none first, then other parts from the end, and always keeps warnings and failures in view.
- **Page details in the status bar:** Resources, System services, Nodes and
  Observability no longer draw a line under their header; where the data
  comes from, how much there is, its state and when it last changed now sit
  in the status bar after the connection, with a stale or failed read in its
  warning colour. In a narrow window the connection shrinks to its glyph so
  the page's details keep the room.
- **Workloads takes the toolbar header and the shared table:** the Health page's workloads open
  with the same 38 dp header as Pods, with the filter beside the title, Only unhealthy (folded into
  "…" on a narrow window) and Refresh as an icon. The deployment, statefulset, daemonset and pod
  counts move into the line below it. Namespaces, workloads and pods list in the shared table, edge
  to edge, with a status glyph per row; a namespace still opens and closes with Enter or a second
  click, and the details sit beside the table or below it.
- Health's workload and pod counts show in the status bar, not under its header.
- The detail pane's YAML tab draws on the shared document view, like the maintenance review.
- **Graph layout:** the service map's layout is its own crate, `freshkube-graph`, with
  tests, and the developer workbench shows it on invented services. The map looks
  the same.
- **Graph routing:** `freshkube-graph` can route each call around the boxes, so no line
  runs through a service and a cycle's call comes back below the row. The workbench's
  graph story shows it on invented services and counts where calls cross.
- **Service map:** a map wider than its card scrolls inside the card's border, with a
  scrollbar under it, instead of running to the edge at narrow windows and large text.
- **Service map:** calls are routed around the boxes, so no line runs through a service,
  a call that skips a column passes between boxes, and a call that closes a cycle comes
  back below the row.
- **Workbench:** a developer tool, `freshkube-workbench`, shows the shared components on
  invented data in both themes, every text size and both window widths; it is not
  shipped in the app.
- **Monitoring legends line up:** every legend row takes half the width, an odd last row too,
  and a long legend stops short of the card's bottom edge. A series that stopped before the end
  of the range shows its last value muted, with the time in the row's tooltip, so its name is no
  longer cut short.
- **Monitoring tables and a header that fits:** dashboard tables and Firing alerts are now the
  same table as Pods, inside their panel: each column as wide as its contents, numbers on the
  right, and a status glyph for a critical or warning severity. A long table shows its first
  100 rows under "Showing 100 of N" until Show all. In a narrow window the time range, Refresh
  and Auto-refresh fold into the header's More menu, the data source moves into the line under
  the title, and the Annotations toggles wrap instead of running off the page.
- Monitoring's chart readout no longer runs off the card's left edge: it sits on the side of the cursor with more room and shortens long series names to fit.
- The advisory Linux platform check caches its test dependencies under a new key, so its test step no longer compiles them on every run.
- The advisory Linux platform check now runs the whole test suite, headless UI tests included, after compiling and linting.
- The app has its own icon, a skull over two crossed decks, in Finder, the Dock and the app switcher.
- `scripts/changelog.sh` runs its Python from `scripts/changelog.py` instead of a here-document, never reads the caller's input, and fails if it reports fewer fragments than `changelog.d` holds.
- **Monitoring:** a crowded chart draws its grey lines again. With many series, such as 65 at the 600 points a panel asks for, the grey lines together passed the most one GPUI path holds, and the chart drew none of them.
- The service map's Connections is now the shared inspector, inside the map's card: beside the map when it fits and under it otherwise. Drag the divider to resize it, and the width is remembered. Open application sits at its foot.
- In a short window, Monitoring's header, variables and annotations scroll away with the page, and the panels get the whole window instead of the strip the header left them.
- Operations' interaction tests step the example run on a virtual clock instead of waiting on the wall clock, so they no longer slow down or time out on a loaded machine.
- **Operations' plan stays inside its card:** at narrow widths and large text, each node's etcd
  verdict wraps beside its chip instead of running past the card's edge.
- **First frame in release builds:** `FRESHKUBE_FIRST_FRAME=1` now prints when the
  window first draws in release builds too, for the Linux and Windows release checks.
- **Monitoring:** a chart with more than 30 series draws the 30 with the highest peaks, and its legend offers Show all. Charts whose queries use topk or bottomk, and stacked charts, still draw every series.
- `scripts/smoke.sh hover X Y` moves the pointer without a click or a wheel event, so a smoke test can check a tooltip; `scroll X Y 0` sends wheel events, which hide a tooltip already shown.
- The status bar's line-fitting check is written the way newer clippy versions accept, so builds with a newer stable toolchain stay free of warnings.
- The node pane on Nodes is an inspector, as on Resources: the node table stays beside it (or above it in a narrow window), its tabs are 28 high, Expand still gives it the whole page, and the width you drag it to is remembered. The node's details wrap rather than cut off, the opened node is highlighted in the card view's list, the opened row stays in view above a stacked pane, and Expand in a short window keeps the node's name and Collapse in view.

## 0.5.0 (2026-10-06)

### Metrics sources

- **Monitoring reads any Prometheus API:** besides Prometheus, discovery finds VictoriaMetrics (single-node and vmselect), Thanos Query and Mimir query frontends, with their ports and path prefixes. Each is confirmed with a query before use, so a missing prefix or refused token shows at once.
- **Settings → Metrics source**, per context: Automatic, one Service through the Kubernetes API (namespace, name, port, path prefix, HTTPS), or a URL read directly from your Mac with an optional bearer token. **Test** checks the form before you save it. The token is kept in the Keychain; `monitoring.json` only notes that one exists.

### Coroot

- **Read-only Incidents:** the project's latest incidents, each with its SLO objective, compliance and burn rates, root-cause analysis and suggested fixes as Coroot reports them, and links from propagated applications to their reports. Missing evidence is marked not reported, never invented. See [COROOT.md](docs/COROOT.md#read-only-incidents-slice).
- **Freshkube remembers the Coroot connection:** server, sign-in choice and last project, in `coroot.json` beside the preferences. **Remember key in Keychain** keeps the key in the system's credential store, sent only to its own server. The page reconnects when it first shows, and Disconnect forgets the key.
- **A visual pass:** Observability gets its own rail icon; chosen chips, toggles and segments are blue everywhere, Settings included; reports merge REST and MCP evidence with formatted units; the service map is layered with weighted, arrowed links; and an unconnected page shows one Connect Coroot state.
- **Applications uses the shared page and table:** namespace groups show their worst health and app count, with comfortable or compact rows, Columns, Show all and a legend. Categories start with applications only, falling back to all when that category is absent; All also includes newly discovered categories. The count follows the filters, which return the list to its first row. Healthy figures stay plain and healthy checks without a figure read “ok”; unknown reports keep their outlined circle and missing reports show “—”. Full captions and compact signal values keep their widths at each text size; failing upstreams show a count with their names in the tooltip. The namespace picker is searchable, tooltips name the app, and time ranges include local dates. Traces and Profiling show a failed first read with Retry.
- **Incidents is a table page like Pods:** a filter, Open and Resolved counts that filter the list, comfortable or compact rows, Columns, and "Showing n of m" with Show all in place of Previous and Next. The selected incident opens in a pane beside the list, or below it in a narrow window, with its objectives, Coroot's analysis and the affected applications. Rows stay selected by incident and application when Coroot answers again. Duration and Impact are hidden by default, since the pane's summary shows both; Columns shows them. Times are local, not UTC. A typed filter and the chosen count survive a reconnect. Example data now goes through the same page as live data, so the example-only mute, fix, edit and diff actions are gone; Incidents stays read-only.
- **Live Traces:** an application's latency and error heatmap, its latest requests, and the selected trace as a waterfall with each span's status, attributes and events. Click a cell to list that bucket's requests, or list the failed ones; choose OpenTelemetry or eBPF when Coroot has both.
- **Live Profiling:** an application's profile types, instances and flame graph, with zoom, search and a comparison with the window before that colours each frame by its change and lists the biggest increases.
- **A tidier Observability page:** the source, project and time picker sit in the page header, with one page Refresh. Coroot connection settings, Disconnect and the Kubernetes link open from the sidebar footer. The column uses the shell's usual rows, so only the open destination is highlighted, and Deployments shows only with example data. Application names take the table's spare width. Traces and Profiling pick an application by typing part of its name. The flame graph names only frames wide enough to read, outlines the selected one and shows its details above the graph.
- **Coroot's markup no longer shows:** report titles and messages lose HTML tags such as `<var>` and decode entities, keeping a plain `<` or `>` in text such as "latency < 500ms".
- **Traces is a table page like Pods:** example and live data now draw the same page. A filter over request name, service and trace id, the sources, All and Failed requests, Columns and the time range sit in the page header. Requests are one-line table rows with a status glyph, service, local start time and duration; a failed request's message is in its tooltip. The selected request's trace opens in a pane beside the list, or below it in a narrow window, and stays selected by trace and span when Coroot answers again. The heatmap's axis reads local time. The example-only error causes, Errors only, Clear selection and heatmap arrow keys are gone. Incident titles read as prose, and long text in both tables ends in "…" instead of clipping mid-word.
- **The Traces heatmap has keys again:** Tab reaches it, the arrows, Home and End move a cursor without reading anything, and Enter or Space lists the cell under it; Escape leaves. The cursor is outlined more heavily than the selected cell and its bucket, time range and rate show above the grid. The heatmap stays drawn while Coroot answers, saying when it is reading, failed or absent.

### Other changes

- On Linux and Windows, preferences move to the platform's folder: `~/.config/freshkube` and `%APPDATA%\Freshkube`. macOS keeps `~/Library/Application Support/Freshkube`.
- Lifecycle's Kubernetes version rules move into core, with no change in behaviour.
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
- **Peak memory agrees with itself:** Overview's card, and the Nodes dot it puts on the rail, turn yellow at 85% and red at 95%, the same thresholds as its bar and its High or Critical word. It used to turn red at 90%, so a node at 92% showed red over the word High.
- **Capture builds:** requesting another CI debug binary on main lets the running build finish, preserving its artifact and cache; newer PR heads still replace their previous capture builds.
- **Capture icons:** downloaded CI debug binaries embed GPUI Kit's icons, so Search, Refresh and other Kit controls draw their glyphs away from the build runner. Ordinary local debug builds keep their existing asset loading.
- **A node's memory warns where its meter does:** a Nodes row now warns from 85% memory and fails from 95%, the thresholds where Overview's Peak memory card says High and Critical. Overview's attention list and the node pane follow. It used to warn only from 90% and never fail, so a node at 87% sat among the healthy ones while Overview called it High.
- **Group actions stay in view:** when a table's columns are wider than its view, as Pods' are with the detail pane open, a group row's details now truncate before its actions, so Select all and Open node stay on screen instead of being cut off at the edge.
- **Group rows line up:** every grouped table now draws its glyph column the same way, centred, and a group row's glyph sits exactly over its rows' glyphs with its label starting where the names do, at every text size. The group glyph used to sit a pixel or two left of the rows' (more on Pods), and its label well left of the names; Overview's Needs attention rows move with their group rows.
- **Short windows keep Nodes and System services usable:** under 620 dp of height their page scrolls and the table keeps at least 180 dp, as Pods already did, instead of squeezing the list to a row or two.
- **Lifecycle's node table fits at every size:** the roster now uses the shared table, so its last column (Discovery · K8s) is no longer cut off at 1280 wide with text size 14 or at 760 wide with text size 20. Its columns fit their content, the table scrolls sideways when the window is narrower than they are, and each node's name stays at the left edge while it does. Rows take the shared 26 dp height, and a long node name shows whole in its row's tooltip.
- **Node logs fit short windows:** under 620 dp of height the node pane scrolls on its Logs tab, which keeps every control and a usable list, and a wheel over a log scrolls the log rather than the frame around it.
- **Monitoring legends and stat tags fit:** a legend under a unit in its title, such as GiB, shows every value in that unit (0.91, not 929 MiB), and its value columns size to the widest value instead of wrapping into the next row. A stat's tag, such as "above 500 ms", drops under a wide value instead of running past the card.
- **One corner dot for the rail and Observability:** the problem dots on the rail's areas and the incidents dot on Observability's collapsed column are now drawn by the shared `badge_dot`, so they look the same as before and follow the theme from one place.
- **Console corners and glyphs on more pages:** the Services tab,
  forwards list, pod cause card, Secret values, suggested fixes and Operations'
  toggles take the Console's corner sizes; Lifecycle's Discovery · K8s column
  shows status glyphs, a node missing from a roster as a warning; and
  Operations marks a selected node's run order with a check icon.
- **Flame graphs and the traces heatmap round at 3 px:** live flame frames, the flame legend's swatches and the heatmap's cells now use the meter radius, 3 px, instead of 2. A selected cell's inner ring is square.
- **Maintenance mode's configuration review:** the generated YAML is drawn with the shared document view, in 20-high lines like the YAML tab. In a narrow window it now scrolls sideways instead of cutting off the end of a long line.
- **etcd looks like Pods:** the same header, table and row heights, with counts by health that
  filter the members and a summary of quorum, leader and alarms. A lost quorum shows a critical
  banner and one with no failure to spare a warning, each with the arithmetic; alarms show as a
  banner of three lines and a count. Selecting a member opens its details beside the table, or
  below it in a narrow window.
- **Folded controls keep their values:** in a narrow window, the "…" menu
  shows what its folded controls are set to (`Namespace · payments`,
  `Columns · 2 hidden`, `Time range · 7d`), and a dot on "…" with a matching
  tooltip says when one of them narrows the list, so a filtered page no
  longer looks like missing rows.
- **Maintenance mode code:** the maintenance view is split into a module directory: the view and its actions, its presentation, the configuration review and the UI tests. Nothing changes on screen.
- **Toolbar overflow menu:** a page header that runs out of room folds its
  controls, rightmost first, into a "…" menu instead of stacking them on
  extra rows, so a narrow or large-text window keeps more of its list.
- **Pages without margins:** Pods, every other Resources kind, Nodes, System services
  and Applications run their table from the column to the window's edge, with the
  header and any banner padded inside the page instead of the whole page inset.
- **Processes use the shared table:** a node's processes are drawn by the same table as Pods, with its header, 26 dp rows and glyph column. The command now comes right after the PID, so the node pane shows what each process is without scrolling; a long command truncates, and hovering the row shows it in full. Zombies and processes waiting on the disk show a warning glyph. Click CPU, CPU time or Memory in the header to sort. Numbers sit under their labels, column widths follow the data, and a narrow table scrolls sideways with the PID kept in view.
- **Sideways scrolls stay sideways:** swiping sideways over a wide table, such as a node's Disks at a narrow window, scrolls only its columns; the page around it no longer scrolls down at the same time. A column that passes under the pinned name column keeps its header label beside it, so a short label such as Flags no longer disappears while its cells still show.
- **Shell tab and log levels draw the shared glyphs:** a running shell shows the
  OK glyph on its tab, and the log level toggles show the information dot for
  errors and the warning glyph for warnings.
- **Context switcher corners:** the context switcher's tile and the rows of its context list round at 8 px, the control radius, instead of 7.
- **Storage and Diagnostics take the toolbar header:** a node's Storage and Diagnostics tabs open
  with the same 38 dp header as Pods and etcd: the title, a Refresh icon and, below it, when the
  data was updated. Storage's counts of disks, total size, volumes and those not ready move into
  that line; its Disks / Volumes switch sits in the header and moves into the "…" menu when the
  pane is narrow; its table runs edge to edge with the selection's details beside or below it.
  A node that isn't responding, or a read that failed, shows its message under the header.
- **Storage uses the shared table:** a node's Disks and Volumes are drawn by the same table as Pods, with its header, 26 dp rows and glyph column. Each volume shows its phase glyph; a disk shows a warning only when it is read-only by surprise. A loop device or an optical drive is read-only by design, so a DVD drive's read-only flag no longer turns yellow. Selection follows the disk or volume by name, not by line, and each tab keeps its own.
- **Page headers as toolbars:** every page's title is a 13 px label in a 38 px
  toolbar row with its filter and controls, each control 24 px high, under a
  hairline on table pages. Every Resources kind now has Pods' header, with the
  namespace picker on namespaced kinds.
- **Monitoring's header:** dashboards use the shared page header and frame, with
  a "Dashboards / title" breadcrumb that opens a collapsed dashboards column, a
  meta line with the panel count, state and time of the last answer, and panels
  that end at the page's padding.
- **A pod's log level chips count its lines:** Error, Warn, Info, Debug and Unknown on a pod's Logs tab read 0 whatever the stream held. They now count every line the pod's container wrote, as a node's Logs tab always did.
- **Native platform CI:** advisory Linux and Windows checks also run weekly on main. Linux reuses a dependency cache saved only from main, while PR runs restore without adding copies; Windows stays uncached to preserve the shared cache budget. Failed native checks still need resolution before review approval.
- **Linux and Windows checks:** advisory CI checks and lints the workspace on both platforms and tests the terminal keyboard. Copy, paste and leaving the terminal use Ctrl-Shift there, while plain Ctrl-C/V still reach the shell; macOS keeps its Command shortcuts.
- **Changelog fragments:** pull requests keep their release notes in separate files, checked by CI and collected in section order before a release, so parallel changes no longer contend on Unreleased.
- **etcd without a leader no longer crashes the app:** when members answer but none reports a
  leader, as after a lost quorum, the etcd page crashed; it now shows the leader as not reported.
- **Memory percentages round down:** Attention, Nodes, the Peak memory card and pod meter tooltips now show memory as a whole percent rounded down, so a node at 94.6 % reads 94 % on its warning row instead of 95 %, the critical threshold.
- **Network tables:** the node's interfaces, connections, listeners and KubeSpan peers use the shared table, with glyphs on interfaces that have errors or drops, half-open or closing sockets and peers. Narrow windows scroll the columns sideways instead of dropping them.

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
