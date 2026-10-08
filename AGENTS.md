# Freshkube agent guide

Freshkube is a native desktop app, built on GPUI Kit, for Talos Linux and Kubernetes clusters. It began as a fork of [talos-pilot](https://github.com/Handfish/talos-pilot) by Ken Udovic; that project's TUI, egui and Dioxus frontends were removed, and its GPUI frontend is now the application.

## Crates

```
crates/
├── talos-rs/            Talos gRPC client
├── freshkube-core/      domain logic shared by every screen, no UI types
├── freshkube-probe/     render probes for UI tests and timing spans for the stress binary
├── freshkube-logs/      the log view, LogView<S: LogSource>: retention, search, selection, follow, wrap, measured rows
├── freshkube-terminal/  the terminal view behind the pod shell: an alacritty_terminal grid drawn with GPUI
├── freshkube-ui/        the look: theme, palette, text size, ui helpers, page frame, table and graph view
├── freshkube-graph/     graph layout without GPUI: columns of boxes and the curves between them
├── freshkube-monitoring/ Grafana dashboards from the cluster's Prometheus, and the CPU and memory history charts
├── freshkube-observability/ Coroot's applications, service map, incidents, traces, profiles and logs
├── freshkube-workbench/ the shared components on invented data, one story at a time; never shipped
└── freshkube-desktop/   the GPUI Kit application (shell, screens, the log sources)
src/main.rs              the `freshkube` binary: CLI options → desktop app
```

Keep cluster logic in `freshkube-core` and presentation in `freshkube-desktop`. A screen calls core functions with real Rust types; there is no serialization boundary. Shared components live in `freshkube-ui`, which depends on neither; desktop reaches its modules by their old paths (`crate::ui`, `crate::palette`, `crate::theme`, `crate::text_size`, `crate::meters`). Monitoring is `freshkube-monitoring`, which desktop reaches as `crate::monitoring`; it sits under desktop and uses only core, `talos-rs`, `freshkube-ui` and `freshkube-probe`, so Resources and Observability depend on it, never the other way. Observability is `freshkube-observability`, which desktop reaches as `crate::observability`; it uses core, `freshkube-ui`, `freshkube-logs`, `freshkube-graph` and `freshkube-monitoring`, and names nothing of desktop: the shell turns its `ObservabilityEvent`s into navigation, and desktop's tests reach its fixture links through its `testing` feature. A new page draws with `freshkube_ui::page` and `freshkube_ui::table` ([DESIGN.md](docs/DESIGN.md#in-code)).

### Module layout

Split code by concern, not by line count. A long file with one tight concern is fine; a file whose parts share private state only because they sit together is not.

- **Turn a module into a directory when it mixes concerns,** or when its code, not counting tests, passes about 1,500 lines. Use `foo/mod.rs` with child modules, as `desktop/`, `resources/` and `logs/` do; don't mix in the `foo.rs` + `foo/` style.
- **Keep the shared type in `mod.rs` and spread its `impl` blocks over the children.** A child module sees its ancestors' private items, so `freshkube-logs`'s `view.rs` and `measure.rs` use `LogView`'s fields without widening them. A method one child calls from another needs `pub(super)`; sibling modules don't see each other's private items.
- **Every log view is `LogView<S: LogSource>`** from `freshkube-logs`. It owns retention, search, selection, copy, the level filter, follow, wrap and row measurement; a source (`TalosLogs` in desktop's `logs/talos/`, `PodLogs` in `logs/pod/`) supplies its toolbar tools and the note under them (`tools`, `notes`), its empty message and per-stream errors, and feeds lines through `ingest`. A source reaches the view only through the crate's `source_api.rs`; the view's other fields stay private to it, so a source's own methods live in an extension trait (`TalosPanel`, `PodLogPanel`) that callers import, and its stream methods on a trait private to `logs/`. Desktop's tests read the view through the crate's `testing` feature, turned on only from dev-dependencies. A new kind of log adds a source; it never copies the view.
- **Keep small unit tests inline** in `#[cfg(test)] mod tests { … }`. When a module's tests are large, as UI tests usually are, put them in a sibling `tests.rs` (`#[cfg(test)] mod tests;`). It stays a child module, so the tests keep their access to private fields.
- **Keep functions short.** A `render` that runs to hundreds of lines is harder to follow than a long file; split it into `render_*` helpers.
- **Split a file when a step works on it,** not in a sweeping pass. Make the split its own commit with no logic changes, so the unchanged tests prove it.

[docs/ROADMAP.md](docs/ROADMAP.md) sets the order of work and its ground rules. When a step lands, move it to Done with its commit in the same change. [docs/REFERENCES.md](docs/REFERENCES.md) maps the reference checkouts and the source-reuse policy. [docs/DESIGN.md](docs/DESIGN.md) holds the chosen look: tokens, type, status glyphs, the app frame and table rules.

Each PR adds a release-note bullet in `changelog.d/<section>/<slug>.md`; never edit `CHANGELOG.md`'s Unreleased entries in a feature PR. Use `coroot` or `other-changes`, give the file a unique slug, and run `scripts/changelog.sh check`. Only release preparation collects the fragments into the changelog. [changelog.d/README.md](changelog.d/README.md) describes the format and adding a section.

## Build, run and test

`talos-rs` generates gRPC code with `protoc`. Install it (`brew install protobuf`) or point `PROTOC` at a binary.

Each worktree builds into its own `target/`; leave `CARGO_TARGET_DIR` unset. Don't share one build directory between worktrees: Cargo fingerprints workspace crates by paths relative to the crate and judges freshness by modification time, so a worktree whose sources are older than another worktree's build links that other branch's code without rebuilding it. The first build in a new worktree compiles the dependencies.

On macOS a dev build keeps its object files in `target/debug/deps`, since the debugger and backtraces read line tables from them. Each incremental rebuild of a workspace crate writes a new set, one `.o` per codegen unit, and leaves the old ones, so a busy worktree's `target/` grows by gigabytes a day: after a day of work one held about 54 GiB of `freshkube_desktop` objects and 11 GiB of `freshkube_core`'s. When the disk runs low, remove those two crates' objects:

```sh
find target/debug/deps -type f \( -name 'freshkube_desktop-*.o' -o -name 'freshkube_core-*.o' \) -delete
```

Cargo doesn't track the objects, so this rebuilds nothing, and the next build writes what it needs. Binaries already linked keep working, but their backtraces lose file and line numbers until they are rebuilt. It isn't `cargo clean`: dependencies, `.rlib`s and incremental state stay. `rm -rf target/*/incremental` frees a few more gigabytes, at the cost of a slower next rebuild.

```sh
cargo build
cargo run -- --fixture            # synthetic example data, no credentials or cluster
cargo run -- --config ~/.talos/config --context <name>
cargo run -- --kubernetes-only --kubeconfig <file> --kube-context <name>   # no talosconfig
cargo run -p freshkube-workbench   # the shared components on invented data, one story at a time; not shipped
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
scripts/check-style.sh            # pages use the shared components; see DESIGN.md "Checks"
```

For the inner loop, use `cargo test -p <crate> --lib <filter>` while iterating on a library crate, then run `cargo clippy -p <crate> --all-targets -- -D warnings` once at the end. Save the full workspace test/clippy/fmt and style checks above for before submitting the PR for review; all remain required.

CI, app bundles and releases are described in [docs/MACOS_PACKAGING.md](docs/MACOS_PACKAGING.md#ci): pull requests run the checks above, merges to `main` build bundles, and a `v*` tag drafts a release from them.

Debug builds open on a page from `FRESHKUBE_PAGE=<slug>` (`overview`, `nodes`, `health`, `resources`, `etcd`, `system-services`, `security`, `lifecycle`, `operations`, `monitoring`, `applications`, `settings`). `FRESHKUBE_PAGE=node-logs` opens the first responding node on its Logs tab. Debug fixture checks also take `node-overview`, `pod-overview`, `workload-logs` (payments/api's logs in the dock), `search`, `kubernetes-only` and `maintenance`, which opens maintenance mode on an invented node (`maintenance/example.rs`) that walks itself to the configuration review and refuses to go further; with `--fixture`, `monitoring` answers its dashboard from example data at once, so a capture shows it. Their entry points live in `desktop/startup/`. `FRESHKUBE_THEME=light|dark` and `FRESHKUBE_WINDOW_SIZE=1280x880|760x560` select appearance and window bounds without changing saved settings; `FRESHKUBE_MOTION=reduced|full` overrides the OS's reduced-motion setting. `FRESHKUBE_KIND=<key>` opens a Kubernetes kind on the Resources page by its kubectl key (`pods`, `deployments.apps`, `nodes`, …; see `resources/navigation.rs`). With `--fixture` it also takes the example custom kinds, such as `certificates.cert-manager.io`. `FRESHKUBE_TEXT_SIZE=<12|13|14|16|18|20>` starts at that text size without saving it; 13 is the default. With `node-logs` or `observability-application`, `FRESHKUBE_LOG_SEARCH=<text>` types that text into the log's search and, once the example lines arrive, steps to the first match (the application page opens on its Logs report, scrolled to the log). With `--fixture`, `FRESHKUBE_ETCD=lost|single|alarms` reshapes etcd's example: a lost quorum, a single member, or five alarms. With `--fixture`, debug and stress builds take `FRESHKUBE_APPLICATIONS=single|partial|refused|unserved|failed|capped`, which reshapes the Applications example: `core-fra` alone, Kargo refused there, every source refused, nothing served, no cluster answering, or Argo CD's Applications capped. They also take `FRESHKUBE_FIXTURE_HOLD=talos|lists|coroot|monitoring|applications|all`, which keeps example data from answering so pages stay on their loading rows: `talos` holds the Talos overview and the Kubernetes summary (Overview, Nodes, System services, Operations), `lists` the Resources lists, `coroot` Observability's applications, incidents and traces, `monitoring` the dashboard's panels, `applications` the Applications list, `all` every one; `scripts/stress.sh … loading <page>` sets it. Use them with `scripts/smoke.sh` (see [Smoke tests](#smoke-tests)).

## Cluster safety

- Read-only by default. Checks against a real cluster list, watch and read; they never create, change or delete resources.
- Pod exec and port forwarding are the two exceptions, and each starts only from an explicit action.
  - Pod exec (`resources/exec/`) creates a `pods/exec`. The app starts one only from an explicit Start, and a live check runs one only on a pod and container the user named, with harmless commands ([docs/POD_EXEC.md](docs/POD_EXEC.md)).
  - Port forwarding (`resources/forward/`) creates a `pods/portforward`. It changes no objects, but it reaches whatever the port serves. The app starts one only from an explicit Forward, and a live check forwards only a pod and port the user named ([docs/PORT_FORWARD.md](docs/PORT_FORWARD.md)).
- Never run Operations or maintenance actions (drain, reboot, upgrade, apply config, reset) during verification unless the user asked for that specific action on a node they named as disposable.
- Use the context the user chose. Never fall back to whatever kubeconfig or talosconfig context is current.
- Never print or commit credentials: talosconfig and kubeconfig contents, tokens, Secret values, keys. Redact them in logs, screenshots and evidence.

## Working with GPUI

GPUI suits Rust well: views are entities changed through `update`, `cx.notify()`, `observe` and `subscribe`, closures hold weak handles, and every frame rebuilds the element tree from current state. These rules cover where it bites. Read GPUI Kit's own [coding](https://gpui-kit.com/docs/coding-guides.md) and [design](https://gpui-kit.com/docs/design-guides.md) guides before UI work.

### Keep render cheap

- `render` runs on every frame for every dirty view, and a notify redraws the whole window; even a hover does. Never sort, filter, group or format collections in `render`. Derive display data when the data changes and keep it on the entity.
- Caching is opt-in. Use `.cached()` views or row caches keyed by a data revision for anything heavy, and give each cache a clear owner that invalidates it.
- Keep repeating motion out of the cached chrome and the pages: anything that ticks inside a view redraws it and every ancestor each frame. Draw it in a small view beside them, as the shell draws the shown page's `LoadingMotion` beside the page and its cached header, rail and column (`desktop/shell/chrome.rs`). Cache those parts beside the motion, never an ancestor of cached pages: a cached view that draws again draws every cached view inside it again. No Kit `.loading` spinner on a Refresh; use `ui::refresh_icon`.
- Don't run app-wide timers that notify a large view. Give periodic updates (clocks, "updated 5 s ago", ages) to the smallest entity that shows them. A 1-second timer on the shell once redrew everything and made the app feel slow.
- Render only the rows a frame can show. Use `uniform_list` for fixed-height rows. For rows whose height depends on the width, use `VirtualList` with heights measured by layout, never computed. A plain list needs a hard cap. [docs/LONG_LISTS.md](docs/LONG_LISTS.md) has the rules and the state of each list.
- A `cx.notify()` from `on_prepaint` or paint only marks the view dirty; no frame follows until some input arrives. Geometry learned while drawing (a list's width, a panel's height) must ask for the redraw with `window.on_next_frame`, as the log view does. Headless tests miss this, since `render_frame` draws regardless; check `simulate_next_frame`.
- Keep `[profile.dev.package."*"] opt-level = 3` in the workspace manifest. Unoptimised GPUI draws a frame about 14× slower, so debug builds without it give a false picture of performance.
- Measure before and after a change that could cost time. `scripts/stress.sh` runs list, watch-burst and log-flood workloads on a release build against a synthetic API and prints per-span timings, CPU and memory; [docs/PERFORMANCE.md](docs/PERFORMANCE.md) has the workloads and the numbers so far.

### Async: two executors

GPUI has its own executor; tonic and kube need Tokio. The binary owns the Tokio runtime and passes its handle to the app.

- Run cluster I/O on Tokio, send the result back to a GPUI task, and update entities there. Never block the UI thread.
- Give ordinary reads both a source/target identity and a request generation. Keep the Tokio `OwnedJob` and GPUI delivery `Task` on their actual owner; replacement drops both, and completion checks identity/generation before publishing. `Snapshot` tracks data and freshness; it does not cancel work or inspect credentials. [State and request lifetimes](docs/STATE_LIFETIMES.md) describes the current owners and exceptions.
- The shell activates and periodically refreshes only the visible `ScreenPanel`; it sends source updates to every retained screen. The trait has no hide hook: navigation alone does not cancel a `Loader`. Operations reads its preview and audit only from `activate`, so a hidden Operations reads nothing. Resources, Monitoring and Observability have explicit hide cancellation. New page-owned reads should start only while shown; preserve the existing log and user-started session lifetimes below. Dock log tabs are user-started: each reads while it is open, on any page, and stops when it closes.
- The shell's Kubernetes summary is session-owned: nine compact reflector stores run on every page and publish after a 500 ms coalescing window. Health and visible Lifecycle consume those observations; selecting a Talos node does not replace the session. The 15 s shell cycle and countdown are for Talos; metrics keep their separate polling schedules.
- Operations mutations have a separate lifetime: a detached delivery task holds the app-wide operation ticket until core's runner ends. Cancellation is cooperative, before/between steps; await a submitted mutation. Never wrap the submitted run in an ordinary `OwnedJob` or abort it when its screen changes.

### Learning the API

Documentation is thin. Before using an API, read the source of the pinned versions in the Cargo registry (`~/.cargo/registry/src/*/gpui-kit-0.7.0`, `gpui-component-0.7.0`, `gpui-base-0.7.0`, `gpui-pre-*`); don't guess from older GPUI examples. Expect odd shapes, such as `window.root::<Root>()` returning `Option<Option<Entity<Root>>>`, and breaking changes whenever the pinned pre-1.0 versions move.

### Keys and focus

- When the focused element isn't drawn, key dispatch starts from the window root and the page's bindings stop working. Give a page's key context to a wrapper that is drawn in every state, not to a list that a placeholder replaces.
- macOS reports Command-Shift-] as `}` with Command alone, so bind `secondary-}`, not `secondary-shift-]`. Command-Shift with a letter keeps Shift: `secondary-shift-g`.
- A deeper context's binding wins. A handler that calls `cx.propagate()` lets the key reach raw listeners, which is how Enter still presses a focused button.
- A focused terminal (`freshkube-terminal`) takes shell keys before any binding runs, through a keystroke interceptor, so Escape, Tab, Control-Tab and plain Ctrl-C/V reach the program. Terminal-wide copy, paste and leave use Command on macOS and Ctrl-Shift on Linux/Windows; the interceptor lets those modifiers reach the keymap.
- `FocusHandle::dispatch_action` runs at once on that node; `window.dispatch_action` is deferred. A Kit dialog remembers what had focus when it opens, so focus its content after `open_dialog`.

### Sizes

The user chooses the text size (`freshkube-ui`'s `text_size.rs`), which becomes the window's rem size, and the whole layout scales with it.

- Size text, rows, padding, gaps, icons and widths with `ui::dp(n)`: n pixels at the default 13 px (`ui::BASE_TEXT`), as rems. Where an API takes `Pixels`, use `ui::dp_px(n, window)`. Size icons with `.size(dp(n))`, not `with_size(px(n))`, which stays fixed.
- Keep borders, hairlines, corner radii and shadows in `px`, as Kit does.
- Compare breakpoints in `dp`: `freshkube_ui::page::page_width` (a page without margins, also `screens::page_width`), `inset_width` (inside its insets, such as the header) and `content_width` (inside a padded page's margins) return them, so a larger text size picks the narrower layout, as a narrower window would.

### UI tests

Headless UI tests render the real app, find elements by id and click or type into them; the suite runs in seconds. Prefer them over manual checks, and give every interactive element a stable, domain-based id.

- Dialogs animate on the real clock, and advancing the test clock does not finish them. The shared test setup in `desktop/tests.rs` turns on reduced motion (`cx.set_reduce_motion(true)`); two confirmation-dialog tests failed every run without it. Do the same in any new test harness.
- `window.render_frame` bypasses view caches. Don't use it to prove that a view was or was not redrawn; use the render probes (`probe::hit`, `probe::count`) from `freshkube-probe`. They count only with its `counting` feature, which a crate turns on from its dev-dependencies, as `freshkube-desktop` does.
- Element snapshots can't tell whether a button is disabled. Assert the outcome instead: click it and check that nothing changed.
- Setting an input's value from code does not emit its change event. Type into it with real input events (`window.input`) when the change handler matters.
- Tests write only under a fresh temporary directory, never into the crate or the user's home.
- Time anything a test depends on with the executor's clock (`cx.background_executor().now()`), not `Instant::now()`, so tests step it with `advance_clock` instead of sleeping. Example data dated from the wall clock must not be generated again for a stream already read: a second later it passes for new lines. Both made tests fail on a loaded machine.

### Smoke tests

**Every change a person would notice in the app — a page, a control, a flow, a layout — gets a smoke test in the running app before it is called done.** UI tests prove the logic; a smoke test proves it looks and works right on screen. Use the helper, don't improvise one:

```sh
scripts/smoke.sh start --page observability-traces          # builds, launches with --fixture, waits for the window
scripts/smoke.sh shot traces                                # → target/smoke/<worktree>/traces.png; open it and look
scripts/smoke.sh key 'keystroke "k" using command down'     # any System Events key clause
scripts/smoke.sh click 640 220                              # points from the window's top-left
scripts/smoke.sh right-click 640 220                        # the same with the right button: a context menu
scripts/smoke.sh hover 640 220                              # the pointer there, no click or wheel: for tooltips
scripts/smoke.sh scroll 900 500 600                         # wheel at a point, 600 points down
scripts/smoke.sh full traces                                # traces-0.png, traces-1.png, … the whole scrolling page
scripts/smoke.sh stop
scripts/smoke.sh pages                                      # every page, one capture each
scripts/smoke.sh start --page overview -- --config <talosconfig> --context <name> --kubeconfig <file>
```

For captures on an Intel Mac, add the `capture` label to your PR to get a debug
binary from the separate [Capture workflow](docs/MACOS_PACKAGING.md#ci). New
commits refresh its artifact; it expires after three days. After the run succeeds,
download the artifact for the PR head you want (replace the example run ID and
short SHA), restore its executable bit, and pass it to the helper:

```sh
CAPTURE_RUN=123456789
CAPTURE_SHA=abc1234
CAPTURE_DIR="$PWD/target/ci-capture/$CAPTURE_SHA"
gh run download "$CAPTURE_RUN" --name "freshkube-debug-x86_64-apple-darwin-$CAPTURE_SHA" --dir "$CAPTURE_DIR"
chmod +x "$CAPTURE_DIR/freshkube"
FRESHKUBE_SMOKE_BINARY="$CAPTURE_DIR/freshkube" scripts/smoke.sh start --page monitoring
```

Use a debug binary for page, kind, theme, window-size and
text-size overrides; release builds and CI app bundles ignore them. Reuse a
recent debug capture set of `main` for a before comparison when one is available.

- Smoke the pages and flows the change touches, in fixture mode and, when the change reads a cluster or Coroot, against the context the user chose. Run `scripts/smoke.sh pages` when a change reaches the frame, the theme or shared components.
- A page moved onto the shared components is compared with Pods side by side: capture both at the same size, theme and text size (`FRESHKUBE_KIND=pods scripts/smoke.sh start --page resources`, then the page) and check that title, padding, header, rows, group rows, glyphs and states line up.
- Look at every capture yourself, then report what you checked and what you saw: the captures that show the change, anything wrong, and anything you could not check.
- Judge a page from all of it, not its first screen: `full` captures each screenful down to the bottom, and `scroll` reaches a part further down to click there.
- `start` takes `--page`, `--theme`, `--size` and `--release`; the slugs are those of `FRESHKUBE_PAGE` (see [Build, run and test](#build-run-and-test)), plus `observability-<destination>`. Open pages with `--page` rather than navigating to them, and click only to exercise the change.
- `FRESHKUBE_SMOKE_BINARY=/absolute/path/to/saved/freshkube` makes `start` use that executable without building; `FRESHKUBE_STRESS_BINARY` selects a saved stress executable the same way.
- Live checks only look and navigate. Never press Operations or maintenance actions, and keep credentials out of captures you share.
- One worktree uses the screen at a time. `start`, `browser.sh open` and `stress.sh` wait for a lock (`scripts/smoke/lock.sh`, in `~/.cache/freshkube/screen.lock`) that `stop` and `close` release; a dead owner's lock, or a smoke test idle for ten minutes, is taken over. Build before you start, keep the session short, and always `stop`. "screen: waiting for …" means another worktree is checking; let it finish. A locked screen spoils captures and keys too, so every command waits for it to be unlocked ("screen: locked; waiting …"); keep the Mac awake for long unattended runs (`caffeinate -d -i`).
- A live `start` after a new build plays a sound: the app may ask Keychain for the remembered Coroot key, and only the user answers it.

To compare a page with the tool it reads from (Coroot, Grafana), `scripts/browser.sh` drives a Chrome window with the same commands: `open URL`, `go URL`, `shot`, `full`, `scroll`, `click`, `key`, `url` and `close`, with captures in `target/smoke/browser/`. Sign-ins are the user's: when a page asks for one, stop and ask them to sign in in that window.

The terminal needs Screen Recording and Accessibility in System Settings → Privacy & Security, and the screen must be unlocked. GPUI stops drawing a covered window, so a capture of a covered window or a locked screen shows only the first frames, before any read finishes. macOS drops keys sent to a window that isn't frontmost, so every helper command brings the app forward first. Keep interaction logic in UI tests too; a smoke test adds to them, it doesn't replace them.

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

### Using Snapshot<T, I> and Loader<T>

Desktop's [state::Snapshot](crates/freshkube-desktop/src/state.rs), core's [`snapshot::Snapshot`](crates/freshkube-core/src/snapshot.rs) with a default identity, retains the last successful value for one identity and rejects superseded request generations. Its default identity is `AppliedConfig`; node reads use `Target`, and provider reads use their own identity. This small example uses the same identity labels as its unit tests:

```rust
use crate::state::Snapshot;

let mut state = Snapshot::<u32, &str>::default();
let initial = state.begin("node-a");
assert!(state.apply(&initial, Ok(7)));
let refresh = state.begin("node-a");
assert!(state.apply(&refresh, Err("offline".into())));
assert_eq!(state.data(), Some(&7));
assert!(state.is_stale());

let replacement = state.begin("node-b");
assert!(state.data().is_none());
assert!(!state.apply(&refresh, Ok(99)));
assert!(state.apply(&replacement, Ok(8)));
```

For ordinary inspection reads, [screens::Loader](crates/freshkube-desktop/src/screens/mod.rs) owns `Snapshot<T, Target>`, `Option<OwnedJob>` and `Option<Task<()>>`. Use `load(target, &runtime, what, work, slot, cx)` for Tokio work, `resolve(target, result)` for fixtures, and `reset()` when `set_source` detects a changed target. [The etcd example](docs/STATE_LIFETIMES.md#ordinary-screen-reads) shows the real call. Render reads `data()`, `error()` and `is_loading()`; retain data alongside a refresh failure banner and its timestamps. Call `cx.notify()` after changes outside render; derive display collections when data changes.

Access identity answers whose data this is; generation answers which request may publish. Follow [ACCESS_IDENTITY.md](docs/ACCESS_IDENTITY.md) when configuration/access changes. Preserve each feature's owner: logs use `LogView<S>`, Monitoring and Observability own their page requests, shells belong to `ShellView`, and forwards belong to the app's `ForwardList`.

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

`resources/` browses Kubernetes kinds on one `Page::Resources`; the rail's groups and their columns pick the kind. It is not a `ScreenPanel`: it doesn't follow the target node, the shell owns it directly and tells it when it is visible, and only a visible page lists and watches. Rows are the server-printed table (wide columns hidden, Age computed locally) and are selected by identity (connection, resource, namespace, name, UID), never by position. The shell's automatic refresh never restarts the watch; only Refresh lists again.

Pods list with `includeObject=Object`, and `resources/rows.rs` derives each row's state, reason, readiness, owner and node from the object when it arrives; watch events carry no columns, so `live::convert` reads them by the last listed ones. The visible pods page also reads metrics.k8s.io every 15 s (`screen/pods.rs`) and keeps the last answer when a read fails, shown grey as last known. The projection groups pods by cause after sorting (`projection.rs`): problems first, healthy pods folded while anything is wrong and no filter is typed; `ListView::All` keeps one sort. A node's pods list and every other kind stay flat. Marks (x) are identities, pruned when their rows go; L, or a row's Logs button, opens a pod's logs in the dock.

The Custom Resources column is `resources/custom.rs`, an entity the shell owns. It discovers the API groups when the column shows and a group's kinds when the group opens, keeps what it found until the connection changes or the user retries, and reads nothing while hidden. It derives the rows' labels, ids and tooltips when an answer arrives, so the column's `render` only reads them. A failed group shows why, never an empty group. When the page's kind answers 404, the screen shows it as not served and emits `NotServed`, and the group is discovered again.

Selecting a row opens the detail pane (`resources/pane/`, with its model in `resources/detail.rs`), a cached view in a drawer (`freshkube_ui::drawer`) over the list's right edge, under the page toolbar. It opens 725 dp wide and keeps within 300 dp and the least of 90% of the list's width and that width less 280 dp, which it measures as it draws; under 600 dp it takes the whole list. Its left edge resizes it, saved under `drawer` in `navigation.json`: one width for the whole page, not one per kind. Another row swaps what it shows; a click beside the rows (empty space, not a row, column header or the list's own buttons, which stop their clicks) closes it and keeps the row selected, as do Logs and Shell, which open in the dock; × and Escape close it and clear the selection. Closed with a row kept, the arrows move the selection without opening it, Escape clears it, and Enter or a click opens it again. A short page scrolls its frame with the list keeping 180 dp; the drawer scrolls itself and takes no more, so its header stays in sight. It has two tabs, Details and YAML. Details is one scrolling page: the Overview (a pod's cause card first), Ports for a pod, Service or workload, then Events, under an index whose buttons jump to a section and mark the one at the top (`Section`). A section asked for, as Forward or All events asks, scrolls there and stays marked until the user scrolls, even when it is too short to reach the top; YAML and back keep the place. A copy button beside the title copies the name alone. It reads the full object itself and watches its events; it doesn't watch the object. The list is its source of truth: the screen passes on each new `resourceVersion` the list sees, which triggers at most one read a second, and tells the pane when the row is gone. Every answer carries the request's sequence number and identity, and a stale one is dropped. Secret values never reach the document: `reveal_secret_value` reads one key again, and every revealed value is forgotten when the page hides or a new version arrives.

Logs open in the dock (`desktop/dock/`), a full-width strip of tabs under the page on every page, as Freelens has it. The pane's Logs button (a pod's opens on the container Overview picks), L on the list, a container's Logs or Previous on Overview and Attention's Logs all emit `ResourceLink::Logs`, which `Pilot` hands to `Dock::open_logs`; `pane.set_tab(Tab::Logs)` asks the same. The Resources drawer steps aside for every Logs link, keeping its row selected; opening an object straight on Logs reads nothing for the drawer. A tab is keyed by (connection, kind, namespace, name), and a pod's also by UID: opening it again selects it, and duplicate titles take "(2)". At most eight log tabs: a ninth asks "Close the oldest log tab (⟨title⟩)?". Each tab owns its view and **reads while it is open, even when the page is hidden or another object opens**; closing the tab (its ×, a middle-click or Command-W), the dock's × or Close all, or another context, kubeconfig or talosconfig drops it; a Kubernetes source missing for a while under the same access identity, as Talos's before its overview answers, keeps it. Nothing reads out of sight: minimized, the dock still shows its tabs, and the status bar's "N logs" opens or minimizes it. A tab's feed (`dock/feed.rs`) watches its one pod by name for its containers (a workload's reads the object once for its selector) and marks a pod that goes as gone, keeping its lines and reading no more; a pod made again with the name opens a new tab. A tab asked for by name, as a restored one, takes the UID of the first pod its watch finds. The dock opens at 300 dp, or until dragged at most half the page cell in a window too short for a table page, and the list above keeps its selected row in sight; it drags from its top edge, minimizes to its bar below 100 dp or with Shift-Escape, and Fit to window fills the page cell; unless fitted, the page keeps at least 100 dp, and the dock keeps the selected log's toolbar and three lines (`LogView::least_height`). Its height, state and tabs by name are saved in `navigation.json`; saved tabs come back only for their context and read nothing until selected and shown.

A pod's tab is `LogView<PodLogs>` (`logs/pod/`), fed by `follow_pod_log` in core: one container's stream, chosen by the tab's picker or the action that opened it. The previous instance is only ever read when the user asks for it, with the toolbar's Previous toggle. A pod with more than one app container also offers All containers at the top of the picker: every app container's stream at once, at most 20 in the pod's order with the rest counted in its note, through the shared `StreamSet` (`logs/streams.rs`, as a workload's tab reads), interleaved by time and tagged by container in the Source column, Copy and Download; a container that ends leaves a marker while the others read on, and Previous reads each one that ran before. Init and ephemeral containers stay picked one at a time and are never part of All. A saved tab keeps the pick (`all_containers`, left out while false).

Every log's toolbar is one row (`logs-tools`): the source's `tools` first (a pod's container and tail pickers, Previous, Timestamps, Stop or Resume and its status tag; a workload's Pod select and Timestamps), then Levels (a menu with each level's count), search, Wrap, Follow, Copy and Download (`freshkube-logs`'s `download.rs`): a menu that saves the visible or every retained line to a file the user names, as Copy copies them (a workload's led by their `pod/container`, the source's `tags_lines`), with the source's `download_name` for those lines (a workload's carries its picked pod only for the visible lines) and the time as the suggested name, in ~/Downloads, else the home folder, else the working folder (`.`). It writes off the UI thread to a temporary file renamed into place, so a failed save leaves no partial file, and says where it saved or why it couldn't under the toolbar. While its save dialog is open, another Download does nothing. Under 40 rem it shows its buttons' icons alone, a menu's caret beside its icon, and a pod's status tag its glyph alone, with its words in the tooltip and the accessibility label, and wraps to at most two rows, as at 760 points or text size 20. A source's `controls` go above the row (Talos services) and its `notes` under it: a pod's one-line note, cut short with its whole text in the tooltip, joins why the log waits with how the container last ended. Example mode applies a container's lines at once and writes on with a background timer, so UI tests advance the clock to see new lines.

A Deployment, StatefulSet, DaemonSet, ReplicaSet or Job tab is `LogView<WorkloadLogs>` (`logs/workload/`): it watches the workload's pods by its selector and follows each app container, at most 20, interleaved by time and tagged `pod/container`. Its Pod select shows one pod's lines and chips, or all pods': a filter, so every stream reads on. A picked pod that leaves keeps its pick, marked "(gone)", and a pod the cap leaves unread says so in the cap note's words.

Shells open in the dock too (`resources/shell/`), one of the two exceptions to read-only on the browsing pages ([docs/POD_EXEC.md](docs/POD_EXEC.md)). A pod's pane has a Shell menu in its header, "Start shell in ⟨container⟩" for each container, enabled for running ones: the pick is the explicit Start, and opening the menu runs nothing. Each container's shell has a tab of its own, "Shell ⟨pod⟩", whose session runs whatever page or object shows; picking the same container again selects the tab, and starts a new session if the last one ended, as the tab's Start again does. In example mode a local shell answers. Closing a running shell's tab (its ×, a middle-click, Command-W, the tab menu or the dock's ×), another context, kubeconfig or talosconfig, closing the window and quitting go through `shell::unless_shell` or `shell::may_close`, which ask "End the shell in ⟨pod⟩?" or "End 2 shells?" first; navigating never asks. A closed tab ends its session politely (`close_session`). Shell tabs are saved by name with their container and come back idle, running nothing until Start. At most eight shell tabs, apart from the eight log tabs, since each is a pod exec while it runs: a ninth takes the place of the oldest that runs nothing, or, when every one runs, asks "End the shell in ⟨pod⟩?" for the oldest. The status bar reads "2 logs · 1 shell".

A pod, Service or workload also has a Ports section in Details (`resources/pane/ports/`), which starts port forwards ([docs/PORT_FORWARD.md](docs/PORT_FORWARD.md)). It lists the ports the document declares, each with a local port field and Forward, and takes any other port. It reads nothing itself: a forward belongs to the app (`forwards/`), not to the pane, and runs until Stop through page, pane, context and kubeconfig changes, on the connection it started with. The status bar's "⇄ 2 forwards" lists every forward with Copy address, Open in browser, Stop, Start again and Remove; an ended one stays listed until removed. Quitting or closing the window asks once for shells and forwards together (`shell::may_close`). In example mode a real loopback listener answers each connection with a short page; the example pods serve 8080 only, so any other port answers like a refused connection. UI tests that connect to it call `allow_parking`, since Tokio wakes GPUI from its own threads.

The keyboard follows one path through the page, and each level owns a key context: `KubeResources` on the list (always drawn, so the page keeps its keys while it shows no rows), `KubeResourcesFilter`, `KubeDetail` on the pane, `KubeDetailTabs` on its tab strip, and, in the dock, `Dock` around `LogPanel`, `LogSearch` and `LogView`. Enter opens the selected row and moves the keyboard into the pane. On the list, X marks the selected row, Shift-X every row of its group (not Healthy), O opens a pod's node, H shows or folds the healthy pods while they may fold, and L opens a pod's logs. A right-click on a row selects it and opens its context menu (`freshkube_ui::table::row_menu`), made of those same actions, each with its key; a group row's menu selects its first row and offers them for it. The group rows keep only the healthy group's chevron; the filter types these letters, since it sits outside the list's context. Escape steps back one level at a time: a search clears and then leaves, the pane hands the keyboard to the list with the pane left open, and the list clears its filter and then closes the pane. Whatever takes the keyboard away (a new context, the namespace picker, the filter) hands it back to the list. Command-F, Command-G and Command-A act on what has focus: in the dock its log, in the pane its YAML. In a log's search, Enter and Shift-Enter step to the next and previous match and keep the keyboard there; the log's `LogSearch` context stops the Enter that Kit's input lets go on, so a page's own Enter doesn't take it. Control-. and Control-, step through the dock's tabs from anywhere, reopening a minimized dock; Command-W (Ctrl-Shift-W on Linux/Windows) closes the selected tab; Escape in the dock clears a search, leaves it, clears a selection, then hands the keyboard back to what had it. In a shell tab's terminal, Escape, every Control key (Control-. and Control-, too) and Option-arrows go to the shell; Command-Escape on macOS or Ctrl-Shift-Q on Linux/Windows hands the keyboard back to what had it. Command-K (`desktop/search/`) opens Search everything in a Kit dialog. Pages, kinds and joined nodes are local; nine metadata-only lists start on open, capped at 2,000 per kind with independent 5 s deadlines. The query derives groups of eight results; refused and capped kinds remain visible. Closing cancels reads, and reopening within 30 s on the same connection reuses the cache. Secret data is never requested by search.

## Adding a screen

1. Pick the page's type from [DESIGN.md](docs/DESIGN.md#page-types) (table page, dashboard, canvas or detail pane) and build it from that type's shared components: the page header, the table, group rows, status glyphs and states. Don't draw your own title, table, glyphs or radii; `scripts/check-style.sh` fails on them, and its allowlist is empty and only shrinks, so there is no exception to add. A table page's line (context, counts, state, refresh time) goes in the status bar through `ScreenPanel::status` and a `status::Segment`, not under its header. Its row actions act on the selection, from the toolbar, keys and the shared row context menu (`TableSource::menu_focus` and `row_menu`), never from a button on the row. A selection's details go in the shared Inspector (`freshkube_ui::inspector`, change 9 in DESIGN.md), beside the table or stacked under it, not in a card of its own.
2. Add cluster pages to `Page` (`desktop/pages.rs`): `ALL`, its slug, its `Area` and column row, and its shortcut. Add inspection views to `ScreenKind`, the screen factory and `Page::screen` or `NodeTab::screen`, according to their scope.
3. Implement `ScreenPanel` in `screens/<name>.rs`, or `screens/<name>/mod.rs` once it mixes concerns. The screen owns its requests (`OwnedJob`), its data and its offline example data.
4. Put cluster logic in `freshkube-core` and keep the screen to presentation.
5. Add UI tests for loading, empty, failure and the main interactions. A table page also calls `desktop::layout_check::assert_table_page` (`freshkube_ui::layout_check`, behind its `testing` feature, as is `settle_header`), which measures its header, rows, group rows, edge-to-edge frame and title against Pods' sizes; a page with several lists calls its halves, `assert_edge_frame` and `assert_table`, a padded page of cards, such as a dashboard, `assert_page_frame`, and a screen embedded under a tab that names it (`PageHeader::untitled`), `assert_untitled_edge_frame`. A page with an Inspector also calls `assert_inspector`, beside and stacked. Then smoke it beside Pods at the same size, theme and text size ([Smoke tests](#smoke-tests)).

Diagnostic checks follow the reliability rules below: find the source of truth first, use the `DiagnosticCheck` constructors, provide an actionable fix where possible, and return `unknown` rather than failing when data is unavailable.

## Quick reference

### Do

- Check actual system state (files, APIs)
- Use desktop `Snapshot<T, I>` and, for ordinary inspection reads, `Loader<T>`; preserve feature-specific owners
- Check access/target identity and request generation before publishing; retain same-identity refresh failures as stale
- Keep submitted Operations runs outside the ordinary read-job abort lifetime
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

The Nodes workspace (`desktop/nodes/`) joins both summaries when they change. Its table scrolls horizontally with its header when narrow, including beside the node's inspector, keeping each row's glyph and name at the left edge; names stay on one line and their row tooltip holds the full name. Beside the node's pane the table keeps its filter, groups and folded healthy rows, and a filter that hides the open node keeps its pane and selection. Its retained pane is an `Inspector` in `inspector::split` (width saved under `nodes`), and embeds the existing node screens and log view; they lay out by the inspector's width (`screens::set_node_pane_width`), not the window's. Its Pods list has its own field selector and filter, drops a previous node's watch by epoch, and opens objects in the main Resources pane. Node Events and YAML use a separate resource pane limited to those views. Enter on a node, a click on a pane tab or a click in the pane moves the keyboard into the pane: to the tab's list, document, log or screen, or to the tab strip for a screen that takes no keys. A click on a row leaves it on the table. Command-Shift-] and [ give it to the new tab's log (Logs) or screen (Processes, Storage, Network, Diagnostics), and to the table for the other tabs. On the table, H shows or folds the healthy nodes while they may fold (the filter and the pane type it); a right-click selects a node as the arrows do, the pane opening only if it was open, and offers Open and the fold, each with its key, and the healthy group keeps only its chevron. Escape steps back as on Resources: the filter clears, then leaves; a tab steps back through its own levels (the Pods list's filter, the Processes find, which clears and then leaves, the Events or YAML find and selection, a log's search and selection), then hands the keyboard to the table with the pane left open, an expanded one giving the table its room back first; the table clears its filter, then closes the pane, keeping the node selected.

The frame (`desktop/shell/`) is a 44 dp header, a 64 dp icon rail, a 208 dp column for the rail's area and a 28 dp status bar ([docs/DESIGN.md](docs/DESIGN.md)). The dock, when it has tabs, sits in the page cell under the page, as a cached view with an explicit height; `freshkube_ui::page::set_below` tells pages the room it takes. The rail's areas (`Area` in `desktop/pages.rs`) are Overview, Nodes, Namespaces, Events, Applications, Monitoring and Observability; the six Kubernetes groups and Custom Resources; and Control plane. Only the groups, Custom Resources and Control plane have a column: a group's kinds (Health first under Workloads), the API groups, or the Talos pages. A group's rail button reopens its last kind. Its problem dots (`RailMarks`) are derived with the Overview's cards. `screens::page_width` subtracts the rail and, when shown, the column. Command-1 through Command-9 follow Overview, Nodes, Namespaces, Events, Health, etcd, System services, Security and Lifecycle; Control-Tab also visits the last other kind. Kubernetes-only mode starts on Overview, and its Control plane column offers a talosconfig instead. Node inspection and service logs live in the node pane; the header holds the context switcher, the location, Search everything, Refresh and Settings, which holds the app version and a Workspace row that opens the Settings page (`Page::Settings`, `desktop/settings/`), a table of the clusters in `workspace.json` that Add, Edit, Remove and Move change and save at once (`settings/edit.rs`; never for example data); it has no rail button. `ScreenKind` retains the inspection panels independently of `Page`.

Overview derives its cards and attention rows when either summary changes (`presentation/overview/`, `presentation/attention/`), including independent unavailable parts. Attention groups its rows as Failing, Warning, then Unknown (last-known evidence from a stale source), with each group's count derived with the rows; it shows eight compact rows, and its showing bar expands them to at most fifty. Node panes use its rows for their own node, uncapped. Object links use `Pilot::open_object`, which retains a matching namespace, and clears a hiding filter before navigating. Unknown UIDs are resolved through metadata-only reads, never full Secret reads.

The Kubernetes summary session (`kubernetes_summary/` in core and desktop) takes a `SubscriptionKey` (applied session/configuration revision, API, scope, selectors and retained representation) and serves only the exact summary key; the returned `Subscription` holds the shared publication and does not retain the key. Summary and visible Lifecycle share the same Node publication; the Resources printer-table watches remain separate. `InitDone` commits a complete replacement, including empty results. Failed or unfinished lists keep previous complete evidence explicitly stale; `Part::loaded()` can return that evidence, so check `is_current()` before inferring absence. A 403 remains `Part::Refused` and does not block other sources. Refresh cancels the watcher generation and relists; 410 relists the affected kind. Context/config replacement drops the `OwnedJob` and all child watches, while selecting a Talos node leaves the cluster session intact. Pods retain compact facts, at most 4 KiB each; object and byte limits fail visibly without evicting live records. Fixture input uses the same reducer with one captured reference time per session.

Monitoring (`freshkube-monitoring`'s `page/`, [docs/MONITORING.md](docs/MONITORING.md)) is its own rail area; its column lists the built-in dashboards and the user's folder of Grafana JSON (Settings → Dashboards folder, saved in `monitoring.json` beside the preferences). The shell owns the page and tells it when it shows; a hidden page finds nothing, resolves nothing and asks no panel. Showing it finds Prometheus for the context (the remembered Service first), resolves the variables, then asks only the panels in or near the viewport. Every change that asks again takes a new generation, and an older answer is dropped; hiding drops every request and the auto-refresh timer. Another source rebuilds the dashboard, so one cluster's answers never show as another's. Panels draw only Console colours. Timeseries also draw deploy and node markers (`monitoring/page/markers.rs`), read per generation from ReplicaSet metadata, Nodes' Ready condition, Node events and, with Talos, boot times; the page filters them by its toggles and the namespace and node variables before handing them to the panels. A pod's Overview and the node pane show CPU and memory over the last hour (`monitoring/history/`) from the Prometheus the page confirmed or remembers for the context (`MonitoringPage::history`, pushed on `MonitoringEvent::History`); without one they show and read nothing. A pod's chart reads while its drawer shows Details, at any scroll, and the node pane's while its Overview shows.

Applications (`applications/`, [#521](https://github.com/skel84/freshkube/issues/521)) is its own rail area, without a column yet. It lists what core's `applications::derive` finds from Kargo Projects, Argo CD ApplicationSets and Applications, and the `app.kubernetes.io/part-of` label, grouped by the rule that found each one, with the selection in the Inspector. The shell owns the page and tells it when it shows: it reads only while shown, when it has nothing or its last answer is 30 s old, and on Refresh; hiding drops the read in flight, whose late answer never lands, and another connection drops what was read. A live read is the open connection alone, through core's GET-only `ReadOnlyClient`, with Argo CD read in `argocd` only: its applications are marked "Read in argocd only, with notes", never read in full. The display is derived when a read arrives (`display.rs`): an application whose rule wasn't read in full in a cluster it spans is marked may be incomplete; a refused, failed or capped source beside found applications is a "May be missing" banner; with nothing found, refused and failed are their own states, and only when every source answered or isn't served is it "No applications found in what was read". Example data is core's acme workspace, six invented clusters with Argo CD and Kargo in `core-fra`.

Pod Overview opens on its cause (`resources/pane/cross_links/cause.rs`): core's `PodStatus::diagnose` reads the status alone, and a pod it can't fault shows no card. Pod Overview relationships (`resources/pane/cross_links/`) read namespace Services once and the ReplicaSet controller once, checking its UID. Runs on shares the shell’s joined node snapshot; recent warnings reuse the pane’s event watch. Owner links carry API version and UID, and custom owner kinds use discovery. Every destination uses the same object entry point; Forward opens Service Ports without starting a forward.
