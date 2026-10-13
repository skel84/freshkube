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
- **`freshkube-reads` holds the shared, generic read owners.** `Loader<T, I>` (`crates/freshkube-reads/src/loader/mod.rs`) and `Watch<T, I>` (`crates/freshkube-reads/src/watch/mod.rs`) need core's `Snapshot`, `OwnedJob` and `spawn_job`, a GPUI `Task` and Tokio, so the crate depends on core, `gpui-kit` and Tokio, and on nothing of `freshkube-ui` or desktop. Features that own their reads keep them: Monitoring (`crates/freshkube-monitoring/src/request.rs`), Observability (`crates/freshkube-observability/src/connection.rs`) and `LogView`.
- **Test-only edges are features or dev-dependencies.** The `testing` features and `freshkube-probe`'s `counting` are turned on only from `[dev-dependencies]` (desktop's, and for the feature crates their own), never from a normal dependency. Desktop also has a plain dev-dependency on `freshkube-graph`, for the map's tests.

Desktop reaches the moved code by its old paths in `crates/freshkube-desktop/src/lib.rs`: `crate::monitoring`, `crate::observability` and `crate::terminal` are `use … as …` lines, and `crate::palette`, `theme`, `text_size` and `meters` come from one `use freshkube_ui::{…}`. `crate::ui` is a module (`crates/freshkube-desktop/src/ui.rs`) that re-exports `freshkube_ui::ui::*` and adds desktop's own helpers, such as `health_tone`, the one map from a core `HealthIndicator` to a tag's tone and icon (`freshkube-ui` cannot name core's type). The delivery chain (Kargo, Argo CD, builds and pods) and the applications derived from it live in `crates/freshkube-delivery/` (`src/applications/`, `change/`, `changes/`, `entry/`), not in core. It sits between core and desktop/observability, depends on core and `talos-rs` only, and has no UI types. Its reads fail with core's `Failure`, whose kind is set where the read fails, and the words for refused, not served, unreachable and unfinished come from `freshkube_delivery::words`. Core's other cluster reads outside delivery are partly converted: the Diagnostics pod and CNI reads, Maintenance's readiness reads, and the Talos and talosconfig reads behind Security and Lifecycle carry `freshkube_core::failure::Failure`. They are built with `Failure::from_kube_text`, `from_talos_text` or `from_talos_formatted`, or with `Failure::new(from_kube(error).kind, …)` where the sentence is composed, so the words on screen stay each call's own text (`from_kube` and `from_talos` word a failure for a banner instead); each page takes `failure.message`. Still `String` are the parse and validation messages, the keychain's, and these kube and Talos reads, left for the step that types `KubeClientSource::client` and the selection runner's `connect`: Operations' `preflight_nodes`, `preflight_kubernetes` and `blocking_pdbs`, the monitoring marker reads (`monitoring/markers.rs`), Maintenance's readiness kubeconfig connect, and `KubeClientSource::client` itself, since desktop's access types word it before core sees it. Its `testing` feature (`freshkube_delivery::testing`, `crates/freshkube-delivery/src/lib.rs`) holds what desktop's tests read and is turned on from desktop's `[dev-dependencies]`.

## What a page is

There are three ways to draw a page on main, and one trait every page has a handle through. #579 is moving pages onto that trait; its first three steps are on main. #591 added `PageEvent`, `PageSource` and `PageEventFn` (`crates/freshkube-desktop/src/screens/page.rs`) and the matching `ScreenPanel`/`ScreenHandle` methods. #599 made the shell tell every page with a handle whether it shows (`set_visible`) and hear the entity pages through `ScreenHandle::subscribe`. Step 3 keeps every page's handle in one map, `Pilot.pages`, a `BTreeMap<PageId, ScreenHandle>` (`crates/freshkube-desktop/src/desktop/mod.rs`): the shell hands each page its source (`set_page_source`), tells it whether it shows, refreshes it, reads its status line and hears its events through that handle, in `PageId`'s order. `PageId`, which keys every page and lists the Talos screens in `PageId::SCREENS`, and the `Page`→`PageId` maps (`Page::id`, and `Page::screen` for the pages a screen draws) are in `crates/freshkube-desktop/src/desktop/pages.rs`. What still differs is how each kind of page draws and what it does with its hooks (below).

**1. A `ScreenPanel`** (`crates/freshkube-desktop/src/screens/mod.rs`). The trait has `set_source`, `activate`, `refresh`, `manual_refresh`, `focus`, `status`, `loading_motion`, `set_embedded`, `set_page_source`, `set_visible` (a no-op by default) and `subscribe_page`. A page makes itself with its own `new`, which the screen factory names; #599 took `new` out of the trait so that entities built another way can implement it. Nine Talos screens implement it: `EtcdScreen`, `WorkloadsScreen` (Health), `SecurityScreen`, `LifecycleScreen`, `OperationsScreen`, `ProcessesScreen`, `StorageScreen`, `NetworkScreen`, `DiagnosticsScreen`, each under `screens/<name>/`. They keep the default `set_visible`, which does nothing; #582 gives `Loader` its hook. They are retained in `Pilot.pages` under their `PageId`s. `ScreenHandle` is the type-erased wrapper that lets the shell call every page from one map. A screen's `set_page_source` is its `set_source` with the source's `ScreenSource`, by default; the shell sends it to every screen and `activate`/`refresh` only to the visible one (`Pilot::push_source`, `Pilot::active_screen`).

**2. An entity the shell owns directly.** Resources, Monitoring, Observability, Applications, Settings, System services and Overview implement `ScreenPanel` (`crates/freshkube-desktop/src/desktop/entity_pages.rs`). Resources, Monitoring, Observability, Applications and Settings take their part of the `PageSource` in `set_page_source`, which hands it to the entity's own `set_source` (a `KubeSource`, Monitoring's `ClusterSource` from `PageSource::cluster`, Observability's access from `PageSource::access`, Applications' entry too), and their `set_visible` calls the entity's own. `refresh` calls the entity's own on Resources, Monitoring, Observability and Applications; Settings' does nothing. `status` gives the line of Resources, Observability, Applications, Settings and System services; Monitoring has none. System services and Overview take no source and keep the default `set_visible`: the shell feeds System services its nodes and the Talos reading, and Overview its cards, its attention rows and its state. `subscribe_page` hears each one's events. Their `set_source` and `activate` do nothing.

