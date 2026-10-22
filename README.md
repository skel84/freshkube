# Freshkube

A native desktop app for [Talos Linux](https://www.talos.dev/) and Kubernetes clusters, built with [GPUI Kit](https://gpui-kit.com).

Freshkube brings Talos machines and Kubernetes resources into one cluster view. Overview and Health share cluster facts, Nodes holds machine inspection, and pod relationships and search connect the two layers.

Freshkube began as a fork of [talos-pilot](https://github.com/Handfish/talos-pilot) by Ken Udovic. talos-pilot's terminal UI, egui and Dioxus frontends were removed; its GPUI desktop frontend is the basis of this application. If you want a terminal tool, use talos-pilot.

**Status:** macOS is the only verified platform.

## Installing

Freshkube needs macOS 15 or later, on Apple silicon or Intel. Install it with [Homebrew](https://brew.sh):

```bash
brew install --cask skel84/tap/freshkube
```

This puts `Freshkube.app` in `/Applications` and links the `freshkube` command; `brew upgrade --cask freshkube` updates it. Builds are ad-hoc signed and not notarized, so macOS blocks the first open: open Freshkube once, then choose **System Settings → Privacy & Security → Open Anyway**. You can also download a ZIP from the [Releases page](https://github.com/skel84/freshkube/releases) ([installing by hand](docs/MACOS_PACKAGING.md#install-a-release)).

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
| **Applications** | Applications found from Kargo Projects, Argo CD Applications and ApplicationSets, and the `app.kubernetes.io/part-of` label, across the workspace's clusters; each application's page lists its parts and how sure each link is |
| **Change page** | One Kargo Freight followed from its pull request through its builds, each Stage's gates and Argo CD to the pods that run it ([Following a change](#following-a-change)) |
| **Workspace** | Your clusters, listed in Settings › Workspace and opened from the header's Clusters list, with Argo CD destinations mapped to them |
| **Monitoring** | Grafana dashboards drawn natively from the cluster's Prometheus, reached through the Kubernetes API; built-in dashboards plus a folder of your own Grafana JSON |
| **Observability** | A Coroot server you connect: applications, service map, incidents, traces, profiles, logs and Deployments |
| **Log dock** | Tabs under every page for pod and workload logs and pod shells; a log tab keeps reading while it is open, with levels, search, follow, copy and download |
| **Port forwards** | Started from Ports on a pod, Service or workload; each runs until stopped, listed in the status bar |

Maintenance mode (`--insecure --endpoint <node>`) opens a bootstrap wizard for nodes that have no configuration yet.

Without a talosconfig, or with `--kubernetes-only`, Freshkube browses Kubernetes alone. It connects to the context `--kube-context` names, else the kubeconfig's current one. It starts on Overview; Control plane is replaced by an Add a talosconfig link. Talos views reached another way ask for a talosconfig.

The header holds the context switcher, whose Clusters list opens a cluster of your workspace, with Search everything, Refresh and Settings. An icon rail on the left picks the area (Overview, Nodes, Namespaces, Events, Applications, Monitoring, Observability, the Kubernetes groups, Custom Resources and Control plane), and a column beside it lists that area's pages. Node inspection views and logs are tabs in Nodes; Settings holds the talosconfig and kubeconfig choice, auto-refresh and light or dark appearance. Unavailable data is shown as unknown, never as failed, and failed refreshes keep the previous data marked stale.

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
- Packet capture streams pcap data back over the same `:50000` connection it runs on, so an unfiltered capture records its own traffic and loops. Freshkube applies a BPF filter that drops traffic on the API port. It is precompiled with `tcpdump -dd` and embedded as bytecode (parameterized by port, with a variant per link type for IPv4/IPv6 and TCP/UDP/SCTP), so no filter compiler is needed on the node. As with tcpdump's `not port`, it reads ports only from a header that directly follows the IP header, so later IPv4 fragments and IPv6 packets with a fragment header are kept, even on the API port.
- Business logic and the desktop app's headless UI tests run without a live node: the app renders headless, and tests find elements by id and click or type into them.

## Building

Requirements:

- Rust (stable, 2024 edition)
- `protoc`, the protobuf compiler, used to generate the Talos gRPC client: `brew install protobuf`
- macOS 15 or later with the Xcode command line tools (see GPUI Kit's [platform prerequisites](https://gpui-kit.com/docs/installation))

`talos-rs` generates its protobuf Rust code at build time from the checked-in
`crates/talos-rs/proto/` schemas into Cargo's `OUT_DIR`. Generated Rust files are
build outputs and are not checked in; separate target directories keep their
outputs independent. Schema changes regenerate the code on the next build,
including imported schemas. The public `talos_rs::proto` module paths stay the
same. Set `PROTOC` to an absolute compiler path if it is not on `PATH`; changing
`PROTOC` or `PROTOC_INCLUDE` also reruns generation. `protoc` is a build dependency
only, and is not needed to launch the app or an installed bundle.

Clone the public [Freshkube repository](https://github.com/skel84/freshkube) and build:

```bash
git clone https://github.com/skel84/freshkube.git
cd freshkube
cargo build --release
./target/release/freshkube
```

The [macOS packaging guide](docs/MACOS_PACKAGING.md) covers local `.app` builds, CI artifacts and signing prerequisites. CI produces ad-hoc signed development bundles for Apple Silicon and Intel; these are not notarized. A Nix flake is included (`nix run .` or `nix develop`).

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

### Following a change

The change page follows one Kargo Freight from its pull request to the pods that run it. It only reads: it never approves, promotes or syncs.

There are two ways in, both under **Applications** on the rail:

- **Changes**, at the top of the column, lists one row per Freight with each Stage's state. Select a row and press Enter.
- Open a Kargo application, select a Stage and choose **Follow ⟨Freight⟩** (or press F).

The page lists the hops in travel order: Build, Freight, then each Stage with its gates, the Argo CD Application, the workload and its pods. Each link says how sure it is:

- **Confirmed:** both sides were read and agree.
- **Claimed:** only one side says so, for example Argo CD's status for workloads on a cluster that wasn't read.
- **Unknown:** a side couldn't be read, and the hop says which cluster and why.

With one cluster, there is nothing to set up. Open the cluster that runs Kargo and Argo CD, for example `freshkube --kubernetes-only --kubeconfig /path/to/kubeconfig --kube-context admin@lab`. Without a workspace file, every part is read from the open connection.

#### Several clusters

When Kargo, Argo CD and your workloads run on different clusters, list them in the [setup file](#the-setup-file). Each entry has a role, and the role says what is read there:

- `cicd`: Kargo and the builds (Pipelines as Code's PipelineRuns).
- `core`: Argo CD. Kargo and the builds too, when no entry is `cicd`.
- `environment`: a cluster your workloads run on.

The first entry with a role wins. Under **Argo CD destinations**, map each destination, by its server URL or its cluster name, to one of the entries. Freshkube never guesses from your kubeconfig. A Stage that deploys to Argo CD's own cluster (`in-cluster`) needs no mapping. **Delivery sources** shows which entry each part is read from and why; **Check sources** reads one item of each to prove it.

#### The setup file

The workspace lives in one file, `freshkube.yaml`, beside the app's preferences: `~/Library/Application Support/Freshkube` on macOS, `~/.config/freshkube` on Linux and `%APPDATA%\Freshkube` on Windows. Freshkube reads it at start and again within a couple of seconds of any change on disk, so you can edit it by hand. A file that doesn't parse is not used: the app names the line and the reason, keeps the last good setup (or starts as with no file) and never writes over the broken one. Keys you add that it doesn't know are kept, not dropped.

Settings saves into the same file. A save keeps the keys it doesn't know but not your comments, so a save over a file you edited by hand first copies that file to `freshkube.yaml.bak`. A save is refused, with the reason, when the file changed on disk since it was read or doesn't parse; nothing is written over it, and the status bar shows what each save did.

Upgrading from `workspace.json` and `coroot.json`: at the first start with no `freshkube.yaml`, Freshkube writes it from those two files, and from any metrics source `monitoring.json` chose for one of your clusters, and renames the first two to `workspace.json.bak` and `coroot.json.bak`. It says so once, and never reads the `.bak` files again. `monitoring.json` stays where it is. If `workspace.json` can't be used, nothing is moved and it stays in use.

Everything below is invented. The file is at most 256 KiB, `version` is required and is `1`, and every path is absolute. No key or token goes in it.

```yaml
version: 1

# The kubeconfig the entries' contexts come from. Optional: without it, the
# default one (KUBECONFIG, then ~/.kube/config).
kubeconfig: /home/acme/.kube/config

# The workspace: one entry per cluster, at most 64. Each id is unique and at
# most 64 bytes; pins and destinations name it.
clusters:
  - id: cluster-a
    role: core                    # core, cicd or environment
    context: cluster-a            # the kubeconfig context
    talosconfig: /home/acme/.talos/config   # optional: Talos pages for this entry
    talos_context: cluster-a                # optional, and needs talosconfig
  - id: cluster-b
    role: cicd
    context: cluster-b
    metrics:                      # optional
      value: ci-b                 # its value of the label, instead of its id
      # source:                   # its own store, under any mode: service: or url:
  - id: cluster-c
    role: environment
    context: cluster-c

# Argo CD destinations, each matched by server or by name (never both), and
# the entry it is. At most 256. `in-cluster` needs no row.
destinations:
  - server: https://cluster-c.example.com:6443
    entry: cluster-c
  - name: acme-prod
    entry: cluster-a

# Pins: a part read from a named entry instead of the one its role picks.
sources:
  kargo: cluster-b
  builds: cluster-b
  argocd: cluster-a

# Where metrics come from: one choice, `mode`. Optional: without it, `automatic`.
#   automatic  find Prometheus in the open cluster (the default)
#   central    one store for every cluster, told apart by a label
#   per-entry  each cluster names its own `metrics.source`
metrics:
  mode: central
  store:                          # central only: a Service in a cluster, or a URL
    entry: cluster-a              # the cluster that hosts it, reached through its API server
    service:
      namespace: monitoring
      name: vmselect
      port: 8481
      https: false                # true if the port speaks TLS
      path: /select/0/prometheus  # optional prefix
    # url: https://metrics.example.com/select/0/prometheus   # instead of entry and service
    # token: keys.json            # a URL's bearer token, kept in keys.json
  label: cluster                  # the label that tells clusters apart; "cluster" if unset

# The Coroot connection. Optional: without it, Coroot isn't connected.
coroot:
  server: https://coroot.example.com
  sign_in: api-key                # api-key, session or anonymous
  key: keys.json                  # the key the app keeps for this server
  # api_key_file: /home/acme/coroot.key   # instead of key: a file holding only the key
  project: acme                   # optional
```

Under `central`, a cluster's metrics are the store's series where the label equals its `metrics.value` (its id if unset), so Monitoring never shows another cluster's numbers as the open one's. Under `per-entry`, a cluster with no `source` shows "No metrics source" and asks nothing. `store` and `label` are kept, unread, under another mode. Without a `metrics` block, or for a context that isn't a workspace entry, metrics are found in the open cluster as before.

A value that breaks a rule, such as a relative path, a duplicate id, a `talos_context` without a `talosconfig`, a destination that names both a server and a name, a `key` other than `keys.json`, `mode: central` without a `store`, a store that sets both a Service and a URL, or a label that isn't a Prometheus label name, makes the whole file unused with the reason, never a partial read. Check sources and Delivery sources show what each entry answers.

Each part is then read from its own entry, whichever cluster is open. A destination that was read can be Confirmed to its pods. An entry that is refused or can't be reached is named on its hop and in a May be missing banner, never shown as none found. Each read has a deadline, so one slow cluster never holds up the others: the change page has 45 s, and a destination's entry 30 s of it; the Changes list keeps what answered within 90 s. An unmapped destination stays Unknown and says so.

Some buttons are greyed out, with the reason in their tooltip:

- **Open in Argo CD** and **Open in Kargo** need the address the cluster records: `url` in the `argocd-cm` ConfigMap, and `ADMIN_ACCOUNT_TOKEN_ISSUER` in the `kargo-api` ConfigMap in the `kargo` namespace.
- A build's pull request, commit and run links need the Pipelines as Code annotations on its PipelineRun.
- **Compare in Observability** needs Coroot connected, with a Coroot cluster linked to the open one, and a Deployment whose pods are confirmed to run the Freight.
- **Open**, a run's **Logs** and **Compare in Observability** act on the open cluster only.

An object on another workspace entry offers **Switch and open** instead. It asks before it switches the window to that entry, then opens the object in Resources once the entry answers.

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

MIT; see [LICENSE](LICENSE). Freshkube includes talos-pilot, copyright Ken Udovic. [NOTICE](NOTICE) lists the third-party components the app ships and their licences: the `alacritty_terminal` terminal emulator under Apache-2.0 ([licenses/Apache-2.0.txt](licenses/Apache-2.0.txt)), the Lucide/Feather icons ([licences](licenses/Lucide.txt)), and the embedded Figtree and IBM Plex Mono fonts under the SIL Open Font License 1.1 (`crates/freshkube-ui/assets/fonts`).

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
