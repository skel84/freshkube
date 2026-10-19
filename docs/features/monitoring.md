# Monitoring

The behaviour of the Monitoring rail area and the CPU and memory history charts, moved from [AGENTS.md](../../AGENTS.md) unchanged apart from its link path: the paragraph starting "Monitoring (`freshkube-monitoring`'s `page/`…". The design, the panels as built and the live checks are in [docs/MONITORING.md](../MONITORING.md).

Code:

- `crates/freshkube-monitoring/src/page/` is the page: `mod.rs` (`MonitoringPage`), `board.rs`, `catalog.rs`, `connection.rs`, `source.rs`, `layout.rs`, `markers.rs`, `settings.rs` and `view.rs`.
- `crates/freshkube-monitoring/src/history/` is the history chart a pod's Overview and the node pane show; `derive/` and `panel/` turn a frame into the Console panels; `store.rs` is `monitoring.json`; `lib.rs` holds the private `dispatch` both readers use.
- `crates/freshkube-core/src/monitoring/` holds discovery, the built-in dashboards, the catalog, the history and marker reads, the transport and the example data.

Tests that prove it (the files with a `#[test]`, `#[tokio::test]` or `#[gpui_kit::test]`, found with `git grep -l '#\[test\]\|#\[tokio::test\]\|#\[gpui_kit::test\]'` run over `crates/freshkube-monitoring` and core's `monitoring/`):

- `crates/freshkube-monitoring/src/`: `page/tests/` (`page/tests/history_recovery.rs`), `history/tests.rs`, `derive/tests.rs`, `panel/tests.rs`; inline tests in `lib.rs`, `colors.rs`, `store.rs`, `page/layout.rs`, `derive/cap.rs`, `derive/text.rs` and `panel/plot.rs`.
- `crates/freshkube-core/src/monitoring/`: `tests.rs`; inline tests in `builtin/mod.rs`, `catalog.rs`, `history.rs` and `markers.rs`.

Monitoring (`freshkube-monitoring`'s `page/`, [docs/MONITORING.md](../MONITORING.md)) is its own rail area; its column lists the built-in dashboards and the user's folder of Grafana JSON (Settings → Dashboards folder, saved in `monitoring.json` beside the preferences). The shell owns the page and tells it when it shows; a hidden page finds nothing, resolves nothing and asks no panel. Showing it finds Prometheus for the context (the remembered Service first), resolves the variables, then asks only the panels in or near the viewport. Every change that asks again takes a new generation, and an older answer is dropped; hiding drops every request and the auto-refresh timer. Another source rebuilds the dashboard, so one cluster's answers never show as another's. Panels draw only Console colours. Timeseries also draw deploy and node markers (`monitoring/page/markers.rs`), read per generation from ReplicaSet metadata, Nodes' Ready condition, Node events and, with Talos, boot times; the page filters them by its toggles and the namespace and node variables before handing them to the panels. A pod's Overview and the node pane show CPU and memory over the last hour (`monitoring/history/`) from the Prometheus the page confirmed or remembers for the context (`MonitoringPage::history`, pushed on `MonitoringEvent::History`); without one they show and read nothing. A pod's chart reads while its drawer shows Details, at any scroll, and the node pane's while its Overview shows.
