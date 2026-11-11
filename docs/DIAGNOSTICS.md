# Diagnostics and the Talos API

The reliability rules for diagnostic checks with their examples, the checklist for a new check, and the Talos reference, moved from AGENTS.md unchanged (#587). AGENTS.md's [Diagnostics](../AGENTS.md#diagnostics) keeps the rules in short.

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

## Code patterns

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

## Reliability Checklist for New Checks

- [ ] What is the source of truth for this state?
- [ ] Can we check that source directly (file, API)?
- [ ] If using logs, do we validate timestamps?
- [ ] What happens if the data source is unavailable?
- [ ] Can this check produce false positives?
- [ ] Is the failure mode graceful?

## The harness

`DiagnosticSnapshot::evaluate` makes every check, from evidence alone, so the harness
(`freshkube-core`'s `diagnostic_runner/harness_tests.rs`, safety guarantee 19 in
[SAFETY.md](SAFETY.md#19-every-diagnostic-check-says-unknown-with-its-source-down-and-reports-no-failure-on-a-healthy-fixture))
runs `evaluate` on snapshots in two cases, for each CNI provider (Flannel, Cilium,
Calico, unrecognised) and for control-plane and worker nodes. You add no entry for a new
check:

- **Source down:** every source of the snapshot unavailable. Every check must say unknown,
  offer no fix, and not panic. Then each source down alone over a healthy snapshot may
  change a check only to unknown. One more scenario has the Talos file reads fail with
  "not found" text.
- **Healthy:** a healthy snapshot gives no failing check.

**What it covers.** Every check `evaluate` makes from a snapshot. The snapshots are not
typed whole: they are built through the decisions the collector makes without I/O, so a
decision that turns an error into a verdict is run too (`file_probe` and
`cni_type_from_files` for the CNI provider, `derive_addon_statuses` and
`unavailable_addon_pod_sources` for addons, `service_health_source` for service
health). Keep a new decision of that kind a pure function and build the harness's
snapshots through it. What the collector reads from a live API stays out of reach.

**How a new check is picked up.**
- A check added to an existing builder runs in both cases with no edit.
- The snapshots use struct literals that name every field, so a new source field doesn't
  compile until the harness gives it a healthy and a down value.
- A test scans `evaluate.rs` for the check ids written as literals and fails when no
  scenario makes one. That finds an id that is never reached, not a branch under an id
  that is (a new branch under `cni` or `cilium_operator` slips through), and its helper
  names (`cni_pod_check(`) are a list kept by hand, like the `ONE_DOWN` table of
  single sources. The all-down case is what finds a source `ONE_DOWN` leaves out.

**Known gaps.** A check that fails a scenario today is listed in `KNOWN_GAPS` with the
scenario and its issue, and is skipped only there. The harness fails when a listed gap no
longer fails, so a fix removes its entry. Today there are none: #1005 (`cni` when the file reads say
"not found") is closed, because only the gRPC `NotFound` code or the file system's own
"no such file or directory" reply counts as a missing file, and any other read error is
unavailable, so the check says unknown.

## Before Merging

1. **No false positives:** Create error condition, fix it, verify check shows healthy
   (`cargo test -p freshkube-core --lib safety_g19` runs all five harness tests)
2. **Graceful degradation:** Disconnect API, verify no crashes (the harness's
   source-down case does this for every check)
3. **Stale log immunity:** Ensure old log messages don't affect current state
4. **Zero warnings:** `cargo clippy --workspace --all-targets -- -D warnings` should pass clean
5. **Formatting:** `cargo fmt --all -- --check` should pass

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
