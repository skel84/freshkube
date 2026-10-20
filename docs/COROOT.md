# Coroot integration

Freshkube connects Applications, the service map, supported application-report evidence and read-only Incidents through `coroot-rs`. Prometheus dashboards remain the separate Monitoring feature. [Freshkube #25](https://github.com/skel84/freshkube/issues/25) tracks the broader integration and stays open for later destinations.

The first slice is implemented in `05d03ce`, with the Disconnect-state correction in `1b16b42` ([PR #31](https://github.com/skel84/freshkube/pull/31)). The validation record below distinguishes completed checks from the remaining integration scope.

## Dependencies and sequence

| Step | Status and boundary |
| --- | --- |
| Preserve present signals | [corust PR #3](https://github.com/skel84/corust/pull/3), `605dced7850271668cf781e6589a0d3e7e66e5bc`, preserves empty healthy, empty unknown and absent signals. Freshkube keeps those distinctions. |
| Access identity | Satisfied by [Freshkube PR #30](https://github.com/skel84/freshkube/pull/30), merged main `4bbc775b5361ac32d8f7876f470325da2e3df1c5`; #2 is closed. Desktop uses `ClusterSource.id`, the opaque access-key adapter described in [ACCESS_IDENTITY.md](ACCESS_IDENTITY.md). |
| Bounded responses | The dependency pins corust `5312f1efc4e68a286d42a0c5c1e58539ed2a243a` ([PR #11](https://github.com/skel84/corust/pull/11)): the incident audit of [PR #7](https://github.com/skel84/corust/pull/7), chart histories and deployment revisions from PRs #8 and #9, the decoders from one answer of PR #10, and the window around a revision of PR #11, with response bounds from [PR #4](https://github.com/skel84/corust/pull/4) and collection-shape validation from [PR #5](https://github.com/skel84/corust/pull/5). Freshkube sets an 8 MiB limit before REST/MCP decoding. No absolute local dependency path is committed. |
| First live slice | Core provider, explicit connection/project/access association, Applications → map → supported reports, shared fixture/live projection and guarded object navigation, implemented in `05d03ce`. Validation is recorded below. |
| Read-only Incidents | Latest-project sample → selected incident → source SLO/RCA → application reports; [contract and bounds below](#read-only-incidents-slice). |
| Traces and Profiling | Read from Coroot's own per-application tracing and profiling views; [contract and bounds below](#traces-and-profiling). |
| Deployments | A revision's window is Coroot's application page over `RevisionWindow` around its start (1 min to 12 h a side, ±30 min by default), split at the start by `ChartHistory::split_at`. See [Deployments](#deployments). |
| What a deploy did, in the chain | Designed, not built: a per-entry link, an outcome on the change page and a way back from a revision. See [What a deploy did, linked to the chain](#what-a-deploy-did-linked-to-the-chain-design). |
| Shared Health/attention | Separate future scope requiring provenance and deduplication. |

The remaining lenient library list readers (nodes, deployments, risks, alerts and alert rules) need endpoint-contract audits before later adoption; tracked in [corust #6](https://github.com/skel84/corust/issues/6). This first slice does not call those readers.

[Guidance cleanup #24](https://github.com/skel84/freshkube/issues/24) is complete; [Diagnostics/Lifecycle extraction #4](https://github.com/skel84/freshkube/issues/4) remains an independent follow-up. It is not a prerequisite for reading Coroot's existing analysis. The separate Table/summary performance follow-up [#22](https://github.com/skel84/freshkube/issues/22) remains open.

## Connection policy

The first slice connects directly to an explicitly entered HTTP(S) base URL reachable from the desktop. TLS verification stays enabled. Redirects are refused so credentials cannot be forwarded to another endpoint. Kubernetes service-proxy transport would require additional library work; this integration does not reuse Monitoring's transport or an ambient corust profile.

API keys and existing `coroot_session` values are entered explicitly. Anonymous access is also an explicit choice. Credentials are masked in the input and never saved in preferences, diagnostics or screenshots. By default a key stays in memory for the window's connection lifetime. **Remember key in Keychain** (Credential Manager on Windows, the Secret Service keyring on Linux) keeps it in the system's credential store instead, under the service `Freshkube` and the account `Coroot <server URL>`, through `secrets.rs`. A stored key is only ever sent to the server it was saved for, and the field stays empty while it is used. When the store can't save it, Remember turns off, the form says why and the key stays in memory; a key is never written to a file instead. Disconnect forgets the key, including while connecting or after a failed connection or credential edit, deletes the stored copy and stops reconnecting on launch.

`coroot.json` beside the preferences remembers the server URL, the sign-in choice, the last project, whether to reconnect and whether to remember the key; it never holds a key. After a successful connection, the first time the page shows on a later launch it connects again: at once for anonymous access, or once the store returns the key, and it reopens the remembered project on the same server. Nothing is read from the store before the page shows. Tests use an in-memory store; only the app's own launch (`GpuiOptions::with_keyring`) uses the system's. Editing, replacing or clearing URL, credentials or authentication mode cancels pending reads and clears the old connection immediately, including when a replacement connection fails.

Connecting lists accessible projects. It does not select the first project; until one is chosen, the page says "Choose a project" rather than "Connect Coroot". The project picker displays both name and server ID, so two projects with matching names stay distinct.

Kubernetes navigation requires a separate explicit association between a Coroot cluster ID observed within that project and the current Freshkube access key. Matching context or cluster names never establish this relationship. Replacing access/configuration cancels reads, clears ordinary evidence and removes the association; the new access must be associated explicitly. A node change or compatible transport retry retaining `ClusterSource.id` retains the association. External applications, unsupported kinds and unmapped clusters remain inspectable without Kubernetes links.

## Ownership and request lifetime

`coroot-rs` owns Coroot HTTP/MCP protocols and typed response decoding. `freshkube-core::coroot` owns provider identity, explicit project selection, capability/error categories, read deadlines, concurrency and retention limits. It has no GPUI types. The library is called directly; no CLI subprocess or serialization boundary sits between core and desktop.

The retained `ObservabilityPage` owns selections, request handles, `Snapshot` values, prepared display data and rendering. It runs network work on Tokio through `OwnedJob` and applies results through GPUI. Fixtures construct the same typed observations and use the same Applications, map and report projections as live responses.

Every request captures the opaque Coroot provider identity, project, optional cluster association, current Freshkube access key, absolute time interval and subject. A separate page request generation guards delivery, and each `Snapshot` checks its identity and generation. Creating a connection gives it a new provider identity even when the URL and credential text are unchanged. An ordinary retry retains the provider identity.

Only the visible destination starts ordinary reads. Hiding, changing destination or replacing the source drops its jobs; there is no background Coroot refresh timer. The Application page reads Coroot's own view of the application and nothing beside it.

Refresh and reopening capture a new Last N hours interval. Destination/report navigation and **Retry** retain the captured absolute interval, shown with local dates and times. Data from a different interval or source is cleared. A failed retry for the same interval can retain successful evidence, marked **stale**, with the last successful local time and the read error. Source failures, loading, successful empty results, application health and unsupported capabilities remain distinct.

## Presentation and navigation

| Surface | First-slice behavior |
| --- | --- |
| Applications | Coroot aggregate health, type, category and 12 named signal columns. Healthy checks without a figure read “ok”; empty unknown checks retain the outlined circle; absent signals show “—”. Full captions and compact values are measured when data or text size changes; wide tables scroll sideways. Failing upstreams show a count, with the original names and state retained in their tooltip and accessibility label. Coroot's status is never recalculated from strings or local thresholds. |
| Signal/report mapping | Errors and latency → SLO; upstreams and network → Net; instances and restarts → Instances; disk usage and disk I/O → Storage; CPU, memory, DNS and logs use their named report. Dynamic report tabs also expose other report names returned by the source. |
| Service map | Stable AppId nodes and directed endpoint-pair links, prepared layered layout (callers left of callees), source status and available request/latency/traffic metrics. Missing metrics say Not reported. Statistics derived by the library from rounded Coroot display values are identified as potentially rounded. |
| Application reports | Coroot's own application view: its map, report tabs, checks and widgets, as Coroot's page shows them and with nothing added. [DESIGN.md](DESIGN.md) H3 describes how each widget draws. |
| Capabilities | Every read uses Coroot's HTTP API; Freshkube calls no MCP tool. Authentication, refusal, missing subjects, unreachable sources, decoding failures, timeouts and size-limit failures have separate messages. |
| Incidents | Bounded latest-project sample, stable key/application selection, source-reported SLO objective/compliance and burn values, RCA source text and problem-propagation application links. No mutation, invented charts or ruled-out evidence. |
| Traces | One application's latency and error heatmap, the latest 100 matching spans, and the selected trace as a waterfall with each span's status, details, attributes and events. A heatmap cell lists that bucket's requests; Failed requests lists the window's errors; OpenTelemetry and eBPF are chosen when Coroot has both. |
| Profiling | One application's profile types, instances and flame graph, optionally compared with the window before it. Frames show their share of the total and, when compared, the change in that share; Biggest increases lists the frames that gained most. |
| Deployments | One application's revisions, as Coroot's Deployments report lists them, with Coroot's findings, and its charts around the selected revision's start. [Deployments](#deployments) has the contract. |

Application and link selections use stable domain identities and survive response reorder. Removing the selected subject clears the selection safely. Sorting, filtering, group counts, labels and graph layout are prepared when observations or controls change. The applications list is virtualized. The map displays prepared pages of 24 applications, adding at most two endpoints for the selected connection. It states the displayed/retained counts; the first 24 connection markers plus the selection are shown on the graph. Every retained connection remains available in the virtualized inspector, including links between different pages. Category and namespace menus show at most 100 choices with an explicit hint; text search searches all retained applications.

A Coroot AppId is not a Kubernetes UID. Supported Pod, Service, Deployment, StatefulSet, DaemonSet, ReplicaSet, Job and CronJob links carry the explicit access, API/kind, namespace and name. The shell verifies that the event still belongs to the displayed source and access, then uses its existing `open_object` path. That path respects running-shell confirmation and resolves missing UIDs with metadata-only reads. Access is checked again after delayed confirmation and metadata completion.

This slice is read-only. Live pages do not offer threshold, mute, rollback or configuration mutations. Fixture threshold/mute choices stay local; rollback/configuration actions are labeled previews and never acquire live write behavior.

## Bounds

| Resource | Limit |
| --- | --- |
| Response body | 8 MiB per response, including error bodies and MCP JSON/SSE; rejected before decoding |
| Reads | Two concurrent reads shared by provider clones; 20 s deadline including a queued slot; 5 s connection timeout |
| Time interval | One minute to seven days in core; the desktop offers its existing hour ranges |
| Projects / applications | 100 projects / 2,000 applications |
| Map | 2,000 nodes / 20,000 directed links (paged 24 nodes at a time) |
| Application view | `GET api/project/{p}/app/{app}`, Coroot's own page for one application: 2,000 instances, 1,000 clients and dependencies, 32 reports of 64 checks and 128 widgets each; 512 charts, 4,096 series of 4,096 points, and 50,000 table cells per page. Read by `Provider::app_view` for the Application page. |
| Incidents | Latest 100 across all states; 10 list rows and 10 propagation applications per rendered page; 100 retained propagation applications / 200 issue entries / 32 burn conditions |
| Incident text | 64 KiB total RCA/propagation text, with individual bounds (root/fixes 16 KiB each, detailed analysis 32 KiB, issue 4 KiB) |
| Strings | Individually bounded IDs, labels, messages, log samples, connectivity text and protocols before display preparation |

Exceeded limits produce an explicit source failure; they do not masquerade as successful empty data. Previous successful evidence survives only a same-identity retry and is marked stale. These limits are first-slice product bounds, not Coroot's server limits.

## Validation

Sanitized fixture and fake-server checks passed before the optional live probe. Cases include signal presence, empty/refused/failed/recovered reads, partial REST/MCP evidence, selection reorder/removal, every request identity coordinate, hidden-page cancellation, credential editing, explicit object-link identity, and shared fixture/live projection. Representative measurements include 2,000 applications with distinct categories/namespaces and a 120-node/300-link map, including headless render cost.

On final code `1b16b42`, `cargo test --workspace` passes all **982 tests** (8 binary, 375 core, 522 desktop, 66 Talos and 11 doctests), `cargo clippy --workspace --all-targets -- -D warnings` passes, and `cargo fmt --all -- --check` passes. The 11 focused Coroot core tests, 19 Observability tests and three shell/metadata navigation regressions are included. Coverage includes queued deadlines, shared concurrency, strict collection shapes, byte limits, partial evidence, recovery, credential edits, Disconnect before a provider exists, delayed answers and stable selection. The reviewed library's ten response-bound tests and seven collection-shape tests also pass independently. The native build passes. [PR CI on this code](https://github.com/skel84/freshkube/actions/runs/37168093542) also passes, including stress-feature Clippy, the two stress API checks and the clean-tree check.

Current-source native fixture captures cover Applications and the service map in dark mode and application reports in light mode at 1280 × 880, plus reports and the map at 760 × 560 with 20 px text. The narrow map has an explicit containment and horizontal-scroll regression. Interactive behavior is exercised by the headless UI tests rather than synthetic native keystrokes.

Maximum-data debug probes on `1b16b42`, run individually after compilation and other tests stopped, measured:

| Fixture | Preparation | Ten forced headless frames |
| --- | --- | --- |
| 2,000 applications, distinct categories/namespaces | 74.50 ms | 267.84 ms |
| 120 map nodes / 300 links | 5.09 ms | 189.07 ms |

The earlier unpaged map took 2,927.92 ms for ten frames while compilation was active; representative paged-map runs under build contention measured 261 and 443 ms. These debug observations have different load conditions, and forced frames bypass normal view caching. They are not release frame-time acceptance; #22's independent Table/summary targets remain open. Reproduce with the `bounded_projection_measurement` desktop test using `--nocapture --test-threads=1`.

The optional anonymous HTTPS check reached the explicitly selected Coroot endpoint; `/api/user` returned HTTP 401. Live authenticated data has not been checked by this branch.

The user authorized read-only verification through `~/code/home-ops/kubeconfig`, explicitly selected context `admin@kubernetes`. This does not authorize an ambient context, mutation or unnamed pod port-forward. Optional live evidence must record counts/statuses only and keep credentials and raw log/trace payloads out of the repository.

## Read-only Incidents slice

The prerequisite is [corust PR #7](https://github.com/skel84/corust/pull/7), audited at `a1fd85d649a9274d8a4ad774d16d671c4278781d`; the pin is now `5312f1efc4e68a286d42a0c5c1e58539ed2a243a`, which keeps it. Its [contract audit](https://github.com/skel84/corust/blob/a1fd85d649a9274d8a4ad774d16d671c4278781d/crates/coroot-rs/INCIDENTS.md) names the upstream Coroot revision and handlers. Malformed envelopes, identities, required impact/duration and supported evidence fail explicitly. A reported zero remains zero; missing optional SLO/RCA evidence remains absent. The additive `incident_view` API preserves existing public record fields and methods.

Coroot's REST list returns a bounded sample ordered open-first then newest. It does **not** filter history by the toolbar window; detail reads use the incident's own time context. The page states both facts. The sidebar labels open counts as sampled, and neither an empty result nor fewer than 100 rows proves a complete history. `data: null` before a world is available is an empty sample, with a refresh explanation; null detail cannot identify an incident and fails.

The existing retained page owns list/detail snapshots, selection by incident key and application ID, prepared text and paging. Each detail has its own owned Tokio read plus GPUI delivery task, so a removed/replaced selection cancels it while retaining a concurrent list read. Page generation and snapshot identity/generation reject obsolete completions. Hiding, destination/source replacement and disconnect drop both jobs. A same-window retry retains successful evidence as stale; a new source/window clears it. A successful list removes an absent selection and its evidence. Detail application identity must match the listed application.

SLO objective/compliance are Coroot-rendered strings. Violated flags and incident severity retain source meaning; affected-request impact is not used to reconstruct compliance. Burn windows, values and thresholds come from the response, never a fixed 14.4× rule. Missing objective/compliance, impact, burn rates and analysis are marked not reported. RCA status, summary, root cause, suggested fixes, analysis errors and problem propagation are read-only source text, with no executable Markdown links or mutation controls. Application links enter the existing report workflow using the toolbar interval, rather than reconstructing a historical report from incident widgets; later Kubernetes links still require explicit association and shell guards. Stale incident links cannot navigate.

List and propagation paging keep every bounded item reachable while rendering at most 10 each. The original fictional H4 fixture remains an explicit preview; fake-server tests exercise the production live projection. Incident charts, RCA widgets/healthy propagation links and structured ruled-out evidence are not represented by this client API. Traces, profiling, deployments, shared Health/attention and #22 performance work remain separate.

### Incidents validation

The implementation is `ec846d031fcc9a7c4424370e34fed43c3f76becc`, merged in [PR #35](https://github.com/skel84/freshkube/pull/35) as `8d3c5d747dc18f44da4a6feee6387f791aa82d87`, after client [PR #7](https://github.com/skel84/corust/pull/7) merged as `362b8d9a6d47110fd2ea148bb5eca6a716281d6a`. Both merge trees match their reviewed heads; the client pin remains reachable from its main branch. On the implementation code, local `cargo test --workspace --locked` passes **992 tests** (8 binary, 377 core, 530 desktop, 66 Talos and 11 doctests), strict workspace/all-target Clippy and formatting pass. [Final PR CI](https://github.com/skel84/freshkube/actions/runs/37199883978) passes on exact reviewed head `ed0fe23a9a78d52edbeb9a5fa0bf4b58728895cf`, including stress-feature Clippy, two stress API tests and the clean-tree check. The client prerequisite at the pinned revision independently passes **67 workspace tests**, strict Clippy, formatting and rustdoc with warnings denied.

The 25 focused Observability checks cover the production Tokio-to-GPUI path with sanitized fake servers: stable selection after reorder, refused refresh retaining stale evidence, recovery, successful empty results, stale-link rejection, real scrolling to an application link, delayed detail replacement, hiding and project replacement. Maximum-data preparation and headless layout checks cover 100 incidents and 100 propagation applications at 760 × 560 with 20 px text. This slice makes no release performance claim or #22 completion claim. Local file/heading links and whitespace are checked. No native Incidents capture or authenticated live probe was performed; the visual agent's separate work is not included in this PR.

## Traces and Profiling

Both pages read one application, chosen from the page's picker or with **Traces** and **Profiling** on its report, in the page's time window. They read only while Observability shows; another application, project, window or connection drops the answers in flight.

| Read | Request | Bounds |
| --- | --- | --- |
| Traces | `GET api/project/{p}/app/{app}/tracing?from&to&trace=source:id:from-to:above-up_to`. The selection is checked before anything is sent: the source is `otel`, `agent` or empty; a cell's window is epoch milliseconds with `0 < from < to`; its bounds are empty, `inf`, or seconds; a trace id is at most 64 letters and digits. | 8 sources, 32 heatmap rows of 4,096 points, 5,000 spans with bounded strings, 256 attributes and 128 events each. Coroot lists at most 100 spans, and the page says when more match. The heatmap is summed into at most 48 columns and the waterfall draws 200 spans. |
| Profiling | `GET api/project/{p}/app/{app}/profiling?from&to&query=…`: `cpu` until a type is chosen, then `{"type","mode","instance"}`, with `mode` `diff` to compare. | A flame graph nests deeper than serde_json's default limit, so its body is checked for at most 4,096 levels and parsed on a thread with a 256 MiB stack. Frames narrower than 0.05% of the total and past 20,000 are left out and counted. The page draws at most 600 frames, 64 rows below the zoomed frame, each at least 0.3% of it, and says how many it leaves out. |

Coroot's own notes, such as which service's traces it used, are shown as plain text. A comparison's change is the frame's share of this window minus its share of the window before, in percentage points.

## Logs

Core reads an application's logs with `Provider::logs`, and the Application page's Logs tab draws them as Coroot does: its histogram of messages by severity, then the messages or the patterns.

| Read | Request | Bounds |
| --- | --- | --- |
| Logs | `GET api/project/{p}/app/{app}/logs?from&to&query={"source","view","filters":[],"limit","since"}`. Only ever GET: a POST to the same path saves the application's log settings. The limit is one of Coroot's 10, 20, 50, 100 or 1,000, and `since` a positive epoch nanosecond; both are checked before anything is sent. | 4 origins, 1,000 messages of at most 64 KiB with 128 attributes each, 8 MiB of messages and attributes together, 1,000 patterns with samples of at most 16 KiB, and each chart as on the application page. |

- **Levels.** Each message keeps the severity Coroot stored: trace and debug read as Debug, info as Info, warning as Warning, error and fatal as Error, and anything else as Unknown. The level is never guessed from the message's words.
- **Order and caps.** Messages are kept oldest first. Coroot's query has a limit but no order, so an answer with as many messages as were asked for is marked capped: some of the matching messages, not the latest ones.
- **Refreshing.** `LogCursor` asks a refresh for messages from the newest one's nanosecond on, since Coroot's `since` is exclusive. It drops a message of that millisecond equal to one already taken, because Coroot stamps messages only to the millisecond. A new message equal in every part to one already taken in the same millisecond is taken for it. A refresh that comes back capped may leave a gap before its messages, and says so.
- **No logs store.** Without one, Coroot answers `unknown` with its reason and the patterns it found in its own metrics. That is information, not a read failure.

The Logs tab (`freshkube-observability`'s `logs/`) reads only while the page shows the Logs report, once per window, and only ever GET.

- **Controls.** The origin (container logs or OpenTelemetry) shows when Coroot lists more than one; Messages and Patterns, and a limit of 10 to 1,000. Each starts the list over. Coroot's note, such as which logs it used, sits above the histogram.
- **Messages.** The shared log view (`LogView<CorootLogs>`, `logs/source.rs`) holds them, oldest first, with Coroot's time and level, search, the level filter, copy and wrap. A message of many lines, such as a stack trace, is one row; past 64 KiB it is cut and says how much it left out.
- **Refreshing.** The list keeps earlier messages; the histogram shows the window. A refresh asks only for messages after the newest one, through `LogCursor`. A capped refresh, or a page hidden until the window moved past its last read, adds a marker where messages may be missing.
- **Capped.** A first read that comes back with as many messages as the limit says these are some of the matching messages, not the latest.
- **Patterns.** Coroot's patterns in a table of level, count and sample; selecting one draws its messages over time under the table.
- **Failures.** A failed read shows why with Retry and keeps the messages already read. Without a logs store the tab shows Coroot's reason and its patterns.
- **Measured** (debug test build, dependencies optimised): a 200-line, 63 KiB trace among 12 messages at 1260 wide draws its first frame in about 11 ms, laying out the four rows on screen; a search that reaches its last line takes about 165–180 ms, about the same as one for a short line; the next frame lays out no row again and takes about 5 ms.

## Deployments

Deployments (`freshkube-observability`'s `deployments/`) lists the chosen application's revisions from its own page: Coroot's Deployments report, decoded by coroot-rs as `DeploymentRevision`s. Coroot keeps an application's last 100, which the status bar's tooltip says, and doesn't filter them by the time range, so the header has no range picker here: the Inspector picks its own window. A revision is keyed by its id, `<hash>:<start seconds>`; its row shows the hash, each image Coroot names by its last path segment and tag (`worker:1.8.2`; Coroot already sends them so, and example data's whole references show in the tooltip and the Inspector; a revision Coroot reports no image for shows "—" in the list and no image line in the Inspector), its start in local time, then Coroot's first finding or note, the images and the finding cut at a word with the whole text in the tooltip. While the application's page is read again, the list keeps the revisions it already read, and Refresh or coming back keeps the chosen one; until a page answers, it claims none.

The selected revision's Inspector shows Coroot's findings in Coroot's words, with no number of Freshkube's: Coroot compares a revision with an earlier one it kept metrics for and doesn't say which. Under them, the application's page is read again over a window around the start (core's `Provider::revision_view`, ±30 min, ±1 h or ±3 h), and the chosen report's charts draw split at the start, each saying how many samples fall on either side, summed across its series ("Before the start: all 90 samples across 3 series · after: 80 of 93 samples across 3 series") and never a value compared across it. When Coroot's data stops before the window's end, as for a rollout minutes ago, the Inspector says where.

| Answer | Shown |
| --- | --- |
| The page over the window | The report's charts, split at the start |
| Coroot's own 404 "Application not found" for the window | "Coroot has no data for this window": the window is older than Coroot keeps, newer than its latest data, or the application sent nothing in it. Never a missing application: the revision came from the application's own page |
| 403 for the window | "Not permitted to read this application's window", its message and Retry |
| Any other 404 or failure | Its message and Retry; a last answer for the same window stays under it as stale |
| 403 for the application's page | "Not permitted to read this application", in the list's place |
| No revisions, a Deployment | "Coroot keeps no deployment of this application": Coroot records one when a Deployment's pods roll out |
| No revisions, another kind | "Coroot records rollouts of Deployments only", naming the application's kind |

The window's read has its own identity (application, revision and window) and generation; another revision or window drops the read in flight, and so does hiding the page. Example data answers from the example hour: the newest revision's window reaches past now and ends early, and one older than seven days has no data.

**Show on charts** opens the Application page on the revision's first finding report, or SLO when there are no findings. It is in the Deployments row's shared context menu, and on each revision in the Application page's Deployments report table. The captured time range stays unchanged. The revision's start second focuses matching markers: their lines draw heavier and their labels stay visible without hovering. A start outside the window does not move or zoom the chart; an older revision says "Started ⟨time⟩, before this ⟨range⟩ window". The before-window line appears only on reports with chart widgets. Another application or source clears that focus. The Deployments Inspector also focuses its selected revision's marker. Each application's fixture answer is dated on its first read and retained for the page's life, so its revision rows, markers and revision windows share stable start times. In the Application page's Deployments report, only the revisions table (identified by Coroot's `Deployment`, `Deployed`, `Summary` header) is replaced; other table widgets remain visible.

The fixture UI tests in `application/deployments.rs`, `deployments/tests.rs` and Monitoring's `panel/tests.rs` prove both navigation paths, the old-revision line, the focused rendering and clearing it.

### From the change page

A Deployment's hop on the change page shows its current ReplicaSet's pod-template hash and offers Compare in Observability (core's `Target::Revision`). Coroot names a revision by that hash, so the join is the hash alone; the change page reads nothing for it. The shell hands `RevisionLink` (the connection, the cluster's name, the Deployment and the hash) to `ObservabilityPage::open_revision`, which opens Deployments and finds the revision in what it reads anyway: Coroot's application is `<cluster>:<namespace>:Deployment:<name>`, where the cluster is the Coroot cluster linked to that connection. Nothing is decided while the page is hidden; it looks once it shows. A banner over the list says what came of it, in blue for a found revision, since finding one judges nothing; the row's glyph says how Coroot judges it. The banner shows only over the application it was decided for, so another one opened from the map or a report hides it, and it goes when the user chooses another application or revision. Another connection drops the link and whatever came of it. Anything but a found revision is looked for again when the page shows, on Refresh, when the window changes and when a Coroot cluster is linked, so linking the cluster the banner asks for finds the revision without another Compare.

| Found | Shown |
| --- | --- |
| The revision, by its hash | It is selected, with its window, under "Revision ⟨hash⟩ of ⟨namespace/name⟩" |
| The application, without that hash | "Coroot keeps no revision ⟨hash⟩", over another revision it keeps: the one already selected, else the newest |
| No such application in Coroot's list | "Coroot lists no application ⟨namespace/name⟩". Asked for before the list answers, the application is chosen and read as any other until the list arrives without it |
| No Coroot cluster linked to the connection | "No Coroot cluster is linked to ⟨cluster⟩", with where to link one |
| The application's revisions couldn't be read | The list's own failure |

The button is greyed out with why for a cluster that isn't the open one, for a Deployment whose current ReplicaSet isn't known (delivery's `current_set` says why), for one whose pods aren't confirmed to run the Freight's digest, since its current revision may then be another change (a spec that names a tag only, pods on another digest, a listing cut short), and for an Argo Rollout, since Coroot keeps revisions of Deployments only. A link that arrives after the connection changed is dropped. Example data: Coroot's example lists `checkout/checkout-api`, whose newest revision has the hash of the ReplicaSet the example change runs on dev-fra and stage-fra (delivery's `change::example::POD_HASH`); Compare is enabled once dev-fra's workspace entry is the open one.

## What a deploy did, linked to the chain (design)

This is the design step of [#269](https://github.com/skel84/freshkube/issues/269) (Part B, its slice 8), written on 19 October 2026 at main `8766bf1`. The owner approved it on 20 October 2026; nothing here is built, and the [decisions](#decisions) record the owner's answers. Every name below is invented: acme, `checkout`, cluster-a, cluster-b, cluster-c.

The change page follows one change to the pods that run it, and today it only links out to Coroot. This design makes it say what Coroot found after the deploy, in Coroot's words, and lets a Coroot revision lead back to the change. It reads Coroot and never writes to it.

### What is left of #269, against main

Checked at `8766bf1`; the columns give what exists and where.

| Item | State | Evidence |
| --- | --- | --- |
| corust pull requests 8 to 11 pinned | Done | `Cargo.toml:73` (`5312f1e`); [Dependencies and sequence](#dependencies-and-sequence) |
| Charts from `ChartHistory`, revision markers | Done | `chart_panel.rs:337` (`with_revisions`), `:361` (`revision_marker`); `application/charts.rs:50-65` |
| Icon-matched annotation markers | Still in the file: `ChartPanel::new` keeps matching `DEPLOY_ICON` | `chart_panel.rs:16`, `:221`. Whether a caller still uses it is for the build to check |
| Deployments: revisions, findings, the window split at the start | Done | `deployments/mod.rs:224` (`prepare_revisions`), `:253` (`read_revision`); `coroot/around.rs:54` (`Provider::revision_view`), `:37-46` (`split`, `ends_early`) |
| Compare in Observability, and what came of it | Done | `change/mod.rs:742` (`open_revision`), `change/detail.rs:262-285` (`Target::Revision`), `delivery/change/live/derive.rs:1227` (`current_hash`), `deployments/wanted.rs:25-38` (`Outcome`), [From the change page](#from-the-change-page) |
| A Deployments read that is shown-only | Done: hiding drops it, a late answer never lands | `connection.rs:279-287` (`set_visible`), `:133-157` (`cancel` takes `revision_job`), `wanted.rs:142-146`; proved by `fake_tests/deployments.rs:299-312` |
| Raw JSON decoded in core | Open (Part A) | `app_view.rs:281-537` (`RawView` … `RawSummary`), `tracing.rs:126`, `profiling.rs:112-139`, `logs.rs:231-268` |
| The Profiling CPU chart is example data | Open (Part A, item 7) | `observability/src/example.rs:422` (`chart`), `plots.rs` |
| A link per workspace entry | **Open.** The key is typed and checked, and nothing reads it | typed: `setup/types.rs:92-110` (`EntryCoroot`), `workspace/mod.rs:90`; read: only the connection, `desktop/startup/setup.rs:53`. The link in use is one for the whole connection, never saved: `connection.rs:172-210` (`Association`), set at `:370-380` (`associate`) |
| What Coroot found after a deploy, on the change page | **Open** | `change/detail.rs:262-285` only offers the link |
| From a revision back to the change | **Open** | nothing names it; `changes/view.rs:202` opens a change from the Changes list only |
| A focused marker, and Show on charts | **Open** (the parked note on #269) | `Marker` has no identity: `core/monitoring/markers.rs:46-57`; no "Show on charts" exists |
| Coroot for a Deployment on another entry | **Open** | `change/mod.rs:598` (`revision_closed`) greys Compare for a cluster that isn't open |

Part A (typed APIs) is not designed here. Its only effect on this design is that the outcome reads `decode_all` (`around.rs:6`) today and the typed layout later; the outcome's own types don't change either way.

### The per-entry Coroot link

**The key.** `clusters[].coroot` in [freshkube.yaml](SETUP_FILE.md), as typed in `setup/types.rs`: `cluster`, the Coroot cluster that is the entry, and an optional `project` that overrides the connection's `coroot.project`. The connection (server, sign-in, key) stays one for the workspace. An entry is linked when it has `cluster`; its project is its own `project`, else `coroot.project`, else none (a state, below). Matching names never link anything: a context called `cluster-a` is not a Coroot cluster called `cluster-a` until the entry says so, as for the connection-wide link today.

**What replaces what.** Observability's "Link Kubernetes cluster…" and its one `Association` for the connection go. Observability reads the open entry's `coroot` key for its own pages (the map's and reports' Kubernetes links), and the change page reads any entry's. The old link was never saved, so nothing is migrated. Settings edits the key in the Clusters Inspector's Observability group ([#697](https://github.com/skel84/freshkube/issues/697), slice 12); until then a person writes it in the file.

**How a hop finds its application and revision.** From a Deployment hop's entry, namespace, name and current pod-template hash:

1. The entry's link gives the project and cluster. Coroot's application is `<cluster>:<namespace>:Deployment:<name>`, as `wanted.rs:165` builds it today.
2. The project's applications list (one read for the page's shown life, shared by every hop of that link) says whether Coroot lists the application.
3. `GET app/<id>` gives its revisions. The revision is the one whose `hash` is the hop's, found as `wanted.rs:76` finds it.
4. `Provider::revision_view` reads the page over the window around the revision's start and splits it ([Deployments](#deployments)).

**What each case says.** The outcome is one of these. Each is its own state with its why, and none reads as fine.

| Outcome | Why, in the section's words |
| --- | --- |
| Found | Coroot's status, its findings, the window and the samples on each side |
| No such revision | "Coroot keeps no revision ⟨hash⟩ of ⟨namespace/name⟩", with that it keeps a Deployment's last 100 |
| No such application | "Coroot lists no application ⟨namespace/name⟩ in ⟨cluster⟩" |
| No such project | "Coroot lists no project ⟨project⟩", naming the key that names it |
| No Coroot connection | "Coroot isn't connected · Open Settings" |
| No Coroot cluster linked to the entry | "No Coroot cluster is linked to ⟨entry⟩", with where to link one |
| No project for the entry | "⟨entry⟩ names no Coroot project, and the connection names none" |
| Refused | "Not permitted to read this application" (the application's page) or "…its window" (the window), as Deployments says them |
| No data for the window | "Coroot has no data for this window", as Deployments says it; never a missing application |
| Data that stops early | The window's last sample, as Deployments says it ("Coroot's data stops at ⟨time⟩") |
| Failed | The read's message and Retry; a last answer for the same identity stays under it, marked stale |
| Not a Deployment | An Argo Rollout's Stage: "Coroot keeps revisions of Deployments only" ([Argo Rollouts](#argo-rollouts)) |
| Not looked for | The hop's current ReplicaSet isn't known or its pods aren't confirmed to run the Freight, with delivery's reason; its revision may be another change's, so nothing is read |

**A Coroot answer for another entry's application never draws as this one's.** Three guards, each proved by a test:

- **Setup refuses the ambiguity.** `Setup::validate` refuses two entries that link the same (project, Coroot cluster): `the Coroot cluster “cluster-a” of project “shop-api” is linked to “cluster-a” and “cluster-c”`. Without it a Coroot application would belong to two entries and no join could say whose it is.
- **Identity.** The read's identity is (the connection's `ProviderId`, project, Coroot cluster, **entry id**, application id, revision id, window), following [ACCESS_IDENTITY.md](ACCESS_IDENTITY.md): identity says whose data this is, a generation says which request may publish. The link is part of the identity, so editing an entry's `cluster` or `project` (a reload of the setup) makes every answer read under the old link foreign: it is cleared, not retained as stale. A same-identity failed refresh keeps its answer, marked stale.
- **Generation.** The change page owns one outcome job (an `OwnedJob` and its delivery task) with a request generation. Selecting another hop, another change, hiding the page or replacing the link drops the job; a late answer checks its identity and generation before publishing and is dropped otherwise.
- **The answer is checked against the question.** The decoded page's application id must equal the asked-for id, as `prepare_revisions` checks `view.map.app.id == *app` (`deployments/mod.rs:239`), and the revision must come from that page's own revisions. A mismatch is a failure, never a draw.

The outcome needs no open Kubernetes connection for the entry: it reads Coroot over the one connection, so a Deployment on cluster-c can be read while cluster-a is open. This removes the `revision_closed` rule (`change/mod.rs:598`) for the outcome; Compare in Observability keeps it, since the Deployments page still reads one open connection's source until slice 3 below.

### What its metrics did after it

**Where it shows.** In the Inspector of a Deployment hop, an **After the deploy** section, below the hop's fields and above its actions. In List, the row's Detail gains nothing; in Chain, the card gains nothing. Once an outcome is read, the hop carries a small Coroot mark (below). The section reads only for the hop that is selected, and only while the change page shows.

**What the section shows, for a found revision:**

- **The revision window.** `Revision ⟨hash⟩ · started ⟨local time⟩ (⟨age⟩) · window ±30 min`. The window is the Deployments default (`RevisionWindow`, `COROOT.md#deployments`); the change page has no picker. A line says how to choose another: Compare in Observability, which stays the way into the charts and the window picker.
- **Coroot's status and findings, in Coroot's words.** Coroot's own status glyph for the revision, then each finding as it sent it, or its note when it has none. Under them, the one line Deployments already says: Coroot compares a revision with an earlier one it kept metrics for and doesn't say which.
- **The charts split at the start.** The chart of the report the revision's first finding names (SLO when there are none), drawn in the shared `PanelView` the Deployments Inspector uses, split at the start, with the samples counted on each side ("Before the start: all 90 samples across 3 series · after: 80 of 93 samples across 3 series"). At most four charts of that report; the rest are Compare's. A value of one side is never subtracted from the other's, and no percentage or arrow is drawn between them. Coroot is the only one that compares, and it does so in its findings.
- **Data that stops early** says where, as Deployments does; **no data for the window** replaces the charts with Coroot's words.

**The mark.** In List and in Chain, a Deployment hop whose outcome has been read shows Coroot's status glyph, small, after its Detail (List) or in the card's corner (Chain), with the tooltip "Coroot: ⟨status⟩ · ⟨first finding⟩". It is not the hop's status glyph and not its link: it is drawn beside both, it never colours a link, and **it is not counted in `StatusChips`**, since a chip counts the hop's own state and the verdict is Coroot's. A hop whose outcome hasn't been read draws no mark, never an "ok".

**Read cost.** Only the selected hop reads (the owner's decision). The first read for a link costs at most three requests: the project's applications list (once per link for the page's shown life, 30 s as the change page's own freshness), the application's page, and the window. A second hop of the same link costs two. One job is in flight at a time, and all three go through the provider's two shared slots with its 20 s deadline. A change with four Deployment hops, read one after another, costs at most 3 + 3 × 2 = 9 requests, never started together.

**Marker focus in Monitoring's chart panel.** Revisions already draw as deploy markers on every chart (`with_revisions`). What the parked note needed is one of them drawn as the one in view: the revision selected on the change page, in Deployments and, from Show on charts, on the Application page. Decision: **the panel takes a focus, not a marker identity.** `PanelView::set_focused_marker(Option<i64>)` takes the revision's start in seconds; the marker whose `at` equals it draws heavier with its label always shown, the others as today. `Marker` and `MarkerKind` don't change, so Monitoring's own markers and `MAX_MARKERS` are untouched. Two revisions that started in the same second would both draw focused; Coroot keys a revision by `<hash>:<start seconds>`, so that needs two rollouts in one second and is accepted. The panel never scrolls or zooms to the focus: the window is the page's. Pull request #520 touches the same marker code, so slice 3 starts by merging main.

**Show on charts** is a row action in Deployments, and a button in the Application page's Deployments report tab: it opens the Application page on the report the revision's first finding names, with the revision focused. A revision older than the chart window still opens the report, saying "Started ⟨time⟩, before this ⟨range⟩ window"; the app never widens the range.

**The Deployments read stays shown-only.** It already is ([the checklist](#what-is-left-of-269-against-main)), so no change is made there. The rule moves to the new read: the change page's outcome job is owned by the page, started only from `set_visible(true)` or a selection while shown, and dropped on hide, as the change page's own read is.

### Argo Rollouts

Decision 4 asks for a spike before code. This is what can be said from the docs and the code, without a live check; the spike confirms it against a test Coroot, read-only.

- **What is known.** Coroot keeps revisions of Deployments only ([Deployments](#deployments)), and delivery's join already finds a Rollout's current ReplicaSet and its pod-template hash (`join/workload.rs:295`, `current_set` at `:345`). Whether Coroot lists an application for a Rollout's pods, and under which kind, is not known from this repository.
- **A. By the Rollout's hash.** Join `<cluster>:<namespace>:<kind>:<name>` and the hash. It works only if Coroot keeps revisions for that application, which the docs say it doesn't.
- **B. By the Rollout's ReplicaSets.** Each ReplicaSet of the Rollout is a rollout of the same pods; Coroot might record them under the application it groups the pods in. This needs the spike to say how Coroot names that application.
- **C. By a window around the Rollout's step changes.** Take the start from Kubernetes (the new ReplicaSet's creation, joined by the hash, or a step's change) and read Coroot's application page over the window, split at it. Coroot keeps no revision, so there is **no Coroot status or finding**: the section shows the charts and sample counts, and says the start is Kubernetes', not a Coroot revision.
- **Recommendation: C, after the spike names the application Coroot keeps for a Rollout's pods.** It is the only option that works whatever Coroot keeps, and it never invents a verdict. Until then a Rollout's Stage says "Coroot keeps revisions of Deployments only". C is its own late slice.

### From a revision back to the change

The selected revision's Inspector offers **Open the change**. The join, with its keys:

1. **The entry.** The revision's application id carries its Coroot cluster, and the connection's project is known; the entry is the one whose link names that (project, cluster). Setup's uniqueness rule makes it at most one. None: greyed out, "No workspace entry links ⟨cluster⟩".
2. **The ReplicaSet.** The Deployment's ReplicaSet whose `pod-template-hash` is the revision's hash, in the application's namespace, read on that entry.
3. **The digest.** The image digest in the `imageID` of that ReplicaSet's pods; what runs is read from the pods, never from the spec ([DESIGN.md](DESIGN.md#the-change-page)).
4. **The Freight.** The Freight, on Kargo's entry, that holds that digest, joined as the change page joins it. More than one Freight holds it: the newest, said so.
5. **The change page**, through a `ChangeLink` event the shell turns into navigation, as the other links.

It is greyed out with why when the entry isn't a workspace entry, when its pods aren't listable ("pods is forbidden on ⟨entry⟩"), when the ReplicaSet's pods carry no digest (a tag-only spec or pods that ran on), when no Freight holds the digest, and when Kargo isn't read. Each reason is its own sentence. The join reads, on demand, from the entry's pool: the ReplicaSet and its pods by label, then Kargo's Freight of the Deployment's Kargo project. Nothing here is a revision-to-change guess by time or name.

### The screens

DESIGN.md's change page text is updated by slice 4, so it doesn't collide with [#698's Chain view](https://github.com/skel84/freshkube/pull/735), which edits the same section. What follows is that text, in DESIGN.md's words.

**After the deploy** (the Inspector section of a Deployment hop, both views). A captioned section, `AFTER THE DEPLOY`, in `ui::caption` as the Inspector's other sections. Rows, top to bottom:

- *Loading*: the section's heading and `LoadingRows`, never a spinner on a Refresh. The hop's other sections show at once.
- *Found*: the revision line, Coroot's glyph and findings, the counts line, the charts, the tools.
- *Every other outcome*: one `ui::banner` whose tone follows the case and whose body is the sentence in the table above, with Retry for failed and refused, and **Open Settings** for no connection, no cluster linked and no project. Tones: no revision, no application, no project, no cluster and not looked for are Warn; refused and failed are Crit, as Deployments shows them; found never has a banner, and no data and early data are Info. Nothing is `Good`: a section never says fine.
- *Stale*: a failed refresh keeps the last answer under a `Couldn't read again` Warn banner with the last good time.
- The section's footer line, `Read from Coroot · ⟨time⟩`, in `muted`.

**The mark** (List's Detail, Chain's card). Coroot's status glyph at 12 dp with the tooltip above. It is absent until read. In Chain it sits in the card's top-right corner, beside the hop's own glyph; links keep their three confidence line styles.

**List and Chain.** Both draw one derived trail, so the section, the mark and the selection are the same in both. A Deployment hop is one card in Chain and one row in List; selecting either shows the Inspector with the section, and switching views keeps the selection and the read. The reverse link adds nothing to the Chain: its key is the image digest the Freight link already shows in its pill.

**Narrow windows and large text.** At 760 × 560 and text size 20 the Inspector stacks under the trail, and the section's charts are one per row at the Inspector's width; counts and findings wrap, never cut. Sizes are `dp`.

**Changes list.** The Inspector of a change shows, for each Stage with a Deployment, `After the deploy: open the change to read it` (a link to the change page) until the change page has read that Stage's outcome this session; then it shows the mark and the status word. A row's Stages cell takes no mark: it is link confidence and Stage state, and the verdict is neither. See [decision 2](#decisions).

### Slices

Each slice is one pull request with a changelog fragment under `changelog.d/coroot/`. **Claude** marks a design or core-join slice; **Codex** a leaf built on a settled interface. #697 slices are Settings' ([#697](https://github.com/skel84/freshkube/issues/697)); #698's is the Chain view ([#698](https://github.com/skel84/freshkube/issues/698), pull request 735).

| # | Slice | Who | Files | After |
| --- | --- | --- | --- | --- |
| 0 | This design | Claude | docs | none |
| 1 | **Setup rule.** `Setup::validate` refuses two entries linking one (project, Coroot cluster), and names an entry with `cluster` but no project anywhere. Round-trip tests on the example in SETUP_FILE.md. | Claude | core `setup/validate.rs` | none (the types landed with #706's slice 1) |
| 2 | **Core: a deploy's outcome.** `coroot::outcome`: `Link` (project, cluster, entry), `Query` (link, namespace, name, hash), `Outcome` with every row of the table above, `Provider::outcome` over `revision_view` and `RevisionWindow`, bounded and cancellable with its caller. Core tests on fake answers: each outcome, a mismatched application id, two entries, a link edit. | Claude | core `coroot/outcome.rs`, `coroot/tests.rs` | 1 |
| 3 | **Marker focus and Show on charts.** `PanelView::set_focused_marker`; Show on charts on a revision row and in the Application page. UI tests, smoke. | Codex | `freshkube-monitoring` `panel/`, observability `deployments/`, `application/` | none; merge #520 first |
| 4 | **The change page's section.** The `OutcomeReader` handed to the shell and the change page; the job, its identity and generation; the section, the mark, every state; example data for every outcome (new Applications shape, listed in AGENTS.md's knob table); DESIGN.md's change page text. UI tests (a late answer doesn't land, hide drops it, another entry's answer never draws), smoke at 1280 × 880 and 760 × 560, light and dark, and at text size 20 once. | Codex (core seam: Claude) | `applications/change/`, `delivery/change/example.rs`, `desktop/shell/` | 2, #697 slice 10 (Observability reads the connection from the setup file), #706 slice 3 (the owner that reloads the setup) |
| 5 | **Per-entry link in Observability.** The map's and reports' Kubernetes links read the open entry's key; `Association` and "Link Kubernetes cluster…" go. | Codex | observability `connection.rs`, `settings.rs`; desktop shell | 4, #697 slice 12 (the per-entry editor) |
| 6 | **The chain's Coroot mark in Chain.** The corner mark on a Deployment card; the same tests in Chain. | Codex | `applications/change/` (the Chain view) | 4, #698's Chain slice |
| 7 | **The Changes list.** The Inspector's per-Stage line and the memo that feeds it. | Codex | `applications/changes/` | 4 |
| 8 | **Core: from a revision to its Freight.** The join in the reverse-link section, with every reason. | Claude | core `coroot/`, `freshkube-delivery` (a seam, no UI types) | 2, 5 |
| 9 | **Open the change.** The Inspector button, `ChangeLink`, the shell's navigation, the greyed-out reasons. UI tests, smoke. | Codex | observability `deployments/`, desktop shell | 8 |
| 10 | **Rollouts.** The spike (read-only, a test Coroot; reports on #269), then the option the owner picks. | Claude | docs; then core | 2 |
| 11 | **Docs and close.** ROADMAP step 6 to Done, this section's text trued up, `features/applications.md`, a read-only live check if a test Coroot is available. | Claude | docs | 4 to 9 |

Slices 1 to 3 can run beside each other and beside Part A. Slice 4 is the earliest the owner sees anything, and it works on example data before any live Coroot is linked.

### Decisions

The owner answered the open questions on 20 October 2026: all as recommended.

1. **Two entries that link the same Coroot cluster are refused,** as a setup error that names both entries. It keeps an application's owner unambiguous, and the reverse link needs it.
2. **The Changes list marks only what the change page has already read** this session. Otherwise its Inspector says "open the change to read it". The Stages cell takes no mark.
3. **The change page's window is fixed at ±30 min.** Compare in Observability keeps the picker.
4. **The Inspector shows the one report's charts,** at most four.
5. **Coroot's verdict doesn't count in the chips.** The chips count a hop's own state; the mark beside the hop is where the verdict shows.
6. **Marker focus goes by start time,** so `Marker` and Monitoring's own markers stay untouched.
7. **Argo Rollouts use option C:** a window around Kubernetes' own start, with no Coroot verdict, after a read-only spike that names the application Coroot keeps for a Rollout's pods.
8. **No fallback to a connection-wide link.** An entry without `coroot.cluster` is "not linked", even in a one-entry workspace.
