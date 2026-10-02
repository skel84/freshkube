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
```

| Scenario | What it does |
| --- | --- |
| `table <pods>` | lists that many pods, then types three filters and clears each |
| `burst <pods> <changes/s>` | lists the pods, then the watch changes them at that rate: one change in ten deletes a pod and adds another, the rest flip a pod between Running and CrashLoopBackOff |
| `pod-logs <lines/s>` | opens a pod's Logs tab while its container writes at that rate; every 50th line carries 300 more characters |
| `talos-logs <lines/s>` | example Talos logs, the collected services writing that many lines a second between them |

A run lasts `FRESHKUBE_STRESS_SECONDS` (30) and leaves the first `FRESHKUBE_STRESS_WARMUP` (5) seconds out of its summary. GPUI stops drawing a covered window or one on a locked screen, so keep the window in front; the script refuses to run on a locked screen.

The `stress` feature turns on spans around the work that matters (`crate::perf`); without it they compile to nothing. Each second the run prints, for every span, its count, median, 99th percentile, maximum and total in milliseconds, then the process's CPU share and resident memory:

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

## Fixes

Each fix is its own commit, with the workload that showed the problem run again after it.

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
