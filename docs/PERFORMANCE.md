# Performance

How Freshkube behaves under load, how to measure it, and what the numbers showed. [LONG_LISTS.md](LONG_LISTS.md) has the rules for long lists; this page has the evidence.

## Targets

- With 20,000 rows, a filter keystroke or a watch batch costs the main thread under about 16 ms, one frame at 60 Hz.
- A log writing 10,000 lines a second keeps up: the newest line on screen stays a bounded time behind the stream.
- Memory stays flat under a sustained load.

## Measuring

The `stress` binary runs the real app in Kubernetes-only mode against a synthetic Kubernetes API on 127.0.0.1. Lists, watches and pod logs go through the real client, decoding and batching; no cluster or credentials are involved. Talos logs use the example data with a faster writer.

```sh
scripts/stress.sh <label> <scenario> [args...]   # builds in release, keeps target/stress/<label>.log
scripts/stress.sh table-20k table 20000
scripts/stress.sh burst-2k burst 20000 2000
scripts/stress.sh pod-logs-10k pod-logs 10000
scripts/stress.sh talos-logs-10k talos-logs 10000
scripts/stress.sh terminal-50k terminal 50000
```

| Scenario | What it does |
| --- | --- |
| `table <pods>` | lists that many pods, then types three filters and clears each |
| `burst <pods> <changes/s>` | lists the pods, then the watch changes them at that rate: one change in ten deletes a pod and adds another, the rest flip a pod between Running and CrashLoopBackOff |
| `summary` | opens Health with 20,000 Pods, 2,000 Deployments and 5,000 warning Events, then leaves the collections quiet |
| `summary-burst <changes/s>` | the summary workload with sustained Pod changes from the same writer as the Resources watch |
| `summary-410 <changes/s>` | the same workload, with one forced Pod watch expiration and relist after 10 s |
| `pod-logs <lines/s>` | opens a pod's Logs tab while its container writes at that rate; every 50th line carries 300 more characters |
| `talos-logs <lines/s>` | example Talos logs, the collected services writing that many lines a second between them |
| `terminal <lines/s>` | a window with only the terminal view, fed coloured lines at that rate from another thread, every 10 ms; each line is new text |
| `terminal-top` | the terminal, redrawn whole on the alternate screen by a `top`-like stream about 60 times a second |
| `terminal-sample` | the terminal showing its colours, styles and wide characters, for visual checks (`FRESHKUBE_STRESS_APPEARANCE=light` or `dark`) |
| `monitoring <dashboard.json> [processes]` | opens that dashboard from a folder of its own against a fake Prometheus behind the service proxy, every query answering one series per Go process (67), five for a GC duration summary, a quarter of them ending early; then sweeps the mouse over the top panels, scrolls and hovers again. The run keeps its own home, so Monitoring reads the folder and the remembered Service from there, and `scripts/stress.sh` fails it when it records no `monitoring.*` span after the warm-up, unless its keys only wait and it drew during the warm-up |

A run lasts `FRESHKUBE_STRESS_SECONDS` (30) and leaves the first `FRESHKUBE_STRESS_WARMUP` (5) seconds out of its summary. GPUI stops drawing a covered window or one on a locked screen, so keep the window in front; the script refuses to run on a locked screen.

The `stress` feature turns on spans around the work that matters (`freshkube_probe::perf`, which desktop reaches as `crate::perf`); without it they compile to nothing. Each second the run prints, for every span, its count, median, 99th percentile, maximum and total in milliseconds, then the process's CPU share and resident memory:

| Name | What it times |
| --- | --- |
| `main.stall` | how late a 4 ms timer on the main thread fires: time the thread spent on something else. Its total is roughly how long the thread was busy |
| `table.apply` | one watch batch on the Resources page: `table.store` (the store), `table.rebuild` (filter and sort) and, after a list, `table.layout` (column widths) |
| `table.batch` | events in a watch batch |
| `table.render`, `logs.render` | building the view's element tree; GPUI's layout and paint come after and show only in `main.stall` |
| `logs.apply`, `logs.batch` | appending a batch of log lines, and its size |
| `logs.lag` | how far behind its due time the newest line of a batch is when it is appended |
| `logs.measure`, `logs.measure_row` | laying out log rows for their heights, per frame and per row |
| `logs.chrome` | laying out the log toolbar and notices |
| `terminal.feed`, `terminal.bytes` | parsing a chunk of bytes into the terminal grid, and its size |
| `terminal.snapshot` | copying the visible rows into style runs, at most once per batch of chunks |
| `terminal.paint` | painting the grid; its count a second is the frame rate |
| `monitoring.page_render`, `monitoring.panel_render`, `monitoring.plot_paint`, `monitoring.cursor` | building the Monitoring page's tree, building a dashboard panel's tree, painting a timeseries' paths, and placing the crosshairs one chart's cursor puts on the others |
| `summary.tokio`, `summary.apply` | deriving compact reflector evidence on Tokio, then applying prepared display data on GPUI; before the watch migration, `summary.tokio` also included API collection |
| `summary.lag` | first dirty-store notification to GPUI apply, including the 500 ms debounce; excludes network and producer backlog |
| `summary.live_sources` | synchronized and current summary collections, from zero through nine; the first nine marks completed initial synchronization |
| `summary.retained_mib`, `summary.staging_peak_mib`, `summary.bytes_per_pod` | retained payload and maximum staging payload in MiB, and mean retained Pod payload bytes; excludes map/Arc/allocator overhead |

