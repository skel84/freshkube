# Coroot integration

Connect the Observability prototype from design2 to `coroot-rs`, the async Rust library in the user's [corust repository](https://github.com/skel84/corust). The first live delivery is Applications → service map → application reports. Prometheus dashboards remain the separate Monitoring feature.

This is the implementation plan, not a claim that a live provider exists. Tracking: [Freshkube #25](https://github.com/skel84/freshkube/issues/25). Source review on 4 October 2026 used Freshkube main `235ad5b`, design2's working Observability module and corust `dd6d3b21640d70cb22a3d524bd2fe23fb7559f61`. No live Coroot calls or runtime measurements were made for this review. Recheck the source and branch state before implementation.

## Order of work

| Step | Work | Dependency and completion evidence |
| --- | --- | --- |
| 1 — Done | Preserve all present application signals in `coroot-rs` ([corust #1](https://github.com/skel84/corust/issues/1)). | [corust PR #3](https://github.com/skel84/corust/pull/3), merge `605dced7850271668cf781e6589a0d3e7e66e5bc`. All 43 workspace tests, strict Clippy and formatting pass; seven fake-server CLI checks cover output modes, filters and ordering. No live calls. |
| 2 | Finish [Freshkube #2](https://github.com/skel84/freshkube/issues/2) access-identity wiring; correct state guidance in [#24](https://github.com/skel84/freshkube/issues/24). | design2 landed in [PR #27](https://github.com/skel84/freshkube/pull/27), merge `762d01c`. The guide correction can proceed now. Use the integrated identity contract before accepting live cross-context behavior; core provider work can proceed independently. |
| 3 | Add the core Coroot connection/project/capability/read boundary, then connect H1 Applications, H2 service map and supported H3 report evidence. | Explicit provider identity, stable selections, fake-server/core/UI tests, fixture/live shared projection, visible-page reads and rejected obsolete results. Pin the reviewed library revision. |
| 4 | Connect incidents and supported trace exploration; extend typed APIs for complete charts, deployment comparisons and CPU profiles as needed. | [corust #2](https://github.com/skel84/corust/issues/2) records data gaps. Validate fields against supported server responses before claiming a prototype page is complete. |
| 5 | Consider sharing Coroot findings with Health/attention. | Separate feature scope: preserve provenance and avoid duplicate reports. Revisit Diagnostics extraction only if this work reaches its existing collection/evaluation path. |

The Diagnostics and Lifecycle refactors in [#4](https://github.com/skel84/freshkube/issues/4) remain useful follow-ups. Diagnostics mixes Cilium operator, Hubble relay and cert-manager reads with rule evaluation; moving those reads allows one pure evaluator for live and example snapshots. Lifecycle still owns Talos/Kubernetes compatibility and version-skew rules in desktop. Neither path is needed to read Coroot's existing application analysis. Application rollout history and Talos/Kubernetes Lifecycle are different consumers.

## What the library provides

The reusable crate is `coroot-rs`; Freshkube should not spawn the `corust` CLI or introduce a JSON serialization boundary between core and desktop. Coroot response decoding stays in the library. Its current models are useful compact observations, but not a complete native-UI data contract.

| Prototype | Existing client support | Work to validate or add |
| --- | --- | --- |
| H1 Applications | `Project::applications`, AppId, categories, aggregate status and named signals; signal presence/status is preserved from `605dced`. | The UI has 11 report columns while the client exposes 12 signal keys, including separate disk I/O and usage. Document the mapping and keep unsupported/unknown values honest. |
| H2 Service map | `service_map`, AppId nodes, directed links and link statistics. | Stable selection and bounded desktop layout. Some statistics are parsed from rounded display text; do not imply precision the source lacks. |
| H3 Application | `app_health`, reports/issues, dependencies and clients; API-key/MCP adds vitals, summarized charts and log patterns. REST provides less detail. | The native reports need an explicit mapping from server reports to tabs. Complete chart histories, units and report conditions may require typed REST detail. Reuse Coroot's analysis; do not derive asserted conditions from the mock's strings or arbitrary local thresholds. |
| H4 Incidents | Incident list/detail, SLO and root-cause models. | Verify each displayed field, time window and evidence link; preserve missing analysis and unsupported capabilities. |
| H5 Deployments | Rollout version, status, start estimate and summary notes. | Historical specs, comparison data and stable revision identity are not exposed by the reviewed deployment model. Verify whether the server provides them. Current Kubernetes YAML is not historical rollout evidence. |
| H6 Profiling | No typed CPU-profile endpoint/model found in the reviewed library. | Add verified profile and comparison reads. The flame graph returned by trace outliers is a separate feature, not CPU profiling. |
| H7 Traces | Endpoint summaries, errors, outliers and spans. Metrics queries also return bounded series. | Verify actual latency/time buckets, selected-bucket samples and outlier decoding. Summary quantiles alone cannot reconstruct the mock heatmap. Keep units and truncation explicit. |

At the reviewed baseline, `application()` omitted a signal if its value was empty and its status below `Info`. [corust #1](https://github.com/skel84/corust/issues/1) is now fixed: empty `ok`, empty `unknown` and absent signals remain distinct, present null/malformed signals become unknown, and Coroot's aggregate status remains unchanged. JSON/JSONL include the previously omitted entries; tables and raw output retain their behavior. Pin the merged revision `605dced7850271668cf781e6589a0d3e7e66e5bc` or a subsequently reviewed descendant when adding the dependency. Freshkube does not consume this library yet.

## Ownership and request lifetime

- `coroot-rs` owns HTTP/MCP protocols, typed response decoding, source IDs, units and transport errors. Add missing typed APIs there instead of parsing Coroot widget trees in GPUI code.
- A focused `freshkube-core` module owns the selected provider/project, capability assessment, bounded reads and source evidence. Use the existing crate boundary; no generic multi-provider framework is needed for this feature.
- Desktop's retained `ObservabilityPage` owns navigation, selections, request handles and prepared display data. Fixtures and live observations feed the same projection. Replace static-only labels and index selections with owned data and stable IDs as those paths are connected.
- Give every request the relevant provider identity, associated Freshkube access identity, project, time range, subject and generation. Reuse Snapshot/OwnedJob contracts where they fit. Run I/O on Tokio and update entities through GPUI; late answers cannot replace current-source data.
- Only the visible destination reads the data it needs. Hiding or replacing the source drops ordinary reads and timers. Retain last-known values only for the same identity, with explicit stale state; a project/context switch clears the previous source's evidence.
- Keep provider/authentication/coverage/freshness errors separate from application health. Empty success, unknown, refused, unsupported and transient failure are distinct. One failed detail read must not discard unrelated successful results.
- Derive filtering, sorting, graph layout and labels when observations or controls change. Bound response retention, graph/series counts and concurrency. Record apply/layout/render cost on representative data; [#22](https://github.com/skel84/freshkube/issues/22) remains a separate unresolved Table/summary performance issue.

Source identity is more than a server URL. Server/project selection and credential revision define the Coroot provider; its association with a Freshkube cluster is explicit and follows #2's integrated access contract. A name match or the CLI's current/default profile is not proof that two clusters are the same. Configuration replacement invalidates affected reads; an ordinary transport retry should not erase a durable selection.

AppId identifies an application, not a Kubernetes object's UID. Pod/workload links need real object identity resolved against the explicitly associated cluster and the existing navigation/shell guards. The prototype's `OpenPod { logs }` event has no subject and must acquire one. External applications and unmapped clusters remain distinguishable instead of navigating to a same-named local object.

## Connection choices to settle during the first slice

The reviewed client accepts a direct HTTP(S) URL with API-key or session authentication and uses `reqwest`. Its custom-HTTP-client hook does not itself provide the Kubernetes service-proxy transport used by Monitoring. An explicit URL is the supported starting option; an in-cluster proxy route would require a deliberate library transport extension. Do not silently borrow Monitoring's transport assumptions.

Record the chosen reachability, explicit project/cluster association and credential storage approach before adding the connection settings UI. Existing corust profiles may be offered only through explicit selection; never inherit an ambient default. Do not store credentials in ordinary preference files, logs or screenshots. A git dependency should use a reviewed pinned revision, not the developer's absolute local path.

Capability state must account for both credentials and server support. The current library requires an API key for MCP-backed traces/metrics and richer health reports; session-backed REST returns less information. Having an API key is not proof that every endpoint/tool is supported. Make unavailable capability visible, without turning it into empty successful data.

Initial integration is read-only. The prototype's rollback/configuration previews, local mute toggle and local threshold values do not authorize live mutations. Keep these controls truthful about their available behavior; live actions require a separately scoped workflow. Coroot's reported check result remains authoritative for Coroot health, while Freshkube's own diagnostics retain their separate evidence and rules.

## Validation and completion

Use sanitized fixtures and fake servers first. Focused tests must cover healthy versus unknown signals; partial success/refusal and recovery; stable selection across reorder/removal; changed server/project/credentials/cluster/range with delayed old answers; hidden-page cancellation; capability-limited responses; real object-link identity; and the fixture/live projection using the same code. Check that fake data cannot populate a live page.

For each delivered slice, run the affected library/core/UI tests and the required workspace checks, with relevant fixture captures and representative performance measurements. Any optional live verification names the selected connection and remains read-only. Do not record raw logs, SQL, trace attributes, response bodies or credentials as test evidence.

Update the roadmap and #25 with the actual landed commit and verified capabilities. A completed Applications/map slice does not complete H1–H7; keep later scope linked and open. Keep mechanical module moves separate from behavior changes, and stop at the boundaries needed for the feature.