| Page | Entity | Home |
| --- | --- | --- |
| Resources | `ResourcesScreen` | `crates/freshkube-desktop/src/resources/screen/` |
| Monitoring | `MonitoringPage` | `crates/freshkube-monitoring/src/page/` |
| Observability | `ObservabilityPage` | `crates/freshkube-observability/src/lib.rs` |
| Applications | `ApplicationsPage` | `crates/freshkube-desktop/src/applications/` |
| Settings | `SettingsPage`; only its Delivery sources list has `set_source` and `set_visible`, and it drops its check on hide | `crates/freshkube-desktop/src/desktop/settings/` (`sources/mod.rs`) |
| System services | `SystemServices`; its hooks do nothing but give its status line and hear `ServiceEvent`: the shell pushes the node roster's services with `set_nodes` (`desktop/target.rs`, `desktop/mod.rs`) and the read state with `set_reading`; the page makes no requests | `crates/freshkube-desktop/src/desktop/system_services/` |
| Overview | `OverviewPage`; its hooks do nothing but hear `OverviewEvent` (a card, an attention row's Open, Logs or Open node, a state's button), which the shell runs: `rebuild_joined_nodes` pushes the cards and attention rows with `set_display` and `set_attention`, and the shell's observer pushes `OverviewState` (where the Talos overview, the summary and the connection stand) after every notify, which redraws the page only when it changed. Show all is the page's own. The states and the stale banner are shared with the node pane's System services tab (`desktop/states.rs`) | `crates/freshkube-desktop/src/desktop/overview/` |