Summary request counters are cumulative per API path, operation and representation.
Use the last (or maximum) value for each key, not the sum of repeated lines. An
initial list can have many pages; a watch opened after the list is a separate
request. `FRESHKUBE_STRESS_BINARY=/absolute/path/to/saved/stress` runs an immutable
release binary without rebuilding it, so polling and watching can be compared
against the same synthetic API. The API has one world writer, stable paginated
snapshots, at most eight active snapshots and a 100,000-event replay ring shared
by typed and Table watches. A replay gap produces a 410 rather than missing
deletes silently. Process memory includes that API and its replay ring.

When local compilation is contended, a manual CI run can build the native Intel
benchmark executable: `gh workflow run ci.yml --ref <branch> -f native_stress=true`.
Download its `freshkube-stress-x86_64-apple-darwin` artifact, restore executable
permission with `chmod +x`, and select it with `FRESHKUBE_STRESS_BINARY`. CI builds
but does not run the GUI benchmark; it still needs the local visible window.
Record the compiler and commit when comparing saved binaries.

The bottom bar's FPS indicator passively samples painted frames during activity;
it never requests a continuous animation. It refreshes its own label at most
once a second: green at 55+, amber at 30–54, red below 30. Gaps of 250 ms or more
and samples with fewer than three frame intervals are neutral idle readings.
This is redraw cadence during bursts, not a GPU throughput benchmark.

When a number looks wrong, profile the run with macOS `sample`:

```sh
target/release/stress burst 20000 1000 2> target/stress/run.log & pid=$!
sleep 10; sample $pid 5 1 -file target/stress/run.sample; wait $pid
```

## Baseline

Release builds on an Intel Core i7-9750H with 16 GB, macOS 15.2, at the harness commit. Times are in milliseconds. The machine is noisy: the same run can differ by half between attempts, so compare medians across several runs before trusting a small change.

### Lists and filtering

| Pods | First list: whole batch (store, column widths) | Filter keystroke: rebuild median / max | Memory at the end |
| --- | --- | --- | --- |
| 5,000 | 18 (15, 3) | 0.3 / 0.6 | 77 MB |
| 20,000 | 107 (98, 8) | 1.2 / 4.0 | 91 MB |
| 50,000 | 257 (237, 15) | 2.8 / 7.2 | 180 MB |

A filter keystroke is well inside a frame even at 50,000 rows. The first list is not: the store builds each row's search key and its identity index on the main thread, about 5 µs a row, and the window stops for a tenth of a second at 20,000 pods.

### Watch bursts on 20,000 pods

| Changes a second | Batches a second | Batch: median / 99th / max | Rebuild median | Main thread busy | CPU |
| --- | --- | --- | --- | --- | --- |
| 1,000 | 51 | 7.0 / 13.1 / 136 | 6.7 | 25 of 30 s | 111% |
| 2,000 | 50 | 4.1 / 11.5 / 18.6 | 3.7 | 18 of 30 s | 95% |
| 5,000 | 50 | 4.1 / 10.6 / 13.4 | 3.2 | 16 of 30 s | 98% |

Each batch fits in a frame, but core sends one every 16 ms while a burst lasts, and each one re-sorts all 20,000 rows and redraws the window. A profile of the 1,000 a second run put 59% of the main thread in drawing the window (about 50 frames a second at about 7 ms each), 29% in applying batches, nearly all of it the sort, and left it idle 7% of the time. The number of changes hardly matters; the number of batches does.

### Log floods

| Workload | Lines appended a second | Behind after 30 s | Render median | CPU |
| --- | --- | --- | --- | --- |
| Pod log, 10,000 a second | 1,970 | 26 s | 18.6 | 101% |
| Pod log, 50,000 a second | 2,020 | 31 s | 18.2 | 102% |
| Talos logs, 10,000 a second | 1,720 | 30 s | 21.9 | 102% |

No log keeps up: the lag grows by the second for as long as the stream runs. A 60-second pod-log run split the render: measuring new rows took 21 of its 23 ms, at 0.27 ms of GPUI layout per row and 64 rows per batch. That caps any log at about 3,000 lines a second, whatever the rest costs. Two more limits wait behind it:

- Both log sources hand the view at most 64 lines per turn, about 4,000 a second.
- Once the buffer evicts, every batch rebuilds a set of every retained line to drop dead measurements (`logs.sweep`, 0.6 ms now and growing with retention).

Memory stayed flat in every run: resident memory held between 90 and 115 MB through a 60-second flood that filled the 8 MiB log buffer.

### Terminal floods

