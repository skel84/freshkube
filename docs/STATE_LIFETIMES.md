# State and request lifetimes

This describes the implementation reviewed for [#24](https://github.com/skel84/freshkube/issues/24) on main `1c328f3`. Domain queries and mutation policy belong to core; desktop entities own presentation, requests and GPUI delivery. Use the narrowest existing owner, rather than making every feature a `Loader`. Start I/O from lifecycle methods or actions, mutate entities through GPUI contexts, and notify after a coherent change. Never start requests in `render`.

## Identity and generation

[ACCESS_IDENTITY.md](ACCESS_IDENTITY.md) is the access contract. These identities have different jobs:

| Identity | What it distinguishes |
| --- | --- |
| Selected configuration/context | The source the user chose; it is not proof that its files or authenticated client are unchanged. |
| `AccessIdentity` / `KubeSource.id` | Access session plus inspected local configuration revision. Ordinary Kubernetes rows, caches, detail, discovery, search and summary use this boundary. |
| `Target` | Desktop node request identity: epoch, context, node and address. Node selection advances the epoch; access replacement also invalidates targets. Selecting a node alone does not replace compatible Kubernetes access. |
| `Request<I>` generation | One `Snapshot::begin` invocation, even for the same identity. A → B → A cannot revive A's earlier completion. |
| Summary `SessionIdentity` | Access key plus observation-session revision; watcher/relist generations and publication revisions remain separate. |
| Provider/request identity | Monitoring's source/dashboard/variables/range and Coroot's server/project/access association/application/report/interval, as represented by each feature. These are not all encoded in `Target`. |

`Snapshot` does not infer access from a path or inspect credentials. The shell's overview uses `Snapshot<ClusterOverview>` with `AppliedConfig`, so [its completion boundary](../crates/freshkube-desktop/src/desktop/access/mod.rs) separately checks the shell epoch, configuration revision and collected access. `invalidate_target` cancels delivery/read jobs, clears ordinary snapshots and dependent views, and stops the summary session. Direct Kubernetes configuration reload has its own access and connection-generation guards. Replacing files at the same path clears incompatible data even when new credentials fail to connect. Files are reinspected at reload/refresh, not watched continuously. A running shell keeps the existing access until replacement is confirmed; declining marks the overview stale and leaves that explicit session intact.

Compatible watch/transport reconnects, client-cache eviction and direct `forget()`/retry retain access identity and compatible selection. Rebuilding an unusable authenticated Talos client starts a new access session. Remote credential rotation, auth-plugin revisions and a shared provider/session registry remain future work; do not claim the local file boundary implements them.

## Snapshot transitions

Core's [snapshot.rs](../crates/freshkube-core/src/snapshot.rs) defines `Snapshot<T, I>` and `Request<I>`, with their unit tests. Desktop's [state.rs](../crates/freshkube-desktop/src/state.rs) names them `state::Snapshot<T, I = AppliedConfig>` and `state::Request<I>`, used by the shell and Observability, and wrapped by screen loaders.

| Transition | Data and status |
| --- | --- |
| `begin(identity)` with the same identity | Retains data and timestamps, advances generation, sets loading, clears the error and marks any retained data stale while reading. |
| `begin(identity)` with a different identity | Clears data and both timestamps, advances generation and starts loading. No stale value from the old identity is retained. |
| `apply(&request, Ok(data))` for the current loading request | Replaces data, stops loading, clears stale/error, records success time and clears failure time. |
| `apply(&request, Err(error))` for the current loading request | Stops loading, records error/failure time, keeps any same-identity data and success time, and marks that data stale. With no successful data, this is a failure without a stale value. |
| Obsolete or already-applied completion | Returns `false` without changing state. Both identity and generation must match, and the snapshot must still be loading. |

`data()` alone does not mean current evidence. Render retained content with loading/error/freshness information; do not turn a failed read into an empty collection or infer absence from stale data. `last_successful()` and `last_failure()` describe freshness. `data_mut()` updates an independent projection without changing request coverage or timestamps.

Resetting a loader replaces its snapshot **and** drops its read and delivery tasks. A bare `Snapshot::default()` starts its generation over; it is not an independent cancellation mechanism. Owners must cancel old delivery or preserve an additional epoch/sequence boundary when resetting state.

## Ordinary screen reads

[screens::Loader<T>](../crates/freshkube-desktop/src/screens/mod.rs) owns a `Snapshot<T, Target>`, a Tokio `OwnedJob` and a GPUI `Task<()>`. `load` calls `begin`, runs a `Send + 'static` future through `backend::spawn_job` under `SCREEN_DEADLINE`, and keeps a GPUI receiver task. That task uses the weak screen entity, finds its loader through `slot`, and calls `apply` before releasing the read job. Failure to receive a result becomes an error. The caller notifies when starting; delivery notifies on completion.

This is the live-read portion of [EtcdScreen::refresh](../crates/freshkube-desktop/src/screens/etcd.rs), inside its `ScreenPanel` implementation, after obtaining `source` and `live`:

```rust
let request = EtcdInspectionRequest::new(source.inspection_target());
self.loader.load(
    source.target.clone(),
    &self.runtime,
    "the etcd status",
    async move {
        collect_etcd_health(live.client, request)
            .await
            .map_err(|error| error.to_string())
    },
    |screen: &mut Self| &mut screen.loader,
    cx,
);
cx.notify();
```

That screen owns `Loader<EtcdHealthSnapshot>`, compares old/new `source.target` in `set_source`, and calls `reset()` on a change. `activate` loads only with no data and no request in flight; `refresh` guards duplicate work. Fixtures use `resolve(source.target.clone(), example(&source))`. Read `data()`, `error()`, `is_loading()` and timestamps during rendering. `Loader` does not expose an `is_stale()` reader; its underlying snapshot tracks staleness and the shared header/failure banner present refresh state. Do not invent a loader method or put cluster I/O on the UI thread.

## Read-job and delivery ownership

[OwnedJob](../crates/freshkube-core/src/job.rs) (core's `job`, which desktop's `backend` re-exports) wraps a Tokio `JoinHandle<()>`; its `Drop` calls `abort()`. Dropping a Tokio join handle alone would detach it, which is why ordinary reads keep this guard. `spawn_job` also selects on `sender.closed()`, so dropping its receiver cancels pending work. It applies a deadline to reads. Already-running `spawn_blocking` work cannot be aborted by this guard; local file reads are bounded and off GPUI.

Keep the GPUI delivery `Task` as well as the `OwnedJob` on the owner. Replacing the pair drops old work and delivery. Use `cx.spawn`/`spawn_in` with weak entities, check the captured identity/generation in `update`/`update_in`, and handle the entity/window disappearing. Cancellation saves work; completion guards establish publication correctness even if an answer raced cancellation. Ordinary read delivery is not detached.

The shell sends `set_source` to all retained `ScreenPanel`s and dispatches `activate`/periodic `refresh` only to the visible one. `ScreenPanel` has no deactivate/hide hook. Navigation alone leaves generic loaders intact, so an in-flight read can finish while hidden. Operations' `set_source` only invalidates its preview and audit; `activate` reads them, so a hidden Operations reads nothing for a new target or a same-target update (#44). Adding universal hide cancellation or redesigning that ownership is outside #24. New page-owned work should use an explicit visibility boundary rather than infer cancellation from navigation.

## Feature owners and visibility

| Owner | Requests and retained lifetime |
| --- | --- |
| `Pilot` summary session | `summary_job` and `summary_task` keep core's nine compact reflector stores on every page. Health consumes publications; visible Lifecycle subscribes to the same Nodes publication. Manual Refresh relists; the automatic Talos refresh and node selection preserve the compatible session. Access replacement or owner drop cancels its driver and child watches. Resources Table watches remain separate. |
| Resources screen/detail pane | Visible list/watch and metrics belong to the Resources entities; hiding cancels ordinary work. Detail reads carry object identity and sequence and respond to list versions, at most once a second. Revealed Secret values are forgotten on hide/new version. A running shell is retained separately. |
| `LogView<S>` with `TalosLogs` | The view owns retention, review/search/selection and row measurement; the source owns its Tokio stream job and GPUI delivery. `LogView::set_visible(false)` coalesces received lines before applying them to retained review data; it does not stop collection. Stop, target/collection replacement or owner drop cancel the source stream. Stream revision/target guards reject old batches. |
| `Dock` log tabs | Each tab owns a `LogView<PodLogs>` or `LogView<WorkloadLogs>` and a feed: a pod's one-pod watch (`follow_pods` by name, checked by UID) or a workload's single object read for its selector, an `OwnedJob` and GPUI delivery per tab. A tab starts when it is first selected while the dock shows, then **reads while it is open, whatever page, object or pane shows**; minimizing the dock keeps it reading and only `set_visible` changes. Closing the tab (its ×, a middle-click, Command-W), the dock's × or Close others/all, another `KubeSource.id`, or none under another access identity (`Pilot::kube_identity`, none while another context loads), drops view, stream, feed job and delivery, while a source missing under the same identity keeps them; a pod that goes ends its tab's stream and keeps its lines; the same id refreshes access. Saved tabs restore unstarted for their context and read nothing until selected and shown. Resume reads from the retained position; previous-instance reads require an explicit choice. Stream sequence and pod identity protect delivery. |
| `MonitoringPage` | Page requests own the read/delivery pairs; the board owns variable and viewport panel requests, with marker requests alongside. A new dashboard, variables, range or refresh takes a new generation. Hide drops discovery/confirmation, variables/panels/markers and the refresh timer, retaining usable same-source state. A different `ClusterSource.id` (the shell's `KubeSource.id`) clears the connection/board/markers. Shown pod/node history has its own owner and reads through the confirmed/remembered Prometheus. |
| `ObservabilityPage` | `Live` holds `ReadJob` pairs and generation, with independent `Snapshot`s for apps/map/REST/extended report evidence. Hide increments generation and clears jobs; completions check visibility/generation and snapshot request identity. Server/authentication edits or Disconnect clear the provider and observations immediately, including failed replacement. Project/access association/application/report/interval changes invalidate incompatible evidence. Same-identity refresh failures retain stale evidence. |
| Dock shell tab `ShellView` | A pick from the pane's Shell menu, or the tab's Start, creates a session on that running container. Starting owns a read/delivery pair; Running owns `ExecSession`, an output-forwarding `OwnedJob` and GPUI delivery. The tab owns the view and a pod watch for its containers, and keeps the session whatever page, object or pane shows. Closing a running shell's tab, another `KubeSource.id`, window close and quit ask first (`unless_shell`/`may_close`); a closed tab ends its session with `close_session`. Explicit End stops it, and minimizing or hiding does not. A restored tab starts idle. Sequence guards protect terminal delivery. |
| App `ForwardList` / `ForwardView` | Explicit Forward captures its connection and specification. The app owns views/session guards and status delivery, independent of the pane/page/context. Stop closes the session; an ended entry remains until Remove. Context replacement does not retarget a running forward. Quit/window close asks for shells and forwards together. |

Sources: [summary](../crates/freshkube-desktop/src/desktop/kubernetes_summary/mod.rs), [Resources](../crates/freshkube-desktop/src/resources/screen/mod.rs), [detail](../crates/freshkube-desktop/src/resources/pane/mod.rs), [logs](../crates/freshkube-logs/src/lib.rs), [Talos](../crates/freshkube-desktop/src/logs/talos/mod.rs), [pod logs](../crates/freshkube-desktop/src/logs/pod/mod.rs), [dock](../crates/freshkube-desktop/src/desktop/dock/mod.rs), [Monitoring](../crates/freshkube-monitoring/src/page/mod.rs), [Observability](../crates/freshkube-observability/src/connection.rs), [shell](../crates/freshkube-desktop/src/resources/shell/mod.rs), [forwards](../crates/freshkube-desktop/src/forwards/mod.rs). Provider/session details stay in [MONITORING.md](MONITORING.md), [COROOT.md](COROOT.md), [POD_EXEC.md](POD_EXEC.md) and [PORT_FORWARD.md](PORT_FORWARD.md).

The summary reducer has its own freshness contract: `InitDone` commits a complete replacement, including empty results; an unfinished/failed relist retains prior complete evidence as stale. A 403 remains refused independently of other sources. `Part::loaded()` can return retained evidence, so check `is_current()` before inferring absence. Its generations reject obsolete watcher events and the shell checks session identity/publication revision before applying. See [coverage and resynchronization](ROADMAP.md#coverage-failure-and-resynchronization).

## Operations mutations

Preview and audit reads in [OperationsScreen](../crates/freshkube-desktop/src/screens/operations/mod.rs) are cancellable ordinary work. A confirmed run follows a separate contract:

1. Desktop prepares a preview from an identity-validated selected Kubernetes client and fresh node/etcd state. Failure evicts the reused client for retry. The confirmation fingerprint covers context, target epoch, operation, ordered node names/addresses, options and safety verdicts. `mutation::confirm` checks the current fingerprint and app-wide slot when Confirm is clicked; stale previews cannot submit.
2. `start_run` captures the confirmed plan/source and acquires `Operations::begin`'s app-wide `OperationTicket`. Source/selection changes clear preview state but leave the captured run intact. A second operation cannot take the occupied slot.
3. Desktop's `run_live` passes a selected-access revalidation future (`forget_kubernetes`, then `kubernetes`) to core's `run_selection`, together with explicit `OperationConfirmation`, cancellation predicate and progress callback. Core polls connection setup only after confirmation/cancellation gates. Local configuration mismatch fails the setup rather than falling back to ambient access.
4. Core owns whole-selection and fresh per-node prechecks, identity/readiness/scheduling and etcd safety policy, ordered execution, drain options, read deadlines, partial outcomes, panic recovery and both step/outcome audits. Unknown etcd safety blocks reboot/shutdown. A frontend preview cannot substitute for current core policy.
5. Tokio runs the mutation future without an `OwnedJob`. An unbounded channel carries progress, node results and completion to a **detached GPUI delivery task**. That task owns the ticket, updates the global status even if the screen is gone, filters screen updates by run id, and calls `Operations::finish` after completion/channel end. Failed sends stop display only; they do not abort the submitted worker or release its slot early.
6. Cancel sets the ticket's atomic cooperative flag. Core checks before/between mutations and during delays/readiness waits. A submitted patch, eviction or Talos mutation is awaited without the ordinary read-job timeout/abort, even when Cancel is requested. Then remaining work stops at its cancellation gates; drain compensation can restore scheduling despite cancellation. Cancellation after an accepted reboot may stop readiness verification and leave the node cordoned, with that outcome reported explicitly.

Do not put a confirmed run in `Loader`, `spawn_job` or a receiver-closed abort branch. Submitted mutations must finish their current step; a closed page or replaced access cannot undo a request already sent. This guarantee applies while the application/runtime lives, not to process termination or a crash.

Sources: [mutation slot and confirmation](../crates/freshkube-desktop/src/mutation.rs), [core selection runner](../crates/freshkube-core/src/operations/selection/mod.rs), [node policy](../crates/freshkube-core/src/operations/mod.rs). Existing core fake-client tests cover cancellation while a mutation is submitted, safety gates, failures and recovery; desktop tests cover confirmation and slot ownership. #24 changes guidance, not this policy.