Resources, Monitoring, Observability and Applications cancel their reads on hide, and Settings drops its Delivery sources check, each from its `set_visible`. `Pilot::sync_visible` (`desktop/pages.rs`) tells the pages that show `true` and every other `false`, every page in the map in key order, whenever the page or the node pane's tab changes (`Pilot::navigate`, `sync_node_visibility` in `desktop/nodes/mod.rs`). A roster change that closes the pane or drops its tab has no window at hand, so `rebuild_joined_nodes` asks for it once its update ends (`Pilot::sync_visible_soon`). Visibility nests: a node pane's screen shows only while Nodes is up with its tab selected (`Pilot::shown_page`), and Nodes keeps showing around it, so both hear `true` (`Pilot::shown_pages`). Nothing else shows at once. Custom Resources (`resources/custom.rs`), the dock (`desktop/dock/`) and Command-K search (`desktop/search/`) are entities too, but are not pages: they sit beside whatever page shows.

**3. Drawn by the shell itself.** Nodes renders as a method on `Pilot` (`render_nodes` in `desktop/nodes/view.rs`); `PageHost` (`desktop/mod.rs`) wraps it in a view of its own so GPUI can reuse the last frame, and is the page's handle, whose hooks do nothing: the shell feeds what it draws. The node pane's System services tab is `NodeServices` (`desktop/services/`), a view of its own that Nodes draws: it owns the tab's filters, selection, focus and scroll, the shell hands it the services read (`set_reading`) and where the Talos overview and the node stand (`set_state`, redrawn only for a change), and it asks the shell through `NodeServicesEvent` for Logs, Restart, Refresh, the node's Pods and Overview. The read and the restart's ticket stay on the shell. The Nodes workspace (`crates/freshkube-desktop/src/desktop/nodes/`) also **embeds** four `ScreenPanel`s in its node pane (`NodeTab::screen`: Processes, Storage, Network, Diagnostics) and the Resources and log views, laid out by the pane's width (`screens::set_node_pane_width`), not the window's. Nodes holds the shell's Talos log view and clones of the four screens' handles, taken from `Pilot.pages` once it is built, only to draw them; the map stays where the screens hear the shell, and the shell still drives the log view.

Cached views, render probes and the reasons for them are in AGENTS.md ("Keep render cheap").

## The session and the source

The shell holds one cluster at a time. The pieces, from the user's choice down to what a page reads:

- **`Registry`** (`crates/freshkube-desktop/src/desktop/session.rs`) owns the active `ClusterSession` and the parked ones. A session keeps the Kubernetes summary, its reflector job and task, and the access it was collected under. Switching workspace entries parks the active session and keeps nothing running. `docs/ACCESS_IDENTITY.md` has the contract.
- **`AppliedConfig`** and **`Target`** (`crates/freshkube-desktop/src/backend.rs`): the applied talosconfig path and context, and a node request identity (epoch, context, node, address). Node selection advances the epoch; access replacement invalidates targets.
- **`ScreenSource`** (`screens/mod.rs`) is what a `ScreenPanel` is given: the `Target`, the node roster (`NodeSummary`) and, unless this is example data, a `LiveSource` (Talos client, cluster overview, collector). `Pilot::screen_source` builds it.
- **`KubeSource`** (`crates/freshkube-desktop/src/resources/screen/mod.rs`) is what the Kubernetes-facing entities are given: an `id` that covers the access but not the target node, the context's name, and a `KubeAccess`: `Example`, `Talos(Box<TalosAccess>)` (the client the Talos side sets up) or `Direct(DirectAccess)` (a kubeconfig context, for `--kubernetes-only`). `Pilot::kube_source` builds it.
- **`AccessIdentity`** (`crates/freshkube-core/src/client_cache.rs`, used by `session.rs`) names an access session plus the inspected configuration revision. With Talos access `KubeSource.id` is its key; kubernetes-only uses `DirectAccess::id()` and the fixture `resources::example::connection(context)`. Monitoring's `ClusterSource` and Observability's access string are derived from it by `PageSource::cluster` and `PageSource::access` (`crates/freshkube-desktop/src/screens/page.rs`), which `Pilot::push_source` hands every page.