The terminal view (`terminal/`, step 1 of [pod exec](POD_EXEC.md)) in a 1150 × 790 window at the default text size, about 155 × 44 cells. Release build, 30 s runs with the first 5 left out.

| Workload | Frames/s | Fed | Paint: median / 99th / max | Snapshot median | Feed median | CPU | Memory |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `terminal 10000` | 60 | 1.1 MB/s | 6.0 / 14.1 / 70.5 | 0.57 | 0.24 | 63% | 91 MB |
| `terminal 50000` | 60 | 5.3 MB/s | 5.8 / 11.1 / 17.6 | 0.48 | 1.31 | 69% | 91 MB |
| `terminal-top` | 50, the source's rate | 0.2 MB/s | 5.0 / 10.0 / 13.4 | 0.47 | 0.11 | 46% | 50 MB |

Every workload keeps the frame rate its source allows, with paint well inside a frame. Memory holds at the 10,000-line scrollback. A shell at rest costs nothing: the view rebuilds and redraws only when bytes arrive.

Against the spike's 60 frames a second at 5 MB/s and about 2 ms of paint:

- **The spike's flood repeated itself.** It sent the same numbered lines every tick, so GPUI's line cache shaped almost nothing. Fed those repeating lines, the view paints in 1.2 ms median and 2.1 ms at the 99th percentile, at 40% CPU. New text each frame, as real output brings, costs about 5 ms more for shaping.
- **Ligatures doubled the cost of shaping.** JetBrains Mono's contextual alternates were most of the paint in a profile of `terminal-top` (9.8 ms median, 75% CPU). The terminal draws without them, as the user chose, which halved both.
- Shaping each row once, rather than each style run, could cut the remaining cost. It isn't needed for the frame rate, so it waits until a real session shows a need.

### Monitoring dashboards

