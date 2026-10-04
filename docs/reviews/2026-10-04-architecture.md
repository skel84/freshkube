# Architecture review critique

> Historical review supplied on 4 October 2026, against main `1836c42`. Its branch status, open questions and test gaps describe that revision, not current main. Operations/quorum and watch have since landed, and the user selected remaining etcd tolerance. Follow the [current roadmap](../ROADMAP.md#architecture-review-follow-up) and [architecture index #7](https://github.com/skel84/freshkube/issues/7) for decisions, verified completion and outstanding work. The review text below is preserved unchanged.

> #24's [compatibility decision](../STATE_LIFETIMES.md#asyncstate-compatibility) supersedes this review's suggested `AsyncState` deletion: contributor guidance uses the current desktop types, while core retains its exported legacy API pending an explicit distribution/migration decision.

An independent check of the earlier architecture review, made on 2026-10-04 against main at `1836c42` and the active branches. It is a source review: no files were changed, no tests were run locally, nothing was measured and no cluster was touched.

**Verdict:** the earlier review's code observations all hold, and its direction is sound. Its order of work needs two corrections:

- The Operations work and an etcd display fix depend on no active branch, so they can start now.
- "Finish watch" is a 5,300-line change to how the app reads the cluster, not a bug fix, and should be reviewed as one.

## Where things stand

Main on GitHub is `1836c42`, with #6 (protobuf generation) and #9 (node health in core) merged. The line counts in the earlier review reproduce exactly at that commit.

| Branch | Covers | State | CI |
| --- | --- | --- | --- |
| main | — | `1836c42` | Green |
| watch | #1, #3 | 14+ commits, no PR. Also carries an unrelated FPS readout in the status bar | Red on every finished run. `4dd5526` failed one Overview test that still expects "Can't list events: forbidden". `446635a` still running |
| debt/2-access-identity | #2 | One commit, `663d1aa`, no PR. Core and Resources identity done; shell wiring waits for watch | Green |
| design2 | Layout, ObservabilityPage | No commits; about 64 files changed in its worktree, based on `7aa1045` (before #6 and #9) | — |

design2 edits `summarize_pods` in `kubernetes_summary/mod.rs`, which watch has moved into new files. Whichever branch lands second has to port design2's `by_namespace` count.

## Verdict on the earlier review

- **Holds.**
  - Every code observation checked is accurate at `1836c42`.
  - Line counts were used to choose what to read, not as proof that a file should split.
- **Overstated.**
  - "Finish watch first" hides what watch does: it replaces a list every 15 s with nine persistent, cluster-wide watches on every page. A minimal fix for #1 needed a few dozen lines.
  - The proposed Nodes entity would mostly add indirection.
- **Mis-ordered.** Operations was placed behind #2, but no active branch touches either Operations file.
- **Missed.**
  - Etcd quorum tolerance is computed inconsistently, and the displays overstate safety.
  - Lifecycle's domain logic, and its own Node list, sit in the desktop crate.
  - Watch makes finding C worse.
  - Contributor guidance is stale well beyond `AsyncState`.
- **Confidence.** Right for a source review. Nothing it said about runtime behaviour or render cost was tested or measured.

## Findings A–E

| Finding | Label | Evidence at `1836c42` | What remains |
| --- | --- | --- | --- |
| A. A failed summary part throws away good updates | Verified; fixed on watch, CI not green | `desktop/kubernetes_summary.rs:80-89` drops the whole new summary when a part is `Failed` or a workload error lacks the word "forbidden" (`kubernetes_summary/mod.rs:148-168`). Everything goes stale and Health gets an error | Get watch green and accept its read model |
| B. Operations mixes the screen with live workflow policy | Verified, worse than stated | `run_live` (`screens/operations.rs:748-871`) has no GPUI types and no test; every test passes `live: None`. `simulate_node` re-implements the step order. Core's `run_rolling_operation` (`operations.rs:1764-1987`) is dead and diverges | Move the loop to core with tests; delete the dead runner |
| C. Pilot owns too much feature state | Partly supported | `PageHost` draws Overview and Nodes through Pilot (`desktop/mod.rs:258-274`). One observe dirties three pages (:450-460). `invalidate_target` makes about 20 resets (:751-787) | Only the visible page draws, and no cost has been measured. Worth moving: Overview's display state and the node Services tab |
| D. Diagnostics duplicates rules and prepares data in render | Partly supported | Memory, CPU and service rules are copied (`diagnostics/example.rs:56-69` against `diagnostic_runner.rs:1585-1600`). Render sorts 20–30 checks and clones the snapshot (`view.rs:362`) | Make evaluation pure first: `build_cni_checks` and `build_addon_checks` read the cluster |
| E. The contributor guide disagrees with the code | Verified, understated | `AsyncState` has no users. AGENTS.md shows the old TUI's `draw(&self, frame: &mut Frame, area: Rect)` and says to use it | Rewrite the guidance around `Snapshot`, `Loader`, `LogView` and the mutation slot; then delete `AsyncState` |

**A.** On watch, each source has its own `Observation` with a typed `ObservationFailure`, and `apply_summary` always applies what was published. Two tests target the original failure:

- `events_timeout_must_not_reject_successful_changed_pods` (desktop);
- `independent_failure_retains_evidence_and_allows_other_successes_and_recovery` (core).

Together they cover keeping old data as stale, refused versus failed, recovery and a context change. Only CI has run them.

**B.** No test reaches the live policy: the access recheck, fresh prechecks per node, `etcd_impact` and the drain options.

Core's dead runner also diverges from the live path:

- it skips fresh prechecks;
- it takes etcd impacts up front;
- it writes different audit records;
- it uses `std::time::Instant`;
- it has no panic capture.

These belong in the desktop: the slot, the channel to the UI, the detached task and the deliberate never-abort (:1957). The loop does not, and core already has a fake Kubernetes client for tests (`resources/metadata/tests.rs:26`).

**C.** On main, Pilot notifies about 4 times per 15 s cycle.

- Overview and Nodes own no jobs, so moving Overview out shortens `invalidate_target` by about 3 lines. The rest of that function is shell coordination.
- Choosing a node is choosing the Talos target, so a Nodes entity mostly adds indirection.
- Resources, Monitoring, CustomResources, SystemServices and design2's ObservabilityPage already show the own-state-and-emit-events pattern working.

Watch makes C worse:

- It adds `set_summary_nodes` to the generic `ScreenPanel`, beside `set_workloads`, which only 1 of 9 screens uses.
- It adds more Pilot fields.
- It runs `apply_summary` on every publication. That is inferred, not measured, at up to about 2 redraws a second.

**D.** The fixture node sits at 91%, one point over the 90% fail line, so a threshold change in core would silently diverge from the fixture.

- Sharing an evaluator waits on moving the Cilium operator, Hubble relay and cert-manager reads into collection.
- Reusing the rules and caching display data are separate changes, both for when someone next works there.

## Problems the review missed

1. **Etcd tolerance disagrees between screens.** With 2 of 3 members answering:
    - Overview shows "Quorum · tolerates 1 member failures" in the good tone (`presentation/overview/mod.rs:172-176`, using `(total-1)/2` from `presentation/mod.rs:313`).
    - The etcd screen says the same, and a UI test asserts that label (`screens/etcd.rs:243-275`, test at :1113).
    - Core Lifecycle computes `can_lose = 0` for the same state (`security_lifecycle.rs:1403`).
    - Operations gates mutations on core's figure, so the error is display only. It still breaks the "No false positives" rule.
    - Fix: one core function returning the quorum size, the designed tolerance and the remaining tolerance.
2. **Lifecycle does domain work in the desktop.**
    - `collect_kubelets` (`lifecycle/mod.rs:187`) is the only direct Kubernetes list in a desktop screen, and it lists Nodes a second time on every refresh.
    - The support matrix and the kubelet skew rules live there too.
    - An agent's claim that the drift logic "disagrees" with core is overstated. Core warns that a difference exists; the desktop decides by majority which nodes differ.
3. **Shared types sit inside feature modules.** Move each one when work reaches it:
    - `NodeRow` and `NodeTab` live in `desktop/nodes`, and Resources depends on them through `ResourceLink::Node`;
    - `presentation` imports `desktop::Page`;
    - the shell guard lives in `resources/pane/shell`.
4. **Two identity types will need to meet.** Both are deliberate:
    - watch's `SessionIdentity` (connection, revision);
    - debt's `AccessIdentity` (session id, configuration revision).

    Check how they map when #2's shell wiring lands on watch.
5. **Small items.**
    - `health_tone` is defined three times (`ui.rs:67`, `lifecycle/mod.rs:801`, `workloads/mod.rs:175`).
    - The Operations tests' `wait_until` uses `Instant::now()` and `thread::sleep` (about :3263), which AGENTS.md forbids.
    - `freshkube-core` lacks `publish = false`.

## Proposed file splits

| File | Verdict | Why |
| --- | --- | --- |
| desktop `screens/operations.rs` | Yes, within B | One commit with no logic change, after the loop moves to core |
| core `operations.rs` | Delete first | Removing the rolling runner takes out about 520 lines; reconsider after |
| core `diagnostic_runner.rs` | Only with D | Without making evaluation pure, a split only moves lines |
| `talos-rs/client.rs` | Partly | Split out packet capture and BPF only (about 540 lines plus 225 of tests). Reject a methods/models split: private `from_proto` and `to_proto` would have to widen |
| `security_lifecycle.rs` | When touched | Two collectors in one file |
| core `inspection.rs` | When touched | Clean along its existing headers, but low value |
| core and desktop `maintenance.rs` | Defer | The reducer/runner boundary exists; moving tests to `tests.rs` is the main gain |

## Order of work

```
START NOW (no active branch touches these)      AFTER THE BRANCHES LAND

1. Operations policy in core (B)                3. watch lands ----- design2 lands
   delete dead runner → core tests →               (green, FPS out)   (summarize_pods conflict)
   move run_live loop → split screen file               |       \            |
                                                         v        \           |
2. One etcd quorum function                         debt/#2 wiring \          |
                                                                     v         v
                                                    After both: typed handles, then C;
                                                                AGENTS.md fix, delete AsyncState
```

- **The real dependency on watch:** #2's shell wiring needs the two identity types to map.
- **Waits only to avoid merge conflicts:** the AGENTS.md fix, because watch and design2 both edit that file.
- **Waits on both branches:** C, because watch and design2 rewrite the same functions.

## The three highest-value next changes

### 1. Make live Operations policy testable (B)

- **Steps:**
  1. Delete the rolling runner in its own commit.
  2. Write core tests against the fake client.
  3. Move the body of `run_live` and `finish_run` into a core `run_selection(…, is_cancelled, on_progress)`.
  4. Split the screen file in a commit with no logic change.
- **Stays in the desktop:** the access recheck, the operation slot, the channel and the progress display.
- **Must survive the move:**
  - fresh prechecks: one for the whole selection under the 45 s timeout, and one per node in a multi-node run;
  - fingerprint confirmation;
  - cooperative cancellation that never aborts a submitted mutation;
  - an explicit outcome for every target, including panic capture and "worker stopped";
  - both audit layers: core's per-step records and one desktop record per node;
  - no audit in example mode.
- **Benefit:** the only untested safety path gets tests, and a policy change no longer needs a 3,750-line GUI file.
- **Risk:** weakening a precheck during the move. Write the tests before moving code.
- **Depends on:** nothing active.

### 2. One core quorum function for every etcd display

- **Scope:**
  - one function returning the quorum size, the designed tolerance and the remaining tolerance;
  - Overview and the etcd screen both call it;
  - tests at 2 of 3 and 3 of 5.
- **Benefit:** fixes a visible "No false positives" breach.
- **Risk:** low. The etcd label test (:1113) has to be updated on purpose.
- **Open question:** should the label show the remaining tolerance, or the designed tolerance with clearer wording?
- **Depends on:** nothing. Watch touches `presentation/overview/mod.rs`, so expect a small rebase there.

### 3. Land watch deliberately, then add typed handles

- **Steps:**
  1. Get CI green.
  2. Move the FPS readout to its own PR.
  3. Review the branch as a read-model change.
  4. Replace `set_workloads` and `set_summary_nodes` with typed handles to Health and Lifecycle.
- **Decision needed:** is watching every Pod on every page acceptable? Watch's stress runs ended near 431 MB resident, synthetic server included.
- **Benefit:** fixes A, and gives #2 its identity contract.
- **Risk:** memory and redraw cost on large clusters. Measure Pilot redraws with render probes before considering an Overview entity.

**A cheap fourth:** correct the AGENTS.md state guidance and delete `AsyncState`, once watch and design2 have landed.

## Deferred and rejected

| Recommendation | Decision | Reason |
| --- | --- | --- |
| Methods/models split of `talos-rs/client.rs` | Reject | Widens private conversion functions; only packet capture is a separate concern |
| A separate Nodes entity | Reject | Choosing a node is choosing the Talos target, which the shell owns |
| Sweeping splits of maintenance and inspection | Reject | Boundaries already exist; split when touched, as AGENTS.md says |
| Overview entity | Defer | Wait for watch and design2, then measure redraws first |
| Neutral module for shared types | Defer | Move `NodeRow`, `NodeTab` and `Page` when work reaches them |
| Diagnostics display caching | Defer | Low cost today; the snapshot clone matters more than the sort |
| Fixture data using core's evaluator | Defer | Waits on making evaluation pure |

## Testing

- **Read:**
  - the Operations UI tests, all on example data;
  - core's `ordered_sequence`, panic and audit tests;
  - watch's summary tests in core and the desktop;
  - the etcd quorum label test;
  - Maintenance's fake-runner tests.
- **Run locally:** none. A local run of watch's core summary tests did not complete. CI is the only run evidence: main and debt/#2 are green, and watch is red on every finished run.
- **Still to write:**
  - [ ] core `run_selection` tests: both gates, cancellation at each point, stop-on-failure, per-node prechecks, the audit records;
  - [ ] quorum tests at 2 of 3 and 3 of 5;
  - [ ] render-probe counts for Pilot redraws, once watch lands;
  - [ ] a pure evaluator test for Cilium and cert-manager, once those reads move.

## Uncertainty and a stopping point

Stop after the three changes and the guide fix. After that, split files only when feature work reaches them.

Still uncertain:

- real redraw and memory costs, which are unmeasured;
- whether watch's `446635a` goes green;
- whether design2's uncommitted work changes Pilot further;
- whether Diagnostics' scroll-to-selection on every render (`view.rs:405`) snaps a scrolled list back, which is unchecked.
