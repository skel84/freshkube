# Freshkube agent guide

Freshkube is a native desktop app, built on GPUI Kit, for Talos Linux and Kubernetes clusters. It began as a fork of [talos-pilot](https://github.com/Handfish/talos-pilot) by Ken Udovic; that project's TUI, egui and Dioxus frontends were removed, and its GPUI frontend is now the application.

## Crates

```
crates/
├── talos-rs/            Talos gRPC client
├── freshkube-core/      domain logic shared by every screen, no UI types
├── freshkube-delivery/  the delivery chain (Kargo, Argo CD, builds → pods) and the applications derived from it; no UI types
├── freshkube-prefs/     the preferences files: a Dir and a typed JSON Document with an atomic save and a keyed update; std, serde and serde_json only
├── freshkube-probe/     render probes for UI tests and timing spans for the stress binary
├── freshkube-reads/     the owners of cluster reads: Loader<T, I>, one read's answer for one identity and its Tokio work; Watch<T, I>, one stream's batches for one identity
├── freshkube-logs/      the log view, LogView<S: LogSource>: retention, search, selection, follow, wrap, measured rows
├── freshkube-terminal/  the terminal view behind the pod shell: an alacritty_terminal grid drawn with GPUI
├── freshkube-ui/        the look: theme, palette, text size, ui helpers, page frame, table and graph view
├── freshkube-graph/     graph layout without GPUI: columns of boxes and the curves between them
├── freshkube-monitoring/ Grafana dashboards from the cluster's Prometheus, and the CPU and memory history charts
├── freshkube-nav/       where a link goes: ResourceLink and its requests, the objects it names and the tabs it opens on; core only, no GPUI
├── freshkube-observability/ Coroot's applications, service map, incidents, traces, profiles and logs
├── freshkube-workbench/ the shared components on invented data, one story at a time; never shipped
└── freshkube-desktop/   the GPUI Kit application (shell, screens, the log sources)
src/main.rs              the `freshkube` binary: CLI options → desktop app
```

Keep cluster logic in `freshkube-core` and presentation in `freshkube-desktop`. A screen calls core functions with real Rust types; there is no serialization boundary. Shared components live in `freshkube-ui`, which depends on neither; desktop reaches its modules by their old paths (`crate::ui`, `crate::palette`, `crate::theme`, `crate::text_size`, `crate::meters`). Monitoring is `freshkube-monitoring`, which desktop reaches as `crate::monitoring`; it sits under desktop and uses only core, `talos-rs`, `freshkube-ui`, `freshkube-probe`, `freshkube-reads` and `freshkube-prefs` (its `monitoring.json`), so Resources and Observability depend on it, never the other way. Observability is `freshkube-observability`, which desktop reaches as `crate::observability`; it uses core, `freshkube-delivery` (for `change::example::POD_HASH`), `freshkube-ui`, `freshkube-logs`, `freshkube-graph`, `freshkube-monitoring` and `freshkube-prefs` (its `coroot.json`), and names nothing of desktop: the shell turns its `ObservabilityEvent`s into navigation, and desktop's tests reach its fixture links through its `testing` feature. Reads are `freshkube-reads`, whose `Loader` the screens own, with `Watch` for a stream and `OnShow` for what showing a hidden page does; it uses only core, GPUI Kit and Tokio, and names nothing of `freshkube-ui` or desktop. Delivery is `freshkube-delivery` (core's old `delivery/` and `applications/`, moved together because they refer to each other): desktop and observability reach it as `freshkube_delivery::{change, changes, entry, join, kargo, read, source, argocd}` and `freshkube_delivery::applications`, the old `freshkube_core::delivery::…` and `freshkube_core::applications::…` tails kept after the crate name. It uses core, `talos-rs` and no UI types; core names nothing of it (`workspace::Sources` and Argo CD's in-cluster name and server stay in core's `workspace`). Its other modules are crate-private, and so is every item no other crate names; a path a screen needs is made `pub` there. Desktop's tests read its example data and parsers through the `testing` feature (`freshkube_delivery::testing`), turned on only from dev-dependencies. Navigation's identities and requests are `freshkube-nav` (`ResourceLink`, `LogsRequest`, `LogsAt`, `ShellRequest`, `ResourceIdentity`, `ObjectRef`, `DetailTarget`, the Resources pane's `Tab` and `NodeTab`); it uses core alone and names nothing of GPUI or desktop, and desktop reaches them by their old paths (`crate::resources::{ResourceLink, LogsRequest, LogsAt, ShellRequest, Tab}`, `crate::resources::model::{ResourceIdentity, ObjectRef}`, `crate::resources::detail::DetailTarget`, `crate::desktop::nodes::NodeTab`). `Page`, `Area`, `PageEvent` and `NodeRow` stay in desktop. A new page draws with `freshkube_ui::page` and `freshkube_ui::table` ([DESIGN.md](docs/DESIGN.md#in-code)).

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) describes the crate graph, the ways to be a page, the session and source, reads, events, example data and the debug knobs, each with its module path.

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

