# talos-pilot

A terminal UI (TUI) and native desktop GUI for managing and monitoring [Talos Linux](https://www.talos.dev/) Kubernetes clusters.

**talos-pilot** provides real-time cluster visibility, diagnostics, log streaming, network analysis, and production-ready node operations through both interfaces.

[![CI](https://github.com/Handfish/talos-pilot/actions/workflows/ci.yml/badge.svg)](https://github.com/Handfish/talos-pilot/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Handfish/talos-pilot)](https://github.com/Handfish/talos-pilot/releases)
![Rust](https://img.shields.io/badge/rust-2024%20edition-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

https://github.com/user-attachments/assets/4c946c32-1f7e-4ab8-9d88-9937516015d1

*Live cluster overview, interleaved log streaming, diagnostics, and safe node operations in the terminal UI.*

## Contents

- [Why talos-pilot?](#why-talos-pilot) · [Relationship to k9s](#relationship-to-k9s)
- [Features](#features)
- [Design Philosophy](#design-philosophy)
- [Engineering Highlights](#engineering-highlights)
- [Installation](#installation) · [Usage](#usage) · [Keyboard Navigation](#keyboard-navigation)
- [Architecture](#architecture) · [Development](#development)

## Why talos-pilot?

Talos Linux removes SSH access for security, replacing it with an API-driven management model. While `talosctl` is powerful, it requires memorizing many subcommands. **talos-pilot** provides:

- **Interactive cluster overview** - See all nodes, services, and health at a glance
- **Real-time monitoring** - CPU, memory, network stats with auto-refresh
- **Unified log viewer** - Stream logs from multiple services simultaneously (Stern-style)
- **Production operations** - Drain, reboot, rolling upgrades with safety checks
- **Diagnostics** - Automated health checks with actionable fix suggestions

### Relationship to k9s

**talos-pilot is complementary to k9s, not a replacement.** They operate at different layers:

| Tool | Layer | API Port | Use Case |
|------|-------|----------|----------|
| **k9s** | Kubernetes | `:6443` | Pods, deployments, services, workload debugging |
| **talos-pilot** | Operating System | `:50000` | Talos services, etcd, kubelet, node health, OS config |

Use **k9s** for "why won't my pod start?"
Use **talos-pilot** for "why won't my node join the cluster?"

## Features

### Cluster Management

| Feature | Description |
|---------|-------------|
| **Cluster Overview** | Multi-cluster monitoring, node list with health indicators |
| **Node Details** | CPU, memory, load averages, Talos/K8s versions |
| **Service Status** | All Talos services with health indicators |

### Monitoring

| Feature | Description |
|---------|-------------|
| **Service Logs** | Scrollable, searchable (`/`), color-coded by level |
| **Multi-Service Logs** | Stern-style interleaved logs from multiple services |
| **Processes View** | htop-like process list with tree view, CPU/MEM sorting |
| **Network Stats** | Interface traffic, connections, KubeSpan peers, packet capture |
| **Storage/Disks** | Disk list with size, transport, serial, system disk indicators |
| **etcd Status** | Quorum health, member list, alarms, leader tracking |
| **Workload Health** | K8s deployments, statefulsets, pod issues by namespace |
| **Lifecycle View** | Version status, config drift detection, cluster alerts |

### Diagnostics & Security

| Feature | Description |
|---------|-------------|
| **System Diagnostics** | Automated health checks with actionable fixes |
| **CNI Detection** | Flannel, Cilium, Calico with provider-specific checks |
| **Addon Detection** | cert-manager, ArgoCD, Flux, and more |
| **Security Audit** | PKI certificate expiry, encryption status |

### Operations

| Feature | Description |
|---------|-------------|
| **Node Drain** | PDB-aware with configurable timeouts |
| **Node Reboot** | Post-reboot verification, auto-uncordon |
| **Rolling Operations** | Sequential multi-node with progress tracking |
| **Audit Logging** | All operations logged to `~/.talos-pilot/audit.log` |

## Design Philosophy

talos-pilot favors reliability in how it reports cluster health:

- State over logs: health is read from system state (procfs files, Talos and Kubernetes API responses), not from log lines. A stale error in an old log does not trigger a false alarm.
- Reliability hierarchy: checks prefer file and procfs state first, then API responses, then log parsing. Where a file like `/run/flannel/subnet.env` exists, it is read directly.
- No false positives: when a data source is unavailable, a check reports `unknown` rather than guessing or crashing.
- Separation of concerns: five crates. `talos-rs` provides the gRPC client, `talos-pilot-core` holds shared business logic, and `talos-pilot-tui`, `talos-pilot-gui` (egui), and `talos-pilot-dioxus` provide separate frontends. Shared logic can be tested without a terminal, desktop window, or live cluster; the TUI remains the default interface.

## Engineering Highlights

Some implementation notes:

- The Talos API is addressed at node endpoints directly, not through the cluster VIP. The VIP depends on a healthy control plane, so using it would break diagnostics when the control plane is down.
- The COSI resource API is not reachable on the public `:50000` port; a direct gRPC client gets `PermissionDenied`. talos-pilot shells out to `talosctl get` for that data instead.
- When the Talos discovery service is disabled, there is no membership list to read. talos-pilot enumerates and targets nodes through the Kubernetes API, so multi-node views and rolling operations still work.
- Packet capture streams pcap data back over the same `:50000` connection it runs on, so an unfiltered capture records its own traffic and loops. talos-pilot applies a BPF filter that drops traffic on the API port. It is precompiled with `tcpdump -dd` and embedded as bytecode (parameterized by port, with a variant per link type for IPv4/IPv6, TCP/UDP/SCTP, and fragmented packets), so no filter compiler is needed on the node.
- Business logic and headless frontend fixtures can be tested without a live node. Use the workspace tests, warning-denying Clippy, and formatting checks described under Development; the opt-in Dioxus desktop feature has separate validation gates.

## Installation

### From Releases (Recommended)

Download the latest release for your platform from the [Releases](https://github.com/Handfish/talos-pilot/releases) page.

Install prebuilt binaries via shell script:
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Handfish/talos-pilot/releases/download/<version>/talos-pilot-installer.sh | sh
```
Install prebuilt binaries via powershell script:
```bash
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Handfish/talos-pilot/releases/download/<version>/talos-pilot-installer.ps1 | iex"
```

Install prebuilt binaries via Homebrew
```bash
brew install Handfish/tap/talos-pilot
```

### From Source

```bash
git clone https://github.com/Handfish/talos-pilot
cd talos-pilot
cargo build --release
./target/release/talos-pilot
```

### NixOS

Talos pilot is available as a Nix flake but can also be run without installing.

#### Run talos-pilot without installing

You can test the app directly by using a nix shell

```bash
nix shell github:Handfish/talos-pilot
```

Or run it directly

```bash
nix run github:Handfish/talos-pilot
```

#### Usage in flakes

```nix
# flake.nix
{
  inputs = {
    # ...
    talos-pilot.url = "github:Handfish/talos-pilot";
  };
  outputs =
    {
      self,
      nixpkgs,
      talos-pilot,
      # ...
    }:
    {
      nixosConfigurations.mymachine = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          {
            # provides `pkgs.talos-pilot`
            nixpkgs.overlays = [ talos-pilot.overlays.default ];
          }
          (
            { pkgs, ... }:
            {
              # install talos-pilot
              environment.systemPackages = [ pkgs.talos-pilot ];
            }
          )
        ];
      };
    };
}
```

### Requirements

- Valid `~/.talos/config` (talosconfig)
- Network access to Talos nodes on port 50000
- (Building from source) Rust 2024 edition (1.85+)
- (Linux GUI source builds) `libxcb-render0-dev`, `libxcb-shape0-dev`, `libxcb-xfixes0-dev`, and `libxkbcommon-dev`
- (Opt-in Dioxus desktop builds) Platform WebView support; Linux additionally requires GTK 3 and WebKitGTK 4.1 development libraries. These dependencies are not enabled by the default build.

## Usage

```bash
# Use default context from talosconfig
talos-pilot

# Use specific context
talos-pilot --context homelab

# Set log tail limit
talos-pilot --tail 1000

# Enable debug logging
talos-pilot --debug --log-file ~/talos-pilot.log

# Start the native desktop GUI
talos-pilot --ui gui

# Show a single context in the native desktop GUI
talos-pilot --ui gui --context homelab
```

The TUI remains the default interface. The native GUI provides persistent navigation for cluster overview, single and multi-service logs, processes, network/KubeSpan/packet capture, etcd, workloads, diagnostics, security, lifecycle, node and rolling operations, audit history, and maintenance bootstrap. Destructive GUI actions present the exact target and require confirmation.

Use **Choose talosconfig…** in the GUI toolbar to select a different talosconfig without restarting. The dashboard reloads contexts from the chosen file; a `--context` filter, when supplied at launch, remains in effect.

### Experimental Dioxus Desktop

Dioxus is a separate, opt-in native WebView frontend. It does not replace the TUI or the egui GUI: `talos-pilot` still starts the TUI, and `--ui gui` still selects egui. Default builds do not include Dioxus; selecting `--ui dioxus` without its feature reports the feature-enabled launch command.

```bash
# Run the Dioxus frontend with an explicit Talos configuration
cargo run --features dioxus-ui -- --ui dioxus \
  --config ~/.talos/config --context homelab

# Build a release binary that includes Dioxus
cargo build --release --features dioxus-ui
./target/release/talos-pilot --ui dioxus --config ~/.talos/config

# Open the Dioxus maintenance workflow for an explicit node
cargo run --features dioxus-ui -- --ui dioxus \
  --insecure --endpoint 192.168.1.100
```

Dioxus maintenance mode requires `--insecure` and an explicit valid `--endpoint` host/IP target. Missing or malformed targets are rejected before logging, runtime creation, or UI launch. This route is separate from authenticated cluster collection; insecure mode is intended only for maintenance nodes.

The cluster dashboard includes configured contexts, authoritative node selection, CPU/memory/load snapshots, Talos versions and service status, manual refresh, and Talosconfig browsing. Dioxus is experimental; this is not a claim of identical behavior or production validation across all frontends.

Select a node to open its feature tabs:

| Dioxus feature | Implemented controls |
|----------------|----------------------|
| **Services** | Per-node status, direct service-log navigation and explicitly confirmed service restart |
| **Logs** | Independently collected service streams; service/level filters; case-insensitive search with cyclic match navigation; presentation pause/follow; reconnect retaining history; paginated row/range selection and clipboard copy |
| **Processes** | CPU percentage/time and resident-memory sorting, text/state filters, full tree or selected subtree, selected process details and command copy |
| **Storage** | Disks and volumes with separately reported source availability, pagination, system/install disk details, filesystem and encryption information |
| **Network** | Sortable interface traffic/rates/errors, searchable connections and listener/interface filters, row copy/related service logs, DNS and IPv4 routes, KubeSpan configuration/peers, bounded PCAP capture and file save |
| **etcd** | Correlated membership/status/alarms, observed quorum and reported leader, database/raft details, member search/pagination and roster-validated control-plane logs |
| **Workloads** | Namespace drilldown, controller readiness/issues, problematic pod details, search/health filters and exact-node kubelet logs (not container logs) |
| **Diagnostics** | Categorized state-based checks and evidence, search/status filters, related logs, remediation preview and confirmation; operator commands are copy-only guidance |
| **Security** | Selected-context Talos/PKI certificate inventory and expiry details, inferred RBAC role, STATE/EPHEMERAL volume encryption and source availability |
| **Lifecycle** | Per-node Talos version/platform, NTP synchronization/offset, resource-version configuration drift, etcd safety snapshot, alerts and related logs |
| **Operations** | Single-node or ordered sequential rolling drain/reboot, latest-state prechecks, typed target confirmation, safe cancellation, per-node partial results and structured audit output |

Inspection panels retain the last snapshot while refreshing and show loading/error/source availability and non-fatal warnings rather than treating missing data as healthy or failed. Raw evidence, selected-item details, progress, and exact-YAML review use labeled keyboard-focusable scroll regions with a visible focus outline; Tab into a region to review longer text with the keyboard. Log collection is distinct from presentation pause: pausing the display does not stop collection. Retained logs are bounded to 5,000 rows and 8 MiB, with at most 32 simultaneous services. Reconnect may replay the requested tail; changing the target or applied configuration resets history.

Network source warnings distinguish partial or unavailable evidence from an unhealthy network; an inspection that has not produced a sample is Unknown, not Failed. Missing optional tools or denied API access remain visible as source limitations.

Mutation previews show the actual Talos context, node and address. Drain/reboot additionally require the exact ordered target phrase and explicit acknowledgment of disruption and data-loss options. A session-wide busy guard prevents overlapping mutations and locks target/configuration/navigation changes until the active operation and any compensation/audit finish. Cancellation is cooperative, not a promise to undo an already-issued API call. Reboot completion requires an observed reboot/offline transition followed by Kubernetes readiness; an already-ready node is not counted as a completed reboot.

Packet capture's editable filter is **classic BPF instruction bytecode**, not a `tcpdump` filter expression: enter one `op jt jf k` instruction per line using decimal or `0x` hexadecimal values. **Load API-port exclusion** fills a link-type-aware program excluding port 50000. An accept-all filter can capture its own management traffic and create feedback; captures may contain credentials and other sensitive data. Capture retention is bounded, stopping terminates even an idle stream, and file-save cancellation/errors are reported separately from capture status.

Security's Kubernetes certificate inventory describes the pinned Talos control plane, not credentials from a separately selected kubeconfig file. RBAC roles are inferred from certificate subjects, not live authorization tests. Lifecycle drift compares Talos machineconfig resource versions, not semantic configuration contents; different node configurations may be legitimate. Lifecycle's safety snapshot is informational—operations repeat prechecks before execution.

Storage, KubeSpan, and interface-address association require `talosctl` on the application host. These resource queries carry the applied Talosconfig, selected context and explicit node; they report unavailable rather than falling back to an ambient target.

#### Kubernetes Source Selection

The Dioxus toolbar provides session-only Kubernetes source selection, separate from the Talosconfig:

- **Automatic** retains the existing overview defaults: the pinned Talos control-plane kubeconfig supplies cluster identity, and ambient `KUBECONFIG` is checked for a mismatch.
- **Talos control plane** uses that cluster's Talos-served kubeconfig and ignores ambient `KUBECONFIG`.
- **Selected kubeconfig** accepts a file and optional named Kubernetes context; otherwise it uses the file's current context. Selected credentials are used only after the context's CA certificates match the pinned Talos cluster identity, with HTTPS and TLS verification enabled.

Draft edits do not take effect until **Apply source** is pressed. **Reset to Automatic** restores the default for this session. The dashboard reports the applied request, effective source, and warnings separately; Kubernetes-backed inspection views also report source availability and may expose fallback warnings. An unreadable, invalid, unverified, or unusable selected file can fall back only to the **same verified, pinned Talos control-plane kubeconfig**, with a warning—not an unrelated ambient Kubernetes context. A selected credential/plugin setup or API-request failure can trigger a bounded pinned retry; the effective source reflects that retry, even if it also fails. When no verified pinned identity is available, Kubernetes-backed data is unavailable instead of guessed. These controls do not rewrite either config file or change the TUI/egui defaults.

#### Dioxus Maintenance Workflow

The feature-enabled insecure launch above opens a separate install/bootstrap workflow rather than connecting to authenticated configured clusters. It requires `talosctl` on the host.

1. Set the cluster name, HTTPS Kubernetes API endpoint, machine role, output directory, generated Talos context and exact node target. Hardware inspection reports insecure Talos version, disks with model/serial evidence, and volume availability.
2. Explicitly select a writable non-optical install disk before generation. The selected disk is passed to configuration generation. Use a dedicated output directory: generation can overwrite its configuration files.
3. Review the exact generated YAML, which contains secrets. Oversized/unreadable review data is rejected, not silently truncated. Application uses the frozen reviewed bytes and requires a typed destructive confirmation identifying the target and selected disk; installation may erase that disk and reboot the node.
4. For the **first control-plane node**, wait for the explicit secure Talos target and separately confirm etcd bootstrap with its exact target phrase. Completion requires API-backed secure Talos, healthy etcd and Kubernetes node readiness evidence; unavailable evidence never counts as healthy. If an expected Kubernetes node name is supplied, that exact node must be Ready; otherwise at least one observed node must be Ready. No IP-to-node-name inference is used.
5. For a **worker**, the install workflow stops when its secure Talos API responds; this is not proof of full cluster readiness. The workflow prohibits bootstrapping etcd on that worker.

Authenticated checks use the credentials generated in the chosen directory's `talosconfig`, not an ambient or previously selected cluster. An optional maintenance kubeconfig must match the authenticated target's CA/TLS identity before credentials or plugins are used; unlike the cluster overview's pinned fallback, a rejected maintenance file reports unavailable. Cancellation stops future steps/polling but does not abort a submitted mutation. **Reset workflow** preserves generated files.

### Experimental GPUI Kit desktop

This small **readonly** experiment uses [Longbridge GPUI Kit](https://gpui-kit.com)
(`gpui-kit` 0.7.0), not `gpuikit` or `gpui-ui-kit`. It adds only cluster overview,
Talos services and multi-service live logs. TUI remains the default; egui and
Dioxus are unchanged. Native GPUI dependencies are optional.

Use `--release` for everyday use. Debug builds optimise dependencies, but the
app's own drawing code still runs unoptimised.

```bash
cargo run --release --features gpui-ui -- --ui gpui \
  --config /path/to/talosconfig

# Optional initial context and log tail
cargo run --release --features gpui-ui -- --ui gpui \
  --config /path/to/talosconfig --context homelab --tail 500

# Offline native interaction fixture (never loads credentials)
cargo run -p talos-pilot-gpui --features desktop --example fixture

# Production-view headless UI integration tests
cargo test -p talos-pilot-gpui --features ui-tests
```

The sidebar holds the screens (Overview, Services, Logs) and the configured
contexts; **Settings** at its foot holds the Talosconfig path (press **Apply**;
draft edits do not change the applied target), auto-refresh and light/dark
appearance. The title bar shows the **target node** that Services and Logs look
at; change it there, by clicking a node card, or with the arrow keys on the node
list. Overview opens with summary tiles (nodes, etcd quorum, service health,
peak memory) and one card per node with a load sparkline (the last 5 minutes,
kept by the UI) and memory use, which turns amber at 85 % and red at 95 %. A
table view is one click away. Refresh is manual or every 15 seconds; a ring
around the refresh button counts down. Unavailable metrics and health remain
explicitly unknown, never failed; failed refreshes keep the previous snapshot
and label it stale. The overview reuses core's validated Kubernetes-source
semantics; where the node list came from is on the **Roster** chip, and warnings
appear inline. This prototype does not add Kubernetes credential selection
controls.

Services is a list with a detail pane that shows the full health message and
opens that service's logs. Logs has one chip per service: click the chip to
collect it, the eye to show or hide its lines. Start/Stop controls stream
ownership and keeps running while you visit other screens (the sidebar shows a
live dot); Following/Paused controls only the viewport. Wheel or scrollbar review
pauses follow. Search navigates matches without hiding nonmatching lines.
Wrapping uses measured variable-height virtual rows; unwrapped lines can scroll
horizontally. Click a line, use Shift-click or Shift-arrows to extend, then
**Copy** or Command/Control-C in the log viewport. Copy covers complete selected
retained lines passing the current filters, not arbitrary cross-row text
dragging. The UI embeds JetBrains Mono and IBM Plex Sans Condensed (both SIL OFL
1.1; licenses in `crates/talos-pilot-gpui/assets/fonts`).

History is bounded to 5,000 lines and 8 MiB raw payload; stream delivery to 256
events, 64 events per UI turn, and 16 concurrent services. Selection is bounded
to 200 complete lines, copy to 1 MiB, and each line to 64 KiB. Eviction is visible.
Live service-choice changes restart from now without replaying unchanged
services' tails; an explicit restart may replay the configured tail and says so.
Command/Control-1/2/3 selects screens; Alt-Up/Down selects configured contexts.

GPUI Kit's [platform prerequisites](https://gpui-kit.com/docs/installation)
include Rust 1.92+, macOS 15+ with Xcode tools, or the corresponding Windows/Linux
native development libraries and graphics driver. Minimal builds and existing
minimal-platform CI jobs do not enable GPUI. `--ui gpui` without `gpui-ui` explains
how to rebuild; insecure maintenance mode is intentionally unsupported.

This is not full frontend parity or a performance claim. Fixture/headless
interaction results do not establish live-cluster or platform-wide correctness.

### Bootstrap Wizard (Insecure Mode)

For bootstrapping new clusters on bare metal or VMs in maintenance mode, talos-pilot provides an interactive wizard:

https://github.com/user-attachments/assets/0955b5a9-e35d-4fc6-a53d-db16e4558fc9

```bash
# Connect to a node in maintenance mode
talos-pilot --insecure --endpoint <node-ip>
```

The wizard is also available in the native GUI:

```bash
# Open the native maintenance-mode bootstrap workflow
talos-pilot --ui gui --insecure --endpoint <node-ip>
```

Both interfaces require selecting an install disk before generating configuration, apply configuration only after confirmation, and determine readiness from Talos, etcd, and Kubernetes API results.

Once complete, you can manage the cluster using standard talos-pilot commands.

### Keyboard Navigation

| Key | Action |
|-----|--------|
| `?` | Help |
| `q` / `Ctrl+C` | Quit |
| `Esc` | Back / Close |
| `j/k` or `↑/↓` | Navigate |
| `Enter` | Select / Expand |
| `Tab` | Next panel |
| `r` | Refresh |
| `a` | Toggle auto-refresh |
| `/` | Search (in logs) |
| `n/N` | Next/prev search match |

### View Shortcuts

| Key | View | Description |
|-----|------|-------------|
| `c` | Security | PKI and encryption audit |
| `s` | Storage | Disk list with system disk indicators |
| `l` | Logs | Single service logs |
| `L` | Multi-Logs | Interleaved multi-service logs |
| `p` | Processes | Process tree view |
| `n` | Network | Interface stats, connections |
| `e` | etcd | Cluster health, members |
| `w` | Workloads | K8s deployment health |
| `y` | Lifecycle | Version status, alerts |
| `d` | Diagnostics | System health checks |
| `o` | Operations | Single node operations |
| `O` | Rolling | Multi-node rolling operations |

## Architecture

A five-crate workspace separates shared collection and operations from frontend rendering, so business logic and headless fixtures can be tested without a terminal or a live cluster:

```
crates/
├── talos-rs/           # Talos gRPC client library
├── talos-pilot-core/   # Shared collection and business logic
├── talos-pilot-tui/    # Default terminal UI (ratatui)
├── talos-pilot-gui/    # Native desktop GUI (egui/eframe)
└── talos-pilot-dioxus/ # Opt-in native WebView frontend (Dioxus)
```

### Core Modules

| Module | Purpose |
|--------|---------|
| `indicators` | HealthIndicator, QuorumState, SafetyStatus |
| `formatting` | format_bytes, format_duration, pluralize |
| `selection` | SelectableList<T>, MultiSelectList<T> |
| `async_state` | Loading/error/refresh state management |
| `diagnostics` | CheckStatus, CniType, PodHealthInfo |
| `constants` | Thresholds, CRD lists, refresh intervals |
| `network` | Port-to-service mapping, classification |
| `errors` | User-friendly error formatting |

### Key Technologies

- **Rust 2024 edition** with async/await
- **tokio** - Async runtime
- **ratatui + crossterm** - TUI framework
- **egui + eframe** - Native desktop GUI
- **Dioxus desktop** - Opt-in WebView frontend
- **tonic + prost** - gRPC client
- **kube-rs** - Kubernetes client
- **color-eyre** - Error handling

## Development

```bash
# Run all tests
cargo test --all

# Run with debug output
RUST_LOG=debug cargo run

# Watch logs in another terminal
tail -f /tmp/talos-pilot.log

# Check for warnings
cargo clippy --all --all-targets -- -D warnings
```

Before merging, also run `cargo fmt --all -- --check`. Changes to the opt-in Dioxus frontend should additionally pass `cargo test --package talos-pilot-dioxus --features desktop` and `cargo clippy --all --all-targets --features dioxus-ui -- -D warnings` on a host with its platform WebView build dependencies. Headless fixtures verify state and rendered semantics; they do not substitute for keyboard/WebView checks or live-cluster operation validation.

### Local Testing with Docker

See [docs/local-talos-setup.md](docs/local-talos-setup.md) for setting up a local Talos cluster.

## Contributing

Contributions are welcome. Please keep changes aligned with the [Design Philosophy](#design-philosophy) above: check real system state, degrade gracefully, and never report failure when "unknown" is the honest answer.

## License

MIT License - see [LICENSE](./LICENSE) for details.

## Acknowledgments

- [Talos Linux](https://www.talos.dev/) by Sidero Labs
- [k9s](https://k9scli.io/) for TUI inspiration
- [ratatui](https://ratatui.rs/) for the TUI framework
- [egui](https://www.egui.rs/) and [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) for the native desktop GUI