`Pilot::push_source` is the single fan-out. It builds one `PageSource` (`Pilot::page_source`) and hands it to every page in the map in two passes, the Talos screens first and the other pages last, in key order; between them search, Custom Resources, the node's Pods list and the dock, which sit beside the pages, get the `KubeSource`. A page compares the new source with the old one and drops its data and jobs when the identity changed.

The kubeconfig-only mode (`desktop/kubernetes_only.rs`) and the fixture mode (`--fixture`, below) are not separate shells: they change what `kube_source` and `screen_source` return.

## Reads: `Snapshot`, `Loader` and the exceptions

- **`Snapshot<T, I>` and `Request<I>`** (`crates/freshkube-core/src/snapshot.rs`) keep the last good value for one identity and refuse an answer for a superseded request. Desktop names it `state::Snapshot<T, I = AppliedConfig>` (`crates/freshkube-desktop/src/state.rs`). The transitions are tabulated in `docs/STATE_LIFETIMES.md`.
- **`OwnedJob` and `spawn_job`** (`crates/freshkube-core/src/job.rs`; re-exported by `crates/freshkube-desktop/src/backend.rs`) run Tokio work under a deadline; dropping an `OwnedJob` aborts it. The GPUI delivery `Task` that applies the answer is kept beside it, and the answer is checked for identity and generation before publishing.
- **`Loader<T, I>`** (`crates/freshkube-reads/src/loader/mod.rs`) is a `Snapshot`, an `OwnedJob` and a `Task` together: `load`, `resolve` (fixtures), `reset`, `data`, `error`, `is_loading`, `last_successful`. The nine Talos screens use desktop's wrapper `screens::Loader<T>` (`crates/freshkube-desktop/src/screens/loader.rs`), which fixes the identity to `Target` and the deadline to `SCREEN_DEADLINE`. `set_visible(visible, OnShow, cx)` (`crates/freshkube-reads/src/visibility.rs`) drops the read on hide and keeps the data as last known; on show it returns whether to read again: `OnShow::ReadIfOlder(threshold)` past that age of the last successful answer, or `OnShow::Never`.
- **`Watch<T, I>`** (`crates/freshkube-reads/src/watch/mod.rs`) is the same for a stream: `start` runs Tokio work that sends into a channel, and a GPUI task hands the owner what arrived in batches paced by `Pace` (`capacity`, `most`, `every`), each checked for identity and generation; `stop`, `is_running`, `revision` and `set_visible`, which stops a running stream on hide and asks for a new start on show. No owner uses it yet; Resources' list watch is the first to move onto it (#582).
- **Features with their own owners** keep them: Monitoring and Observability hold read/delivery pairs and a generation of their own; `LogView<S>` (`crates/freshkube-logs/`) owns its source's stream; Resources owns its list, watch and detail reads. `docs/STATE_LIFETIMES.md` lists each owner.

**The two exceptions to "a page reads only while it is shown":**

1. **Work the user or the session started, which runs while hidden.** The cluster summary session (nine reflector stores, `ClusterSession.summary_*` in `session.rs`, `crates/freshkube-desktop/src/desktop/kubernetes_summary/`); each dock log tab (`crates/freshkube-desktop/src/desktop/dock/`, `feed.rs`); a pod shell (`crates/freshkube-desktop/src/resources/shell/`); a port forward (`crates/freshkube-desktop/src/forwards/`). A log tab ends when closed or on a context, kubeconfig or talosconfig change; a running shell does too, after the shell asks (`shell::unless_shell`); a forward ends only on Stop and runs on through page, context and kubeconfig changes (`forwards/mod.rs`, `desktop/switch.rs`).
2. **Operations mutations.** A submitted run holds the app-wide operation ticket in a detached delivery task until core's runner ends (`crates/freshkube-core/src/operations/`, `crates/freshkube-desktop/src/mutation.rs`); it is never an ordinary `OwnedJob` and navigation does not cancel it.

