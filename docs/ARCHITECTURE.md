# Architecture

What the code is today, on main `6b5fdc9` (after #589, #591 and #593). It describes; [AGENTS.md](../AGENTS.md) holds the rules and the other docs hold each feature's behaviour. Every claim names the module that makes it true, so a stale line is one you can check. Paths are from the repository root.

## Crates and their direction

A crate may depend only on the crates below it. Read the arrows off each `crates/*/Cargo.toml`, not off this page.

```
src/main.rs (freshkube)      → freshkube-desktop, freshkube-probe
freshkube-desktop            → core, delivery, logs, monitoring, observability, probe, reads, terminal, ui, talos-rs
freshkube-observability      → core, delivery, graph, logs, monitoring, ui
freshkube-delivery           → core, talos-rs
freshkube-monitoring         → core, probe, ui, talos-rs
freshkube-logs               → core, probe, ui
freshkube-terminal           → probe, ui
freshkube-workbench          → graph, probe, ui          (never shipped)
freshkube-reads              → core
freshkube-ui                 → graph, probe
freshkube-core               → talos-rs
freshkube-graph, freshkube-probe, talos-rs                (no workspace dependencies)
```

The rules the graph shows:

- **`freshkube-core` has no GPUI.** Its manifest has no `gpui` dependency; cluster logic, domain types and the snapshot (`crates/freshkube-core/src/snapshot.rs`) are plain Rust. A screen calls core with real Rust types; there is no serialization boundary.
- **`freshkube-ui` has no core.** The look (theme, tables, page frame, drawer, inspector) knows nothing of clusters. It depends on `freshkube-graph` (curve layout) and `freshkube-probe` only.
- **Feature crates sit under desktop and never name it.** Monitoring and Observability reach desktop only through their events (below). Observability uses Monitoring; Monitoring does not use Observability.
- **`freshkube-reads` holds the one shared, generic read owner.** `Loader<T, I>` (`crates/freshkube-reads/src/loader/mod.rs`) needs core's `Snapshot` and `spawn_job` and a GPUI `Task`, so it depends on core and `gpui-kit`, and on nothing of `freshkube-ui` or desktop. Features that own their reads keep them: Monitoring (`crates/freshkube-monitoring/src/request.rs`), Observability (`crates/freshkube-observability/src/connection.rs`) and `LogView`.
- **Test-only edges are features or dev-dependencies.** The `testing` features and `freshkube-probe`'s `counting` are turned on only from `[dev-dependencies]` (desktop's, and for the feature crates their own), never from a normal dependency. Desktop also has a plain dev-dependency on `freshkube-graph`, for the map's tests.

