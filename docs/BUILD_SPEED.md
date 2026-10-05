# Build and change speed

Research for [#53](https://github.com/skel84/freshkube/issues/53), measured from
`e27a83d24acbf5ab3656c12b2e3fdfdbf25625ae` on 5 October 2026. This is a research
record: no application, dependency, profile or CI changes are proposed by this PR.

The user has ended the broad measurement batch. The separate [PR #88](https://github.com/skel84/freshkube/pull/88) from current
main adds focused inner-loop guidance, disable desktop doctests (there are
none), and set workspace dev debug information to line tables, as CI already does.
It will record one warmed, one-line desktop library-test rebuild on main and one
on the branch, then run the standard checks once.

**Parked: linker changes, nightly/Cranelift, nextest, further disk work and crate
splits — revisit only if a real loop shows the need.** The user subsequently
approved scoped sccache adoption for real worktree builds, described below. No
nightly tool was installed. Earlier proposed experiments below are the research
record, not pending work. #53 step 3 also remains held until Monitoring's page
migration lands.

## What is established

- Desktop contains 88,357 Rust lines across 186 files, including tests. Core is
  40,695 lines across 81 files. Splitting a file does not change the compilation
  unit; splitting a crate still invalidates its dependants when it changes.
- The shared UI and core are low in the dependency graph. Their edits reach
  desktop and the app binary. A feature crate's isolated test command can avoid
  desktop; a full app build cannot assume the same saving.
- Four recent successful CI checks took 315–355 seconds. The ordinary test step
  accounted for 137–158 seconds; stress checks took another 57–91 seconds.
- Local workspace crates retain full debug information, while CI explicitly
  requests line tables. This is a configuration candidate to measure first.
- Cargo already defaults to unpacked debug information on macOS. Adding that
  setting explicitly is not a demonstrated optimization.

The cold observation and CI evidence below are retained. Broader local comparisons
were not run after the user narrowed the work; no missing measurement implies a
gain. The focused implementation PR will carry its own validation and timings.

## Measurement conditions

The machine is an Intel Core i7-9750H (12 logical CPUs), 16 GiB RAM, with
Homebrew Rust 1.98.1 / LLVM 22.1.8, Apple linker 1115.7.3 and Homebrew LLD 23.1.2.
There was no `target/` before the first build. Registry sources were already
present, so this is an empty-artifact build, not a cold download experiment.

Every Cargo command uses this worktree's own `target/`, with `CARGO_TARGET_DIR`
unset and `CARGO_BUILD_JOBS=2`. The lead grants build slots. Timed commands start
only below a one-minute load of 10. Profile comparisons need repeated alternating
runs in one session; machine load and warm-up runs are recorded separately.

The cold run started at 09:13 UTC with no other Cargo/rustc process. Another
approved build later overlapped it. At approximately 09:42 UTC the lead reported
stopping a Docker VM that had been consuming about four cores. Consequently this
cold sample has changing contention; do not compare it with later quiet runs to
claim an optimization. The load history and compiler-process samples are retained
with the local measurement records. Sampling from 09:19 UTC captured loads
3.74–24.83; 60 of 184 samples were at or above 10. This run is excluded from
controlled comparisons; the later edited-build sessions must meet the load gate.

At the cold baseline, nightly, sccache, nextest, cargo-llvm-lines, cargo-bloat and
sold were absent. Sccache 0.18.0 was installed afterward for ordinary team use. Stable Cargo's timing report
separates frontend and codegen sections for library units. It does not separate
individual type-checking or monomorphization queries, and binary units do not
expose the same split. The codegen section is an upper bound for a backend change,
not a promise that Cranelift can remove all that time.

| Baseline | Result |
| --- | --- |
| Empty-artifact `cargo build --locked --timings` | Passed: 3,134.0 s (52m 14s); start/end load 6.91/5.24; mixed contention |
| Desktop one-line edit → build / test compile / clippy | Not run; broader measurement batch parked |
| Core one-line edit → build / package test compile | Not run; broader measurement batch parked |
| UI one-line edit → build / package test compile | Not run; broader measurement batch parked |
| Workspace test binaries and link cost | Not run; broader measurement batch parked |
| Build + test + clippy artifact size | Not run; broader measurement batch parked |

### Empty-artifact build

Cargo recorded 847 compiler/build-script units across 634 package versions. The
largest units were GPUI Component (311.0 s), GPUI Base (250.6 s), GPUI Pre
(207.3 s), the AWS-LC native build script (186.9 s), and image (126.2 s).
These durations overlap; adding them does not give a wall-time saving. They
identify dependencies worth testing with a compiler cache, while retaining the
required dependency optimization.

| Workspace unit, first compile | Frontend | Codegen | Total |
| --- | ---: | ---: | ---: |
| desktop library | 33.52 s | 86.56 s | 120.08 s |
| core library | 19.42 s | 60.48 s | 79.90 s |
| UI library | 1.79 s | 5.46 s | 7.25 s |
| app binary | Not exposed | Not exposed | 6.42 s |

These are **cold compilation** numbers, not the requested one-line rebuild
fractions and not the trigger for installing nightly. Incremental codegen may
reuse most of this work; measure the edited case before ranking an alternate
backend.

The finished `target/` occupies 6.390 GiB of allocated blocks (6.162 GiB logical
file bytes), deduplicating hard links: 4.529 GiB in dependency outputs, 1.484 GiB
incremental, 0.200 GiB build-script outputs and 0.152 GiB in the app executable.
The deps directory includes workspace outputs too. `talos-rs` generated six Rust
files, 4,775 lines / 176,998 bytes; its build-script run took 0.72 s. Its library
compile took 17.36 s. Re-running protobuf generation alone cannot explain the
desktop rebuild cost.

## Workspace dependency graph

This graph comes directly from the eight manifests; it excludes external
dependencies and does not run Cargo. An arrow means “depends on”.

```mermaid
graph TD
    app[freshkube] --> desktop[freshkube-desktop]
    desktop --> core[freshkube-core]
    desktop --> logs[freshkube-logs]
    desktop --> terminal[freshkube-terminal]
    desktop --> ui[freshkube-ui]
    desktop --> probe[freshkube-probe]
    desktop --> talos[talos-rs]
    logs --> core
    logs --> ui
    logs --> probe
    terminal --> ui
    terminal --> probe
    core --> talos
```

The lockfile contains 1,007 packages, including 996 registry packages, three git
packages and eight workspace packages. This includes other platforms and optional
features: it is not the number compiled for a macOS build.

| Edited crate | Workspace crates potentially rebuilt for the app |
| --- | --- |
| desktop | desktop, app |
| core | core, logs, desktop, app |
| ui | ui, logs, terminal, desktop, app |
| logs | logs, desktop, app |
| terminal | terminal, desktop, app |
| probe | probe, logs, terminal, desktop, app |
| talos-rs | talos-rs, core, logs, desktop, app |

## Recent editing surface

Counted non-merge commits since 5 July 2026: 230. A module counts once per commit;
renames and move-only commits are included, so these are activity indicators, not
independent feature changes. Line counts include tests and comments.

| Desktop module | Rust lines | Commits touching its paths |
| --- | ---: | ---: |
| desktop (shell) | 16,930 | 75 |
| resources | 18,321 | 53 |
| screens | 20,061 | 24 |
| logs (sources remaining in desktop) | 2,747 | 23 |
| monitoring | 9,812 | 14 |
| observability | 10,383 | 12 |

The most frequently changed files are `desktop/mod.rs` (43 commits),
`desktop/tests.rs` (30), `resources/screen/mod.rs` (28) and `desktop/pages.rs` (18).
Any proposal needs to address the shell's real feature wiring rather than just
move infrequently edited helpers.

## CI observations

These are historical observations, not controlled comparisons. The workflow uses
`macos-15`; local Intel/two-job timings must not be compared directly with it.

| Run | Check job | Cache restore | Clippy | Tests | Stress checks |
| --- | ---: | ---: | ---: | ---: | ---: |
| [37286108961](https://github.com/skel84/freshkube/actions/runs/37286108961) | 315 s | 35 s | 42 s | 149 s | 57 s |
| [37286076745](https://github.com/skel84/freshkube/actions/runs/37286076745) | 318 s | 28 s | 41 s | 137 s | 83 s |
| [37284339580](https://github.com/skel84/freshkube/actions/runs/37284339580) | 355 s | 30 s | 50 s | 158 s | 91 s |
| [37283197388](https://github.com/skel84/freshkube/actions/runs/37283197388) | 318 s | 32 s | 36 s | 139 s | 82 s |

Runs 37286108961 and 37284339580 checked the same head, `fd299bd`, yet differed by
40 seconds. Single-run improvements of this size need repeat measurements.

The workflow already skips draft PRs and docs-only changes, runs formatting and
style before compilation, and does not add a redundant ordinary `cargo build`
after `cargo test`. It checks the stress feature separately because enabling it
for ordinary UI tests changes application lifetime.

### Compilation versus test execution

The logs print eight unit-test executables (the app and seven libraries), with
no standalone integration-test files in this checkout. The separate stress
command adds a ninth executable. Doctests are additional rustdoc work, not eight
more persistent Cargo test targets.

| Run | Cargo test compilation + linking | Share of test step | Desktop suite | All unit suites |
| --- | ---: | ---: | ---: | ---: |
| 37286108961 | 104 s | 70% | 37.72 s | 40.00 s |
| 37286076745 | 92 s | 67% | 37.86 s | 40.15 s |
| 37284339580 | 110 s | 70% | 40.77 s | 43.11 s |
| 37283197388 | 95 s | 68% | 35.17 s | 37.75 s |

These compile times are rounded to whole seconds by Cargo; suite durations are
the harness's reported elapsed times. Remaining step time includes starting the
command/binaries and doctests. The first run lists only workspace packages as
`Compiling` in this step: the 104 seconds primarily cover workspace compilation
and linking, with Cargo overhead. CI did not capture per-link timing or a compiler
profile, so attributing a numerical share to LLVM versus the linker would be
unsupported. Local link measurements below must not be projected onto a different
CI architecture as if measured there.

The toolchain action sets `CARGO_INCREMENTAL=0`, and the cache action logs
`cache-workspace-crates: false`. At audit time the repository had 51 caches,
51,008,404,591 bytes (47.51 GiB), and the sampled check restored a 1,243,500,212-byte
archive in a 35-second step. Saving PR caches alone does not retain the workspace
artifacts that this policy removes. The action's documented cleanup also removes
incremental artifacts. [rust-cache behavior](https://github.com/Swatinem/rust-cache#cache-details)

There is no integration-test binary explosion to consolidate here. Nextest still
builds test binaries with `cargo test --no-run`; it could improve scheduling in the
38–43-second execution portion, but cannot remove the 92–110-second compile/link
phase. Its process-per-test execution would need validation with GPUI's harness
and separate doctest coverage. [nextest execution model](https://nexte.st/docs/design/how-it-works/)

Moving clippy and tests into separate jobs can overlap work, but cannot make
clippy's metadata a substitute for linked test objects. It also adds another
runner/cache setup. Treat that as a latency-versus-runner-cost experiment, not a
way to share a single compilation; they already share the same `target/` in one
job.

The stress step's own logs split into 20.87–32.21 seconds of clippy and
35.79–60 seconds of test compilation; its two tests run in 0.01 seconds in the
sampled run. Moving both lint commands to a concurrent lint job could, in an
ideal model with identical setup and cache behavior, remove 63–80 seconds from
the test job's critical path (about 20–23% of these complete checks). This is a
modeled ceiling, not a measured improvement. It increases runner work and cache
traffic; preserve all checks, keep ordinary tests free of the stress feature,
and keep an aggregate required-check result. Repeated workflow runs are needed
to establish the actual benefit.

## Feature work today

| Feature | Implementation and central wiring | Recompile consequence |
| --- | --- | --- |
| An ordinary inspection page | `screens/<feature>/`, declarations/re-exports in `screens/mod.rs`, `Page`/`ScreenKind` and their lists/mappings in `desktop/pages.rs`, factory in `desktop/mod.rs`, navigation/actions where needed, tests and core collection | Desktop for presentation; core and its dependants if domain collection changes |
| A built-in Resources kind | `core/resources/kinds.rs`'s 47-entry catalog and `desktop/resources/navigation.rs`; fixture/test data as needed | Core → logs → desktop → app; no new page or screen factory |
| A discovered custom kind | Existing API discovery and server-printed table path | No source change for ordinary discovered kinds |
| A Resources column | Server-supplied ordinary columns need no registration; a derived column touches `resources/screen/layout.rs`, `source.rs`/`cells.rs`, and possibly `model.rs`/`rows.rs`/`projection.rs` | Desktop, or core too if new collection is required; no page enum edit |
| A diagnostic check | `core/diagnostic_runner.rs` collects evidence and builds typed checks; existing diagnostic presentation consumes the result; tests stay with the owner | Focused core tests avoid desktop; a complete app build still invalidates desktop |

Not every feature needs a new page or a new crate. Preserve the existing
data-driven Resources path. A registration table could reduce repeated page
mappings, but would not itself reduce the desktop compilation unit.

Monitoring's 9,812 lines comprise page 4,067, panel 2,620, derivation 1,857, history
708, colors 299, store 251 and the 10-line module. Its Resources dependency is
concrete: page/history imports `KubeAccess`/`KubeSource`, while the Resources pane
embeds `HistoryView`. A split needs an access seam that preserves connection
identity and request ownership. Observability's 10,383 lines already emit typed
navigation events; remaining seams include `KubeSource`, backend request helpers,
shared chart colors, content width, secrets and snapshot state. These are better starting boundaries
than a crate per page. The two modules contain 96 test attributes (61 monitoring,
35 observability); moving their tests can make their focused commands independent
of the rest of desktop, even if a full app rebuild remains expensive.

### Structural proposals (parked)

The ranking below is by boundary readiness and usefulness for focused agent work;
none is a measured full-app speedup. The lead is holding #53 step 3 until
Monitoring's page migration lands; these are proposals only. File splitting within
desktop does not create an independently compilable target.

| Priority | Boundary | Code and tests | Work required / expected benefit |
| --- | --- | --- | --- |
| 1 | Stage the planned Monitoring crate with `panel/`, `derive/`, and `colors.rs` | 4,776 lines: 2,620 + 1,857 + 299; 35 test attributes, including 8 GPUI tests; paths touched in 11 recent commits | These modules have no Resources access, secrets, backend or page/history dependency. Retarget UI and probe/perf aliases, export a small typed panel/event API, and retain test instrumentation. Chart edits gain an isolated test target without first resolving the page's access cycle. |
| 2 | Observability page and its owned requests | 10,383 lines, 35 test attributes | Preserve typed navigation events. Its `set_source` only reads `KubeSource.id`, so pass that existing identity rather than the whole Resources access object. Share request cancellation/snapshot and secret-store contracts without moving ownership away from the page. Move the tests with their owner. |
| 3 | Finish Monitoring page/history/store extraction | Remaining 5,026 lines after step 1; 26 further test attributes | Resolve the real `KubeAccess` client/reconnect seam, Talos target input and secret-store contracts. Keep page/history cancellation and existing entities. This completes the 9,812-line capability boundary. |

The first step can use `freshkube-monitoring` from the outset; it does not require
an extra permanent crate for every chart helper. Before Observability becomes a
separate crate, place the chart palette it shares at a stable lower boundary so
unrelated Monitoring page edits do not also invalidate Observability. Keep model
conversions out of `freshkube-ui`, whose independence from core is intentional.

Panel probes currently include `cfg(test)` calls through `desktop::probe`.
Moving the files blindly would remove those probes when desktop tests link the
new crate as a dependency. Follow the existing probe counting/testing feature
pattern and compare the test list and render/stress results. This is a small
seam-first extraction, not a promised move-only PR.

Keep the shell (16,930 lines) and Resources browser (18,321) together for now.
They are the highest-churn areas but also hold navigation, access, selection and
user-started session wiring. A crate per page or a new `dyn` layer would increase
the interface work without evidence of a faster edit loop. Core's Coroot boundary
is only 1,935 lines and still maps objects through Resources kinds; extracting it
is lower priority than isolating the desktop features above.

`ScreenHandle` already erases views into `AnyView` and lifecycle closures.
`TableSource` is `Sized`, has associated row/key/column types and takes
`Context<Self>`; converting it into `dyn TableSource` is not a mechanical change.
`LogView<S>` similarly instantiates for its sources in desktop. If profiling
identifies these as expensive, first consider moving non-generic body work behind
small typed adapters, retaining domain keys and existing entities. Do not trade
away list virtualization, caching or identity for an unmeasured compile saving.

Do not infer an exponentially nested widget type from a long builder chain:
pinned GPUI's `ParentElement::child` converts its argument to `AnyElement` before
storing it (`gpui-pre` 0.3.7, `src/element.rs`). Entity/listener/source
instantiations still deserve measurement, but generic syntax and source lines
alone do not establish their compile cost.

## Focused follow-up: avoid duplicate desktop variants

The second-opinion review with buildtalk prioritizes the local loop before another
crate split. Desktop enables GPUI test support and component testing features only
through dev-dependencies. An ordinary library, a library with test features,
a test harness and a clippy metadata pass are potentially distinct compilation
work. Desktop currently has no fenced library doctests; the only fences in its
Rust files are shell examples in the stress binary.

The proposed isolated command matrix and A/B/A/B whole-loop study were cancelled
when the user chose to ship the three small changes. Use
`cargo test -p <crate> --lib <filter>` during iteration and package clippy once at
the end, while retaining workspace test/clippy/fmt/style before the PR. Desktop's
`doctest = false` does not remove the ordinary desktop library needed by the app's
workspace test. One main/branch narrow-loop sample is observational validation,
not a controlled statistical estimate of the full-loop saving.

## Dependency and profile candidates

The manifests deliberately keep GPUI test support in dev-dependencies. Tests
enable `gpui-kit/test-support`, probe counting, log/terminal test readers and
core's Tokio test utilities. Resolver 2 keeps these out of ordinary builds. A
first test build therefore has additional feature variants to compile; a second
ordinary build should reuse its own variants. This distinction needs fingerprint
evidence before calling it cache thrashing. [Cargo feature resolution](https://doc.rust-lang.org/cargo/reference/features.html#feature-resolver-version-2)

`talos-rs/build.rs` writes generated protobufs into `OUT_DIR` and declares
`proto/`, `PROTOC` and `PROTOC_INCLUDE` as rerun inputs. Source inspection shows no
timestamp or generated file written into the source tree. Repeated fingerprint
logs will test whether this correctly remains fresh after unrelated edits.

One cold-build hypothesis is redundant TLS provider compilation. The direct
`tokio-rustls = "0.26"` dependency enables its AWS-LC default; workspace Rustls
also enables ring, which `core/resources/connection.rs` and
`cluster_overview.rs` explicitly install. This is not yet proof that AWS-LC can
be removed: inspect all activating feature paths and TLS consumers, measure its
critical-path contribution, and validate connections before a separate change.
Similarly, the lockfile's two resvg versions come from GPUI and Component;
resolving that duplication is an upstream compatibility task, not a blind lockfile
edit.

Codegen-unit, incremental and alternate-backend experiments must preserve the
required optimized dependency profile. Changing code generation can affect
runtime performance and needs before/after stress measurements before adoption.
Debug line tables trade variable/type inspection for source-line backtraces;
`debug = 0` also loses that line information. Explicit `split-debuginfo =
"unpacked"` already matches the macOS commands emitted by this checkout. [Rust codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html)

## Agreed action order and parked options

1. Ship focused inner-loop guidance and desktop `doctest = false`, retaining all
   required whole-workspace checks before a PR.
2. Match CI with workspace `[profile.dev] debug = "line-tables-only"`. Keep
   dependency `opt-level = 3` and incremental compilation. Source-line backtraces
   remain; variable/type inspection is reduced.
3. Revisit only if a real loop shows the need: linker changes, nightly
   codegen/profiling, nextest, more disk investigation and structural splits.
   None has a measured local improvement in this record.

### Scoped compiler cache adopted after the research batch

The user approved sccache for the next real worktree builds without another
synthetic cold-build experiment. Its upstream 0.18.0 Intel macOS binary is stored
under `~/.cache/freshkube/tools/sccache-v0.18.0/`; the release archive SHA-256 was
verified against GitHub's release metadata. The wrappers are in parent-directory
Cargo configs at `~/.herdr/worktrees/freshkube/.cargo/config.toml` and
`~/code/kube-gui/.cargo/config.toml`, each using the binary's absolute path. There
is no repository/CI change and no wrapper in `~/.cargo/config.toml`.

The native sccache config at
`~/Library/Application Support/Mozilla.sccache/config` sets a 25 GiB disk limit;
`--show-stats` confirms that limit. The cache starts empty. Record subsequent real
worktree cache stats and load with their build times. The 52-minute observation
above remains noisy context, not a controlled speedup baseline.

The requested Homebrew install first attempted to upgrade Rust from source and
unlinked the existing tools. That installer was stopped; the existing Homebrew
Rust 1.98.1 links were restored and verified. No new Cellar package was installed.
The upstream binary avoids that toolchain change.

Local incremental workspace units are not sccache candidates. New worktrees may
reuse eligible dependency compiles; CI's existing incremental-off mode may reuse
unchanged workspace libraries. Changed libraries and linked test executables
must be measured separately. The 0.18.0 parser accepts metadata-only library
compilation but excludes incremental invocations and system-linker crate types.
[sccache Rust parser](https://github.com/mozilla/sccache/blob/v0.18.0/src/compiler/rust.rs)

Its GitHub Actions backend exists, but transfer/setup overhead and actual hits
would need measurement. No remote backend or CI cache was modified here.
[sccache GitHub Actions backend](https://github.com/mozilla/sccache/blob/v0.18.0/docs/GHA.md)

Keep writable targets separate. A read-only dependency seed needs a verified
compiler/profile/target/feature key and must exclude workspace artifacts and stale
build-script paths. Copying a complete target does not solve the stale-workspace
failure documented in `AGENTS.md`.

Nightly, self-profile, share-generics and Cranelift remain unmeasured. The earlier
conditional installation plan is parked with the broader batch. Upstream lists
panic unwinding as unsupported on macOS, so a faster build would not establish a
usable test loop. [Cranelift support](https://github.com/rust-lang/rustc_codegen_cranelift#not-yet-supported)

## Technical references

- [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html):
  workspace overrides, debug information, incremental compilation and macOS
  unpacked debug information.
- [Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html): the
  meaning and limits of per-unit build timing reports.
- [GPUI Kit coding guides](https://gpui-kit.com/docs/coding-guides.md): capability
  boundaries, acyclic dependencies and preserved state ownership.
