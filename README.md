# Freshkube

A native desktop app for [Talos Linux](https://www.talos.dev/) and Kubernetes clusters, built with [GPUI Kit](https://gpui-kit.com).

Freshkube brings Talos machines and Kubernetes resources into one cluster view. Overview and Health share cluster facts, Nodes holds machine inspection, and pod relationships and search connect the two layers.

Freshkube began as a fork of [talos-pilot](https://github.com/Handfish/talos-pilot) by Ken Udovic. talos-pilot's terminal UI, egui and Dioxus frontends were removed; its GPUI desktop frontend is the basis of this application. If you want a terminal tool, use talos-pilot.

**Status:** macOS is the only verified platform.

## Features

| Page | What it shows |
| --- | --- |
| **Nodes** | Kubernetes and Talos machines in one table or card grid; open a node for its summary, Pods, system services, processes, storage, network, diagnostics, logs, Events and YAML |
| **Overview** | Eight cards for Kubernetes and Talos, plus Needs attention with links to objects, logs and node inspection; Kubernetes-only mode has four cards |
| **System services** | Cluster service health with filters and links to the node pane; per-node details and restart stay in Nodes |
| **Node Logs** | Live, interleaved logs from several services: level filters, search, follow or pause, wrapping, selection and copy |
| **Node Processes** | Process list and tree with CPU and memory sorting |
| **Node Storage** | Disks with size, transport, serial and system-disk indicators |
| **Node Network** | Interface traffic, connections, KubeSpan peers and packet capture |
| **Node Diagnostics** | Automated health checks (system, Kubernetes components, CNI, addons, services) with actionable fixes |
| **etcd** | Quorum health, members, alarms and leader |
| **Resources** | Kubernetes lists, watched tables and object panes with Overview, YAML and Events; pods add Logs, Shell and Ports |
| **Health** | Deployments, StatefulSets, DaemonSets and pods by namespace, with issues highlighted |
| **Security** | PKI certificate expiry and encryption status |
| **Lifecycle** | Talos and Kubernetes versions, configuration drift and alerts |
| **Operations** | Cordon, uncordon, drain, reboot and shutdown of one node or a rolling selection, with a preflight preview, etcd safety checks, confirmation and an audit log |

Maintenance mode (`--insecure --endpoint <node>`) opens a bootstrap wizard for nodes that have no configuration yet.

Without a talosconfig, or with `--kubernetes-only`, Freshkube browses Kubernetes alone. The sidebar lists the kubeconfig's contexts and connects to its current one, or to the one `--kube-context` names. It starts on Overview; Control plane is replaced by an Add a talosconfig link. Talos views reached another way ask for a talosconfig.

The context switcher sits above Cluster, Resources and Control plane navigation. Node inspection views and logs are tabs in Nodes; Settings holds the talosconfig and kubeconfig choice, auto-refresh and light or dark appearance. Unavailable data is shown as unknown, never as failed, and failed refreshes keep the previous data marked stale.

Pod Overview links to its node, controllers and matching Services, with container logs, previous instances and recent warnings. Owner links work on other kinds too; the kubelet’s node detail links to its Pods tab.

Search everything (⌘K) finds pages, nodes, kinds and object names. It lists metadata when opened, shows permission refusals per kind, and never reads Secret values. Enter opens the result.

## Design philosophy

- **State over logs.** Health is read from system state (procfs files, Talos and Kubernetes API responses), not from log lines, so a stale error in an old log never raises a false alarm.
- **Reliability hierarchy.** Checks prefer file and procfs state first, then API responses, then log parsing.
- **No false positives.** When a data source is unavailable, a check reports `unknown` rather than guessing or crashing.
- **Safe operations.** Every change to a cluster is previewed, confirmed against exactly what was shown, bound to the context it started in, and audited.

## Engineering highlights

- The Talos API is addressed at node endpoints directly, not through the cluster VIP. The VIP depends on a healthy control plane, so using it would break diagnostics when the control plane is down.
- The COSI resource API is not reachable on the public `:50000` port; a direct gRPC client gets `PermissionDenied`. Freshkube shells out to `talosctl get` for that data instead.
- When the Talos discovery service is disabled, there is no membership list to read. Freshkube enumerates and targets nodes through the Kubernetes API, so multi-node views and rolling operations still work.
- Packet capture streams pcap data back over the same `:50000` connection it runs on, so an unfiltered capture records its own traffic and loops. Freshkube applies a BPF filter that drops traffic on the API port. It is precompiled with `tcpdump -dd` and embedded as bytecode (parameterized by port, with a variant per link type for IPv4/IPv6, TCP/UDP/SCTP, and fragmented packets), so no filter compiler is needed on the node.
- Business logic and the desktop app's headless UI tests run without a live node: the app renders headless, and tests find elements by id and click or type into them.

## Building

Requirements:

- Rust (stable, 2024 edition)
- `protoc`, the protobuf compiler, used to generate the Talos gRPC client: `brew install protobuf`
- macOS 15 or later with the Xcode command line tools (see GPUI Kit's [platform prerequisites](https://gpui-kit.com/docs/installation))

Clone the public [Freshkube repository](https://github.com/skel84/freshkube) and build:

```bash
git clone https://github.com/skel84/freshkube.git
cd freshkube
cargo build --release
./target/release/freshkube
```

There are no published Freshkube releases yet. The [macOS packaging guide](docs/MACOS_PACKAGING.md) covers local `.app` builds, CI artifacts and signing prerequisites. CI produces ad-hoc signed development bundles for Apple Silicon and Intel; these are not notarized. A Nix flake is included (`nix run .` or `nix develop`).

## Usage

```bash
# Use the remembered Talos selection, or the default from ~/.talos/config
freshkube

# Use a specific talosconfig and context
freshkube --config /path/to/talosconfig --context homelab

# Use a specific kubeconfig for Kubernetes data
freshkube --config /path/to/talosconfig --kubeconfig /path/to/kubeconfig

# Browse Kubernetes only (also the default when no talosconfig is found)
freshkube --kubernetes-only --kubeconfig /path/to/kubeconfig --kube-context admin@lab

# Explore with synthetic example data; no credentials or cluster are used
freshkube --fixture

# Bootstrap a node in maintenance mode
freshkube --insecure --endpoint 192.168.1.100

# Debug logging (written to <temp dir>/freshkube.log, or --log-file)
freshkube --debug
```

### Keyboard

| Key | Action |
| --- | --- |
| ⌘1 … ⌘9 | Overview, Nodes, Namespaces, Events, Health, etcd, System services, Security, Lifecycle |
| Ctrl-Tab / Ctrl-Shift-Tab | Next or previous page |
| Alt-↑ / Alt-↓ | Previous or next Talos context |
| ⌘R | Refresh |
| ↑ ↓ | Move through node and service lists |
| ⌘C | Copy selected log lines |
| ⌘Q | Quit |

## Architecture

```
crates/
├── talos-rs/            Talos gRPC client
├── freshkube-core/      domain logic shared by every page; no UI types
└── freshkube-desktop/   the GPUI Kit application
src/main.rs              the freshkube binary
```

Cluster I/O runs on a Tokio runtime owned by the binary; results return to the UI's executor, and every request is tied to the target it was made for, so a late answer for another node or context is discarded. Pages that are not visible never contact the cluster.

## Development

```bash
cargo test --workspace                                   # including headless UI tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
FRESHKUBE_PAGE=diagnostics cargo run -- --fixture        # debug builds open a page directly
```

- [AGENTS.md](AGENTS.md) holds the development rules, including how to work with GPUI.
- [docs/ROADMAP.md](docs/ROADMAP.md) records what has landed and what comes next; [docs/FUTURE_IDEAS.md](docs/FUTURE_IDEAS.md) holds the longer-term backlog.
- [docs/GPUI_FRICTION.md](docs/GPUI_FRICTION.md) records GPUI Kit friction and strengths.
- [test-clusters/](test-clusters/) creates local Talos clusters for testing.

Contributions should follow the design philosophy above: check real system state, degrade gracefully, and never report failure when "unknown" is the honest answer.

## License

MIT; see [LICENSE](LICENSE). Freshkube includes talos-pilot, copyright Ken Udovic. [NOTICE](NOTICE) lists the third-party components the app ships and their licences: the `alacritty_terminal` terminal emulator under Apache-2.0 ([licenses/Apache-2.0.txt](licenses/Apache-2.0.txt)), the Lucide/Feather icons ([licences](licenses/Lucide.txt)), and the embedded Lato and Source Code Pro fonts under the SIL Open Font License 1.1 (`crates/freshkube-desktop/assets/fonts`).

## Acknowledgments

- [talos-pilot](https://github.com/Handfish/talos-pilot) by Ken Udovic, which Freshkube grew out of
- [Talos Linux](https://www.talos.dev/) by Sidero Labs
- [GPUI Kit](https://gpui-kit.com) by Longbridge, and [GPUI](https://www.gpui.rs/) by Zed Industries
- [Kubeli](https://github.com/atilladeniz/Kubeli) and [k9s](https://k9scli.io/) for Kubernetes browsing ideas

The shell refreshes Kubernetes health alongside the Talos overview every 15 s, from the API server cache. Workload health uses that shared snapshot, including in Kubernetes-only mode. A refused list leaves the other summary parts available.

Nodes joins names first and addresses second. Its pane follows the selected Talos target, remembers its tab across nodes, and expands with Command-Shift-Return. Below the split width it shows a Back button. The Pods tab lists across namespaces with `spec.nodeName` and opens a pod on Resources through the shell confirmation guard.

### Fixture layout review

Debug builds can open each review view without window automation:

```sh
FRESHKUBE_PAGE=overview FRESHKUBE_THEME=dark FRESHKUBE_TEXT_SIZE=20 FRESHKUBE_WINDOW_SIZE=760x560 cargo run -- --fixture
```

Pages also include `nodes`, `node-overview`, `pod-overview`, `search` and `kubernetes-only`. Use `FRESHKUBE_KIND=<key>` for a Resources list. The [layout design](docs/HOLISTIC_LAYOUT.md) records the checks and cost gate.
