# Freshkube agent guide

Freshkube is a native desktop app, built on GPUI Kit, for Talos Linux and Kubernetes clusters. It began as a fork of [talos-pilot](https://github.com/Handfish/talos-pilot) by Ken Udovic; that project's TUI, egui and Dioxus frontends were removed, and its GPUI frontend is now the application.

## Crates

```
crates/
├── talos-rs/            Talos gRPC client
├── freshkube-core/      domain logic shared by every screen, no UI types
└── freshkube-desktop/   the GPUI Kit application (shell, screens, logs, theme)
src/main.rs              the `freshkube` binary: CLI options → desktop app
```

Keep cluster logic in `freshkube-core` and presentation in `freshkube-desktop`. A screen calls core functions with real Rust types; there is no serialization boundary.

### Module layout

Split code by concern, not by line count. A long file with one tight concern is fine; a file whose parts share private state only because they sit together is not.

- **Turn a module into a directory when it mixes concerns,** or when its code, not counting tests, passes about 1,500 lines. Use `foo/mod.rs` with child modules, as `desktop/`, `resources/` and `logs/` do; don't mix in the `foo.rs` + `foo/` style.
- **Keep the shared type in `mod.rs` and spread its `impl` blocks over the children.** A child module sees its ancestors' private items, so `logs/view.rs` and `logs/measure.rs` use `LogView`'s fields without widening them. A method one child calls from another needs `pub(super)`; sibling modules don't see each other's private items.
- **Every log view is `LogView<S: LogSource>`** in `logs/`. It owns retention, search, selection, copy, the level filter, follow, wrap and row measurement; a source (`TalosLogs` in `logs/talos.rs`, `PodLogs` in `logs/pod/`) supplies its controls, empty message and per-stream errors, and feeds lines through `ingest`. A new kind of log adds a source; it never copies the view.
- **Keep small unit tests inline** in `#[cfg(test)] mod tests { … }`. When a module's tests are large, as UI tests usually are, put them in a sibling `tests.rs` (`#[cfg(test)] mod tests;`). It stays a child module, so the tests keep their access to private fields.
- **Keep functions short.** A `render` that runs to hundreds of lines is harder to follow than a long file; split it into `render_*` helpers.
- **Split a file when a step works on it,** not in a sweeping pass. Make the split its own commit with no logic changes, so the unchanged tests prove it.

[docs/ROADMAP.md](docs/ROADMAP.md) sets the order of work and its ground rules. When a step lands, move it to Done with its commit in the same change. [docs/REFERENCES.md](docs/REFERENCES.md) maps the reference checkouts and the source-reuse policy.

## Build, run and test

`talos-rs` generates gRPC code with `protoc`. Install it (`brew install protobuf`) or point `PROTOC` at a binary.

```sh
cargo build
cargo run -- --fixture            # synthetic example data, no credentials or cluster
cargo run -- --config ~/.talos/config --context <name>
cargo run -- --kubernetes-only --kubeconfig <file> --kube-context <name>   # no talosconfig
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Debug builds open on a page from `FRESHKUBE_PAGE=<slug>` (`overview`, `services`, `logs`, `processes`, `storage`, `network`, `diagnostics`, `etcd`, `workloads`, `security`, `lifecycle`, `resources`, `operations`). `FRESHKUBE_KIND=<key>` opens a Kubernetes kind on the Resources page by its kubectl key (`pods`, `deployments.apps`, `nodes`, …; see `resources/navigation.rs`). With `--fixture` it also takes the example custom kinds, such as `certificates.cert-manager.io`. Use them for screenshots instead of driving the window from outside (see [Visual checks](#visual-checks)).

## Cluster safety

- Read-only by default. Checks against a real cluster list, watch and read; they never create, change or delete resources.
- Never run Operations or maintenance actions (drain, reboot, upgrade, apply config, reset) during verification unless the user asked for that specific action on a node they named as disposable.
- Use the context the user chose. Never fall back to whatever kubeconfig or talosconfig context is current.
- Never print or commit credentials: talosconfig and kubeconfig contents, tokens, Secret values, keys. Redact them in logs, screenshots and evidence.

## Working with GPUI

GPUI suits Rust well: views are entities changed through `update`, `cx.notify()`, `observe` and `subscribe`, closures hold weak handles, and every frame rebuilds the element tree from current state. These rules cover where it bites. Read GPUI Kit's own [coding](https://gpui-kit.com/docs/coding-guides.md) and [design](https://gpui-kit.com/docs/design-guides.md) guides before UI work.

### Keep render cheap

- `render` runs on every frame for every dirty view, and a notify redraws the whole window; even a hover does. Never sort, filter, group or format collections in `render`. Derive display data when the data changes and keep it on the entity.
- Caching is opt-in. Use `.cached()` views or row caches keyed by a data revision for anything heavy, and give each cache a clear owner that invalidates it.
- Don't run app-wide timers that notify a large view. Give periodic updates (clocks, "updated 5 s ago", ages) to the smallest entity that shows them. A 1-second timer on the shell once redrew everything and made the app feel slow.
- Render only the rows a frame can show. Use `uniform_list` for fixed-height rows. For rows whose height depends on the width, use `VirtualList` with heights measured by layout, never computed. A plain list needs a hard cap. [docs/LONG_LISTS.md](docs/LONG_LISTS.md) has the rules and the state of each list.
- Keep `[profile.dev.package."*"] opt-level = 3` in the workspace manifest. Unoptimised GPUI draws a frame about 14× slower, so debug builds without it give a false picture of performance.
- Measure before and after a change that could cost time. `scripts/stress.sh` runs list, watch-burst and log-flood workloads on a release build against a synthetic API and prints per-span timings, CPU and memory; [docs/PERFORMANCE.md](docs/PERFORMANCE.md) has the workloads and the numbers so far.

### Async: two executors

GPUI has its own executor; tonic and kube need Tokio. The binary owns the Tokio runtime and passes its handle to the app.

- Run cluster I/O on Tokio, send the result back to a GPUI task, and update entities there. Never block the UI thread.
- Tie each request to its target (the epoch in `Target`) and keep its handle (`OwnedJob`) on the entity that needs the result. Replacing or dropping the handle cancels the work; a result for a target that is no longer current is ignored.
- Hidden screens never contact the cluster.

### Learning the API

Documentation is thin. Before using an API, read the source of the pinned versions in the Cargo registry (`~/.cargo/registry/src/*/gpui-kit-0.7.0`, `gpui-component-0.7.0`, `gpui-base-0.7.0`, `gpui-pre-*`); don't guess from older GPUI examples. Expect odd shapes, such as `window.root::<Root>()` returning `Option<Option<Entity<Root>>>`, and breaking changes whenever the pinned pre-1.0 versions move.

### Keys and focus

- When the focused element isn't drawn, key dispatch starts from the window root and the page's bindings stop working. Give a page's key context to a wrapper that is drawn in every state, not to a list that a placeholder replaces.
- macOS reports Command-Shift-] as `}` with Command alone, so bind `secondary-}`, not `secondary-shift-]`. Command-Shift with a letter keeps Shift: `secondary-shift-g`.
- A deeper context's binding wins. A handler that calls `cx.propagate()` lets the key reach raw listeners, which is how Enter still presses a focused button.
- `FocusHandle::dispatch_action` runs at once on that node; `window.dispatch_action` is deferred. A Kit dialog remembers what had focus when it opens, so focus its content after `open_dialog`.

### UI tests

Headless UI tests render the real app, find elements by id and click or type into them; the suite runs in seconds. Prefer them over manual checks, and give every interactive element a stable, domain-based id.

- Dialogs animate on the real clock, and advancing the test clock does not finish them. The shared test setup in `desktop/tests.rs` turns on reduced motion (`cx.set_reduce_motion(true)`); two confirmation-dialog tests failed every run without it. Do the same in any new test harness.
- `window.render_frame` bypasses view caches. Don't use it to prove that a view was or was not redrawn; use the render probes (`probe::hit`, `probe::count`).
- Element snapshots can't tell whether a button is disabled. Assert the outcome instead: click it and check that nothing changed.
- Setting an input's value from code does not emit its change event. Type into it with real input events (`window.input`) when the change handler matters.
- Tests write only under a fresh temporary directory, never into the crate or the user's home.
- Time anything a test depends on with the executor's clock (`cx.background_executor().now()`), not `Instant::now()`, so tests step it with `advance_clock` instead of sleeping. Example data dated from the wall clock must not be generated again for a stream already read: a second later it passes for new lines. Both made tests fail on a loaded machine.

### Visual checks

macOS ignores synthetic keystrokes once the window loses focus, so driving the running app from outside is unreliable. Open the page you need with `FRESHKUBE_PAGE`, capture it, and keep interaction checks in UI tests.

GPUI stops drawing a window that is covered or on a locked screen. Such a capture shows the first frames only, before any read finishes. Fixture pages still look right because their data is there at once. Live pages don't, so check that the screen is unlocked before trusting a capture of one.

## Talos Linux Reference

### Network Ports

| Port | Protocol | Service | Used By |
|------|----------|---------|---------|
| 50000 | TCP | apid (Talos API) | talosctl, control plane nodes |
| 50001 | TCP | trustd | Worker nodes for TLS certs |
| 6443 | TCP | kube-apiserver | kubectl, kubelets |
| 2379 | TCP | etcd client | kube-apiserver |
| 2380 | TCP | etcd peer | etcd cluster members |
| 10250 | TCP | kubelet | kube-apiserver |
| 10259 | TCP | kube-scheduler | Health checks |
| 10257 | TCP | kube-controller-manager | Health checks |

### Talos Machine API Methods

Key gRPC methods we use (all in `MachineService`):

| Method | Description |
|--------|-------------|
| `Version` | Get Talos version |
| `ServiceList` | List all services |
| `ServiceRestart` | Restart a service |
| `Logs` | Stream service logs |
| `Memory` | Memory usage |
| `LoadAvg` | CPU load averages |
| `CPUInfo` | CPU info |
| `Processes` | Process list |
| `NetworkDeviceStats` | Network interface stats |
| `Netstat` | Network connections |
| `Read` | Read file from node |
| `Dmesg` | Kernel ring buffer |
| `EtcdStatus` | etcd member status |
| `EtcdMemberList` | etcd cluster members |
| `EtcdAlarmList` | etcd alarms |
| `Kubeconfig` | Get kubeconfig |
| `ApplyConfiguration` | Apply config patch |
| `PacketCapture` | Capture packets (pcap) |
| `Reboot` | Reboot node |
| `Shutdown` | Shutdown node |

### COSI Resource API - NOT EXTERNALLY ACCESSIBLE

> **IMPORTANT:** The COSI State API is an **internal service** and is **NOT exposed** through port 50000.

**Do NOT attempt to implement COSI gRPC client code** - it will fail with `PermissionDenied`. Use `talosctl get` as a subprocess instead.

---

## Core Philosophies

### 1. State Over Logs

**The most important principle for diagnostics:**

> Check actual system state, not log messages. Logs are history; APIs and files are truth.

```rust
// GOOD: Direct state check
let healthy = client.read_file("/run/flannel/subnet.env").await.is_ok();

// GOOD: K8s API query for current state
let pods = kube_client.list::<Pod>(&params).await?;

// BAD: Log parsing for health determination
let logs = client.logs("kubelet", 100).await?;
let healthy = !logs.contains("error");  // DON'T DO THIS
```

### 2. Reliability Hierarchy

When implementing any check, prefer data sources in this order:

| Tier | Source Type | Example | Reliability |
|------|-------------|---------|-------------|
| 1 | File/procfs state | `/run/flannel/subnet.env` | Highest |
| 2 | API responses | Talos API, K8s API | High |
| 3 | Log parsing | kubelet logs | Last resort |

### 3. Graceful Degradation

When data sources are unavailable, degrade gracefully:

```rust
// Good: Show unknown state, don't crash
match client.read_file("/some/path").await {
    Ok(content) => DiagnosticCheck::pass(...),
    Err(_) => DiagnosticCheck::unknown("check_id", "Check Name"),
}
```

### 4. No False Positives

A diagnostic showing failure for a healthy system is worse than showing unknown.

---

## Code Patterns

### Using AsyncState<T>

Data-holding state uses `AsyncState<T>` for consistent loading/error handling:

```rust
pub struct MyComponent {
    state: AsyncState<MyData>,
    // ... UI state (selection, scroll, etc.)
}

#[derive(Debug, Clone, Default)]
pub struct MyData {
    // All async-loaded data goes here
}

impl MyComponent {
    pub async fn refresh(&mut self, client: &TalosClient) -> Result<()> {
        self.state.start_loading();

        match load_data(client).await {
            Ok(data) => self.state.set_data(data),
            Err(e) => self.state.set_error_with_retry(format_talos_error(&e)),
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        if self.state.is_loading() && !self.state.has_data() {
            // Show loading spinner
        } else if let Some(error) = self.state.error() {
            // Show error message
        } else if let Some(data) = self.state.data() {
            // Render data
        }
    }
}
```

### Using HasHealth Trait

Implement `HasHealth` for health-related enums in core; the desktop app maps health to a `ui::Tone` for tags and colours:

```rust
// In core
impl HasHealth for MyStatus {
    fn health(&self) -> HealthIndicator {
        match self {
            MyStatus::Good => HealthIndicator::Healthy,
            MyStatus::Bad => HealthIndicator::Error,
        }
    }
}

// In freshkube-desktop - a presentation::Health maps to a tone and icon for a tag
let (tone, icon) = ui::health_tone(health);
let tag = ui::tag(tone, Some(icon), "Healthy", cx);
```

### Diagnostic Checks

```rust
// Creating checks
DiagnosticCheck::pass("memory", "Memory", "2.1 GB / 4.0 GB (52%)")
DiagnosticCheck::fail("cni", "CNI", "Not initialized", Some(fix))
DiagnosticCheck::warn("cpu_load", "CPU Load", "High load: 4.5")
DiagnosticCheck::unknown("etcd", "Etcd")  // When data unavailable
```

---

## Kubernetes resources

`resources/` browses Kubernetes kinds on one `Page::Resources`; the sidebar's KUBERNETES groups pick the kind. It is not a `ScreenPanel`: it doesn't follow the target node, the shell owns it directly and tells it when it is visible, and only a visible page lists and watches. Rows are the server-printed table (wide columns hidden, Age computed locally) and are selected by identity (connection, resource, namespace, name, UID), never by position. The shell's automatic refresh never restarts the watch; only Refresh lists again.

The sidebar's Custom Resources section is `resources/custom.rs`, an entity the shell owns. It discovers the API groups when the section opens and a group's kinds when the group opens, keeps what it found until the connection changes or the user retries, and reads nothing while closed. It derives the rows' labels, ids and tooltips when an answer arrives, so the sidebar's `render` only reads them. A failed group shows why, never an empty group. When the page's kind answers 404, the screen shows it as not served and emits `NotServed`, and the group is discovered again.

Selecting a row opens the detail pane (`resources/pane/`, with its model in `resources/detail.rs`), a cached view in a resizable split beside or below the list. It reads the full object itself and watches its events; it doesn't watch the object. The list is its source of truth: the screen passes on each new `resourceVersion` the list sees, which triggers at most one read a second, and tells the pane when the row is gone. Every answer carries the request's sequence number and identity, and a stale one is dropped. Secret values never reach the document: `reveal_secret_value` reads one key again, and every revealed value is forgotten when the page hides or a new version arrives.

A pod's pane adds a Logs tab, `LogView<PodLogs>` (`logs/pod/`), fed by `follow_pod_log` in core. Nothing is read until the tab first shows for that pod; then one container's stream lives while the pod stays open, on any tab, and is dropped when another object opens, the pane closes or the page hides. Showing the page again reads on from the last line, never repeating one. The previous instance is only ever read when the user asks for it. Example mode applies a container's lines at once and writes on with a background timer, so UI tests advance the clock to see new lines.

The keyboard follows one path through the page, and each level owns a key context: `KubeResources` on the list (always drawn, so the page keeps its keys while it shows no rows), `KubeResourcesFilter`, `KubeDetail` on the pane, `KubeDetailTabs` on its tab strip, and `LogPanel`, `LogSearch` and `LogView` inside the Logs tab. Enter opens the selected row and moves the keyboard into the pane. Escape steps back one level at a time: a search clears and then leaves, the pane hands the keyboard to the list with the pane left open, and the list clears its filter and then closes the pane. Whatever takes the keyboard away (a new context, the namespace picker, the filter) hands it back to the list. Command-F, Command-G and Command-A act on what has focus. Command-K (`desktop/kind_switcher.rs`) is a Kit `Command` palette in a dialog that opens any kind.

## Adding a screen

1. Add the page to `Page` (`desktop/pages.rs`): `ALL`, `SCREENS` if it is a `ScreenPanel`, its slug, sidebar entry and shortcut.
2. Implement `ScreenPanel` in `screens/<name>.rs`. The screen owns its requests (`OwnedJob`), its data and its offline example data.
3. Put cluster logic in `freshkube-core` and keep the screen to presentation.
4. Add UI tests for loading, empty, failure and the main interactions.

Diagnostic checks follow the reliability rules below: find the source of truth first, use the `DiagnosticCheck` constructors, provide an actionable fix where possible, and return `unknown` rather than failing when data is unavailable.

## Quick reference

### Do

- Check actual system state (files, APIs)
- Use `AsyncState<T>` for loaded data
- Implement `HasHealth` for health enums
- Provide actionable fix suggestions
- Degrade gracefully when sources are unavailable
- Add tests for new core functionality and UI tests for new screens
- Use constants from `freshkube_core::constants`

### Don't

- Parse logs to determine health status
- Use string matching without timestamp validation
- Crash when data sources are unavailable
- Show "failed" when "unknown" is more accurate
- Use a VIP for the Talos API endpoint (it fails when etcd is down)
- Duplicate health indicator logic (use the `HasHealth` trait)
- Compute display data in `render`

### Reliability Checklist for New Checks

- [ ] What is the source of truth for this state?
- [ ] Can we check that source directly (file, API)?
- [ ] If using logs, do we validate timestamps?
- [ ] What happens if the data source is unavailable?
- [ ] Can this check produce false positives?
- [ ] Is the failure mode graceful?

---

## Testing

```sh
cargo test --workspace
```

### Before Merging

1. **No false positives:** Create error condition, fix it, verify check shows healthy
2. **Graceful degradation:** Disconnect API, verify no crashes
3. **Stale log immunity:** Ensure old log messages don't affect current state
4. **Zero warnings:** `cargo clippy --workspace --all-targets -- -D warnings` should pass clean
5. **Formatting:** `cargo fmt --all -- --check` should pass

---
