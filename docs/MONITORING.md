# Monitoring

Grafana-like dashboards in the Console look (F4 in [DESIGN.md](DESIGN.md)), drawn natively from Prometheus. This file records what the review of grafaui found, the decisions taken with the user, and the build order. The [roadmap](ROADMAP.md) tracks its progress. Observability on Coroot's data (O1–O7) is a separate, later piece of work.

## Feasibility

Checked on 3 October 2026 by reading `~/code/grafaui` (github.com/skel84/grafaui, MIT, the user's own project) at `4e95e99`. Nothing has touched a cluster yet.

- **Same toolkit.** Grafaui pins `gpui-kit =0.7.0`, as Freshkube does, and uses the same `dp()` and `dp_px()` sizing rules.
- **Three crates.**
  - `grafaui-model` (about 10,000 lines) parses dashboard JSON (panels, overrides, transforms, units, variables, the grid layout) and makes up fake data (`FakeSource`). It has no UI types. Its only style is Grafana's palettes in `color.rs`. It is checked against 98 real dashboards.
  - `grafaui-prometheus` (about 1,500 lines) interpolates variables and macros, builds range and instant requests (`query::requests`) and converts answers into frames (`response::convert`, `response::merge`), all as pure functions. Only `Client` does I/O: blocking `reqwest`, one URL, no authentication, POST for queries, four requests at a time.
  - `grafaui-desktop` (about 6,000 lines) draws the panels: timeseries, stat, gauge, bar, table, pie, heatmap and geomap. Its only public API is `run` and `run_catalog`, which own the application and its window. Every view is `pub(crate)`.
- **Its look is Grafana's,** on purpose: ring gauges, the 16-colour classic palette, threshold-coloured lines and "Warning" badges on panels, Menlo for queries. The Console rules ask for the opposite: six fixed series colours and status shown by glyph, never as a series colour.
- **What F4 needs that grafaui lacks:** a cursor linked across charts, event markers (Grafana annotations), legend hover that fades the other lines, and the PromQL query on hover.
- **Variable resolution** (`Client::resolve_variables`) mixes the plan with the requests: it calls `label_values`, `query_result` and the like in dashboard order. It is the one part of the query crate that needs splitting before another transport can use it.
- **Reaching Prometheus.** The Kubernetes API proxies to a Service at `/api/v1/namespaces/<ns>/services/<scheme>:<name>:<port>/proxy/<path>`, with the chosen context's credentials, with no port forward. A GET through the proxy needs `get` on `services/proxy`. A POST needs `create`, even though a Prometheus query changes nothing.

Verdict: reuse the model and query crates as libraries, and draw the panels ourselves. Both crates are style-free, and they hold the hard part. Grafaui's desktop crate would bring Grafana's look and an app it owns.

## Decisions

Taken with the user on 3 October 2026.

- **Option 2.** Freshkube depends on `grafaui-model` and `grafaui-prometheus` as git dependencies pinned to a revision, and draws its own Console panels. Grafaui stays its own app and keeps working unchanged. Plot code from `grafaui-desktop` (axes, projection, hover) may be lifted where it fits. It is the user's own MIT code, but its origin is still noted in the module.
- **Read-only, GET only.** Queries go through the service proxy as GET, so they stay within the read verbs. A 403 says "Not allowed to query Prometheus (services/proxy)", never an empty dashboard.
- **Coroot later.** The user runs Coroot. Observability (O1–O7) will read Coroot's API rather than reimplement its analysis, but not in this work, and it needs no mocks now.
- **No panel editor and no Explore screen,** as F4 says.

Defaults the plan follows until the user says otherwise:

- **Its own rail area,** Monitoring, after Events and before the Kubernetes groups, with its dashboards in the 208 dp column.
- **Built-ins and a folder.** Two or three dashboards written for the Console look ship with the app, and Settings can point at a folder of the user's own Grafana JSON. The grafana.com exports in grafaui's fixtures carry various licences and are not bundled.
- **In-cluster Prometheus first.** A custom URL with a bearer token (Thanos, Mimir, VictoriaMetrics, an external Prometheus) is a later step, not part of this plan.

## Design

### Grafaui: a transport seam

- `grafaui-prometheus` gains a sans-I/O planner for variables: it hands out the next request (path and parameters), takes its JSON answer, and ends with the resolved `Variables` and warnings. `Client::resolve_variables` becomes a loop over that planner. Panels already work this way through `query::requests` and `response::convert`.
- Requests can be built as GET with query parameters as well as POST forms. Query strings stay within proxy limits because dashboards' expressions are short; a request that would pass 8 KiB is refused with a clear message, never silently cut.
- `Client` and `reqwest` move behind a default `client` feature, so Freshkube builds the crate without them.
- The existing tests pass unchanged, and new ones cover the planner on dependent variables, `includeAll` and an empty answer.

### Core: `freshkube-core::monitoring`

- **Discovery** lists Services cluster-wide, or in the usual monitoring namespaces (`monitoring`, `prometheus`, `observability`, `kube-prometheus-stack`, `default`, `kube-system`) when that is refused, and ranks the candidates: `prometheus-operated`, then Services labelled `app.kubernetes.io/name=prometheus` or `app=prometheus`, then Services named like Prometheus but not like its companions (Alertmanager, the operator, exporters, Grafana), then any Service with a port named `web` or numbered 9090. The port is the one named `web` or `http-web`, else 9090; a port named `https` or numbered 443 goes through the proxy as `https:`. The choice is remembered per context in the preferences, by namespace, name and port, and tried first next time. Up to six candidates are confirmed with `/api/v1/status/buildinfo`, in rank order, with 8 s each, before one is used. A proxy refusal ends discovery as refused, since RBAC applies to every Service alike.
- **Transport** sends each planned request as a GET through `kube::Client::send` on Tokio with a 20 s timeout and at most four requests at a time per connection, and reads at most 64 MiB, as grafaui does; a larger answer is refused whole. Errors name the category (refused, not found, unavailable, timed out, bad answer, rejected by Prometheus, unsupported) and never the URL or credentials. A request whose query string would pass 8 KiB is never sent.
- **The scrape interval** comes from `/api/v1/targets` where it reports one, otherwise 30 s.
- **Example mode** (`--fixture`) answers from grafaui's `FakeSource`, so pages, tests and screenshots need no cluster.

### Desktop: `freshkube-desktop::monitoring`

- **Panels.** Timeseries (lines and areas, stacked or not, linear and log axes, thresholds as a dashed line with a muted band), stat (with a sparkline), bar gauge, table (our table rules: the mono numbers, muted units, right-aligned values) and a bar list. A Grafana gauge is drawn as a stat with a bar; a pie as a bar list. A kind we don't draw (geomap, canvas, logs, nodeGraph and others) shows one line, "Not drawn here: geomap", in its grid cell.
- **Colours: the Console palette, never Grafana's.** The F4 mock (`ConsoleMetrics`) sets them, and every colour a dashboard asks for is ignored: palette modes, fixed and named colours, overrides, continuous schemes and threshold colours. Drawn on the card colour (#17181B):
  - Series take the six Console slots in their fixed order: #3987e5, #d95926, #199e70, #c98500, #d55181, #008300, as 2 px lines. Past six, the remaining series draw in faint grey (#5E636B) and take their own slot colour only while hovered or picked in the legend.
  - Ordered levels (series whose legends read as quantiles, such as p50, p95 and p99, or le buckets) use the blue ramp #1c5cab, #3987e5, #86b6ef, as do heatmap cells from light to dense.
  - A threshold is a 1 px dashed line in warning amber (#F5A623) or critical red (#F0484E), the only place status colours touch a chart, besides the stat value that crossed it.
  - A sparkline in a list of subjects takes its row's status tone (accent #4797FF when fine, amber, red), as the mock's attention list does; it stands for that row's status, not for a series.
  - Areas fill with their line's colour at low opacity; grids and axes use the hairline (#24262A) and faint text (#5E636B), numbers in Source Code Pro with tabular figures.
- **The page.** A header row with the dashboard's title, its variables as dropdowns, the time picker (the dashboard's quick ranges, relative to now) and auto-refresh (off, 30 s, 1 m, 5 m). Collapsible rows as in Grafana. The grid follows the dashboard's 24-column layout and becomes one column under the narrow breakpoint.
- **Interactions.** One cursor shared by every timeseries on the page: hovering one draws the line and the values in all of them. Hovering a legend entry fades the other series. Hovering a panel's title shows its PromQL after interpolation, in Source Code Pro, with Copy.
- **Reads.** Only a visible page queries, and only for panels in or near the viewport. Every answer carries the dashboard, time range and variable epoch, and an older one is dropped. A panel keeps its last frame, marked stale, when a refresh fails. Display data (paths, axis ticks, legend values) is derived when a frame arrives or the width changes, never in `render`.
- **States.** Each one is distinct: no Prometheus found (with what was looked for and a way to pick a Service), refused, failed, loading, empty answer, and a stale panel.

### Markers

- **Deploys:** a Deployment's ReplicaSet changing, read from the events and ReplicaSets the resource store already lists, drawn as a thin vertical line in accent blue (#4797FF, 55% opacity) with the Deployment's name and new revision on hover, as in the mock.
- **Nodes:** a node turning NotReady or Ready, and with Talos a reboot from the node's boot time, drawn the same way in critical red (#F0484E, 55% opacity), and Ready again in the same line at a lower opacity.
- Markers follow the variables: a dashboard filtered to one namespace or node shows only its own.

## Build order

Each step is its own commit (or a preparation commit and then the step), with unit and UI tests where behaviour changes, and its row in the roadmap's Done table. The app stays usable after every step.

0. **Grafaui seam,** in the grafaui repository: the variable planner, GET requests, `Client` behind the `client` feature, and tests. Pushed only with the user's go-ahead, then pinned by revision in Freshkube.
1. **Core.** `monitoring/` with discovery, the proxy transport, the scrape interval and the example source; unit tests against a fake API server (found, several candidates, none, 403, 404, timeout, an oversized answer).
2. **Panels.** Console timeseries, stat, bar gauge, table and bar list from a frame, in the Console palette; unit tests for slot order, the grey overflow, the quantile ramp and derived paths, and UI tests that each kind draws from example frames.
3. **Monitoring page.** The rail area, the dashboard column with the built-ins and the user's folder, variables, the time picker, auto-refresh, the shared cursor, legend fading and the PromQL on hover; `FRESHKUBE_PAGE=monitoring`. UI tests for every state in the Design section, a variable change, a time range change, the cursor across two charts and the page hiding mid-request.
4. **Markers.** Deploy and node markers on every timeseries; UI tests with example events.
5. **Charts in existing screens.** CPU and memory history in a pod's Overview and the node pane when Prometheus is connected, from the same panels and transport; nothing changes when it isn't.
6. **Live check.** With the user's go-ahead, on a context they name: discovery, the built-in dashboards and one of their own against their Prometheus, a 403 if one is at hand, and the stress numbers for a 30-panel dashboard in [PERFORMANCE.md](PERFORMANCE.md).
