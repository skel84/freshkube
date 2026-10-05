# Build and change speed

Research for [#53](https://github.com/skel84/freshkube/issues/53), measured from
`e27a83d24acbf5ab3656c12b2e3fdfdbf25625ae` on 5 October 2026. This is a research
draft: no application, dependency, profile or CI changes are proposed by this PR.

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

Local timings and the final ranking will be filled in as the approved build
sessions finish. Missing measurements below are not estimates of a gain.

## Measurement conditions

The machine is an Intel Core i7-9750H (12 logical CPUs), 16 GiB RAM, with
Homebrew Rust 1.98.1 / LLVM 22.1.8, Apple linker 1115.7.3 and Homebrew LLD 23.1.2.
There was no `target/` before the first build. Registry sources were already
present, so this is an empty-artifact build, not a cold download experiment.

Every Cargo command uses this worktree's own `target/`, with `CARGO_TARGET_DIR`
unset and `CARGO_BUILD_JOBS=2`. The lead grants build slots. Timed commands start
only below a one-minute load of 10. Profile comparisons need repeated alternating
runs in one session; machine load and warm-up runs are recorded separately.

Nightly, sccache, nextest, cargo-llvm-lines, cargo-bloat and sold are not installed.
No tool has been installed for this research. Stable Cargo's timing report
separates compilation units; it does not prove a split between type checking,
monomorphization and LLVM inside one unit.

| Baseline | Result |
| --- | --- |
| Empty-artifact `cargo build --locked --timings` | Running; start load 6.91, no other Cargo/rustc process at start |
| Desktop one-line edit → build / test compile / clippy | Pending |
| Core one-line edit → build / package test compile | Pending |
| UI one-line edit → build / package test compile | Pending |
| Workspace test binaries and link cost | Pending |
| Build + test + clippy artifact size | Pending |

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

## Candidates to rank after measurement

1. **Configuration:** compare workspace line tables with full debug info, while
   preserving dependency `opt-level = 3` and workspace incremental compilation.
2. **CI:** separate test compilation from execution in timing evidence; examine
   feature-specific cache reuse and the cost of the stress checks before changing
   the cache policy or runner.
3. **Structure:** evaluate monitoring and observability as focused-test boundaries
   (20,195 lines combined), with explicit access/request/event seams and retained
   lifecycle owners. Do not promise faster complete app rebuilds from a move.
4. **Generic code:** inspect concrete instantiations before replacing typed GPUI
   APIs with dynamic dispatch. The existing shared table and log view are generic;
   their bodies may still be compiled in the consuming crate.
5. **Tools:** measure the already-installed Apple and LLVM linkers; keep sccache,
   nextest, nightly profiling and alternative codegen as unmeasured follow-ups
   unless the lead authorizes installation and the required runtime checks.

## Technical references

- [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html):
  workspace overrides, debug information, incremental compilation and macOS
  unpacked debug information.
- [Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html): the
  meaning and limits of per-unit build timing reports.
- [GPUI Kit coding guides](https://gpui-kit.com/docs/coding-guides.md): capability
  boundaries, acyclic dependencies and preserved state ownership.
