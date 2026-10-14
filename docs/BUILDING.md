# Building in a worktree

Why each worktree keeps its own `target/`, and how to free disk space, moved from AGENTS.md's [Build, run and test](../AGENTS.md#build-run-and-test) unchanged (#587). AGENTS.md keeps the rules.

## One target directory per worktree

Each worktree builds into its own `target/`; leave `CARGO_TARGET_DIR` unset. Don't share one build directory between worktrees: Cargo fingerprints workspace crates by paths relative to the crate and judges freshness by modification time, so a worktree whose sources are older than another worktree's build links that other branch's code without rebuilding it. The first build in a new worktree compiles the dependencies.

## When the disk runs low

On macOS a dev build keeps its object files in `target/debug/deps`, since the debugger and backtraces read line tables from them. Each incremental rebuild of a workspace crate writes a new set, one `.o` per codegen unit, and leaves the old ones, so a busy worktree's `target/` grows by gigabytes a day: after a day of work one held about 54 GiB of `freshkube_desktop` objects and 11 GiB of `freshkube_core`'s. When the disk runs low, remove those two crates' objects:

```sh
find target/debug/deps -type f \( -name 'freshkube_desktop-*.o' -o -name 'freshkube_core-*.o' \) -delete
```

Cargo doesn't track the objects, so this rebuilds nothing, and the next build writes what it needs. Binaries already linked keep working, but their backtraces lose file and line numbers until they are rebuilt. It isn't `cargo clean`: dependencies, `.rlib`s and incremental state stay. `rm -rf target/*/incremental` frees a few more gigabytes, at the cost of a slower next rebuild.
