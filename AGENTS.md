# Freshkube agent guide

Freshkube is a native desktop app, built on GPUI Kit, for Talos Linux and Kubernetes clusters. It began as a fork of [talos-pilot](https://github.com/Handfish/talos-pilot) by Ken Udovic; that project's TUI, egui and Dioxus frontends were removed, and its GPUI frontend is now the application.

## Crates

```
crates/
├── talos-rs/            Talos gRPC client
├── freshkube-agent/     the agent in the dock: the ACP client, the opencode profile and its lock-down, the transcript; no GPUI, not yet used by desktop
├── freshkube-cli/       the read commands of the binary: check and get, JSON and exit codes; no GPUI
├── freshkube-core/      domain logic shared by every screen, no UI types
├── freshkube-agent/     the agent's loopback read endpoint, its session token and the saved per-cluster access; no GPUI
├── freshkube-followed/  what the user follows (a pod, a change, an incident): the app-wide owner, its rules, the toasts and the bell; no desktop types
├── freshkube-delivery/  the delivery chain (Kargo, Argo CD, builds → pods) and its applications; no UI types
├── freshkube-prefs/     the preferences files: a typed JSON Document per file, saved atomically
├── freshkube-probe/     render probes for UI tests and timing spans for the stress binary
├── freshkube-reads/     the owners of cluster reads: Loader<T, I> for one read, Watch<T, I> for one stream
├── freshkube-resources/ the Resources page: kind lists kept live by watches, the detail pane, pod shells and port forwards
├── freshkube-applications/ the Applications page, an application's page, the Changes list and the change page
├── freshkube-logs/      the log view, LogView<S: LogSource>
├── freshkube-terminal/  the terminal behind the pod shell (alacritty_terminal)
├── freshkube-ui/        the look: theme, palette, text size, ui helpers, page frame, table and graph view
├── freshkube-graph/     graph layout without GPUI: columns of boxes and the curves between them
├── freshkube-monitoring/ Grafana dashboards from the cluster's Prometheus, and the CPU and memory history charts
├── freshkube-nav/       where a link goes: ResourceLink, its requests, objects and tabs, and the deep link's closed ViewState; no GPUI
├── freshkube-observability/ Coroot's applications, service map, incidents, traces, profiles and logs
├── freshkube-workbench/ the shared components on invented data; never shipped
└── freshkube-desktop/   the GPUI Kit application (shell, screens, the log sources)
src/main.rs              the `freshkube` binary: CLI options → desktop app
```

Keep cluster logic in `freshkube-core` and presentation in `freshkube-desktop`. A screen calls core functions with real Rust types; there is no serialization boundary. A crate depends only on what its arrow in [ARCHITECTURE.md](docs/ARCHITECTURE.md#crates-and-their-direction) lists: shared components live in `freshkube-ui`, which depends on neither core nor desktop; core and `freshkube-delivery` have no UI types, and core names nothing of delivery; the feature crates sit under desktop and never name it, and Monitoring never names Resources or Observability; `freshkube-reads` and `freshkube-nav` name nothing of `freshkube-ui` or desktop, and nav has no GPUI. In `freshkube-delivery`, every module and item no other crate names stays crate-private; a path a screen needs is made `pub` there. A crate's `testing` feature is turned on only from dev-dependencies. Desktop reaches the moved crates by their old paths ([ARCHITECTURE.md](docs/ARCHITECTURE.md#crates-and-their-direction)). A new page draws with `freshkube_ui::page` and `freshkube_ui::table` ([DESIGN.md](docs/DESIGN.md#in-code)).

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) describes the crate graph, the ways to be a page, the session and source, reads, events, example data and the debug knobs, each with its module path.

### Module layout

Split code by concern, not by line count. A long file with one tight concern is fine; a file whose parts share private state only because they sit together is not.

- **Turn a module into a directory when it mixes concerns,** or when its code, not counting tests, passes about 1,500 lines. Use `foo/mod.rs` with child modules, as `desktop/`, `resources/` and `logs/` do; don't mix in the `foo.rs` + `foo/` style.
- **Keep the shared type in `mod.rs` and spread its `impl` blocks over the children.** A child module sees its ancestors' private items, so `freshkube-logs`'s `view.rs` and `measure.rs` use `LogView`'s fields without widening them. A method one child calls from another needs `pub(super)`; sibling modules don't see each other's private items.
- **Every log view is `LogView<S: LogSource>`** from `freshkube-logs`. A new kind of log adds a source; it never copies the view. A source reaches the view only through the crate's `source_api.rs`, so a source's own methods live in an extension trait that callers import ([ARCHITECTURE.md](docs/ARCHITECTURE.md#log-views)).

- **Keep small unit tests inline** in `#[cfg(test)] mod tests { … }`. When a module's tests are large, as UI tests usually are, put them in a sibling `tests.rs` (`#[cfg(test)] mod tests;`). It stays a child module, so the tests keep their access to private fields.
- **Keep functions short.** A `render` that runs to hundreds of lines is harder to follow than a long file; split it into `render_*` helpers.
- **Split a file when a step works on it,** not in a sweeping pass. Make the split its own commit with no logic changes, so the unchanged tests prove it.

[docs/ROADMAP.md](docs/ROADMAP.md) sets the order of work and its ground rules. When a step lands, move it to Done with its commit in the same change. [docs/REFERENCES.md](docs/REFERENCES.md) maps the reference checkouts and the source-reuse policy. [docs/DESIGN.md](docs/DESIGN.md) holds the chosen look: tokens, type, status glyphs, the app frame and table rules.

Each PR adds a release-note bullet in `changelog.d/<section>/<slug>.md`; never edit `CHANGELOG.md`'s Unreleased entries in a feature PR. Use `coroot` or `other-changes`, give the file a unique slug, and run `scripts/changelog.sh check`. Only release preparation collects the fragments into the changelog. [changelog.d/README.md](changelog.d/README.md) describes the format and adding a section.

## Build, run and test

`talos-rs` generates gRPC code with `protoc`. Install it (`brew install protobuf`) or point `PROTOC` at a binary.

Each worktree builds into its own `target/`; leave `CARGO_TARGET_DIR` unset, and don't share one build directory between worktrees, or one can link another branch's code without rebuilding it ([why](docs/BUILDING.md)). When the disk runs low, delete the `freshkube_desktop-*.o` and `freshkube_core-*.o` objects in `target/debug/deps`; that rebuilds nothing ([how](docs/BUILDING.md#when-the-disk-runs-low)).

```sh
cargo build
cargo run -- --fixture            # synthetic example data, no credentials or cluster
cargo run -- --config ~/.talos/config --context <name>
cargo run -- --kubernetes-only --kubeconfig <file> --kube-context <name>   # no talosconfig
cargo run -- --fixture 'freshkube://open?page=etcd'   # opens a link once the window is up (also --open-link)
cargo run -p freshkube-workbench   # the shared components on invented data, one story at a time; not shipped
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
scripts/check-style.sh            # pages use the shared components; see DESIGN.md "Checks"
```

For the inner loop, use `cargo test -p <crate> --lib <filter>` while iterating on a library crate, then run `cargo clippy -p <crate> --all-targets -- -D warnings` once at the end. Save the full workspace test/clippy/fmt and style checks above for before submitting the PR for review; all remain required, and clippy and fmt must pass clean.

CI, app bundles and releases are described in [docs/MACOS_PACKAGING.md](docs/MACOS_PACKAGING.md#ci): pull requests run the checks above, merges to `main` build bundles, and a `v*` tag drafts a release from them.

Debug builds open on a page from `FRESHKUBE_PAGE=<slug>` (`overview`, `nodes`, `health`, `databases`, `resources`, `etcd`, `system-services`, `security`, `lifecycle`, `operations`, `monitoring`, `alerts`, `rules`, `silences` (the Monitoring column's three alert pages, live or example), `applications`, `changes` (the Applications column's Changes list, live or example), `waiting` (the Applications column's Waiting to ship, live or example), `settings`, which opens Clusters, and each section of Settings, `settings-general`, `settings-connection`, `settings-monitoring`, `settings-coroot`, `settings-notifications`, `settings-audit-log`, `settings-about`, `settings-clusters`, `settings-destinations` and `settings-sources`). `FRESHKUBE_PAGE=node-logs` opens the first responding node on its Logs tab. Debug fixture checks also take `node-overview`, `pod-overview`, `workload-logs` (payments/api's logs in the dock), `search`, `application` (acme's `checkout` on its application page), `change` (checkout's Freight `wonky-otter` on the change page, prod-ams's promotion gate selected), `kubernetes-only` and `maintenance`, which opens maintenance mode on an invented node (`maintenance/example.rs`) that walks itself to the configuration review and refuses to go further; with `--fixture`, `monitoring` answers its dashboard from example data at once, so a capture shows it. Their entry points live in `desktop/startup/`. The other knobs are in the tables below: what a page opens on and shows, the window, and the example data. They are one table, split in three for length.

| Knob | Values | What it does | Read by |
|------|--------|--------------|---------|
| `FRESHKUBE_PAGE` | a page slug or fixture check above | Opens on that page. | debug and stress builds |
| `FRESHKUBE_KIND` | a kubectl key (`pods`, `deployments.apps`, `nodes`, …; see `resources/navigation.rs`); with `--fixture` also the example custom kinds, such as `certificates.cert-manager.io` | Opens that Kubernetes kind on the Resources page. | debug and stress builds |
| `FRESHKUBE_DASHBOARD` | a path | A dashboard of the Monitoring folder, by path. | debug and stress builds |
| `FRESHKUBE_LOG_SEARCH` | text | With `node-logs` or `observability-application`, types that text into the log's search and, once the example lines arrive, steps to the first match (the application page opens on its Logs report, scrolled to the log). | debug and stress builds |
| `FRESHKUBE_ETCD` | `lost`, `single`, `alarms` | With `--fixture`, reshapes etcd's example: a lost quorum, a single member, or five alarms. | debug builds |

| Knob | Values | What it does | Read by |
|------|--------|--------------|---------|
| `FRESHKUBE_THEME` | `light`, `dark` | Selects appearance without changing saved settings (also the maintenance example's). | debug and stress builds |
| `FRESHKUBE_SIDEBAR` | `collapsed`, `expanded` | Starts the sidebar in that state. Anything else is ignored. | debug and stress builds |
| `FRESHKUBE_WINDOW_SIZE` | `1280x880`, `760x560` | Selects the window bounds without changing saved settings. | debug and stress builds |
| `FRESHKUBE_MOTION` | `reduced`, `full` | Overrides the OS's reduced-motion setting. | debug and stress builds |
| `FRESHKUBE_TEXT_SIZE` | `12`, `13`, `14`, `16`, `18`, `20` | Starts at that text size without saving it; 13 is the default. | debug builds |
| `FRESHKUBE_APP_MENU` | `button` | Shows the Linux menu button on any platform. | debug builds |

| Knob | Values | What it does | Read by |
|------|--------|--------------|---------|
| `FRESHKUBE_APPLICATIONS` | `single`, `partial`, `refused`, `unserved`, `failed`, `capped`, `spread`, `not-found`, `unlabelled`, `stages`, `workloads`, `unreachable`, `argo-refused`, `destination`, `image-only`, `causes`, `promoting`, `github-limited`, `github-refused`, `github-offline`, `outcome-<name>` | With `--fixture`, reshapes the Applications example: `core-fra` alone, Kargo refused in `cicd-fra`, every source refused, nothing served, no cluster answering, Argo CD's Applications capped, Argo CD found in five namespaces with two past the cap, Argo CD's namespace not found, nothing labelled as Argo CD's so it is read in `argocd`, its default, `checkout`'s Stages not answering, or Deployments refused in `prod-lon`, `cicd-fra` unreachable, Argo CD refused in `core-fra`, or the destination `dev-fra` unreachable, or `checkout`'s Freight naming an image and no commit so its build is joined by the image digest, or GitHub's pull requests not readable (`github-limited`: the rate limit is reached, `github-offline`: no answer, both keep the pull requests read before, marked stale, and under `github-limited` Waiting to ship also shows its two newest as `not read (rate limit)`; `github-refused`: the token is refused, so none is shown), with the banner of what may be missing saying why, and, on the change page, the pull request line (`Pull request #418 · Merged · …`) reading as kept and stale or as refused; `unreachable`, `argo-refused`, `destination`, `image-only` and the three `github-` shapes reshape the change page and the Changes list too (delivery's `change::example::Shape`); `causes` gives `checkout`'s change page all four causes of an Unknown at once (Waiting on prod-ams's gates, Can't read on its pods, Not found on the policy, Not set up on the stage Application); `promoting` has a Promotion of `wonky-otter` to `prod-ams` running, made by someone else, so that Stage's Promote is Abort promotion (the Stage actions' dialogs open in every shape, with a runner that sends nothing); `outcome-<name>` makes every Deployment hop of the change page show one outcome of what Coroot found after its deploy (`found`, `ends-early`, `no-revision`, `no-application`, `no-project`, `no-connection`, `not-linked`, `no-project-for-entry`, `refused`, `refused-window`, `no-data`, `failed`, `not-deployment`, `not-looked`, `stale`; `applications/change/outcome/example.rs`), where the other shapes show what Coroot's example keeps for the hop's hash. | debug and stress builds |
| `FRESHKUBE_METRICS_LABELS` | `capped` | With `--fixture`, Settings' Check entries gets an example answer cut at its cap, so the capped wording shows. | debug and stress builds |
| `FRESHKUBE_DATABASES` | `not-installed`, `refused`, `unsupported`, `failed`, `empty`, `backup-refused`, `partial`, `old`, `backup-failed`, `no-primary`, `stale`, `no-backup`, `no-schedule`, `suspended`, `cert-expiring`, `switchover-stuck`, `mariadb`, `coroot-none`, `coroot-not-found`, `coroot-no-report`, `coroot-unreachable`, `coroot-disagree`, `lag-high`, `scheduled-only`, `engine-refused` | With `--fixture`, reshapes the Databases inventory using invented records. The default has a CloudNativePG and a MariaDB database; `mariadb` keeps only the MariaDB one, `cert-expiring` and `switchover-stuck` give one a certificate under 14 days and a switchover stuck for 25 minutes, and `scheduled-only` gives the MariaDB a schedule and no run Backup, `engine-refused` has the MariaDB operator refuse after a good read so its rows are kept as retained, and the `coroot-` shapes give Coroot's answer: no link, application not found, found without a report, unreachable (the last answer dimmed), or disagreeing with the operator on WAL archiving. `lag-high` gives the replicas 74 s of lag. | debug and stress builds |
| `FRESHKUBE_FIXTURE_HOLD` | `talos`, `lists`, `coroot`, `monitoring`, `applications`, `all` | Keeps example data from answering so pages stay on their loading rows: `talos` holds the Talos overview and the Kubernetes summary (Overview, Nodes, System services, Operations), `lists` the Resources lists, `coroot` Observability's applications, incidents and traces, `monitoring` the dashboard's panels and the alert reads, `applications` the Applications list, `all` every one; `scripts/stress.sh … loading <page>` sets it. | debug and stress builds |
| `FRESHKUBE_ALERTS` | `firing`, `quiet`, `not-found`, `refused`, `several`, `no-label`, `stale`, `rules-only` | With `--fixture`, reshapes the Alerts, Rules and Silences example and the session alert read: `firing` (the default) alerts of every state with a broken rule, `quiet` a current read with nothing active, `not-found` no Alertmanager or vmalert, `refused` the proxy refusing the read, `several` another Alertmanager answering beside the one read, `no-label` a central store whose alerts carry no cluster label (nothing is shown), `stale` a failed refresh over an earlier read, `rules-only` PrometheusRule objects with no evaluator, so every health is Unknown (`monitoring/alerts/example.rs`). | debug and stress builds |
| `FRESHKUBE_TIMELINE` | `busy`, `quiet`, `partial`, `short-events`, `none-read` | With `--fixture`, reshapes the Timeline tab of an application's page: `busy` (the default) `checkout` with a promotion, a sync, a deploy, an incident, Warning events, a restart and a node change, `quiet` every source read with nothing in the window, `partial` Coroot refused and Argo CD not installed, `short-events` the cluster keeping Events for 15 minutes, `none-read` no source answering (`freshkube-delivery`'s `timeline/example.rs`). | debug and stress builds |
| `FRESHKUBE_SERVICES` | `mixed`, `healthy`, `nothing`, `draining`, `legacy`, `pending`, `no-selector`, `external-name`, `refused`, `services-refused` | With `--fixture`, shapes what the Service pane's published endpoints and a workload's traffic chain read: `mixed` (the default) ready, draining, not ready and unrouted endpoints with a pod of each unpublished reason, `healthy` two ready, `nothing` a selector that matches a pod and publishes no endpoint, `draining` one ready and one draining, `legacy` the EndpointSlice API not served so the legacy Endpoints answer, `pending` a LoadBalancer with no ingress address, `no-selector` and `external-name` a Service read as what it is, `refused` the endpoint reads refused (unknown, not zero), `services-refused` the Services list refused (a workload's chain only). Open with `FRESHKUBE_KIND=services` or `deployments.apps` (`freshkube-resources`' `example/traffic.rs`). | debug and stress builds |
| `FRESHKUBE_FOLLOWS` | `bell`, `states`, `quiet`, `hidden`, `full`, `unwatched`, `grouped` | With `--fixture`, seeds the followed items and the bell: `bell` three follows with New and Earlier, `states` one in each state, `quiet` quiet hours with a held notice and a failure that came through, `hidden` the window opens on the protected `prod-ams`, so Hide details resolves to On, and a notice is told at once with the summary alone, `full` the 20 follows so the next is refused, `unwatched` follows of a cluster that is not open (Not watching), `grouped` three landings told in a row, the first at once and the other two as one notice when the 2-minute window ends; without it a follow plays a small story. | debug and stress builds |
| `FRESHKUBE_TABS` | `clusters`, `missing` | With `--fixture`, opens the view tabs on invented clusters: `clusters` three tabs each bound to another example cluster (`dev-fra`, `prod-ams`, `stage-fra`, so the strip shows the "cluster ·" prefix, each level's glyph and the caps), `missing` a second tab bound to an entry the file does not list (a dashed ring and "Choose a cluster"). | debug and stress builds |
| `FRESHKUBE_LINKS` | `refused`, `not-found`, `not-served`, `replaced`, `several`, `none`, `ca-only`, `unchecked` | With `--fixture`, shapes deep links. The first four force the outcome of every link opened (`--open-link`, ⌘K's Open link): the read is refused, the object isn't found, the kind isn't served, or the name belongs to another object now, so the pane's slot shows it. The last four reshape how the example clusters identify themselves: `dev-fra` and `stage-fra` are one cluster to a link (`several`, so the picker asks); no example cluster is the one a link was made in (`none`); each answers on its CA alone (`ca-only`, so every match is marked CA only); nothing is checked, not even the open cluster (`unchecked`). | debug and stress builds |
| `FRESHKUBE_CRASH` | `notice`, `panic`, `task` | `notice` shows the "last session ended unexpectedly" notice, with an invented crash when no real one was found (with `--fixture` it needs no marker file); `panic` forces a panic on the main thread and `task` one in a Tokio task, three seconds after the window opens, so the next start shows the notice for real (`freshkube_core::crash_log`, [docs/features/settings.md](docs/features/settings.md#about-and-the-crash-log)). | debug builds |
| `FRESHKUBE_CRASH_SEED` | any text | A credential-shaped value that the startup log line (`token=…`) and the forced `FRESHKUBE_CRASH` panic (`password=…`) carry, so a smoke can grep every log and the copied diagnostics for it and find nothing; the redaction is what keeps it out. | debug builds |
| `FRESHKUBE_TEAM` | `none`, `joined`, `update`, `trust`, `failed` | With `--fixture`, shapes Settings › Team setup (`settings-team`): `none` is not in a team (the default), `joined` is joined at the newest version with two ignored keys and two overrides of your own, `update` has a newer version waiting with no trust change, `trust` has one that changes a signing key, two addresses and a looser protection level, and `failed` has the last fetches failing since 30 hours ago while the applied version stays. | debug and stress builds |

The app's binaries parse all of these knobs once into `DebugOptions` (`crates/freshkube-desktop/src/debug_options.rs`) and the pages are handed their value; none reads the environment ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#where-the-debug-knobs-are-parsed)). A unit test there fails when these tables and `DebugOptions` name different knobs. Use them with `scripts/smoke.sh` (see [Smoke tests](#smoke-tests)).

## Cluster safety

- **The safety suite is the named list of what must never break** ([docs/SAFETY.md](docs/SAFETY.md)): ten guarantees, each proved by tests named `safety_gNN_…` and run together with `cargo test --workspace --locked safety_g`, as their own CI step. A change that adds a write, exec, forward or reveal path adds its guarantee to the suite in the same change, and proves the test fails with its guard removed.
- **An agent's checks stay read-only.** A check against a real cluster lists, watches and reads; it never creates, changes or deletes resources, except an action the maintainer names, on an object the maintainer names as disposable. The app's write model below is for the people who use the app. It does not widen what an agent may do: a cluster's `open` or `guarded` level is not permission for an agent.
- **The app's writes follow the write model** ([docs/WRITE_MODEL.md](docs/WRITE_MODEL.md)):
  - Every cluster entry has a protection level in `freshkube.yaml`: `read-only`, `protected`, `guarded` or `open`. An entry with none is `guarded`. A person sets a level; it is never inferred from a context's name.
  - Every write has a risk tier: session, reversible, spec change or destructive. The action declares it in core, not the page. The tier and the level together decide what the user must do before the write is sent.
  - Every change to a Kubernetes object goes through core's `write` module, which is where the model is enforced: access review, the UID and resourceVersion check, the server-side dry run, server-side apply with field manager `freshkube` and no forced takeover, the observed outcome and the audit log. A page describes an action and draws what core answers; it never builds a write request, skips a requirement or decides a level.
  - The level is always shown, in the header and the status bar.
  - A submitted write outlives its page, is never replayed and is counted once ([docs/STATE_LIFETIMES.md](docs/STATE_LIFETIMES.md)).
- A follow only reads (`freshkube-followed`): a watch on one pod or one Kargo project's Stages, and Coroot's incidents once a minute, with the user's own access. It notifies and changes nothing.
- Pod exec and port forwarding are the session tier. They change no object through the write module and keep their own owners, and each starts only from an explicit action.
  - Pod exec (`resources/exec/`) creates a `pods/exec`. The app starts one only from an explicit Start, and a live check runs one only on a pod and container the user named, with harmless commands ([docs/POD_EXEC.md](docs/POD_EXEC.md)).
  - Port forwarding (`freshkube-resources`' `forwards/`) creates a `pods/portforward`. It changes no objects, but it reaches whatever the port serves. The app starts one only from an explicit Forward, and a live check forwards only a pod and port the user named ([docs/PORT_FORWARD.md](docs/PORT_FORWARD.md)).
- Operations, a service's Restart and Diagnostics' Apply fix keep their own preflight and confirmation, and a `read-only` cluster refuses them. Maintenance mode keeps its own rules and is not governed by the level: it isn't tied to a cluster entry, since it works on a node that has no cluster yet.
- **A deep link only navigates** ([docs/features/deep-links.md](docs/features/deep-links.md)). It is parsed into closed, bounded types (`freshkube_nav::Link`), carries identities only and no credential, and never opens a dialog, starts a shell or a forward, reaches Operations or maintenance, or moves focus onto a button. The object it names is read with the reader's own access, and a context is never switched unasked.
- Never run Operations or maintenance actions (drain, reboot, upgrade, apply config, reset) during verification unless the user asked for that specific action on a node they named as disposable.
- Use the context the user chose. Never fall back to whatever kubeconfig or talosconfig context is current.
- Never print or commit credentials: talosconfig and kubeconfig contents, tokens, Secret values, keys. Redact them in logs, screenshots, evidence and the audit log.

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

- The shell activates and periodically refreshes only the visible `ScreenPanel`; it sends source updates to every page in its one map, the screens first, and tells every page whether it shows through `ScreenPanel::set_visible` ([who drops what on hide](docs/STATE_LIFETIMES.md#read-job-and-delivery-ownership)). New page-owned reads should start only while shown; preserve the existing log and user-started session lifetimes below. Dock log tabs are user-started: each reads while it is open, on any page, and stops when it closes.
- The shell's Kubernetes summary is session-owned: it runs on every page, and selecting a Talos node does not replace the session ([docs/features/shell.md](docs/features/shell.md)).
- Followed items are the app's third user-started read lifetime beside dock log tabs and forwards: `freshkube_followed::Followed` is app-wide like `ForwardList`, a follow reads from the moment the user turns it on until they stop it, on any page and with the window closed, and quitting stops every read. At most 20 follows, one watch per object or Kargo project, and Coroot once a minute. A follow that can't be read shows "Not watching", once, never as a failure. Follows are saved in `follows.json` beside the preferences ([STATE_LIFETIMES.md](docs/STATE_LIFETIMES.md#followed-items)), never in `freshkube.yaml`.
- The alert read is session-owned beside it ([docs/features/monitoring.md](docs/features/monitoring.md#alerts-rules-and-silences)): one read of the active alerts per live session, every 60 s and bounded, started only once discovery found an Alertmanager or vmalert. It runs on every page, stops on a context or session change with nothing from the old identity publishing, and says Unknown, never "no alerts", when its source wasn't found, is stale or refused, or (central) lacks the cluster label. The Alerts, Rules and Silences pages read only while shown.
- Operations mutations have a separate lifetime: a detached delivery task holds the app-wide operation ticket until core's runner ends. Cancellation is cooperative, before/between steps; await a submitted mutation. Never wrap the submitted run in an ordinary `OwnedJob` or abort it when its screen changes.

### Snapshot and Loader

Use desktop's `state::Snapshot<T, I, E>` for a value read for one identity and, for ordinary inspection reads, `screens::Loader<T>` ([how, with an example](docs/STATE_LIFETIMES.md#snapshot-and-loader-in-brief)): `load` for Tokio work, `resolve` for fixtures, and `reset()` when `set_source` detects a changed target. Render reads `data()`, `error()` and `is_loading()`; retain data alongside a refresh failure banner and its timestamps. Call `cx.notify()` after changes outside render; derive display collections when data changes.

Access identity answers whose data this is; generation answers which request may publish. Follow [ACCESS_IDENTITY.md](docs/ACCESS_IDENTITY.md) when configuration/access changes. Preserve each feature's owner: logs use `LogView<S>`, Monitoring and Observability own their page requests, shells belong to `ShellView`, and forwards belong to the app's `ForwardList`.

### Learning the API

Documentation is thin. Before using an API, read the source of the pinned versions in the Cargo registry (`~/.cargo/registry/src/*/gpui-kit-0.7.0`, `gpui-component-0.7.0`, `gpui-base-0.7.0`, `gpui-pre-*`); don't guess from older GPUI examples. Expect odd shapes, such as `window.root::<Root>()` returning `Option<Option<Entity<Root>>>`, and breaking changes whenever the pinned pre-1.0 versions move.

### Keys and focus

- When the focused element isn't drawn, key dispatch starts from the window root and the page's bindings stop working. Give a page's key context to a wrapper that is drawn in every state, not to a list that a placeholder replaces.
- macOS reports Command-Shift-] as `}` with Command alone, so bind `secondary-}`, not `secondary-shift-]`. Command-Shift with a letter keeps Shift: `secondary-shift-g`. Those two keys step the shell's view tabs everywhere; a pane's or a node's tabs use Option-Command-] and [, bound as `secondary-alt-]` and `secondary-alt-[` as Zed does. Prove a new bracket binding with a headless `window.press` and say that the running app on macOS is the check that cannot be made headless.
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
- A fake Kubernetes API behind `kube::Client::new` never asserts or panics inside its service: the client runs the service in a tower Buffer worker, so a panic there kills only the worker and the read sees an ordinary error. Record each request (`freshkube_core::fake_api::Recorder`, behind core's `testing` feature), answer an unknown route with a 500, and assert on the recorded list after the read.
- A test that waits on work in a real Tokio runtime or socket (a fake server, a worker thread) waits through `freshkube_ui::real_io` (`RealIo::wait_for_io`, `until_io`), on the drawn element, the fake's recorded request or the state it then asserts: `wait_for`'s timeout runs on the test clock and can pass before the real work answers, and `run_until_parked` alone can return before it does. The helper's one allowance is 30 s of real time.
- Time anything a test depends on with the executor's clock (`cx.background_executor().now()`), not `Instant::now()`, so tests step it with `advance_clock` instead of sleeping. Example data dated from the wall clock must not be generated again for a stream already read: a second later it passes for new lines. Both made tests fail on a loaded machine.

### Smoke tests

**Every change a person would notice in the app — a page, a control, a flow, a layout — gets a smoke test in the running app before it is called done.** UI tests prove the logic; a smoke test proves it looks and works right on screen. Use the helper, `scripts/smoke.sh`, don't improvise one; [docs/SMOKE.md](docs/SMOKE.md) has its commands, the rules in full, captures from a CI-built debug binary (`freshkube-debug-<architecture>-apple-darwin-<short-sha>`, `aarch64` or `x86_64`) and `scripts/browser.sh`.

```sh
scripts/smoke.sh start --page <slug>    # builds, launches with --fixture, waits for the window
scripts/smoke.sh shot <name>            # → target/smoke/<worktree>/<name>.png; open it and look
scripts/smoke.sh stop
```

- Smoke the pages and flows the change touches, in fixture mode and, when the change reads a cluster or Coroot, against the context the user chose. Run `scripts/smoke.sh pages` when a change reaches the frame, the theme or shared components.
- Compare a page moved onto the shared components with Pods side by side, at the same size, theme and text size, and check that title, padding, header, rows, group rows, glyphs and states line up.
- Look at every capture yourself, then report what you checked and what you saw: the captures that show the change, anything wrong, and anything you could not check. Judge a page from all of it, not its first screen.

- Open pages with `--page` from a debug binary rather than navigating to them, and click only to exercise the change.
- Live checks only look and navigate. Never press Operations or maintenance actions, and keep credentials out of captures you share. Sign-ins are the user's: stop and ask.
- One worktree uses the screen at a time: build before you start, keep the session short, always `stop`, and let another worktree's check finish.
- Keep interaction logic in UI tests too; a smoke test adds to them, it doesn't replace them.

## Diagnostics

Diagnostic checks follow these rules; [docs/DIAGNOSTICS.md](docs/DIAGNOSTICS.md) has them with examples, the checklist for a new check, and the Talos ports and Machine API methods we use.

- **State over logs** is the most important principle: check actual system state, not log messages. Logs are history; APIs and files are truth.
- Prefer file or procfs state (`/run/flannel/subnet.env`), then API responses (Talos API, K8s API); parse logs only as a last resort, and then validate their timestamps.
- Degrade gracefully: when a data source is unavailable, show unknown (`DiagnosticCheck::unknown`), don't crash.
- No false positives: a diagnostic showing failure for a healthy system is worse than showing unknown.
- Find the source of truth first, use the `DiagnosticCheck` constructors (`pass`, `fail`, `warn`, `unknown`) and provide an actionable fix where possible. Implement `HasHealth` for health enums in core; desktop maps it to a tag's tone and icon with `ui::health_tone`.
- Before merging a check: create its error condition, fix it and see the check show healthy; disconnect the API and see no crash; make sure old log messages don't affect the current state.
- The COSI State API is internal and not exposed through port 50000: don't implement a COSI gRPC client (it fails with `PermissionDenied`); run `talosctl get` as a subprocess.

## Adding a screen

1. Pick the page's type from [DESIGN.md](docs/DESIGN.md#page-types) (table page, dashboard, canvas or detail pane) and build it from that type's shared components: the page header, the table, group rows, status glyphs and states. Don't draw your own title, table, glyphs or radii; `scripts/check-style.sh` fails on them, and its allowlist is empty and only shrinks, so there is no exception to add. A table page's line (context, counts, state, refresh time) goes in the status bar through `ScreenPanel::status` and a `status::Segment`, not under its header. Its row actions act on the selection, from the toolbar, keys and the shared row context menu (`TableSource::menu_focus` and `row_menu`), never from a button on the row. A selection's details go in the shared Inspector (`freshkube_ui::inspector`, change 9 in DESIGN.md), beside the table or stacked under it, not in a card of its own.

2. Add cluster pages to `Page` (`desktop/pages.rs`): `ALL`, its slug, its `Area` and column row, and its shortcut. Add inspection views to `PageId` and `PageId::SCREENS`, the screen factory and `Page::id` or `NodeTabs::screen`, according to their scope.
3. Implement `ScreenPanel` in `screens/<name>.rs`, or `screens/<name>/mod.rs` once it mixes concerns. The screen owns its requests (`OwnedJob`), its data and its offline example data.
4. Put cluster logic in `freshkube-core` and keep the screen to presentation.
5. Add UI tests for loading, empty, failure and the main interactions. A table page also calls `desktop::layout_check::assert_table_page` (`freshkube_ui::layout_check`, behind its `testing` feature, as is `settle_header`), which measures its header, rows, group rows, edge-to-edge frame and title against Pods' sizes; a page with several lists calls its halves, `assert_edge_frame` and `assert_table`, a padded page of cards, such as a dashboard, `assert_page_frame`, and a screen embedded under a tab that names it (`PageHeader::untitled`), `assert_untitled_edge_frame`. A page with an Inspector also calls `assert_inspector`, beside and stacked. Then smoke it beside Pods at the same size, theme and text size ([Smoke tests](#smoke-tests)).

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

## Features

Behaviour that is not a rule lives in one doc per feature, each starting with its module paths and the tests that prove it ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#feature-docs)).

- The command line, `freshkube check` and `freshkube get` (`crates/freshkube-cli/`): [docs/features/cli.md](docs/features/cli.md). Read-only; the cluster is always named, never the current context; Secret values are never requested or printed.
- The agent's reads, the per-cluster access gate and the loopback endpoint (`freshkube-core`'s `agent_reads/`, `crates/freshkube-agent/`): [docs/features/agent.md](docs/features/agent.md). Read-only (`get`, `list`, the `log` subresource); Secrets as metadata; off for every cluster until a person turns it on; no UI and no desktop dependency yet.
- Databases (`core/databases/`, `screens/databases/`): [docs/features/databases.md](docs/features/databases.md). CloudNativePG first; no Secret or credential reads or mutations.
- View tabs (`desktop/tabs/`, `freshkube_ui::tab_strip`): [docs/features/tabs.md](docs/features/tabs.md). Rules kept here: a tab is a `ViewState` of the cluster that is open and nothing else is saved; every link still goes through `Pilot::open_object_in` and its guard first; the strip is a cached view beside the page and the shell draws again only when its layout or answers change; frozen rows are bounded (8 tabs, 5,000 rows each, 64 MiB) and make a row action wait.
- Resources, the dock, shells and ports (`crates/freshkube-resources/`, `desktop/dock/`): [docs/features/resources.md](docs/features/resources.md). Rules kept here: a dock log tab reads while it is open, on any page, and so does a followed item (below), and nothing else reads out of sight; Resources reads only while its page shows; Secret values never reach the detail pane's document, search or a link's resolution (metadata-only reads).
- Deep links, Copy link and opening a `freshkube://open` link (`crates/freshkube-nav/src/view/mod.rs`, `desktop/deep_link/`): [docs/features/deep-links.md](docs/features/deep-links.md). Rules kept here: a link holds identities only in closed, bounded types and only navigates; Copy link always means a Freshkube link (the change page's is Copy Kargo link).
- The agent in the dock (`crates/freshkube-agent/`): [docs/features/agent.md](docs/features/agent.md). The agent's only tools are Freshkube's four reads; it gets no credential, an empty workspace and a cleared environment; the token goes only into `session/new`; tests and CI never run opencode.
- Applications, the Changes list, Waiting to ship and the change page (`crates/freshkube-applications/`): [docs/features/applications.md](docs/features/applications.md). The change page's Stage actions (Refresh, Reverify, Promote, Abort) are the first writes a page offers, through core's `write` module ([docs/WRITE_MODEL.md](docs/WRITE_MODEL.md#stage-actions-855)): never from a deep link, never in a live check unless the maintainer names a disposable Stage.
- The Nodes workspace and its node pane (`desktop/nodes/`): [docs/features/nodes.md](docs/features/nodes.md).
- The frame, the Overview and the Kubernetes summary session (`desktop/shell/`, `presentation/`, `kubernetes_summary/`): [docs/features/shell.md](docs/features/shell.md).
- Settings, its sections and the setup file's saves (`desktop/settings/`, `setup_file.rs`, `freshkube_ui::form`): [docs/features/settings.md](docs/features/settings.md).
- Monitoring and the CPU and memory history (`freshkube-monitoring`): [docs/features/monitoring.md](docs/features/monitoring.md), with its design in [docs/MONITORING.md](docs/MONITORING.md). Rules kept here: a hidden Monitoring page finds nothing, resolves nothing and asks no panel; panels draw only Console colours. Alerts, Rules and Silences (`freshkube-monitoring`'s `alerts/`, core's `monitoring/alerts/`) are read-only: nothing silences or expires.
