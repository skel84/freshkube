# State and request lifetimes

Every ordinary read must be owned by a `Loader<T, I>` or a `Watch<T, I>` from
[`freshkube-reads`](../crates/freshkube-reads/src/lib.rs). The primitive owns the
work and its GPUI delivery, and checks which request may publish. The feature
chooses the identity, deadline, visibility scope and refresh policy. Domain
queries belong in core or the domain crate; presentation belongs in the view.
Start reads from lifecycle methods or explicit actions, never from `render`.

There are exactly [two exception classes](#exceptions): Operations mutations
and user-started sessions. The remaining counters outside the primitives
are the [allowed list](#allowed-counters) from
[#582](https://github.com/skel84/freshkube/issues/582), each with its specific
reason to remain.

## Snapshot and Loader in brief

[`freshkube_reads::Loader<T, I>`](../crates/freshkube-reads/src/loader/mod.rs)
wraps core's [`Snapshot<T, I, String>`](../crates/freshkube-core/src/snapshot.rs)
and owns an optional Tokio `OwnedJob`, a GPUI delivery `Task<()>`, visibility,
answer age and a display revision. `Snapshot<T, I, E = Failure>` retains the
last answer for one identity; it does not own or cancel work. Desktop's
[`state::Snapshot`](../crates/freshkube-desktop/src/state.rs) supplies the default
identity `AppliedConfig`.

Use `load(identity, &runtime, deadline, what, work, slot, cx)` for a bounded
Tokio read. `slot` finds the same loader on the weak view entity when its
answer arrives. Desktop's
[`screens::Loader<T>`](../crates/freshkube-desktop/src/screens/loader.rs) is a
wrapper around `Loader<T, Target>` whose `load` supplies `SCREEN_DEADLINE`.

When landing needs a window, derived display data or a typed result kept
elsewhere, use `start(identity)` to get a `Request<I>`, `run(job, task)` to give
the loader ownership of the work, and `land(&request, result, cx)` to publish.
Only update dependent state when `land` returns `true`. A completion loader
may store `()` while the feature keeps its data and typed errors; that feature
must apply the same identity and stale-on-failure rules to its retained data.

For immediate example answers use `resolve(identity, result)`. It cancels
pending work and preserves the request sequence, but supplies no freshness
age. Use `start` and `land` when an example needs the same timed freshness as
a live answer; a held loading example can `start` without `run`. `reset()`
cancels work and clears data, errors and timestamps while retaining visibility;
`expire()` only clears the answer's freshness age.

Render reads `data()`, `error()`, `is_loading()`, `is_stale()`,
`last_successful()` and `last_failure()`. Derive display collections when the
loader's `revision()` changes and call `cx.notify()` after coherent state
changes outside render.

## Identity and generation

[ACCESS_IDENTITY.md](ACCESS_IDENTITY.md) defines the access boundary. Identity
answers whose data this is; generation answers which request may publish.

| Boundary | Meaning |
| --- | --- |
| Selected configuration/context | The user's choice; a path alone does not prove unchanged files or authenticated access. |
| `AccessIdentity` / `ClusterSource.id` | The access session and inspected local configuration revision. Use this boundary for Kubernetes rows, discovery, details, search and summary. |
| `Target` | A node request's epoch, context, node and address. Node selection invalidates node work without replacing compatible Kubernetes access. |
| Provider or detail identity | The source and subject, plus the dashboard, variables, range, object version or report that determine whether data can be reused. |
| `Request<I>` or a watch's internal ticket | The identity and request generation captured when work starts. Even A → B → A must reject A's earlier completion. |

`Snapshot::begin` advances the request generation. `apply` accepts only the
current identity and generation while that request is loading; a duplicate
completion is also rejected. `Loader::land` additionally rejects a read cut
short by hiding. Cancellation saves work; these checks establish publication
correctness even if an answer races cancellation.

Keep a primitive on its actual owner across requests. `resolve` invalidates a
completion without restarting the generation sequence. `reset` replaces the
snapshot and cancels both handles; do not retain an old token and manually
land it after resetting to the same identity. Likewise, stop or restart a
`Watch` rather than replace it inside its own delivery, where a fresh
primitive's generation could repeat.

The primitives do not inspect credentials or infer access from a file path.
The caller must replace incompatible identities and apply the access checks
at the completion boundary. A failed connection after a file replacement
must not expose the previous identity's data.

## Allowed counters

The lead's [list after slice J](https://github.com/skel84/freshkube/issues/582#issuecomment-6270431283)
permits only these counters outside `freshkube-reads`. These describe identity,
coverage, scheduling, local files or the two exception classes; they do not
permit another ordinary read lifetime. Core's `Snapshot` supplies the request
generation used inside `Loader`.

| Counter | Why it stays |
| --- | --- |
| Summary reducer's `State.generation` (`freshkube-core/src/kubernetes_summary/session.rs`) | Identifies reducer coverage independently of the driver's Watch. |
| Packet capture's `CaptureState.generation` (`freshkube-core/src/pcap.rs`) | Guards the explicit capture/save session. |
| Forward routes' `Route.generation` (`freshkube-core/src/resources/forward/mod.rs`) | Rejects obsolete route/session updates. |
| `LogView.generation` (`freshkube-logs/src/source_api.rs`) | Invalidates the view's retained review and measurements when its source changes. |
| Table projection's `ResourceProjection.generation` (`freshkube-resources/src/projection.rs`) | Numbers derived table projections, not cluster reads. |
| `Pilot.epoch` (`freshkube-desktop/src/desktop/mod.rs` and `target.rs`) | Identifies the source/target supplied to read owners and action guards. |
| `ResourceStore.epoch` (`freshkube-resources/src/store.rs`) | Identifies the observation session used as its Watch's identity. |
| `MonitoringPage.generation` (`freshkube-monitoring/src/page/board.rs`) | Schedules panels around their Loaders for the selected dashboard window. |
| `Detail.reveal_epoch` (`freshkube-resources/src/detail.rs`) | Invalidates deferred UI reveal work. |
| `SetupFile.write_generation` (`freshkube-desktop/src/setup_file.rs`) | Keeps local preference-file inspection from overwriting a newer write. |
| `Memory.setup_generation` (`freshkube-observability/src/remember.rs`) | Rejects obsolete inspection of the app's local setup file. |
| Log streams' `StreamSet.next_generation`, copied `Stream.generation` and `Fed.generation`, and example `Stream.sequence` (`freshkube-desktop/src/logs/streams.rs`) | Distinguish retries and number example lines within user-started log sessions. |
| `Maintenance.generation` (`freshkube-desktop/src/maintenance/mod.rs`) | Guards a mixed read/run workflow under the mutation exception until its own migration. |

Paths in this table are relative to `crates/`. Observability's page reads and
workspace-entry location use completion Loaders; their former `Live.generation`
and `Pilot.entry_generation` are gone. The entry owner retains its request
token after location completes, so held links and draft inspection still
reject an entry that moved on.

## Snapshot transitions

Core's [`snapshot.rs`](../crates/freshkube-core/src/snapshot.rs) defines the
state transitions and their tests; the loader delegates publication to it.

| Transition | Data and status |
| --- | --- |
| Start for the same identity | Keep data and timestamps, advance generation, set loading, clear the error and mark retained data stale while reading. |
| Start for a different identity | Clear data and timestamps, advance generation and start loading. |
| Current success | Replace data, stop loading, clear stale/error, record success time and clear failure time. |
| Current failure | Stop loading, record error/failure time, retain same-identity data and its success time, and mark retained data stale. Without a prior success there is no stale value. |
| Obsolete or duplicate completion | Return `false` and change nothing. |

Do not turn a failed read into an empty collection or infer absence from stale
data. Show the retained value alongside refresh/error state and timestamps.
`data_mut()` changes data in place without advancing the revision or request
coverage; its caller is responsible for updating any display projection.

## Streams and visibility

[`freshkube_reads::Watch<T, I>`](../crates/freshkube-reads/src/watch/mod.rs)
owns a stream's Tokio job and GPUI delivery. `start` replaces the previous
stream; each batch checks its start's identity and generation before the
owner's apply callback runs. `revision()` rises per accepted batch. `stop`
advances the generation and drops work and delivery. A natural end clears the
running state; `start_then` also calls the owner's end hook after its last
batch. Explicit stop or replacement does not call that hook.

`Pace` bounds the channel capacity and batch size and sets the minimum interval
between batches. Delivery yields between batches so a busy channel cannot
hold the UI thread. `open` creates a shared delivery without a Tokio job;
`sender` supplies additional writers. A sender from an earlier start cannot
publish after stop or restart.

A watch keeps no application data or error state itself. Its owner retains
what the batches build, clears incompatible data on identity replacement, and
marks retained evidence stale after failure or loss of coverage. A failed
relist cannot prove an empty collection. Reducer generations and publication
revisions describe data coverage, not another read lifetime; see
[coverage and resynchronization](ROADMAP.md#coverage-failure-and-resynchronization).

[`OnShow`](../crates/freshkube-reads/src/visibility.rs) defines a loader's
show policy. Both primitives start visible; an initially hidden owner must
forward its initial state too.

| Visibility change | Primitive contract |
| --- | --- |
| `Loader::set_visible(false, policy, cx)` | Drop job and delivery, keep last-known data, stop reporting loading and reject a cut read's late answer, including a held read with no work. |
| `Loader::set_visible(true, ReadIfOlder(threshold), cx)` | Return whether the owner should read: true when the last successful answer is at least the threshold old by the executor's clock, or has no age. Failure, `resolve`, `expire` and a cut read count as old. |
| `Loader::set_visible(true, Never, cx)` | Never request a read merely because the owner shows; explicit actions or the owner's existing scheduling still decide when to read. |
| `Watch::set_visible(false)` | Stop a running stream and keep what it delivered on the owner. |
| `Watch::set_visible(true)` | Return true if hiding stopped the stream, regardless of age. A stream that was not running when hidden is not started by showing. |

These methods return a decision; they do not start work. The caller performs
the read or restart and keeps its existing timing and deadlines. Repeated
visibility values do nothing. A stream explicitly stopped while hidden still
asks to restart on show; the owner can decline that restart.

## Read-job and delivery ownership

Core's [`job::OwnedJob`](../crates/freshkube-core/src/job.rs) wraps a Tokio join
handle and aborts it on drop; dropping a bare join handle would detach work.
`spawn_job` also stops when its result receiver closes and applies the read's
deadline. Already-running `spawn_blocking` work cannot be aborted this way;
local file reads must remain bounded and off GPUI. The primitive retains both
the job and the GPUI delivery task so replacement or drop cancels both.
Delivery uses weak entities and must tolerate a disappearing entity or window.

The shell's [`Pilot::sync_visible`](../crates/freshkube-desktop/src/desktop/pages.rs)
notifies every retained `ScreenHandle` through
[`ScreenPanel::set_visible`](../crates/freshkube-desktop/src/screens/mod.rs),
including hidden pages. Source updates reach every page; activation and
periodic page refresh reach only the shown page. Node-tab visibility nests
inside Nodes. Forward the visibility boundary to the primitive; a page that
keeps the trait's default no-op still has a migration gap.

Use the lifetime of the actual consumer: a page-owned read stops on hide, a
shared connection remains needed while any visible consumer uses it, and a
session-owned summary remains needed across page navigation. These are
ordinary `Loader`/`Watch` owners, not exception classes. For example, the
summary driver's watch is retained by
[`Registry`](../crates/freshkube-desktop/src/desktop/session.rs), stopped on
session replacement or parking, and keyed by context/access. Its reducer's
coverage rules remain separate. Monitoring's
[connection owner](../crates/freshkube-monitoring/src/page/connection.rs) must
still receive a hide and retain recovery requested by visible history while
the dashboard is hidden. See [Monitoring](features/monitoring.md).

### The change page's outcome read

What Coroot found after a deploy
([`change/outcome/`](../crates/freshkube-applications/src/change/outcome/mod.rs),
[COROOT.md](COROOT.md#what-its-metrics-did-after-it)) is an ordinary
page-owned read with one owner, the change page.

| Boundary | Owner and lifetime |
| --- | --- |
| The read | One `Loader<(), Key>` per page and one job in flight, for the selected Deployment hop. Its identity is the hop, the entry's link, the Deployment, its hash, the window and the `ProviderId`. Hiding the page, selecting another hop, replacing the reader or a `Connection` that changes the connection drops the job and its delivery. A late answer checks its request and identity, that it is `is_for` the question, and that the reader's connection is still the one it was read over; otherwise it never lands. |
| What it read | A memo per hop, kept while its question and connection stay the same. A failed read over an answer to the same question keeps it, marked stale. A link edit, a refreshed change that names another revision or a new reader clears it. |
| The project's applications | Kept for the next look of the same link while the page shows and for 30 s; hiding drops it. |
| The Changes list’s summaries | An app-lifetime [`SessionMemo`](../crates/freshkube-applications/src/change/outcome/session.rs) keeps only the summaries completed change-page reads publish, keyed by the same hop, link, Deployment, hash, window and provider. It drops other providers on a write and keeps at most 256 summaries, evicting the oldest completed read. Closing a change page keeps its summary; the list observes this memo and starts no Coroot read or connection. |
| Wanting the connection | Each change page tells Observability, by its own entity, that it wants Coroot connected while it shows (and on release). While any does and Observability is hidden, a reconnecting setup makes the **provider alone** (`Live.provider_only`: no request, no project list, no read; it is built on the I/O worker). Showing Observability lists the projects over that provider. Not wanting it drops no provider, and nothing connects when no change page shows, when the person chose not to reconnect, or for example data. |

## Exceptions

Only the following two classes follow a lifetime other than ordinary reads.

### Operations mutations

A submitted mutation must finish its current step even if a page hides or
access changes. [`mutation.rs`](../crates/freshkube-desktop/src/mutation.rs)
owns the app-wide operation slot and confirmation ticket;
[Operations](../crates/freshkube-desktop/src/screens/operations/mod.rs) captures
the confirmed plan and source. Preview and audit reads remain ordinary reads.

Confirmation rechecks the preview fingerprint and selected access. Core's
[selection runner](../crates/freshkube-core/src/operations/selection/mod.rs)
owns fresh safety checks, ordered execution, partial outcomes and recovery.
A detached GPUI delivery task holds the operation ticket until the worker
ends, updates global progress and filters screen updates by run id. Losing
the screen or progress receiver must not abort the submitted worker or free
the slot early.

Cancel is cooperative, before and between steps and during cancellable waits.
Await a submitted patch, eviction or Talos mutation; never put the confirmed
run inside `Loader`, `Watch`, `spawn_job` or a receiver-closed abort branch.
Maintenance mutations use the same operation slot. This guarantee lasts while
the application/runtime lives, not across process termination.

### User-started sessions

Logs, shells, forwards and packet capture belong to the explicit session the
user opened or started. Page visibility is not their stop signal. Their
streams may use `Watch`, but must preserve collection while hidden instead of
forwarding the page's hide to `Watch::set_visible`.

- **Logs:** [`LogView<S>`](../crates/freshkube-logs/src/lib.rs) owns retained
  lines, review and search; its source owns collection. Hiding coalesces
  received lines rather than stopping it. The
  [dock](../crates/freshkube-desktop/src/desktop/dock/mod.rs) starts a restored
  tab only when first selected and shown, then keeps it reading while open on
  any page, including when minimized. Closing, explicit Stop, or incompatible
  source replacement ends collection. Source paths are
  [`logs/talos`](../crates/freshkube-desktop/src/logs/talos/mod.rs),
  [`logs/pod`](../crates/freshkube-desktop/src/logs/pod/mod.rs),
  [`logs/workload`](../crates/freshkube-desktop/src/logs/workload/mod.rs) and
  [`logs/streams.rs`](../crates/freshkube-desktop/src/logs/streams.rs);
  [`dock/feed.rs`](../crates/freshkube-desktop/src/desktop/dock/feed.rs) owns
  each tab's pod/selector feed. Per-container generations reject retried
  streams' old updates queued in the shared feed. Previous-instance logs
  require an explicit choice.
- **Shells:** [`ShellView`](../crates/freshkube-resources/src/shell/mod.rs)
  starts only from the user's container choice or Start. Running owns the
  `ExecSession` and output delivery across navigation and minimization.
  Explicit End or confirmed close/replacement ends it; restored tabs are idle.
  See [pod exec](POD_EXEC.md).
- **Forwards:** [`ForwardList` and `ForwardView`](../crates/freshkube-resources/src/forwards/mod.rs)
  capture the selected connection and specification on Forward. The app owns
  the session independently of pages or context changes; replacement must
  not retarget a running forward. Stop ends it; Remove discards its ended
  entry. Quit/window close asks about shells and forwards together. See
  [port forwarding](PORT_FORWARD.md).
- **Packet capture:** [`screens/network/capture.rs`](../crates/freshkube-desktop/src/screens/network/capture.rs)
  owns the explicit capture and its delivery; core's
  [`pcap.rs`](../crates/freshkube-core/src/pcap.rs) owns capture/save state.
  Leaving the Network tab does not cancel it. Stop, capture completion,
  target replacement or owner drop ends its work.
