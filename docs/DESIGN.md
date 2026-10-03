# The Console look

Freshkube's visual direction, chosen in a design session with the user on 2–3 October 2026. It is inspired by the dark mode of the UniFi network console, not copied from it. This file records what was decided, what was explored and left open, and the order in which it is built. The [roadmap](ROADMAP.md) tracks the commits.

The mockups are on the design canvas "Freshkube holistic layout" (https://claude.ai/artifact/U5UUnSURiNJJ3qtVxMRbi1). They use fictional example data and were never rendered or checked, so they show intent, not pixels. Where this file and a mockup differ, this file wins.

| Board | File | Status |
| --- | --- | --- |
| Original holistic proposal (Today, 1–6, Sidebar) | `Today`, `Main`, `Nodes`, `Node`, `Pod`, `Search`, `KubeOnly`, `Sidebar` | Built ([HOLISTIC_LAYOUT.md](HOLISTIC_LAYOUT.md)) |
| A–E′ Basalt, Glacier, Phosphor, Nightshade, Nord Grey dark and light | `Theme*` | Explored, not chosen; kept for reference |
| F Console dashboard | `ThemeConsole` | Built on the canvas, not committed to |
| F2, F2′ Pods | `ConsolePods`, `ConsolePodsCompact` | Superseded by F2″ |
| F2″ Pods, the reference table | `ConsolePodsGlyph` | Decided |
| F3 Pod detail | `ConsolePod` | Decided |
| F4 Monitoring | `ConsoleMetrics` | Built on the canvas, not committed to |
| O1–O7 Observability | `Obs*` | Explored, open |

Rubick was a reference the user liked but it must not be copied: no neutral charcoal look and no multicolour hashed resource names.

## Decided

### Tokens

Dark mode. A light mode for the Console look is not designed yet (see [Open](#open)).

| Role | Value |
| --- | --- |
| Background | `#0E0F11` |
| Rail, header and status bar | `#0A0B0C` |
| Card | `#17181B` |
| Raised (popovers, segmented controls, selected navigation) | `#1E2024` |
| Hover | `#25272C` |
| Tooltip | `#2A2D33` |
| Hairline | `#24262A` |
| Strong border | `#33363C` |
| Text | `#F2F3F5` |
| Secondary text | `#C9CCD1` |
| Muted text | `#8E939B` |
| Faint text | `#5E636B` |
| Accent (links, selection, focus) | `#4797FF` |
| Primary button | `#0A64E6` fill, white text |
| OK | `#3DD68C` |
| Warning | `#F5A623` |
| Critical | `#F0484E`; as text `#FF6B70` |
| Integration required | `#9085E9` |

Soft fills behind status chips are the status colour at 14–16% opacity; the selected rail button is the accent at 16%.

**Type.** Lato 400, 700 and 900 for the interface. Source Code Pro for resource names, addresses and numbers. Section and column captions are Lato 700 at 11 px, uppercase, slightly tracked, in the muted colour. Weight carries hierarchy; 700 is the usual weight for labels, buttons and tabs.

**Radii.** Cards 12 px, buttons and inputs 8 px, chips fully rounded, rail buttons 10 px.

**Chart series.** Six slots in a fixed order, checked against the card colour for contrast and colour blindness: `#3987e5`, `#d95926`, `#199e70`, `#c98500`, `#d55181`, `#008300`. Ordered levels such as p50, p95 and p99 use one blue ramp: `#1c5cab`, `#3987e5`, `#86b6ef`. Status colours are reserved for status and are never used as series colours.

### App frame

- **Header, 52 px:** the context switcher (cluster name and "Talos + Kubernetes"), section tabs, Search (⌘K) and Settings.
- **Icon rail, 64 px, on the left:** icon-only buttons with tooltips. A coloured dot on a button flags problems in that area.
- **Second column, 208 px, contextual:** resource kinds and namespaces with counts under Workloads, the dashboard list under Monitoring, observability navigation under Observability. An icon-only rail can't hold twenty-odd Kubernetes kinds, so this column is required.
- **Status bar, 28 px:** connection, port forwards and the last refresh.

### Status language

Shape and colour together, never colour alone, everywhere in the app:

| State | Glyph |
| --- | --- |
| OK | green filled dot |
| Warning, at risk | amber outlined triangle |
| Critical, failing | red filled diamond |
| Pending, unknown | hollow grey circle |
| Completed | grey tick |
| Integration required | violet outlined square |
| Errors in logs | blue dot |

Colour and shape say how bad it is; words say why, and only where a group header doesn't already say it.

### Tables

F2″ (`ConsolePodsGlyph`) is the reference.

- **One-line toolbar:** title, filter, status filter buttons, density toggle and columns.
- **Density:** comfortable 34 px rows or compact 26 px rows.
- **Problems first by default** when anything is wrong. The status filter starts on Problems, rows are grouped by cause (for example "On a NotReady node"), and the healthy rows collapse into one line. A banner, "Showing 25 of 140 · Show all", also hosts bulk actions.
- **Status is a 16 px glyph column,** not a text column. Selecting a row swaps its glyph for a checkbox.
- **Ready and restarts share a cell** ("0/1 ↻14"). The freed width goes to an Owner column (`deploy/`, `sts/`, `ds/`).
- **Node names drop the prefix every node shares,** with the full name on hover, but only when such a prefix is detected.
- **CPU and memory mini bars:** the fill is use against the limit, a tick marks the request, and grey means last known (stale, for example on a NotReady node).
- **Names:** the namespace prefix is muted and a random hash suffix is dimmed.
- **Keyboard:** sort indicators, a visible focus row; ↑ and ↓ move, ↵ opens, x selects, l opens logs.

### Tooltips

Every compact element has a hover tooltip: glyphs, shortened node names, meters, restart counts, owner kinds and icon-only buttons. The rich pattern, as for an at-risk pod, says what happened, why the shown state can't be trusted, the side effects (endpoints dropped, eviction time) and the keyboard shortcuts. Tooltips flip upward near the bottom edge of a table.

### Pod detail

F3 (`ConsolePod`) opens on the cause: a "Why it's failing" card with the state, last exit code, next restart and last log lines. Then the restart timeline, containers, Runs on (the node, Talos kubelet health and a link to its system services), Relations (ReplicaSet → Deployment, Service, ServiceAccount, IP and QoS) and recent events. Logs is the primary action.

## Built on the canvas, not committed to

- **F, the Console dashboard:** ring gauges with legends, a "Needs attention" table and a Health-by-area card.
- **F4, Monitoring:** Grafana-like dashboards in the Console look, with dashboard-wide filter dropdowns, a time picker, a cursor linked across charts, deploy and node event markers, thresholds, legend hover that fades the other lines, and the PromQL query on hover. A panel editor and an Explore screen are explicitly not wanted for now.

## Open

- **Observability, O1–O7,** uses Coroot's concepts (statuses, built-in checks and thresholds, the health table, service map, incidents and root cause, deployments), read from its source, without its visuals:
  - O1 an applications health matrix of per-app checks (errors, latency, upstreams, instances, restarts, CPU, memory, disk, network, DNS, logs), healthy values muted and problems as glyph and value, grouped into Applications, Control plane and Monitoring;
  - O2 a service map in tiers, link colour for status and width for traffic, with a side panel for the selected link;
  - O3 an app page: who calls it and what it calls, one tab per report with a status glyph, checks written as sentences with an editable "Condition: … > threshold" line;
  - O4 an incident: SLO compliance and error-budget burn rate (opens at 14.4× over 1 h/5 m or 6× over 6 h/15 m), a written root cause, the cause chain, one-click fixes and what was ruled out;
  - O5 deployments: per-release summaries, the Deployment spec diff and Roll back;
  - O6 profiling: a flame graph coloured by change against the previous 24 h;
  - O7 traces: a latency and error heatmap with the SLO line, error causes and a sample trace waterfall.

  The data sources shown are Prometheus, Coroot's node agent (eBPF), ClickHouse (logs and traces) and the Talos API. Still to decide: embed Coroot through its API or reimplement the concepts, which screens come first, and the Risks, Logs patterns and Costs screens, which aren't designed.
- **A light mode** for the Console look.
- **The default row density,** compact or comfortable.
- **Grouping problems by cause** beyond "pods on a NotReady node". Linking a crashing pod to the database it depends on is harder.

## Build order

Each step is its own commit (or a preparation commit and then the step), with UI tests where behaviour changes. The app stays usable after every step.

1. **Tokens and type** (done). The dark theme and palette take the Console tokens; Lato and Source Code Pro replace IBM Plex Sans Condensed and JetBrains Mono; radii follow the table above. Until a Console light mode is designed, the light theme keeps its colours and takes only the new type and radii.
2. **Status glyphs.** One glyph set for the status language, used by every tag, list and attention row, each with a tooltip.
3. **App frame.** Header, icon rail, contextual second column and status bar replace today's sidebar and title bar, keeping every page, shortcut and key context.
4. **Tables.** The F2″ rules on the Resources list, starting with pods: toolbar, density, glyph column, merged cells, Owner, mini bars, name shortening and tooltips; then problems-first grouping for the causes core can establish.
5. **Pod detail.** The F3 layout in the pod's Overview tab.

The Console dashboard, Monitoring, Observability and the light mode wait for the user's decision.