Debug builds open on a page from `FRESHKUBE_PAGE=<slug>` (`overview`, `nodes`, `health`, `resources`, `etcd`, `system-services`, `security`, `lifecycle`, `operations`, `monitoring`, `applications`, `changes` (the Applications column's Changes list, live or example), `settings`). `FRESHKUBE_PAGE=node-logs` opens the first responding node on its Logs tab. Debug fixture checks also take `node-overview`, `pod-overview`, `workload-logs` (payments/api's logs in the dock), `search`, `application` (acme's `checkout` on its application page), `change` (checkout's Freight `wonky-otter` on the change page, prod-ams's promotion gate selected), `kubernetes-only` and `maintenance`, which opens maintenance mode on an invented node (`maintenance/example.rs`) that walks itself to the configuration review and refuses to go further; with `--fixture`, `monitoring` answers its dashboard from example data at once, so a capture shows it. Their entry points live in `desktop/startup/`. The other knobs are in the table below.

| Knob | Values | What it does | Read by |
|------|--------|--------------|---------|
| `FRESHKUBE_PAGE` | a page slug or fixture check above | Opens on that page. | debug and stress builds |
| `FRESHKUBE_KIND` | a kubectl key (`pods`, `deployments.apps`, `nodes`, …; see `resources/navigation.rs`); with `--fixture` also the example custom kinds, such as `certificates.cert-manager.io` | Opens that Kubernetes kind on the Resources page. | debug and stress builds |
| `FRESHKUBE_THEME` | `light`, `dark` | Selects appearance without changing saved settings (also the maintenance example's). | debug and stress builds |
| `FRESHKUBE_SIDEBAR` | `collapsed`, `expanded` | Starts the sidebar in that state. Anything else is ignored. | debug and stress builds |
| `FRESHKUBE_DASHBOARD` | a path | A dashboard of the Monitoring folder, by path. | debug and stress builds |
| `FRESHKUBE_WINDOW_SIZE` | `1280x880`, `760x560` | Selects the window bounds without changing saved settings. | debug and stress builds |
| `FRESHKUBE_MOTION` | `reduced`, `full` | Overrides the OS's reduced-motion setting. | debug and stress builds |
| `FRESHKUBE_TEXT_SIZE` | `12`, `13`, `14`, `16`, `18`, `20` | Starts at that text size without saving it; 13 is the default. | debug builds |
| `FRESHKUBE_LOG_SEARCH` | text | With `node-logs` or `observability-application`, types that text into the log's search and, once the example lines arrive, steps to the first match (the application page opens on its Logs report, scrolled to the log). | debug and stress builds |
| `FRESHKUBE_ETCD` | `lost`, `single`, `alarms` | With `--fixture`, reshapes etcd's example: a lost quorum, a single member, or five alarms. | debug builds |
| `FRESHKUBE_APP_MENU` | `button` | Shows the Linux menu button on any platform. | debug builds |
| `FRESHKUBE_APPLICATIONS` | `single`, `partial`, `refused`, `unserved`, `failed`, `capped`, `spread`, `not-found`, `unlabelled`, `stages`, `workloads`, `unreachable`, `argo-refused`, `destination` | With `--fixture`, reshapes the Applications example: `core-fra` alone, Kargo refused in `cicd-fra`, every source refused, nothing served, no cluster answering, Argo CD's Applications capped, Argo CD found in five namespaces with two past the cap, Argo CD's namespace not found, nothing labelled as Argo CD's so it is read in `argocd`, its default, `checkout`'s Stages not answering, or Deployments refused in `prod-lon`, `cicd-fra` unreachable, Argo CD refused in `core-fra`, or the destination `dev-fra` unreachable; the last three reshape the change page and the Changes list too (delivery's `change::example::Shape`). | debug and stress builds |
| `FRESHKUBE_FIXTURE_HOLD` | `talos`, `lists`, `coroot`, `monitoring`, `applications`, `all` | Keeps example data from answering so pages stay on their loading rows: `talos` holds the Talos overview and the Kubernetes summary (Overview, Nodes, System services, Operations), `lists` the Resources lists, `coroot` Observability's applications, incidents and traces, `monitoring` the dashboard's panels, `applications` the Applications list, `all` every one; `scripts/stress.sh … loading <page>` sets it. | debug and stress builds |

The app's binaries parse all of these knobs once into `DebugOptions` (`crates/freshkube-desktop/src/debug_options.rs`) and the pages are handed their value; none reads the environment ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#where-the-debug-knobs-are-parsed)). A unit test there fails when this table and `DebugOptions` name different knobs. Use them with `scripts/smoke.sh` (see [Smoke tests](#smoke-tests)).

## Cluster safety

- Read-only by default. Checks against a real cluster list, watch and read; they never create, change or delete resources.
- Pod exec and port forwarding are the two exceptions, and each starts only from an explicit action.
  - Pod exec (`resources/exec/`) creates a `pods/exec`. The app starts one only from an explicit Start, and a live check runs one only on a pod and container the user named, with harmless commands ([docs/POD_EXEC.md](docs/POD_EXEC.md)).
  - Port forwarding (`resources/forwards/`) creates a `pods/portforward`. It changes no objects, but it reaches whatever the port serves. The app starts one only from an explicit Forward, and a live check forwards only a pod and port the user named ([docs/PORT_FORWARD.md](docs/PORT_FORWARD.md)).
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
- The shell activates and periodically refreshes only the visible `ScreenPanel`; it sends source updates to every page in its one map, the screens first. Every page has a `ScreenHandle` in the shell's one map (`Pilot.pages`, keyed by `PageId`) and hears `ScreenPanel::set_visible` through it: whenever the page or the node pane's tab changes, a roster change that closes the pane or drops its tab included, `Pilot::sync_visible` tells the pages that show `true` and every other `false`, in key order, and a page told what it already knows does nothing. Visibility nests: the page that shows hears `true`, and while Nodes' pane shows a Talos screen in its selected tab, that screen hears `true` inside Nodes, which keeps hearing `true` too (`Pilot::shown_pages`). Nothing else hears `true` at once. A page drops its reads on hide there: Resources, Monitoring, Observability, Applications, Nodes' read of its nodes' use and Settings' Delivery sources do. The Talos screens keep the default, which does nothing, so navigation alone does not cancel a `Loader` until it has its hook (#582). Operations reads its preview and audit only from `activate`, so a hidden Operations reads nothing. New page-owned reads should start only while shown; preserve the existing log and user-started session lifetimes below. Dock log tabs are user-started: each reads while it is open, on any page, and stops when it closes.
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

- Dialogs animate on the real clock, and advancing the test clock does not finish them. The shared test setup in `desktop/testing.rs` turns on reduced motion (`cx.set_reduce_motion(true)`); two confirmation-dialog tests failed every run without it. Do the same in any new test harness.
- `window.render_frame` bypasses view caches. Don't use it to prove that a view was or was not redrawn; use the render probes (`probe::hit`, `probe::count`) from `freshkube-probe`. They count only with its `counting` feature, which a crate turns on from its dev-dependencies, as `freshkube-desktop` does.
- Element snapshots can't tell whether a button is disabled. Assert the outcome instead: click it and check that nothing changed.
- A page's event reaches the shell, and the shell's push reaches a page, only after the current update ends. A test that clicks something a page turns into an event, or changes what the shell hands a page, calls `cx.run_until_parked()` outside `update_window` before asserting the result.
- Setting an input's value from code does not emit its change event. Type into it with real input events (`window.input`) when the change handler matters.
- Tests write only under a fresh temporary directory, never into the crate or the user's home.
- A test that waits on work in a real Tokio runtime or socket (a fake server, a worker thread) waits through `freshkube_ui::real_io` (`RealIo::wait_for_io`, `until_io`), on the drawn element, the fake's recorded request or the state it then asserts: `wait_for`'s timeout runs on the test clock and can pass before the real work answers, and `run_until_parked` alone can return before it does. The helper's one allowance is 30 s of real time.
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

### Using Snapshot<T, I, E> and Loader<T>

Desktop's [state::Snapshot](crates/freshkube-desktop/src/state.rs), core's [`snapshot::Snapshot`](crates/freshkube-core/src/snapshot.rs) with a default identity, retains the last successful value for one identity and rejects superseded request generations. Its default identity is `AppliedConfig`; node reads use `Target`, and provider reads use their own identity. Its error type defaults to core's `Failure`; a read that still words its own errors names `String` as the third parameter (`Loader` does), until it moves to `Failure`. This small example uses the same identity labels as its unit tests:

```rust
use crate::state::Snapshot;

let mut state = Snapshot::<u32, &str, String>::default();
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

For ordinary inspection reads, [screens::Loader](crates/freshkube-desktop/src/screens/loader.rs) is `freshkube-reads`'s [`Loader<T, Target>`](crates/freshkube-reads/src/loader/mod.rs), which owns `Snapshot<T, Target>`, `Option<OwnedJob>` and `Option<Task<()>>`, with `SCREEN_DEADLINE` plugged in. Use `load(target, &runtime, what, work, slot, cx)` for Tokio work, `resolve(target, result)` for fixtures, and `reset()` when `set_source` detects a changed target. [The etcd example](docs/STATE_LIFETIMES.md#ordinary-screen-reads) shows the real call. Render reads `data()`, `error()` and `is_loading()`; retain data alongside a refresh failure banner and its timestamps. Call `cx.notify()` after changes outside render; derive display collections when data changes.

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

// In freshkube-desktop - any HasHealth value maps to a tone and icon for a tag
let (tone, icon) = ui::health_tone(health.health());
let tag = ui::tag(tone, icon, "Healthy", cx);
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

## Adding a screen

1. Pick the page's type from [DESIGN.md](docs/DESIGN.md#page-types) (table page, dashboard, canvas or detail pane) and build it from that type's shared components: the page header, the table, group rows, status glyphs and states. Don't draw your own title, table, glyphs or radii; `scripts/check-style.sh` fails on them, and its allowlist is empty and only shrinks, so there is no exception to add. A table page's line (context, counts, state, refresh time) goes in the status bar through `ScreenPanel::status` and a `status::Segment`, not under its header. Its row actions act on the selection, from the toolbar, keys and the shared row context menu (`TableSource::menu_focus` and `row_menu`), never from a button on the row. A selection's details go in the shared Inspector (`freshkube_ui::inspector`, change 9 in DESIGN.md), beside the table or stacked under it, not in a card of its own.
2. Add cluster pages to `Page` (`desktop/pages.rs`): `ALL`, its slug, its `Area` and column row, and its shortcut. Add inspection views to `PageId` and `PageId::SCREENS`, the screen factory and `Page::id` or `NodeTabs::screen`, according to their scope.
3. Implement `ScreenPanel` in `screens/<name>.rs`, or `screens/<name>/mod.rs` once it mixes concerns. The screen owns its requests (`OwnedJob`), its data and its offline example data.
4. Put cluster logic in `freshkube-core` and keep the screen to presentation.
5. Add UI tests for loading, empty, failure and the main interactions. A table page also calls `desktop::layout_check::assert_table_page` (`freshkube_ui::layout_check`, behind its `testing` feature, as is `settle_header`), which measures its header, rows, group rows, edge-to-edge frame and title against Pods' sizes; a page with several lists calls its halves, `assert_edge_frame` and `assert_table`, a padded page of cards, such as a dashboard, `assert_page_frame`, and a screen embedded under a tab that names it (`PageHeader::untitled`), `assert_untitled_edge_frame`. A page with an Inspector also calls `assert_inspector`, beside and stacked. Then smoke it beside Pods at the same size, theme and text size ([Smoke tests](#smoke-tests)).

Diagnostic checks follow the reliability rules below: find the source of truth first, use the `DiagnosticCheck` constructors, provide an actionable fix where possible, and return `unknown` rather than failing when data is unavailable.

## Quick reference

### Do

- Check actual system state (files, APIs)
- Use desktop `Snapshot<T, I, E>` and, for ordinary inspection reads, `Loader<T>`; preserve feature-specific owners
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

## Features

Behaviour that is not a rule lives in one doc per feature, each starting with its module paths and the tests that prove it ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#feature-docs)).

- Resources, the dock, shells and ports (`resources/`, `desktop/dock/`): [docs/features/resources.md](docs/features/resources.md). Rules kept here: a dock log tab reads while it is open, on any page, and nothing else reads out of sight; Resources reads only while its page shows; Secret values never reach the detail pane's document, search or a link's resolution (metadata-only reads).
- Applications, the Changes list and the change page (`applications/`): [docs/features/applications.md](docs/features/applications.md).
- The Nodes workspace and its node pane (`desktop/nodes/`): [docs/features/nodes.md](docs/features/nodes.md).
- The frame, the Overview and the Kubernetes summary session (`desktop/shell/`, `presentation/`, `kubernetes_summary/`): [docs/features/shell.md](docs/features/shell.md).

Monitoring (`freshkube-monitoring`'s `page/`, [docs/MONITORING.md](docs/MONITORING.md)) is its own rail area; its column lists the built-in dashboards and the user's folder of Grafana JSON (Settings → Dashboards folder, saved in `monitoring.json` beside the preferences). The shell owns the page and tells it when it shows; a hidden page finds nothing, resolves nothing and asks no panel. Showing it finds Prometheus for the context (the remembered Service first), resolves the variables, then asks only the panels in or near the viewport. Every change that asks again takes a new generation, and an older answer is dropped; hiding drops every request and the auto-refresh timer. Another source rebuilds the dashboard, so one cluster's answers never show as another's. Panels draw only Console colours. Timeseries also draw deploy and node markers (`monitoring/page/markers.rs`), read per generation from ReplicaSet metadata, Nodes' Ready condition, Node events and, with Talos, boot times; the page filters them by its toggles and the namespace and node variables before handing them to the panels. A pod's Overview and the node pane show CPU and memory over the last hour (`monitoring/history/`) from the Prometheus the page confirmed or remembers for the context (`MonitoringPage::history`, pushed on `MonitoringEvent::History`); without one they show and read nothing. A pod's chart reads while its drawer shows Details, at any scroll, and the node pane's while its Overview shows.
