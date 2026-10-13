# Architecture

What the code is today, on main `38aee6a` (after #589, before #591 and #593). It describes; [AGENTS.md](../AGENTS.md) holds the rules and the other docs hold each feature's behaviour. Every claim names the module that makes it true, so a stale line is one you can check. Paths are from the repository root.

## Crates and their direction

A crate may depend only on the crates below it. Read the arrows off each `crates/*/Cargo.toml`, not off this page.

```
src/main.rs (freshkube)      → freshkube-desktop, freshkube-probe
freshkube-desktop            → core, logs, monitoring, observability, probe, reads, terminal, ui, talos-rs
freshkube-observability      → core, graph, logs, monitoring, ui
freshkube-monitoring         → core, probe, ui, talos-rs
freshkube-logs               → core, probe, ui
freshkube-terminal           → probe, ui
freshkube-reads              → core
freshkube-ui                 → graph, probe
freshkube-core               → talos-rs
freshkube-workbench          → graph, probe, ui          (never shipped)
freshkube-graph, freshkube-probe, talos-rs                (no workspace dependencies)
```

The rules the graph shows:

- **`freshkube-core` has no GPUI.** Its manifest has no `gpui` dependency; cluster logic, domain types and the snapshot (`crates/freshkube-core/src/snapshot.rs`) are plain Rust. A screen calls core with real Rust types; there is no serialization boundary.
- **`freshkube-ui` has no core.** The look (theme, tables, page frame, drawer, inspector) knows nothing of clusters. It depends on `freshkube-graph` (curve layout) and `freshkube-probe` only.
- **Feature crates sit under desktop and never name it.** Monitoring and Observability reach desktop only through their events (below). Observability uses Monitoring; Monitoring does not use Observability.
- **`freshkube-reads` is the only place a read's owner lives** that is neither core nor desktop: `Loader<T, I>` (`crates/freshkube-reads/src/loader/mod.rs`) needs core's `Snapshot` and `spawn_job` and a GPUI `Task`, so it depends on core and `gpui-kit`, and on nothing of `freshkube-ui` or desktop.
- **Test-only edges are features.** `testing` (core, logs, monitoring, observability, reads, terminal, ui) and `freshkube-probe`'s `counting` are turned on from desktop's `[dev-dependencies]` only.

Desktop reaches the moved code by its old paths: `crate::ui`, `crate::monitoring`, `crate::observability`, `crate::terminal` are `use … as …` lines in `crates/freshkube-desktop/src/lib.rs` (and `crates/freshkube-desktop/src/ui.rs` for the look). Inside core, `delivery/` and `applications/` are still core modules; #593 moves them together into a `freshkube-delivery` crate that sits between core and desktop/observability. This page describes them as core's until it lands.

## What a page is

There are three ways to be a page on main. #579 is moving them to one trait in `crates/freshkube-desktop/src/screens/page.rs` (#591, not merged); this page describes what is on main.

**1. A `ScreenPanel`** (`crates/freshkube-desktop/src/screens/mod.rs`). The trait has `new`, `set_source`, `activate`, `refresh`, `manual_refresh`, `focus`, `status`, `loading_motion` and `set_embedded`, and there is no hide hook. Nine impls: `EtcdScreen`, `WorkloadsScreen` (Health), `SecurityScreen`, `LifecycleScreen`, `OperationsScreen`, `ProcessesScreen`, `StorageScreen`, `NetworkScreen`, `DiagnosticsScreen`, each under `screens/<name>/`. They are retained: `Pilot.screens` (`crates/freshkube-desktop/src/desktop/mod.rs`) holds `(ScreenKind, ScreenHandle)` pairs. `ScreenHandle` is the type-erased wrapper that lets the shell call them from one list. `ScreenKind` and the `Page`→`ScreenKind` map (`Page::screen`) are in `crates/freshkube-desktop/src/desktop/pages.rs`. The shell sends `set_source` to every screen and `activate`/`refresh` only to the visible one (`Pilot::push_source`, `Pilot::active_screen`).

**2. An entity page the shell owns directly,** with its own `set_source` and `set_visible`, called from `Pilot::navigate` (`desktop/pages.rs`) and `Pilot::push_source` (`desktop/mod.rs`):

| Page | Entity | Home |
| --- | --- | --- |
| Resources | `ResourcesScreen` | `crates/freshkube-desktop/src/resources/screen/` |
| Monitoring | `MonitoringPage` | `crates/freshkube-monitoring/src/page/` |
| Observability | `ObservabilityPage` | `crates/freshkube-observability/src/lib.rs` |
| Applications | `ApplicationsPage` | `crates/freshkube-desktop/src/applications/` |
| Settings | `SettingsPage` (its Delivery sources list has `set_visible`) | `crates/freshkube-desktop/src/desktop/settings/` |
| System services | `SystemServices` | `crates/freshkube-desktop/src/desktop/system_services/` |

Resources, Monitoring, Observability and Applications cancel their reads on hide. Custom Resources (`resources/custom.rs`), the dock (`desktop/dock/`) and Command-K search (`desktop/search/`) are entities too, but are not pages: they sit beside whatever page shows.

**3. Drawn by the shell itself.** Overview, Nodes and Services render as methods on `Pilot` (`render_overview`, `render_nodes`, …); `PageHost` (`desktop/mod.rs`) wraps each in a view of its own so GPUI can reuse the last frame. The Nodes workspace (`crates/freshkube-desktop/src/desktop/nodes/`) also **embeds** four `ScreenPanel`s in its node pane (`NodeTab::screen`: Processes, Storage, Network, Diagnostics) and the Resources and log views, laid out by the pane's width (`screens::set_node_pane_width`), not the window's.

Cached views, render probes and the reasons for them are in AGENTS.md ("Keep render cheap").

## The session and the source

The shell holds one cluster at a time. The pieces, from the user's choice down to what a page reads:

- **`Registry`** (`crates/freshkube-desktop/src/desktop/session.rs`) owns the active `ClusterSession` and the parked ones. A session keeps the Kubernetes summary, its reflector job and task, and the access it was collected under. Switching workspace entries parks the active session and keeps nothing running. `docs/ACCESS_IDENTITY.md` has the contract.
- **`AppliedConfig`** and **`Target`** (`crates/freshkube-desktop/src/backend.rs`): the applied talosconfig path and context, and a node request identity (epoch, context, node, address). Node selection advances the epoch; access replacement invalidates targets.
- **`ScreenSource`** (`screens/mod.rs`) is what a `ScreenPanel` is given: the `Target`, the node roster (`NodeSummary`) and, unless this is example data, a `LiveSource` (Talos client, cluster overview, collector). `Pilot::screen_source` builds it.
- **`KubeSource`** (`crates/freshkube-desktop/src/resources/screen/mod.rs`) is what the Kubernetes-facing entities are given: an `id` that covers the access but not the target node, the context's name, and a `KubeAccess`: `Example`, `Talos(TalosAccess)` (the client the Talos side sets up) or `Direct(DirectAccess)` (a kubeconfig context, for `--kubernetes-only`). `Pilot::kube_source` builds it.
- **`AccessIdentity`** (`crates/freshkube-core/src/client_cache.rs`, used by `session.rs`) names an access session plus the inspected configuration revision. `KubeSource.id` is its key; Monitoring's `ClusterSource` and Observability's access string are derived from it in `Pilot::push_source`.

`Pilot::push_source` is the single fan-out: it hands the screens a `ScreenSource` and every other entity (search, Custom Resources, node pods, Monitoring, Observability, Applications, Delivery sources, the dock, Resources) a `KubeSource` or something derived from it. A page compares the new source with the old one and drops its data and jobs when the identity changed.

The kubeconfig-only mode (`desktop/kubernetes_only.rs`) and the fixture mode (`--fixture`, below) are not separate shells: they change what `kube_source` and `screen_source` return.

## Reads: `Snapshot`, `Loader` and the exceptions

- **`Snapshot<T, I>` and `Request<I>`** (`crates/freshkube-core/src/snapshot.rs`) keep the last good value for one identity and refuse an answer for a superseded request. Desktop names it `state::Snapshot<T, I = AppliedConfig>` (`crates/freshkube-desktop/src/state.rs`). The transitions are tabulated in `docs/STATE_LIFETIMES.md`.
- **`OwnedJob` and `spawn_job`** (`crates/freshkube-core/src/job.rs`; re-exported by `crates/freshkube-desktop/src/backend.rs`) run Tokio work under a deadline; dropping an `OwnedJob` aborts it. The GPUI delivery `Task` that applies the answer is kept beside it, and the answer is checked for identity and generation before publishing.
- **`Loader<T, I>`** (`crates/freshkube-reads/src/loader/mod.rs`) is a `Snapshot`, an `OwnedJob` and a `Task` together: `load`, `resolve` (fixtures), `reset`, `data`, `error`, `is_loading`, `last_successful`. The nine Talos screens use desktop's wrapper `screens::Loader<T>` (`crates/freshkube-desktop/src/screens/loader.rs`), which fixes the identity to `Target` and the deadline to `SCREEN_DEADLINE`.
- **Features with their own owners** keep them: Monitoring and Observability hold read/delivery pairs and a generation of their own; `LogView<S>` (`crates/freshkube-logs/`) owns its source's stream; Resources owns its list, watch and detail reads. `docs/STATE_LIFETIMES.md` lists each owner.

**The two exceptions to "a page reads only while it is shown":**

1. **Work the user or the session started, which runs while hidden.** The cluster summary session (nine reflector stores, `ClusterSession.summary_*` in `session.rs`, `crates/freshkube-desktop/src/desktop/kubernetes_summary/`); each dock log tab (`crates/freshkube-desktop/src/desktop/dock/`, `feed.rs`); a pod shell (`crates/freshkube-desktop/src/resources/shell/`); a port forward (`crates/freshkube-desktop/src/forwards/`). They end when closed, stopped, or on a context, kubeconfig or talosconfig change.
2. **Operations mutations.** A submitted run holds the app-wide operation ticket in a detached delivery task until core's runner ends (`crates/freshkube-core/src/operations/`, `crates/freshkube-desktop/src/mutation.rs`); it is never an ordinary `OwnedJob` and navigation does not cancel it.

Everything else is meant to read only while shown; `ScreenPanel` screens do not yet drop an in-flight read on hide (#582, #579).

## How a page reaches the shell

A page never holds the shell. It emits a typed event, and the shell subscribes when it builds the page.

- **Screens** emit `ScreenEvent` (`RefreshSummary`, `Back`, `OpenLogs`, `SelectNode`, `RetryCluster`, `OpenLogsOn`; `screens/mod.rs`). `Pilot::screen_entity` subscribes each one to `Pilot::screen_event` (`desktop/mod.rs`).
- **Entity pages** emit their own types, routed by `Pilot::subscribe_page_events` (`desktop/mod.rs`): `ResourceLink` (Resources, Applications), `RevisionLink` and `SwitchLink` (Applications), `NodePodsEvent` (the node's Pods list), `DockEvent` (the dock), `ObservabilityEvent`, `MonitoringEvent` and `NotServed` (Resources).
- **Object links** all end in `Pilot::open_object` (`desktop/pages.rs`) through `Pilot::resource_link` (`crates/freshkube-desktop/src/desktop/object_links/`); a link to a cluster that is not open is refused with a notice, and one to another workspace entry goes through `Pilot::open_on_entry` (`desktop/switch.rs`).
- **Redraws** use `cx.observe` on the dock and Custom Resources, which notify the shell.

There are about a dozen event types and two ways to subscribe. #579 is collapsing them into one page event (`PageEvent`) and one `subscribe`.

## Where example data lives

`--fixture` builds `GpuiOptions::fixture()` (`crates/freshkube-desktop/src/lib.rs`), which sets `Pilot.fixture`; no credentials or cluster are touched. Pages then resolve from example data instead of reading:

- Talos side: `crates/freshkube-desktop/src/fixture.rs` (the cluster and its nodes, and the hold knobs), plus `screens/<name>/example.rs` for each screen.
- Resources: `crates/freshkube-desktop/src/resources/example/` (objects, the acme workspace's Kargo and Argo CD kinds); `resources/shell/example.rs` (a local shell); `crates/freshkube-desktop/src/forwards/example.rs` (a loopback listener).
- Applications and changes: `crates/freshkube-desktop/src/applications/example.rs`; in core, `crates/freshkube-core/src/applications/example.rs` and `crates/freshkube-core/src/delivery/change/example.rs`, `delivery/changes/example.rs` and `delivery/fixtures.rs`.
- Monitoring and Observability: `crates/freshkube-core/src/monitoring/example.rs`; `crates/freshkube-observability/src/example.rs`, `application/example.rs`, `logs/example.rs`.
- Maintenance mode: `crates/freshkube-desktop/src/maintenance/example.rs`.

Example names are invented (acme, cluster-a/b/c, shop-api, `core-fra`-style entries); a test or capture must never need a real cluster.

## Where the debug knobs are parsed

The knobs are read from the environment where each feature starts, not in one place (#585 gathers them):

- Page, theme, motion, kind, dashboard, sidebar and log search: `Pilot::open_startup_page` and `startup_selection` in `crates/freshkube-desktop/src/desktop/startup/mod.rs` (`FRESHKUBE_PAGE`, `_THEME`, `_MOTION`, `_KIND`, `_DASHBOARD`, `_SIDEBAR`, `_LOG_SEARCH`); window size in `crates/freshkube-desktop/src/desktop/mod.rs` (`FRESHKUBE_WINDOW_SIZE`, via `startup::window_size`).
- Text size: `crates/freshkube-ui/src/text_size.rs` (`FRESHKUBE_TEXT_SIZE`).
- Fixture holds: `crates/freshkube-desktop/src/fixture.rs` (`FRESHKUBE_FIXTURE_HOLD`), also read by `freshkube-monitoring` and `freshkube-observability` (`page/mod.rs`, `connection.rs`) and Applications.
- Example shapes: `FRESHKUBE_ETCD` in `screens/etcd/mod.rs`; `FRESHKUBE_APPLICATIONS` in `applications/example.rs` and core's `delivery/change/example.rs`.
- The app menu button: `crates/freshkube-desktop/src/desktop/app_menu/mod.rs` (`FRESHKUBE_APP_MENU`).
- First-frame timing: `crates/freshkube-probe/src/first_frame.rs` and `src/main.rs` (`FRESHKUBE_FIRST_FRAME`).
- The stress binary: `crates/freshkube-desktop/src/stress.rs` and `crates/freshkube-desktop/src/bin/stress/` (`FRESHKUBE_STRESS_*`).
- `FRESHKUBE_KUBECONFIG` (the default kubeconfig file): `crates/freshkube-core/src/resources/contexts.rs`.

Debug and stress builds honour the page, theme, size and text-size knobs; release builds ignore them. The command-line options are `src/main.rs`'s `Cli`, which builds a `GpuiOptions` (`lib.rs`).
