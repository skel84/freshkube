# State and request lifetimes

Every ordinary read must be owned by a `Loader<T, I>` or a `Watch<T, I>` from
[`freshkube-reads`](../crates/freshkube-reads/src/lib.rs). The primitive owns the
work and its GPUI delivery, and checks which request may publish. The feature
chooses the identity, deadline, visibility scope and refresh policy. Domain
queries belong in core or the domain crate; presentation belongs in the view.
Start reads from lifecycle methods or explicit actions, never from `render`.

There are exactly [two exception classes](#exceptions): mutations (Operations
and [Kubernetes writes](#kubernetes-writes)) and user-started sessions (logs,
shells, forwards, packet capture and [followed items](#followed-items)). The remaining counters outside the primitives
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
| `SetupFile.write_generation` (`freshkube-desktop/src/setup_file/mod.rs`) | Keeps local preference-file inspection from overwriting a newer write. |
| `SetupFile.setup_revision` (`freshkube-desktop/src/setup_file/mod.rs`) | Advances for effective setup/source-availability changes and successful explicit Reload, including identical bytes whose referenced key files may have changed. Changed warnings/refusals publish independently through owner events and the diagnostic setter; quiet polls publish nothing. |
| `Pages.delivery_revision` (`freshkube-desktop/src/desktop/page_map.rs`) | Remembers `SetupFile.setup_revision`, which advances only for effective configuration/source-availability changes or successful explicit Reload. Sanitized diagnostics publish independently and deduplicate against Applications’ current text, preserving reads across warning-only or refusal-only edits. |
| `Fetch::Live.setup_revision` and `ChangesPage.setup_revision` (`freshkube-applications/src/change/read.rs` and `changes/mod.rs`) | Identify the setup used by each delivery read owner. Reloading trust advances them before resetting the Loader, so an old completion cannot match a new setup even when its request generation repeats. A workspace remap keeps the setup identity and expires the retained read. |
| Databases' `evidence` Loader (`freshkube-desktop/src/screens/databases/mod.rs`) | Owns the Coroot read for the inventory's databases. Its identity is the inventory's revision, the Coroot connection and the entry's link; the inventory landing starts it, a changed connection or link restarts it, and a late answer for an older identity or request is rejected. Hiding drops its job and task (`set_visible(false)`), showing reads again after 30 s; with no link or connection it asks nothing and resolves at once. |
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

### The session alert read

The active alerts of the open cluster
([`desktop/alerts/`](../crates/freshkube-desktop/src/desktop/alerts/mod.rs),
core's [`monitoring/alerts/`](../crates/freshkube-core/src/monitoring/alerts/mod.rs))
are a session-owned read beside the Kubernetes summary, not a page's: it feeds
Overview's attention rows and the rail's Monitoring mark, so it runs on every
page.

| Boundary | Owner and lifetime |
| --- | --- |
| The read | One [`Watch`](../crates/freshkube-reads/src/watch/mod.rs) per live session, `Registry.alerts_watch`, keyed by the context, the connection id and the alert `Scope` (how the setup's metrics mode says the entry's alerts are found). Starting replaces it; `stop_summary`, a parked or replaced session and `reset_active` stop it, and a delivery for another identity or an older generation never lands. `Pilot::apply_alerts` checks the identity again against the session's own before it publishes. |
| What it reads | Core's `drive`: it looks for the Alertmanager and vmalert once, and starts polling only when one answered; it then reads the active alerts every 60 s, bounded (answers 8 MiB, 500 alerts kept, the rest counted). With none found, or after a refusal, it publishes that and ends; after a source was once found nothing ends it, and a source that stops answering is looked for again after three failed reads. |
| What it knows | `ClusterSession.alerts`: its identity and the last `State`. Unread, stale, refused or without the central store's cluster label is Unknown, never "no alerts"; only a current read with nothing active is quiet, and PrometheusRule objects alone say nothing of health. A failed read keeps the one before it as stale evidence. |
| Sharing | One read per live session and one per source. #846's Followed owner reads `Pilot::alert_state` instead of polling. The Alerts, Rules and Silences pages are not part of it: each is an ordinary `Loader` owner that reads only while shown (`OnShow::ReadIfOlder(30 s)`), and Refresh reads again. |

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

Settings' setup finders are page-section reads of this kind. The Argo CD
destinations list owns a `Loader<DestinationFindings, FindIdentity>`: it starts
only on press (`OnShow::Never`), `set_visible(false)` drops the job and its
delivery task, and a late answer fails `land`. Findings that landed are kept
and the suggestions are derived from them again whenever the setup changes; a
changed source, kubeconfig or example mode resets the loader. Ignored
suggestions live on the settings page for the session and are never saved.
The Delivery sources list owns a second one, a `Loader<BuildFindings,
FindIdentity>`, with the same rules: it counts PipelineRuns and lists Kargo's
Projects only on press, hiding drops it, and the item is derived from the kept
findings and the setup's `delivery.build_namespace`. Its keys (`f`, `a`, Enter,
Backspace) act only when there is something to start, accept or ignore, and
otherwise let the key through to the focused control.

### Deep links' reads

The fingerprint reads of [deep links](features/deep-links.md#which-cluster) are ordinary reads owned by `deep_link::cluster::Identities` on `Pilot`: each `OwnedJob` and its delivery `Task` stay there. The open cluster's read belongs to the source it started under and is ignored when the window moved to another; a Check is one job per context, started only by the reader's click, replaced by none and dropped with the window; the kubeconfig read (server and CA hashes) completes into whoever waited for it. `fingerprints.json` is a cache, never an identity: a record counts only while the kubeconfig still names the same server and CA.

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

### Kubernetes writes

A change to a Kubernetes object goes through core's `write` module
([WRITE_MODEL.md](WRITE_MODEL.md)) and has Operations' lifetime, with its own
owner in [`write_flow/`](../crates/freshkube-desktop/src/write_flow/mod.rs).

- **Checking is an ordinary read.** Preflight (the access review, the UID and
  resourceVersion, the server-side dry run) runs under `spawn_job` and belongs to
  the dialog: closing the dialog, or replacing the check with "Review the new
  version", drops it. It sends no write and changes nothing.
- **Confirming takes the app-wide operation slot** (`Operations::begin`, which
  refuses a `read-only` cluster and a busy slot) and only then consumes the
  prepared action: core's `Runner::execute` takes `Prepared` by value, so a
  confirmed action cannot be sent twice, a second press finds nothing to send,
  and a new attempt is a new preflight with a new run id.
- **The runner has no owner.** It is spawned on Tokio and never aborted. A
  detached GPUI task owns the ticket and frees the slot once, when the runner has
  ended, whatever became of the page, the dialog or the toast. Hiding or closing
  the page, or letting go of the flow, stops nothing. The window's close guard
  (`mutation::may_close`) covers it like any other operation.
- **Cancel is cooperative and early.** A destructive action waits 5 s after
  confirming with its Cancel in the dialog (the status bar's cancel is the same
  flag); after the `sending` line is written, nothing cancels it.
- **Never replayed.** A request that got no answer is reported as unknown and is
  not sent again; a conflict is shown, never retried, and there is no force.
- **Counted once.** One run id links a write's audit lines; the log is the count.
  The `sending` line is written before the first request, and if it cannot be
  written nothing is sent.
- **The level is read again at the send.** The runner follows the window's level
  cell and, if it got stricter since the dialog opened, sends nothing and the
  flow goes back to its dialog under the new requirement.
- **Observing and Undo are part of the write.** Looking for the new state
  (30 s by default, 2 min at most) is the runner's, on Tokio's clock; the toast
  and Undo's 30 s are the report entity's, and Undo is its own flow and run, asking what
  its level asks, through the same preflight and slot.

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
  Its dock tab captures its cluster (`Origin`: source, entry, context; its level is read live)
  when it starts (#878), as a forward does: switching tab or context ends
  nothing, the tab's source is refreshed only by the same connection's, and
  its Start gate reads the entry's level, or its context's when the window has
  no entry, never the shown one. See
  [pod exec](POD_EXEC.md).
- **Forwards:** [`ForwardList` and `ForwardView`](../crates/freshkube-resources/src/forwards/mod.rs)
  capture the selected connection and specification on Forward. The app owns
  the session independently of pages or context changes; replacement must
  not retarget a running forward. Stop ends it; Remove discards its ended
  entry. Quit/window close asks about shells and forwards together. See
  [port forwarding](PORT_FORWARD.md). Applications' `results_forward::Binding`
  lives on the Changes list/change page. A weak session registry shares their
  selected owner without holding its guard. Reads include the owner's `(id, start_count)` epoch;
  route/epoch changes cancel old work, including while hidden.
  `Loader::retain_stale` preserves shown evidence across that route replacement
  and rejects old completions; it never applies to source/access/setup changes,
  which reset the loader. Status-only notifications do not read again. Stop
  or Remove cannot silently select another matching forward. Only an explicit
  Forward action starts or restarts one.
- **Packet capture:** [`screens/network/capture.rs`](../crates/freshkube-desktop/src/screens/network/capture.rs)
  owns the explicit capture and its delivery; core's
  [`pcap.rs`](../crates/freshkube-core/src/pcap.rs) owns capture/save state.
  Leaving the Network tab does not cancel it. Stop, capture completion,
  target replacement or owner drop ends its work.
- **Followed items:** [`Followed`](../crates/freshkube-followed/src/owner/mod.rs)
  is the app's follows, made once at startup like `ForwardList` and kept as
  a global. See [Followed items](#followed-items).

### Followed items

A follow is something the user turned on to be told about: a pod that keeps
crash-looping, a Freight that reaches a Stage, a verification that fails, a
Coroot incident that opens (issue #846). It is a user-started session, not
a page's read, so page visibility is not its stop signal.

| Boundary | Owner and lifetime |
| --- | --- |
| The follows | `Followed` ([`owner/`](../crates/freshkube-followed/src/owner/mod.rs)), an app-wide entity. A follow lives from the user's choice until Stop, through page and context changes and with the window closed. They are saved in `follows.json` beside `preferences.json` ([`saved.rs`](../crates/freshkube-followed/src/saved.rs)) and come back at the next launch reading nothing: each shows "Not watching" until the shell offers its context's connection. |
| The reads | One `Watch<Seen, u64>` per object or Kargo project of a context ([`owner/feeds.rs`](../crates/freshkube-followed/src/owner/feeds.rs)), shared by every follow that needs it, and Coroot's incidents read once a minute. The watch's job and delivery drop together when its last follow stops. Reads are the user's own, through a read-only client; nothing writes. |
| The bounds | At most 20 follows, refused beyond that; a repeat of one follow is refused. A listing or watch that fails retries with a growing wait and says "Not watching" once, never as a failure. |
| Quit | `⌘Q` calls `stop_all` before the app ends: every feed is dropped and nothing starts again. The follows stay saved. |
| What they tell | Pure [`rules`](../crates/freshkube-followed/src/rules/mod.rs) turn a read into updates: the same state never repeats, a crash loop fires once and again only after 5 minutes Ready. [`notice`](../crates/freshkube-followed/src/notice/mod.rs) groups updates within 2 minutes and holds all but failures in quiet hours. The owner emits an event for each notice that shows; the shell, which has the window, pushes it as a toast. The bell lists every notice. |

### Database explorer

An explorer tab (Databases › Explore, [#948](https://github.com/skel84/freshkube/issues/948)) is a user-started session: the user pressed Explore, and the connection lives with its tab until the tab closes or the connection changes. Hiding the page is not its stop signal for the connection, but it is for its reads.

| Boundary | Owner and lifetime |
| --- | --- |
| The tab | `ExploreView` ([`explore/mod.rs`](../crates/freshkube-desktop/src/screens/databases/explore/mod.rs)), one per database, held by the Databases page. Closing the tab, or a change of cluster connection, drops it; its release stops and removes the forward it started. |
| The forward | The app's `ForwardList` ([PORT_FORWARD.md](PORT_FORWARD.md)); the tab starts one pod forward and observes it. No second forward owner exists. A forward that ends while the tab is connected ends the connection and says so. |
| The reads | Pick, connect, schemas, each schema's tables, columns and rows are `Loader`s keyed by the tab's connection epoch (and the table, for the last two): a Tokio job plus a delivery task, dropped on hide and on connecting again, landing only if their request is still the latest for their identity. A hidden tab starts no read; showing it resumes what hiding cut short. Reads of one connection share its session one at a time. |
| The login | Read once per connection inside the connect job, held by a wiping value, dropped when the session is open. Not kept by the tab, the loaders or the forward. |
| The session | Kept while hidden; every request is its own read-only transaction, rolled back. A read cut by hiding may leave its transaction open until the next request's rollback or `idle_in_transaction_session_timeout`. |

