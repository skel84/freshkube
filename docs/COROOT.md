# Coroot integration

Freshkube connects Applications, the service map, supported application-report evidence and read-only Incidents through `coroot-rs`. Prometheus dashboards remain the separate Monitoring feature. [Freshkube #25](https://github.com/skel84/freshkube/issues/25) tracks the broader integration and stays open for later destinations.

The first slice is implemented in `05d03ce`, with the Disconnect-state correction in `1b16b42` ([PR #31](https://github.com/skel84/freshkube/pull/31)). The validation record below distinguishes completed checks from the remaining integration scope.

## Dependencies and sequence

| Step | Status and boundary |
| --- | --- |
| Preserve present signals | [corust PR #3](https://github.com/skel84/corust/pull/3), `605dced7850271668cf781e6589a0d3e7e66e5bc`, preserves empty healthy, empty unknown and absent signals. Freshkube keeps those distinctions. |
| Access identity | Satisfied by [Freshkube PR #30](https://github.com/skel84/freshkube/pull/30), merged main `4bbc775b5361ac32d8f7876f470325da2e3df1c5`; #2 is closed. Desktop uses `KubeSource.id`, the opaque access-key adapter described in [ACCESS_IDENTITY.md](ACCESS_IDENTITY.md). |
| Bounded responses | The dependency pins incident-audited corust `a1fd85d649a9274d8a4ad774d16d671c4278781d` ([PR #7](https://github.com/skel84/corust/pull/7)), including response bounds from [PR #4](https://github.com/skel84/corust/pull/4) and collection-shape validation from [PR #5](https://github.com/skel84/corust/pull/5). Freshkube sets an 8 MiB limit before REST/MCP decoding. No absolute local dependency path is committed. |
| First live slice | Core provider, explicit connection/project/access association, Applications → map → supported reports, shared fixture/live projection and guarded object navigation, implemented in `05d03ce`. Validation is recorded below. |
| Read-only Incidents | Latest-project sample → selected incident → source SLO/RCA → application reports; [contract and bounds below](#read-only-incidents-slice). |
| Traces and Profiling | Read from Coroot's own per-application tracing and profiling views; [contract and bounds below](#traces-and-profiling). |
| Later destinations | Complete report histories and historical deployment comparisons remain in [corust #2](https://github.com/skel84/corust/issues/2). |
| Shared Health/attention | Separate future scope requiring provenance and deduplication. |

The remaining lenient library list readers (nodes, deployments, risks, alerts and alert rules) need endpoint-contract audits before later adoption; tracked in [corust #6](https://github.com/skel84/corust/issues/6). This first slice does not call those readers.

[Guidance cleanup #24](https://github.com/skel84/freshkube/issues/24) is complete; [Diagnostics/Lifecycle extraction #4](https://github.com/skel84/freshkube/issues/4) remains an independent follow-up. It is not a prerequisite for reading Coroot's existing analysis. The separate Table/summary performance follow-up [#22](https://github.com/skel84/freshkube/issues/22) remains open.

## Connection policy

The first slice connects directly to an explicitly entered HTTP(S) base URL reachable from the desktop. TLS verification stays enabled. Redirects are refused so credentials cannot be forwarded to another endpoint. Kubernetes service-proxy transport would require additional library work; this integration does not reuse Monitoring's transport or an ambient corust profile.

API keys and existing `coroot_session` values are entered explicitly. Anonymous access is also an explicit choice. Credentials are masked in the input and never saved in preferences, diagnostics or screenshots. By default a key stays in memory for the window's connection lifetime. **Remember key in Keychain** (Credential Manager on Windows, the Secret Service keyring on Linux) keeps it in the system's credential store instead, under the service `Freshkube` and the account `Coroot <server URL>`, through `secrets.rs`. A stored key is only ever sent to the server it was saved for, and the field stays empty while it is used. When the store can't save it, Remember turns off, the form says why and the key stays in memory; a key is never written to a file instead. Disconnect forgets the key, including while connecting or after a failed connection or credential edit, deletes the stored copy and stops reconnecting on launch.

`coroot.json` beside the preferences remembers the server URL, the sign-in choice, the last project, whether to reconnect and whether to remember the key; it never holds a key. After a successful connection, the first time the page shows on a later launch it connects again: at once for anonymous access, or once the store returns the key, and it reopens the remembered project on the same server. Nothing is read from the store before the page shows. Tests use an in-memory store; only the app's own launch (`GpuiOptions::with_keyring`) uses the system's. Editing, replacing or clearing URL, credentials or authentication mode cancels pending reads and clears the old connection immediately, including when a replacement connection fails.

Connecting lists accessible projects. It does not select the first project. The project picker displays both name and server ID, so two projects with matching names stay distinct.

Kubernetes navigation requires a separate explicit association between a Coroot cluster ID observed within that project and the current Freshkube access key. Matching context or cluster names never establish this relationship. Replacing access/configuration cancels reads, clears ordinary evidence and removes the association; the new access must be associated explicitly. A node change or compatible transport retry retaining `KubeSource.id` retains the association. External applications, unsupported kinds and unmapped clusters remain inspectable without Kubernetes links.

## Ownership and request lifetime

`coroot-rs` owns Coroot HTTP/MCP protocols and typed response decoding. `freshkube-core::coroot` owns provider identity, explicit project selection, capability/error categories, read deadlines, concurrency and retention limits. It has no GPUI types. The library is called directly; no CLI subprocess or serialization boundary sits between core and desktop.

The retained `ObservabilityPage` owns selections, request handles, `Snapshot` values, prepared display data and rendering. It runs network work on Tokio through `OwnedJob` and applies results through GPUI. Fixtures construct the same typed observations and use the same Applications, map and report projections as live responses.

Every request captures the opaque Coroot provider identity, project, optional cluster association, current Freshkube access key, absolute time interval and subject. A separate page request generation guards delivery, and each `Snapshot` checks its identity and generation. Creating a connection gives it a new provider identity even when the URL and credential text are unchanged. An ordinary retry retains the provider identity.

Only the visible destination starts ordinary reads. Hiding, changing destination or replacing the source drops its jobs; there is no background Coroot refresh timer. A report requests REST and MCP evidence independently, so one unavailable source cannot erase the other.

Refresh and reopening capture a new Last N hours interval. Destination/report navigation and **Retry** retain the captured absolute interval, shown with local dates and times. Data from a different interval or source is cleared. A failed retry for the same interval can retain successful evidence, marked **stale**, with the last successful local time and the read error. Source failures, loading, successful empty results, application health and unsupported capabilities remain distinct.

## Presentation and navigation

| Surface | First-slice behavior |
| --- | --- |
| Applications | Coroot aggregate health, type, category and 12 named signal columns. Present empty healthy values say Healthy; present empty unknown values say Unknown; absent signals say Not reported. Coroot's status is never recalculated from strings or local thresholds. |
| Signal/report mapping | Errors and latency → SLO; upstreams and network → Net; instances and restarts → Instances; disk usage and disk I/O → Storage; CPU, memory, DNS and logs use their named report. Dynamic report tabs also expose other report names returned by the source. |
| Service map | Stable AppId nodes and directed endpoint-pair links, prepared layered layout (callers left of callees), source status and available request/latency/traffic metrics. Missing metrics say Not reported. Statistics derived by the library from rounded Coroot display values are identified as potentially rounded. |
| Application reports | REST evidence, then only what MCP adds, in one card whose footer keeps each source's state and Retry: Coroot report statuses/issues, dependencies/clients, and available vitals, labeled chart summaries and log patterns. Dependency health and connection health remain separate. Summaries identify missing points and omitted series; complete histories are not implied. |
| Capabilities | API-key presence allows an MCP attempt but does not prove server support. Session/anonymous access keeps REST available and reports richer evidence as unsupported. Authentication, refusal, missing subjects, unreachable sources, decoding failures, timeouts and size-limit failures have separate messages. |
| Incidents | Bounded latest-project sample, stable key/application selection, source-reported SLO objective/compliance and burn values, RCA source text and problem-propagation application links. No mutation, invented charts or ruled-out evidence. |
| Traces | One application's latency and error heatmap, the latest 100 matching spans, and the selected trace as a waterfall with each span's status, details, attributes and events. A heatmap cell lists that bucket's requests; Failed requests lists the window's errors; OpenTelemetry and eBPF are chosen when Coroot has both. |
| Profiling | One application's profile types, instances and flame graph, optionally compared with the window before it. Frames show their share of the total and, when compared, the change in that share; Biggest increases lists the frames that gained most. |
| Later destinations | Live Deployments shows the current integration limitation. Its fixture screen remains a preview. |

Application and link selections use stable domain identities and survive response reorder. Removing the selected subject clears the selection safely. Sorting, filtering, group counts, labels and graph layout are prepared when observations or controls change. The applications list is virtualized. The map displays prepared pages of 24 applications, adding at most two endpoints for the selected connection. It states the displayed/retained counts; the first 24 connection markers plus the selection are shown on the graph. Every retained connection remains available in the virtualized inspector, including links between different pages. Category and namespace menus show at most 100 choices with an explicit hint; text search searches all retained applications.

Report evidence is paged at 24 entries or about 16 KiB of prepared text, with a single long entry kept together. Every bounded entry remains available with its complete source text. A late answer from one report source preserves the other source's page for the same provider/access, application, report and time interval; changed identity resets it, and shorter evidence clamps it.

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
| Report evidence | 32 reports, 200 total issue/log entries, 128 summarized series, 32 vitals, 300 dependency/client links |
| Series | 512 summary points, 32 labels per series |
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
| 200 long report issues | 3.40 ms | 57.69 ms |

The earlier unpaged map took 2,927.92 ms for ten frames while compilation was active; representative paged-map runs under build contention measured 261 and 443 ms. These debug observations have different load conditions, and forced frames bypass normal view caching. They are not release frame-time acceptance; #22's independent Table/summary targets remain open. Reproduce with the `bounded_projection_measurement` and `large_report_evidence_is_paged_without_losing_source_text` desktop tests using `--nocapture --test-threads=1`.

The optional anonymous HTTPS check reached the explicitly selected Coroot endpoint; `/api/user` returned HTTP 401. Live authenticated data has not been checked by this branch.

The user authorized read-only verification through `~/code/home-ops/kubeconfig`, explicitly selected context `admin@kubernetes`. This does not authorize an ambient context, mutation or unnamed pod port-forward. Optional live evidence must record counts/statuses only and keep credentials and raw log/trace payloads out of the repository.

## Read-only Incidents slice

The prerequisite is [corust PR #7](https://github.com/skel84/corust/pull/7), pinned at `a1fd85d649a9274d8a4ad774d16d671c4278781d`. Its [contract audit](https://github.com/skel84/corust/blob/a1fd85d649a9274d8a4ad774d16d671c4278781d/crates/coroot-rs/INCIDENTS.md) names the upstream Coroot revision and handlers. Malformed envelopes, identities, required impact/duration and supported evidence fail explicitly. A reported zero remains zero; missing optional SLO/RCA evidence remains absent. The additive `incident_view` API preserves existing public record fields and methods.

Coroot's REST list returns a bounded sample ordered open-first then newest. It does **not** filter history by the toolbar window; detail reads use the incident's own time context. The page states both facts. The sidebar labels open counts as sampled, and neither an empty result nor fewer than 100 rows proves a complete history. `data: null` before a world is available is an empty sample, with a refresh explanation; null detail cannot identify an incident and fails.

The existing retained page owns list/detail snapshots, selection by incident key and application ID, prepared text and paging. Each detail has its own owned Tokio read plus GPUI delivery task, so a removed/replaced selection cancels it while retaining a concurrent list read. Page generation and snapshot identity/generation reject obsolete completions. Hiding, destination/source replacement and disconnect drop both jobs. A same-window retry retains successful evidence as stale; a new source/window clears it. A successful list removes an absent selection and its evidence. Detail application identity must match the listed application.

SLO objective/compliance are Coroot-rendered strings. Violated flags and incident severity retain source meaning; affected-request impact is not used to reconstruct compliance. Burn windows, values and thresholds come from the response, never a fixed 14.4× rule. Missing objective/compliance, impact, burn rates and analysis are marked not reported. RCA status, summary, root cause, suggested fixes, analysis errors and problem propagation are read-only source text, with no executable Markdown links or mutation controls. Application links enter the existing report workflow using the toolbar interval, rather than reconstructing a historical report from incident widgets; later Kubernetes links still require explicit association and shell guards. Stale incident links cannot navigate.

List and propagation paging keep every bounded item reachable while rendering at most 10 each. The original fictional H4 fixture remains an explicit preview; fake-server tests exercise the production live projection. Incident charts, RCA widgets/healthy propagation links and structured ruled-out evidence are not represented by this client API. Traces, profiling, historical deployment comparisons, shared Health/attention and #22 performance work remain separate.

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