Desktop reaches the moved code by its old paths in `crates/freshkube-desktop/src/lib.rs`: `crate::monitoring`, `crate::observability` and `crate::terminal` are `use … as …` lines, and `crate::palette`, `theme`, `text_size` and `meters` come from one `use freshkube_ui::{…}`. `crate::ui` is a module (`crates/freshkube-desktop/src/ui.rs`) that re-exports `freshkube_ui::ui::*` and adds desktop's own helpers, such as `health_tone`, the one map from a core `HealthIndicator` to a tag's tone and icon (`freshkube-ui` cannot name core's type). The delivery chain (Kargo, Argo CD, builds and pods) and the applications derived from it live in `crates/freshkube-delivery/` (`src/applications/`, `change/`, `changes/`, `entry/`), not in core. It sits between core and desktop/observability, depends on core and `talos-rs` only, and has no UI types. Its reads fail with core's `Failure`, whose kind is set where the read fails, and the words for refused, not served, unreachable and unfinished come from `freshkube_delivery::words`. Its `testing` feature (`freshkube_delivery::testing`, `crates/freshkube-delivery/src/lib.rs`) holds what desktop's tests read and is turned on from desktop's `[dev-dependencies]`.

## What a page is

There are three ways to draw a page on main, and one trait for showing, hiding and hearing them. #579 is moving pages onto that trait. Its first two steps are on main. #591 added `PageEvent`, `PageSource` and `PageEventFn` (`crates/freshkube-desktop/src/screens/page.rs`) and the matching `ScreenPanel`/`ScreenHandle` methods. #599 made the shell tell every page with a handle whether it shows (`set_visible`) and hear the entity pages through `ScreenHandle::subscribe`. The shell does not call `set_page_source` yet (`#![expect(dead_code)]`, "from #579's step 3"), so each kind of page still gets its source its own way (below).

**1. A `ScreenPanel`** (`crates/freshkube-desktop/src/screens/mod.rs`). The trait has `set_source`, `activate`, `refresh`, `manual_refresh`, `focus`, `status`, `loading_motion`, `set_embedded`, `set_page_source`, `set_visible` (a no-op by default) and `subscribe_page`. A page makes itself with its own `new`, which the screen factory names; #599 took `new` out of the trait so that entities built another way can implement it. Nine Talos screens implement it: `EtcdScreen`, `WorkloadsScreen` (Health), `SecurityScreen`, `LifecycleScreen`, `OperationsScreen`, `ProcessesScreen`, `StorageScreen`, `NetworkScreen`, `DiagnosticsScreen`, each under `screens/<name>/`. They keep the default `set_visible`, which does nothing; #582 gives `Loader` its hook. They are retained: `Pilot.screens` (`crates/freshkube-desktop/src/desktop/mod.rs`) holds `(ScreenKind, ScreenHandle)` pairs. `ScreenHandle` is the type-erased wrapper that lets the shell call them from one list. `ScreenKind` and the `Page`→`ScreenKind` map (`Page::screen`) are in `crates/freshkube-desktop/src/desktop/pages.rs`. The shell sends `set_source` to every screen and `activate`/`refresh` only to the visible one (`Pilot::push_source`, `Pilot::active_screen`).

**2. An entity the shell owns directly,** most with their own `set_source`, which `Pilot::push_source` (`desktop/mod.rs`) calls. Resources, Monitoring, Observability, Applications and Settings also implement `ScreenPanel` (`crates/freshkube-desktop/src/desktop/entity_pages.rs`). Their `set_visible` calls the entity's own, `refresh` its own and `subscribe_page` hears its events. Their `set_source` and `activate` do nothing there, since `push_source` still hands each a `KubeSource` or a source derived from it. `Pilot.pages` holds their `(Page, ScreenHandle)` pairs beside `Pilot.screens`; #579's step 3 joins the two.

| Page | Entity | Home |
| --- | --- | --- |
| Resources | `ResourcesScreen` | `crates/freshkube-desktop/src/resources/screen/` |
| Monitoring | `MonitoringPage` | `crates/freshkube-monitoring/src/page/` |
| Observability | `ObservabilityPage` | `crates/freshkube-observability/src/lib.rs` |
| Applications | `ApplicationsPage` | `crates/freshkube-desktop/src/applications/` |
| Settings | `SettingsPage`; only its Delivery sources list has `set_source` and `set_visible`, and it drops its check on hide | `crates/freshkube-desktop/src/desktop/settings/` (`sources/mod.rs`) |
| System services | `SystemServices`; no `set_source` or `set_visible`: the shell pushes the node roster's services with `set_nodes` (`desktop/target.rs`, `desktop/mod.rs`) and the read state with `set_reading`; the page makes no requests | `crates/freshkube-desktop/src/desktop/system_services/` |

Resources, Monitoring, Observability and Applications cancel their reads on hide, and Settings drops its Delivery sources check, each from its `set_visible`. `Pilot::sync_visible` (`desktop/pages.rs`) tells the one page that shows `true` and every other `false`, Talos screens and entity pages alike, whenever the page or the node pane's tab changes (`Pilot::navigate`, `sync_node_visibility` in `desktop/nodes/mod.rs`). A roster change that closes the pane or drops its tab has no window at hand, so `rebuild_joined_nodes` asks for it once its update ends (`Pilot::sync_visible_soon`). Overview, Nodes and System services have no handle yet and hear nothing. A node pane's screen shows only while Nodes is up with its tab selected (`Pilot::shown_page`). Custom Resources (`resources/custom.rs`), the dock (`desktop/dock/`) and Command-K search (`desktop/search/`) are entities too, but are not pages: they sit beside whatever page shows.

**3. Drawn by the shell itself.** Overview and Nodes render as methods on `Pilot` (`render_overview` in `desktop/overview/mod.rs`, `render_nodes` in `desktop/nodes/view.rs`); `PageHost` (`desktop/mod.rs`) wraps each in a view of its own so GPUI can reuse the last frame, and a third `PageHost` shows the `SystemServices` entity. `Pilot::render_services` (`desktop/services.rs`) is something else: the node pane's System services tab. The Nodes workspace (`crates/freshkube-desktop/src/desktop/nodes/`) also **embeds** four `ScreenPanel`s in its node pane (`NodeTab::screen`: Processes, Storage, Network, Diagnostics) and the Resources and log views, laid out by the pane's width (`screens::set_node_pane_width`), not the window's.

Cached views, render probes and the reasons for them are in AGENTS.md ("Keep render cheap").

## The session and the source

The shell holds one cluster at a time. The pieces, from the user's choice down to what a page reads:

- **`Registry`** (`crates/freshkube-desktop/src/desktop/session.rs`) owns the active `ClusterSession` and the parked ones. A session keeps the Kubernetes summary, its reflector job and task, and the access it was collected under. Switching workspace entries parks the active session and keeps nothing running. `docs/ACCESS_IDENTITY.md` has the contract.
- **`AppliedConfig`** and **`Target`** (`crates/freshkube-desktop/src/backend.rs`): the applied talosconfig path and context, and a node request identity (epoch, context, node, address). Node selection advances the epoch; access replacement invalidates targets.
- **`ScreenSource`** (`screens/mod.rs`) is what a `ScreenPanel` is given: the `Target`, the node roster (`NodeSummary`) and, unless this is example data, a `LiveSource` (Talos client, cluster overview, collector). `Pilot::screen_source` builds it.
- **`KubeSource`** (`crates/freshkube-desktop/src/resources/screen/mod.rs`) is what the Kubernetes-facing entities are given: an `id` that covers the access but not the target node, the context's name, and a `KubeAccess`: `Example`, `Talos(Box<TalosAccess>)` (the client the Talos side sets up) or `Direct(DirectAccess)` (a kubeconfig context, for `--kubernetes-only`). `Pilot::kube_source` builds it.
- **`AccessIdentity`** (`crates/freshkube-core/src/client_cache.rs`, used by `session.rs`) names an access session plus the inspected configuration revision. With Talos access `KubeSource.id` is its key; kubernetes-only uses `DirectAccess::id()` and the fixture `resources::example::connection(context)`. Monitoring's `ClusterSource` and Observability's access string are derived from it in `Pilot::push_source`.

`Pilot::push_source` is the single fan-out: it hands the screens a `ScreenSource` and every other entity (search, Custom Resources, node pods, Monitoring, Observability, Applications, Delivery sources, the dock, Resources) a `KubeSource` or something derived from it. A page compares the new source with the old one and drops its data and jobs when the identity changed.

The kubeconfig-only mode (`desktop/kubernetes_only.rs`) and the fixture mode (`--fixture`, below) are not separate shells: they change what `kube_source` and `screen_source` return.

## Reads: `Snapshot`, `Loader` and the exceptions

- **`Snapshot<T, I>` and `Request<I>`** (`crates/freshkube-core/src/snapshot.rs`) keep the last good value for one identity and refuse an answer for a superseded request. Desktop names it `state::Snapshot<T, I = AppliedConfig>` (`crates/freshkube-desktop/src/state.rs`). The transitions are tabulated in `docs/STATE_LIFETIMES.md`.
- **`OwnedJob` and `spawn_job`** (`crates/freshkube-core/src/job.rs`; re-exported by `crates/freshkube-desktop/src/backend.rs`) run Tokio work under a deadline; dropping an `OwnedJob` aborts it. The GPUI delivery `Task` that applies the answer is kept beside it, and the answer is checked for identity and generation before publishing.
- **`Loader<T, I>`** (`crates/freshkube-reads/src/loader/mod.rs`) is a `Snapshot`, an `OwnedJob` and a `Task` together: `load`, `resolve` (fixtures), `reset`, `data`, `error`, `is_loading`, `last_successful`. The nine Talos screens use desktop's wrapper `screens::Loader<T>` (`crates/freshkube-desktop/src/screens/loader.rs`), which fixes the identity to `Target` and the deadline to `SCREEN_DEADLINE`.
- **Features with their own owners** keep them: Monitoring and Observability hold read/delivery pairs and a generation of their own; `LogView<S>` (`crates/freshkube-logs/`) owns its source's stream; Resources owns its list, watch and detail reads. `docs/STATE_LIFETIMES.md` lists each owner.

**The two exceptions to "a page reads only while it is shown":**

1. **Work the user or the session started, which runs while hidden.** The cluster summary session (nine reflector stores, `ClusterSession.summary_*` in `session.rs`, `crates/freshkube-desktop/src/desktop/kubernetes_summary/`); each dock log tab (`crates/freshkube-desktop/src/desktop/dock/`, `feed.rs`); a pod shell (`crates/freshkube-desktop/src/resources/shell/`); a port forward (`crates/freshkube-desktop/src/forwards/`). A log tab ends when closed or on a context, kubeconfig or talosconfig change; a running shell does too, after the shell asks (`shell::unless_shell`); a forward ends only on Stop and runs on through page, context and kubeconfig changes (`forwards/mod.rs`, `desktop/switch.rs`).
2. **Operations mutations.** A submitted run holds the app-wide operation ticket in a detached delivery task until core's runner ends (`crates/freshkube-core/src/operations/`, `crates/freshkube-desktop/src/mutation.rs`); it is never an ordinary `OwnedJob` and navigation does not cancel it.

Everything else is meant to read only while shown. The entity pages drop their reads from `set_visible`; the Talos screens do not yet drop an in-flight read on hide (#582).

## How a page reaches the shell

A page never holds the shell. It emits a typed event, and the shell subscribes when it builds the page.

- **Screens** emit `ScreenEvent` (`RefreshSummary`, `Back`, `OpenLogs`, `SelectNode`, `RetryCluster`, `OpenLogsOn`; `screens/mod.rs`). `Pilot::screen_entity` subscribes each one, as `PageEvent::Screen`, to `Pilot::page_event`, which hands it to `Pilot::screen_event` (`desktop/mod.rs`).
- **Entity pages** emit their own types: `ResourceLink` (Resources, Applications), `RevisionLink` and `SwitchLink` (Applications), `NodePodsEvent` (the node's Pods list), `DockEvent` (the dock), `ObservabilityEvent`, `MonitoringEvent` and `NotServed` (Resources). The shell hears Resources, Monitoring, Observability and Applications through their `ScreenHandle::subscribe` (`Pilot::subscribe_pages`, `desktop/entity_pages.rs`), with a `WeakEntity<Pilot>` trampoline into `Pilot::page_event`. It subscribes in `Pilot::new` before any page emits, since GPUI activates a subscription only after the current update. The node's Pods list and the dock are still subscribed in `Pilot::subscribe_page_events` (`desktop/mod.rs`), which wraps their events in `PageEvent` for the same `page_event`. System services emits `ServiceEvent` (`desktop/system_services/mod.rs`), which `Pilot::new` subscribes on its own.
- **Object links** all end in `Pilot::open_object` (`desktop/pages.rs`) through `Pilot::resource_link` (`crates/freshkube-desktop/src/desktop/object_links/`); a link to a cluster that is not open is refused with a notice, and one to another workspace entry goes through `Pilot::open_on_entry` (`desktop/switch.rs`); `open_object` can also divert a link from another entry's ended connection (`divert_link`, `switch.rs`).
- **Redraws** use `cx.observe`, among others on the dock and Custom Resources, which notify the shell.

There are about a dozen event types. `PageEvent` (`screens/page.rs`) wraps all of them except `ServiceEvent`, and `Pilot::page_event` handles every variant. The pages in `Pilot.pages` are heard through `ScreenHandle::subscribe`; the Talos screens, the node's Pods list and the dock still through `cx.subscribe_in`. #579's step 3 keeps every page in one map and hears them all one way.

## Where example data lives

`--fixture` builds `GpuiOptions::fixture()` (`crates/freshkube-desktop/src/lib.rs`), which sets `Pilot.fixture`; no credentials or cluster are touched. Pages then resolve from example data instead of reading:

- Talos side: `crates/freshkube-desktop/src/fixture.rs` (the cluster and its nodes, and the hold knobs), plus `screens/<name>/example.rs` for each screen but etcd, whose example is in `screens/etcd/mod.rs`; `crates/freshkube-desktop/examples/fixture.rs` runs it standalone.
- Resources: `crates/freshkube-desktop/src/resources/example/` (objects, the acme workspace's Kargo and Argo CD kinds); `resources/shell/example.rs` (a local shell); `crates/freshkube-desktop/src/forwards/example.rs` (a loopback listener).
- Applications and changes: `crates/freshkube-desktop/src/applications/example.rs`; in delivery, `crates/freshkube-delivery/src/applications/example.rs`, `change/example.rs` and `changes/example.rs`. Its `fixtures.rs` is a fake cluster for tests only (`#[cfg(test)]`), not example data.
- Monitoring and Observability: `crates/freshkube-core/src/monitoring/example.rs`; `crates/freshkube-observability/src/example.rs`, `application/example.rs`, `logs/example.rs`.
- Maintenance mode: `crates/freshkube-desktop/src/maintenance/example.rs`.
- The shared components on invented data: `crates/freshkube-workbench/src/stories/`.

Example names are invented (acme, cluster-a/b/c, shop-api, `core-fra`-style entries); a test or capture must never need a real cluster.

## Where the debug knobs are parsed

Page, kind, theme, sidebar, dashboard, log search, motion and window size are parsed in one place; the rest are still read from the environment where each feature starts (#585 gathers them). Which builds honour them varies, so check the `cfg` at the read.

- Page, kind, theme, sidebar, dashboard, log search, motion and window size: `DebugOptions` in `crates/freshkube-desktop/src/debug_options.rs` (`FRESHKUBE_PAGE`, `_KIND`, `_THEME`, `_SIDEBAR`, `_DASHBOARD`, `_LOG_SEARCH`, `_MOTION`, `_WINDOW_SIZE`). `DebugOptions::from_env` reads them once, in `src/main.rs`, the stress binary and `examples/fixture.rs`, under `any(debug_assertions, feature = "stress")`; any other build gets the default. It travels through `GpuiOptions::with_debug`; `desktop/startup/mod.rs` and `desktop/mod.rs` read its fields and never the environment. `DebugOptions::parse` takes a lookup function, so its tests set no variables.
- Debug builds only: acting on `FRESHKUBE_LOG_SEARCH` (`startup/mod.rs`), `FRESHKUBE_PAGE=maintenance` (`desktop/mod.rs`, from the struct; its theme still read in `maintenance/example.rs`) and text size (`FRESHKUBE_TEXT_SIZE`, `crates/freshkube-ui/src/text_size.rs`).
- Fixture holds: `FRESHKUBE_FIXTURE_HOLD` is parsed only in `crates/freshkube-desktop/src/fixture.rs`. `Pilot::new` (`desktop/mod.rs`) passes it to the Monitoring and Observability pages through their `hold_examples`, and Applications reads `fixture::hold().applications`. The stress binary sets it (`crates/freshkube-desktop/src/bin/stress/main.rs`).
- Example shapes: `FRESHKUBE_ETCD` in `screens/etcd/mod.rs`; `FRESHKUBE_APPLICATIONS` in `crates/freshkube-desktop/src/applications/example.rs` (`Variant::from_env`), which maps to delivery's `Shape` in `crates/freshkube-delivery/src/change/example.rs`; delivery reads no environment for it.
- The app menu button: `crates/freshkube-desktop/src/desktop/app_menu/mod.rs` (`FRESHKUBE_APP_MENU`).
- First-frame timing: `first_frame::start()` in `crates/freshkube-probe/src/first_frame.rs`, called from `src/main.rs` (`FRESHKUBE_FIRST_FRAME`).
- The stress binary: `crates/freshkube-desktop/src/stress.rs` and `crates/freshkube-desktop/src/bin/stress/` (`FRESHKUBE_STRESS_*`).
- `FRESHKUBE_KUBECONFIG` (the default kubeconfig file): `crates/freshkube-core/src/resources/contexts.rs`.
- Outside the app: the workbench reads `FRESHKUBE_STORY`, `_THEME` and `_WINDOW_SIZE` (`crates/freshkube-workbench/src/lib.rs`); core's `examples/probe.rs` reads `FRESHKUBE_CONTEXT`, `_NAMESPACE` and `_VERSION`.

The command-line options are `src/main.rs`'s `Cli`, which builds a `GpuiOptions` (`crates/freshkube-desktop/src/lib.rs`).

## Feature docs

Behaviour that does not belong in the rules lives in one doc per feature, each starting with its module paths and the modules whose tests prove it (#587).

- [docs/features/applications.md](features/applications.md): the Applications area, the Changes list and the change page.
- [docs/features/resources.md](features/resources.md): the Resources page, the dock's logs, shells and ports, and a pod's cause card.
- [docs/features/nodes.md](features/nodes.md): the Nodes table, its node pane and their keys.
- [docs/features/shell.md](features/shell.md): the app frame, the Overview and Attention, and the Kubernetes summary session.