Everything else is meant to read only while shown. The entity pages drop their reads from `set_visible`; the Talos screens do not yet drop an in-flight read on hide (#582): `Loader::set_visible` is the hook, and no screen calls it yet.

## How a page reaches the shell

A page never holds the shell. It emits a typed event, and the shell subscribes when it builds the page.

- **Screens** emit `ScreenEvent` (`RefreshSummary`, `Back`, `OpenLogs`, `SelectNode`, `RetryCluster`, `OpenLogsOn`; `screens/mod.rs`), which reaches `Pilot::page_event` as `PageEvent::Screen` and goes on to `Pilot::screen_event` (`desktop/mod.rs`).
- **Entity pages** emit their own types: `ResourceLink` (Resources, Applications), `RevisionLink` and `SwitchLink` (Applications), `NodePodsEvent` (the node's Pods list), `DockEvent` (the dock), `ObservabilityEvent`, `MonitoringEvent` and `NotServed` (Resources). System services emits `ServiceEvent` (`desktop/system_services/mod.rs`) and Overview `OverviewEvent` (`desktop/overview/mod.rs`), which reach `page_event` as `PageEvent::Service` and `PageEvent::Overview`. The shell hears every page in the map through its `ScreenHandle::subscribe` (`Pilot::subscribe_pages`, `desktop/entity_pages.rs`), with a `WeakEntity<Pilot>` trampoline into `Pilot::page_event`. It subscribes in `Pilot::new` before any page emits, since GPUI activates a subscription only after the current update. The node's Pods list and the dock, which aren't pages, are subscribed in `Pilot::subscribe_page_events` (`desktop/mod.rs`), which wraps their events in `PageEvent` for the same `page_event`. The node pane's System services tab, which isn't a page either, emits `NodeServicesEvent` (`desktop/services/mod.rs`); `Pilot::new` subscribes to it directly and runs it in `Pilot::node_services_event`.
- **Object links** all end in `Pilot::open_object` (`desktop/pages.rs`) through `Pilot::resource_link` (`crates/freshkube-desktop/src/desktop/object_links/`); a link to a cluster that is not open is refused with a notice, and one to another workspace entry goes through `Pilot::open_on_entry` (`desktop/switch.rs`); `open_object` can also divert a link from another entry's ended connection (`divert_link`, `switch.rs`).
- **Redraws** use `cx.observe`, among others on the dock and Custom Resources, which notify the shell.

There are about a dozen event types. `PageEvent` (`screens/page.rs`) wraps all of them, and `Pilot::page_event` handles every variant. Every page in `Pilot.pages` is heard through `ScreenHandle::subscribe`; the node's Pods list and the dock through `cx.subscribe_in`.

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

Every desktop knob is parsed in `DebugOptions`, and the build that honours it is decided in `DebugOptions::from_env`. What stays outside is the probe's first-frame switch, stress's own options, `FRESHKUBE_KUBECONFIG` in core and the workbench's own.

- Page, kind, theme, sidebar, dashboard, log search, motion and window size, the etcd and Applications example shapes, the fixture holds, the app menu button and the text size: `DebugOptions` in `crates/freshkube-desktop/src/debug_options.rs` (`FRESHKUBE_PAGE`, `_KIND`, `_THEME`, `_SIDEBAR`, `_DASHBOARD`, `_LOG_SEARCH`, `_MOTION`, `_WINDOW_SIZE`, `_ETCD`, `_APPLICATIONS`, `_FIXTURE_HOLD`, `_APP_MENU`, `_TEXT_SIZE`). `DebugOptions::from_env` reads them once, in `src/main.rs`, the stress binary and `examples/fixture.rs`, under `any(debug_assertions, feature = "stress")`; any other build gets the default (`_ETCD`, `_APP_MENU` and `_TEXT_SIZE` were only ever honoured by debug builds, so `from_env` clears them outside `debug_assertions`). It travels through `GpuiOptions::with_debug`; `desktop/startup/mod.rs` and `desktop/mod.rs` read its fields and never the environment; the pages are handed their value by `Pilot::new`. `DebugOptions::parse` takes a lookup function, so its tests set no variables.
- Debug builds only: acting on `FRESHKUBE_LOG_SEARCH` (`startup/mod.rs`) and `FRESHKUBE_PAGE=maintenance` (`desktop/mod.rs`, which hands the maintenance example the struct's theme).
- Fixture holds: `fixture::Hold::parse` turns `FRESHKUBE_FIXTURE_HOLD` into `DebugOptions::hold`. `Pilot::new` (`desktop/mod.rs`) passes it to the Monitoring and Observability pages through their `hold_examples`, to Resources through `ResourcesScreen::holding`, to Applications and the Changes list through their constructors, and to the Talos pages as `fixture_hold`; `GpuiOptions::holding_talos` (tests) holds the Talos pages beside it. The stress binary sets it for its child (`crates/freshkube-desktop/src/bin/stress/main.rs`).
- Example shapes: `FRESHKUBE_ETCD` is `DebugOptions::etcd`, handed to `EtcdScreen::with_example` (`screens/etcd/mod.rs`); `FRESHKUBE_APPLICATIONS` is `DebugOptions::applications` (`applications/example.rs`'s `Variant::parse`), handed to `ApplicationsPage::new`, which maps it to delivery's `Shape` (`crates/freshkube-delivery/src/change/example.rs`) for the Changes list; delivery reads no environment for it.
- The app menu button: `DebugOptions::app_menu_button` (`FRESHKUBE_APP_MENU`), read by `Pilot::new` into `app_menu::menu_platform` (`crates/freshkube-desktop/src/desktop/app_menu/mod.rs`). The text size: `DebugOptions::text_size` (`FRESHKUBE_TEXT_SIZE`), passed to `freshkube_ui::text_size::install_with` (`crates/freshkube-ui/src/text_size.rs`, which reads no environment; its `parse` keeps one of `STEPS`).
- First-frame timing: `first_frame::start()` in `crates/freshkube-probe/src/first_frame.rs`, called from `src/main.rs` (`FRESHKUBE_FIRST_FRAME`).
- The stress binary: `crates/freshkube-desktop/src/stress.rs` and `crates/freshkube-desktop/src/bin/stress/` (`FRESHKUBE_STRESS_*`).
- `FRESHKUBE_KUBECONFIG` (the default kubeconfig file): `crates/freshkube-core/src/resources/contexts.rs`.
- Outside the app: the workbench reads `FRESHKUBE_STORY`, `_THEME`, `_WINDOW_SIZE` and `_TEXT_SIZE` (`crates/freshkube-workbench/src/lib.rs`); core's `examples/probe.rs` reads `FRESHKUBE_CONTEXT`, `_NAMESPACE` and `_VERSION`.

The command-line options are `src/main.rs`'s `Cli`, which builds a `GpuiOptions` (`crates/freshkube-desktop/src/lib.rs`).

## Feature docs

Behaviour that does not belong in the rules lives in one doc per feature, each starting with its module paths and the modules whose tests prove it (#587).

- [docs/features/applications.md](features/applications.md): the Applications area, the Changes list and the change page.
- [docs/features/resources.md](features/resources.md): the Resources page, the dock's logs, shells and ports, and a pod's cause card.
- [docs/features/nodes.md](features/nodes.md): the Nodes table, its node pane and their keys.
- [docs/features/shell.md](features/shell.md): the app frame, the Overview and Attention, and the Kubernetes summary session.