`monitoring` in the stress window (1320 × 860) at the default text size, against 67 Go processes. Release build, 30 s runs with the first 5 left out, four runs of the 30-panel dashboard and two of the built-in Cluster, on 6 October 2026 (#181):

```sh
scripts/stress.sh mon-30 monitoring crates/freshkube-desktop/src/bin/stress/dashboards/thirty.json
scripts/stress.sh mon-cluster monitoring crates/freshkube-core/src/monitoring/builtin/cluster.json
```

The load average was between 8 and 24 with no cargo running, so the absolute numbers are noisy; read the medians, not single values. Times are in milliseconds unless marked; the 30-panel row gives the median of its first three runs, Cluster's the range of its two.

| Dashboard | Main-thread stall: median / 99th / total | `panel_render`: count, median / 99th / total | `plot_paint`: count, median / 99th / total | `cursor` total | CPU | Memory |
| --- | --- | --- | --- | --- | --- | --- |
| 30 timeseries, 67 series each | 81 / 114 / 26.0 s | 9,330, 0.20 / 1.96 / 1.7 s | 4,665, 0.11 / 3.08 / 1.0 s | 0.13 s | 86% | 267 MB |
| Cluster (10 panels) | 58 / 77–84 / 24.2–24.4 s | 3,870–3,960, 0.17–0.18 / 2.7–3.0 / 0.9 s | 1,161–1,188, 0.25–0.28 / 3.3 / 0.4 s | 0.04 s | 86% | 213 MB |

- **Thirty charts saturate the main thread while the mouse moves.** It never rests: the stall totals more than the 25 s measured, about 12 frames a second. The page's own spans account for under 3 s of that. The rest is GPUI laying out and painting the 30 panels, their legends and their readouts, which is where #22 should look first.
- **Ten panels saturate it too.** Cluster's three timeseries of 67 series each, with their legends, keep the thread busy for 24 of the 25 s measured, at about 16 frames a second. Its spans account for about 1.4 s.
- The fake Prometheus answers between 0.01 and 0.95, so Cluster's fixed 0–100% charts draw every line inside the plot. A dashboard that opens on a row of stat cards, as Cluster does, is swept lower (`hover_rows` in the stress binary), so the mouse crosses its charts. The 30-panel dashboard's fourth run, after the values were scaled, gave 78 / 110 / 26.0 s, within the spread of the first three.

## Fixes

Each fix is its own commit, with the workload that showed the problem run again after it.

### Watch bursts apply at most ten times a second

A watch batch that arrives within 100 ms of the last one applied now waits for the rest of that window, and everything that arrived meanwhile is applied with it. A change after a quiet spell still shows at once. The list re-sorts and redraws about 10 times a second during a burst instead of 50, and is never more than 100 ms behind.

| Changes a second, on 20,000 pods | Batches a second, before → after | Batch median / max, after | Main thread busy, before → after | CPU, before → after |
| --- | --- | --- | --- | --- |
| 1,000 | 51 → 9.6 | 2.2 / 8.8 | 25 → 5.0 s | 111% → 23% |
| 2,000 | 50 → 9.6 | 2.6 / 6.3 | 18 → 4.8 s | 95% → 26% |
| 5,000 | 50 → 9.4 | 3.9 / 9.4 | 16 → 5.1 s | 98% → 37% |

### Logs keep up with a flood

A log view now lays out only the new lines a frame can show, as it already did during a resize, and gives the rest the mean height of the rows measured so far. Once lines stop arriving for 150 ms it measures the rest within its 8 ms a frame. Both log sources hand the view up to 1,024 lines a turn instead of 64, and the buffer reports the lines it evicts, so their measurements are dropped without a pass over every retained line.

| Workload | Lines appended a second, before | After | Behind: median / 99th / max, after | Render median, after | CPU, after |
| --- | --- | --- | --- | --- | --- |
| Pod log, 10,000 a second | 1,970 | 10,300 | 6 ms / 10–70 ms / 113 ms | 9 | 100% |
| Pod log, 50,000 a second | 2,020 | 34,900 | 4.0 s / 8.4 s / 8.4 s, growing | 5.7 | 103% |
| Talos logs, 10,000 a second | 1,720 | 10,630 | 7 ms / 42 ms / 83 ms | 11.4 | 45% |

At 10,000 lines a second both logs keep up, a few milliseconds behind. At 50,000 every turn takes the full 1,024 lines and the pod log falls behind by about 0.3 s a second. Memory held between 74 and 116 MB.

The main thread is still busy throughout a flood: frames come about 15 times a second, and a key press can wait up to about 130 ms. A profile of the Talos run put 35% of the main thread in `CAMetalLayer nextDrawable`, waiting for the GPU to hand back a drawable, 23% shaping the text of new rows on screen, about 27% in GPUI's layout and paint, and 5% appending lines. Shaping is the cost of drawing new text at all; the drawable wait is inside GPUI's Metal renderer.

### A first list no longer stops the window

A reset now arrives ready to swap in. Where the list is read, on Tokio, core's rows are converted, their search keys built, duplicates merged, identities indexed and the widest printed text of each column counted. The main thread replaces the store's contents and sizes the columns from those counts instead of reading every row.

| Pods | Whole batch, before | After |
| --- | --- | --- |
| 20,000 | 107 ms | 0.3 ms |
| 50,000 | 257 ms | 1.1 ms |

What is left is the projection's first sort, which is quick for the default order because a list arrives in it. A Refresh still drops the old rows on the main thread.


### A pointer move redraws only the chart under it (#22)

A sample profile of the 30-panel hover put three quarters of the main thread in GPUI laying out and painting panels. A move redrew all 30 panels for two reasons. The page passed the cursor on by notifying every other panel. And the shell drew the page as a cached view: while a cached view renders again, GPUI sets `window.refreshing`, and no cached view inside it may reuse its last frame, so the hovered panel's notify alone, which marks the page dirty, redrew them all. Now the page keeps the other charts' crosshairs and draws them beside the cached panels (`panel::Linked`), and the shell draws the page uncached. A move redraws the panel under the pointer, and the page places its panels and crosshairs and derives nothing. That page render runs with every frame the shell draws while Monitoring shows.

Same settings as the baseline, with the saved #181 binary as before, and the runs alternated. The one-minute load average at each start was 13.1 (before), 13.0 (after), 13.8 (before) and 14.2 (after) for the 30-panel runs, and 11.2 (before) and 16.9 (after) for Cluster, with no cargo running.

| Dashboard | Main-thread stall: median / 99th / total | Stall samples | `panel_render`: count, total | `plot_paint`: count, total | `page_render` total | Memory |
| --- | --- | --- | --- | --- | --- | --- |
| 30 timeseries, before | 73–75 / 93–97 / 25.7–25.8 s | 342–349 | 10,290–10,470, 1.7–1.8 s | 5,145–5,235, 1.0 s | — | 257–269 MB |
| 30 timeseries, after | 36–37 / 45–56 / 23.5–23.7 s | 626–648 | 9,870–10,221, 0.6–0.7 s | 495–516, 0.2 s | 0.28–0.34 s | 226–227 MB |
| Cluster, before | 58.5 / 72 / 24.5 s | 615 | 3,960, 0.9 s | 1,188, 0.45 s | — | 218 MB |
| Cluster, after | 26.7 / 46 / 21.9 s | 963 | 720, 0.55 s | 720, 0.51 s | 0.32 s | 194 MB |

- **The stall halves and the frame rate about doubles.** Cluster draws one panel a frame: its panel, plot, cursor and page counts are all about 720.
- **The page's own render is about 1.3% of the busy time.** A profile of the after run puts the window's root layout, which now includes the page, at 7.5% of main-thread samples, and the panels' layout at 14%, down from 35%.
- **The thread is still busy, because loading panels animate.** The page asks only the panels in or near the viewport, so about 16 of the 30 stay loading, and Kit's skeleton pulses with an animation that notifies its panel every frame, off screen too. Those are the 9,700 renders of 0.02 ms, and while any panel is unanswered the window never stops drawing. Drawing only the slots near the viewport, #22's next step, takes them out.

### A dashboard draws only the panels near the view (#22)

After the change above, the 30-panel run still rendered about 16 panels a frame. The page asks only the panels in or near the viewport, so the rest stayed loading, and Kit's skeleton pulses with an animation that notifies its panel every frame, off screen too. With a dashboard open, the window never stopped drawing. Now the page draws the same panels it asks (`Viewport::reach`: what is in view, half a screen above and a screen below) and an empty card at each other slot's fixed place. GPUI redraws a window only for the views it drew in the last frame, so a panel that isn't drawn can't make it draw.

Release build, same settings, with the build of the change above as before; runs alternated, with no cargo, rustc or clang running. The one-minute load average at each start is in the table; WindowServer was the busiest other process, at about 40%.

| 30 timeseries, hover and scroll | Load | Main-thread stall: median / 99th | Stall samples | `panel_render`: count, total | `page_render` count | CPU |
| --- | --- | --- | --- | --- | --- | --- |
| Before 1 | 31.1 | 43.6 / 66.1 ms | 554 | 8,759, 604 ms | 554 | 85% |
| After 1 | 21.4 | 29.7 / 46.7 ms | 1,070 | 557, 361 ms | 555 | 83% |
| Before 2 | 22.9 | 36.2 / 45.7 ms | 643 | 10,154, 696 ms | 643 | 85% |
| After 2 | 18.7 | 25.8 / 49.8 ms | 1,066 | 544, 419 ms | 543 | 82% |

The idle runs open the same dashboard and touch nothing (`FRESHKUBE_STRESS_KEYS=wait:30000`). The frame rate is `page_render`'s count each second after the first five, since the shell draws the page with every frame.

| 30 timeseries, idle | Load | Frames a second | CPU | Main-thread stall median |
| --- | --- | --- | --- | --- |
| Before 1 | 21.4 | 24–31 | 84% | 32.6 ms |
| After 1 | 15.4 | 0 after the 4th second | 5% | 1.2 ms |
| Before 2 | 18.7 | 26–31 | 83% | 32.8 ms |
| After 2 | 15.2 | 0 after the 5th second | 5% | 1.2 ms |

- **Idle, the window stops drawing** once the panels in reach have answered. Before, it drew for as long as the dashboard showed, at most of a core.
- **Moving, a frame renders one panel instead of about 16.** The main thread is free twice as often, but CPU and frame rate hardly change: the hover script moves every 16 ms, and each frame's cost is now the window's own layout and drawing.
- `scripts/stress.sh` used to fail such a run with "the dashboard never drew", since its summary leaves out the warm-up and nothing drew after it. When the keys only wait and the per-second lines show the dashboard drawing, it now reports "monitoring: 0 frames after warmup" and passes; a run that never drew, or one whose keys do more than wait, still fails (`scripts/stress.test.sh`).

### A pointer move redraws no panel (#22)

After the change above, a move still drew the panel under the pointer: its card, header, legend and plot. The crosshair and readout were drawn inside the panel, so the panel had to render again to move them. Now they are a view of their own, `panel::CursorOverlay`, drawn beside the cached panel by the page, the history charts and Observability's application charts. The panel keeps the pointer's handlers and tells the overlay when the cursor, the focus or the marker under the pointer change. A sample profile of the 30-panel hover had put a third of the main thread under `PanelView`, but most of a frame was GPUI laying out and drawing the whole window, and a fifth was the Metal renderer copying path vertices into a fresh vector each frame.

Release build, same settings, with the build of the change above as before; runs alternated, with no cargo, rustc or clang running. SSMenuAgent (macOS Remote Management) used 114–210% of a core throughout; the one-minute load average at each start is in the table.

| 30 timeseries, hover and scroll | Load | Frames | `panel_render` | `plot_paint` | Stall median, moving seconds | Stall samples | CPU |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Before 1 | 12.2 | 557 | 557 | 557 | 37.1 ms | 1,124 | 83% |
| After 1 | 8.6 | 621 | 0 | 0 | 32.3 ms | 1,197 | 80% |
| Before 2 | 12.0 | 557 | 557 | 557 | 37.3 ms | 1,134 | 83% |
| After 2 | 9.8 | 622 | 0 | 0 | 32.6 ms | 1,216 | 81% |

- **A move draws no panel, and the window draws about 11% more frames.** Frames are `page_render`'s count, since the page draws with every frame.
- **The stall is read only in the seconds the pointer moves.** In each second the median is either about 1.2 ms, when the hover script paused, or 30–40 ms while it moved, so the median over a whole run swings with the mix: 4.9 ms before and 23 ms after here. Over the moving seconds alone it drops by about 13%.
- The rest of the frame is GPUI's layout and paint of the whole window, which the page can't avoid while it draws every frame the pointer moves.

### A chart draws no more samples than its pixels can show (#22)

Thinning a chart's paths, so that each pixel column draws only its first, lowest, highest and last sample, keeps the peaks while it cuts vertices, but only where a column holds more than four samples. Every dashboard timeseries asks Prometheus for 200 samples per series (`monitoring/page/board.rs`), and the pod and node history charts ask for 120. A temporary page test measured the plot each panel draws, by grid width, at the default page width (1,048 pt: a 1,320 pt window less the rail and column) and at the narrow one (488 pt):

| Timeseries plot | Default page | Narrow page |
| --- | --- | --- |
| Grid width 3 | 3.36 samples per px (60 px) | 1.28 |
| Grid width 4 | 1.97 | 1.28 |
| Grid width 6 | 1.08 | 1.28 |
| Grid width 8 | 0.74 | 1.28 |
| Grid width 12 | 0.46 | 0.53 |
| Grid width 24 | 0.21 | 0.53 |
| Cluster: CPU and memory by node | 0.47 | 0.54 |
| Cluster: API server latency | 0.75 | 1.29 |
| Pod and node history, 120 samples | about 0.84, from a capture | |

The test's text is wider than the app's, so its value axis takes more room and these plots are a little narrower than on screen: the real ratios are a little lower. No panel reaches four samples per pixel, so thinning would draw every sample it draws today, and it isn't built. Lowering the sample count is no substitute, since Prometheus evaluates only at each step and a coarser step loses the peaks between. Revisit this only if a real dashboard shows plots above four samples per pixel. A 30-panel dashboard's vertices come from the number of series, up to 67 a panel in the stress dashboard, not from oversampling.

### Kubernetes summary

Before the watch migration, the shell read the Kubernetes summary from the API server cache on its 15 s cycle on every page. `scripts/stress.sh summary-20k summary` served 20,000 pods, 2,000 deployments and 5,000 warning events through the real client. Typed objects were discarded on Tokio after deriving the summary and Health data. These historical measurements describe that polling implementation.

Release build, 40 s, with the first 5 s left out; two periodic refreshes, while the macOS packaging worktree was also compiling. The run opened Health. Timings include the synthetic server's JSON generation and client decoding.

| Span | Median / 99th / max |
| --- | --- |
| Collection and derivation on Tokio | 1,337.90 / 1,337.90 / 1,337.90 ms |
| Apply on the main thread | 2.45 / 2.45 / 2.45 ms |
| Main-thread stalls | 0.61 / 1.42 / 12.62 ms |

Process CPU was 4.76% at the median and 106.01% at the 99th percentile; resident memory ended at 397 MB and peaked at 404 MB, including the synthetic server in the same process. With only two refresh samples the percentiles select the larger sample, as the stress reporter does; this is a cost gate, not a latency distribution.

The main-thread apply stayed under its 16 ms budget, so the summary kept its all-page refresh. No visibility fallback was needed. The raw report is `target/stress/summary-20k.log`.


The completed layout was checked again with `FRESHKUBE_STRESS_SECONDS=40 scripts/stress.sh summary-final-20k summary`, after an initial attempt was refused because the screen was locked. The rerun used the unlocked screen, opened Health and had no concurrent build or capture. It includes the joined node rows, card and attention derivation added after the first gate. The first 5 s were excluded, leaving two periodic refresh samples.

| Span | Median / 99th / max, completed layout |
| --- | --- |
| Collection and derivation on Tokio | 724.19 / 724.19 / 724.19 ms |
| Apply on the main thread | 7.01 / 7.01 / 7.01 ms |
| Main-thread stalls | 0.48 / 1.19 / 11.09 ms |

CPU was 4.18% at the median and 87.80% at the 99th percentile and maximum. Resident memory ended at 385 MB and peaked at 430 MB, including the synthetic server. The 7.01 ms apply met the 16 ms budget, so the all-page refresh remained enabled. The same two-sample percentile caveat applies. The raw report is `target/stress/summary-final-20k.log`.

### Watch-backed summary: read-model acceptance, 4 October 2026

The shell now retains compact facts for nine kinds and derives on changes. This
is an intentional all-page read model: Overview, Attention, Health, Nodes and
rail marks need cluster-wide Pod evidence even when Resources is hidden.
Lifecycle shares the Node observation. A visible Resources page still owns its
separate Table representation; this change does not deduplicate those streams.

The comparison below excludes the FPS indicator, which is isolated in
[PR #19](https://github.com/skel84/freshkube/pull/19). Both sides used the same
counted synthetic API, including its single mutation writer and bounded replay.
The machine was an Intel Core i7-9750H Mac with 16 GiB RAM, macOS 15.2, with an
unlocked screen and the stress window visible. Other worktrees were compiling;
the measurements are affected by contention and memory pressure. They are
repeatability checks, not a controlled compiler or CPU benchmark.

| Build | Provenance |
| --- | --- |
| Polling baseline | `5754fda` (documentation over `7aa1045`), exported before production changes, with the counted stress harness; local release, rustc 1.98.1 |
| FPS-free watch comparison | `aeefcdf`, native Intel release artifact from [CI 37157649913](https://github.com/skel84/freshkube/actions/runs/37157649913), rustc 1.99.0 |
| Final typed Health/Lifecycle handoff | `d871826`, native Intel release artifact from [CI 37158341599](https://github.com/skel84/freshkube/actions/runs/37158341599), rustc 1.99.0; separate final verification below |

Each comparison workload ran three times: `summary` for 40 s, `table 20000`
for 60 s, and `burst 20000 2000` for 60 s. The first five seconds are excluded
from reported span and CPU statistics. Table filters started at 30 s, after
the initial list, using `app-1`, `crash` and `ns-4`, clearing between each.
The summary scenario includes 20,000 Pods, 2,000 Deployments and 5,000 warning
Events; table/burst scenarios have the 20,000 Pods and empty Events/workloads.

In these tables, **median** is the median of the three run medians; **max** is
the largest sample across all three runs. RSS is decimal MB for the entire
process, including GPUI, the synthetic API, pagination snapshots and replay.
CPU includes the synthetic API too; 100% is one logical core's time.
End RSS is the median of the three final samples; peak RSS is the largest run
peak. The previously discussed roughly 431 MB figure described polling plus
the synthetic server, not the new reflector payload.

| Workload | Summary apply, median / max ms, before → after | Process CPU median, before → after | End / peak RSS MB, before → after |
| --- | --- | --- | --- |
| Quiet summary | 14.92 / 20.74 → 6.49 / 8.13 | 37.44% → 8.04% | 428 / 505 → 109 / 159 |
| Table with filters | 9.51 / 28.19 → 4.05 / 4.79 | 20.95% → 13.45% | 453 / 608 → 148 / 197 |
| Table plus 2,000 mutations/s | 31.31 / 35.23 → 5.22 / 64.75 | 88.77% → 149.57% | 412 / 571 → 174 / 197 |

| Resources span | Median / max ms, before → after |
| --- | --- |
| Table initial apply | 27.11 / 37.57 → 24.50 / 36.46 |
| Table projection, including filters | 15.27 / 47.71 → 16.58 / 55.04 |
| Burst batch apply | 72.68 / 185.63 → 44.60 / 187.89 |
| Burst projection | 43.93 / 155.25 → 34.03 / 173.55 |

The burst CPU increase is a real limitation of this comparison. The visible
Table applied 301–390 batches after warm-up, versus 76–104 before (about
5.5–7.1 versus 1.4–1.9 updates/s), while the new summary also processed every
Pod change. Lower per-batch medians do not imply lower total work. Summary apply
99th percentiles were 9.31, 18.60 and 32.00 ms, with a 64.75 ms worst sample.
**The roughly 16 ms main-thread target is not met under the combined burst.**
Table projection also misses it on both implementations. Profiling summary
publication together with Table projection/redraw remains performance work;
these results do not justify claiming an overall CPU or frame-time win.

#### Synchronization and request counts

All nine summary sources first became current at 7–8 s in the quiet runs and
5–8 s across table/burst runs. Initial partial publications retained incomplete
coverage. Once synchronized, the quiet summary produced no more publications,
lists or version reads during the rest of each 40 s run. The first five-second
exclusion therefore leaves only initial-sync samples in the quiet apply row,
not a steady-state latency distribution.

The polling summary made three complete nine-kind collection cycles and four
version requests in 40 s: 31 requests. The watch summary made **one initial
paginated list per kind**, nine watch opens and one version request: 60 list
pages plus 10 other requests, or 70 total. Pods accounted for 40 pages,
Events 10 and Deployments four. It makes more initial HTTP requests because
it paginates; the improvement is the absence of repeated full collection reads,
not a lower short-run request count.

On the 60 s table/burst workloads, polling started five summary cycles and
made six version requests. Watching made 48 initial summary list pages, nine
watch opens and one version request. Both sides additionally made the same
40-page Pod Table list, one Table watch and one namespace Table request
(93 total requests before, 100 after). The Node watch was opened once.
Shared-Node HTTP and UI tests separately verify that showing Lifecycle adds
no Node request and selecting a Talos node does not replace the observation.

`summary.tokio` is not comparable as a speedup ratio: before it included API
collection and decoding; after it measures derivation from retained facts.
Under the combined burst its after median was 170.34 ms and worst sample
492.61 ms. Dirty-notification-to-apply lag had a median of run medians of
828.74 ms and a worst sample of 1,381.44 ms. This includes the fixed 500 ms
debounce and excludes network delay and any producer backlog.

#### Memory bounds, relisting and sustained churn

Compact Pod payload averaged **282.51 bytes** in the quiet 20,000-Pod workload,
rising to at most 284.47 bytes during delete/recreate churn. The full summary
retained 12.02 MiB initially and at most 12.06 MiB under churn; the Pods-only
table workload retained 5.47–5.50 MiB. These counters account for owned payload
allocation capacities, but exclude map buckets, Arc allocations and allocator
overhead. They are not process-memory limits.

The implementation enforces 4 KiB per projected Pod, 100,000 objects per kind
and 128 MiB of committed payload per session, with a separate 128 MiB staging
budget during resynchronization. Oversized or over-capacity observations stop
that kind with incomplete coverage; they cannot silently evict a live object
or establish absence. Only summary fields survive projection: no full Pod
spec, environment, annotations, managed fields or container messages. Queues
retain one latest notification/snapshot, rather than a history of snapshots.

| FPS-free recovery/churn run | 40 s forced Pod 410 | 120 s summary churn |
| --- | --- | --- |
| Mutations/s | 2,000 | 2,000 |
| First nine current sources | 7 s | 8 s |
| Summary apply median / 99th / max | 8.92 / 15.06 / 15.06 ms | 9.37 / 13.67 / 16.54 ms |
| Dirty-to-apply lag median / max | 1,179.22 / 1,847.58 ms | 952.77 / 2,203.81 ms |
| Peak staging payload | 10.97 MiB | 10.57 MiB |
| Process CPU median | 60.32% | 62.25% |
| Process end / peak RSS | 166 / 172 MB | 124 / 166 MB |

The 410 run temporarily showed eight current sources at second 10 and then
recovered nine. Only Pods relisted (80 pages and two watch opens total); each
other kind stayed at one list/watch, with one version request for the session.
The 120 s run published 134 times after warm-up, kept nine sources current
after initial sync, and made no additional lists or watch opens. One mutation
in ten deletes and recreates, so 2,000 mutations/s produce 2,200 events/s; this
run outlasts the 100,000-event replay ring's fill time. Retained payload stayed
at 12.05–12.06 MiB. Median RSS was 156.5 MB during seconds 60–89 and 129 MB
during seconds 90–120, with no upward trend after the ring filled. Falling RSS
under host memory pressure is not evidence that every allocation was freed.

**Decision:** retain the bounded, always-on compact Pod read model for these
consumers. The measured 20,000-Pod case fits its payload budget and continues
publishing under sustained churn without accumulating snapshots or relisting
quiet kinds. This accepts the read-model memory tradeoff, not the burst CPU
or 16 ms latency result. Capacity-failure behavior is unit-tested with reduced limits; the
100,000-object ceiling is not validated here as an interactive workload. Real-cluster latency,
pathological object sizes, longer-duration allocator behavior and separate
client/server RSS remain unmeasured. No live cluster was used.

#### Final typed-handle verification

The final `d871826` executable repeated quiet summary, forced 410 and 120 s
churn after the Health/Lifecycle handoff changed to typed entity handles.
Both typed handles point to the same entities used by generic navigation;
there is no second screen or duplicate observation.

| Final-build run | First nine sources | Apply median / 99th / max ms | CPU median | End / peak process RSS |
| --- | --- | --- | --- | --- |
| Quiet, 40 s | 6 s | 6.37 / 6.37 / 6.37 | 8.25% | 120 / 155 MB |
| Forced Pod 410, 40 s | 6 s | 6.71 / 29.72 / 29.72 | 61.05% | 124 / 180 MB |
| Churn, 120 s | 5 s | 8.31 / 13.46 / 16.80 | 62.15% | 134 / 169 MB |

Quiet and churn each made 60 initial list pages, nine watch opens and one
version request, with no later relist. The 410 added only 40 Pod pages and
one Pod watch, restoring coverage after briefly showing eight current sources
at second 10. Final churn delivered 152 publications after warm-up, all with
nine current sources, at 795.64 ms median and 1,462.21 ms maximum dirty-to-apply
lag. Retained payload stayed at 12.05–12.06 MiB; peak staging was 9.45 MiB
(9.89 MiB in the 410 run). Final churn RSS medians were 160.5 MB at seconds
60–89 and 133 MB at seconds 90–120. Recovery and single-page churn can also
exceed the 16 ms target, as their maxima show.

[CI 37158341599](https://github.com/skel84/freshkube/actions/runs/37158341599)
passed all 911 workspace/doc tests, two stress-API tests, strict workspace
Clippy, formatting, source-cleanliness checks and both native macOS bundles.
This includes the fixture UI regressions for initial sync, change without
Refresh, refused kind, 410 staging, context/same-path replacement and the
Talos timer split, plus shared Lifecycle Nodes and delayed Talos completions.

After these measurements, watch was rebased onto #20's merge `bcbdd71`,
preserving its Operations policy, voting-member quorum calculation and ROADMAP
entries. The measured executables above precede that integration; the compact
store and typed-handle behavior did not change. The integrated source is
validated by the checks on [PR #21](https://github.com/skel84/freshkube/pull/21).

Raw reports are retained under `target/stress/`: `watch-before-counted-{1,2,3}`,
`watch-before-table-ready-{1,2,3}`, `watch-before-burst-ready-{1,2,3}`,
`watch-after-summary-{1,2,3}`, `watch-after-table-ready-{1,2,3}`,
`watch-after-burst-ready-{1,2,3}`, `watch-after-410`, `watch-after-churn`,
`watch-final-summary`, `watch-final-410` and `watch-final-churn`
(each `.log`). The before and after aggregate JSON files and each saved
binary's `build.txt` record provenance. Locked-screen attempts and runs whose
filters preceded list completion are excluded.
