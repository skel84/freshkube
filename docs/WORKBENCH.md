# Workbench

`freshkube-workbench` shows the shared components from `freshkube-ui` one story at a time, on invented data. A component can be checked in both themes, at every text size and at both window widths without starting the app or reaching a cluster. It is a developer tool: the app bundle never holds it, and `scripts/package-macos.py` fails a bundle with any executable other than `freshkube`. Issue [#53](https://github.com/skel84/freshkube/issues/53) tracks it.

## Run it

```sh
cargo run -p freshkube-workbench
FRESHKUBE_STORY=data-table cargo run -p freshkube-workbench   # open one story
scripts/smoke.sh start --story data-table --theme dark --size 760x560
```

The window has the story list at the left, and a strip above the story with the theme (Light, Dark), the text size (12 to 20) and the window's width (1280, 760). Nothing is saved. Debug builds also take the app's `FRESHKUBE_THEME`, `FRESHKUBE_WINDOW_SIZE` and `FRESHKUBE_TEXT_SIZE`, so `smoke.sh start --story` works like `--page` for theme and size. Release builds ignore them.

`FRESHKUBE_FIRST_FRAME=1` prints the time from process start to a story's first frame with rows, in debug builds. The app prints the same for its first Pods frame with rows (`FRESHKUBE_KIND=pods FRESHKUBE_PAGE=resources cargo run -- --fixture`). `freshkube-probe`'s `first_frame` module isn't compiled into release builds.

## Stories

- **`data-table`:** `DataTable` on invented pods (`shop-api`, `app-a`, `cluster-a`, `registry.example`). It shows group rows with status glyphs, a pinned glyph and name while the wider columns scroll sideways, and sorting by name, restarts or age. The header's chips filter by status. Its controls switch between rows, loading, empty and failed, and between 24 and 2,000 rows.

- **`graph`:** `GraphView` (`freshkube_ui::graph`) on invented services, laid out by `freshkube-graph`. Callers sit left of what they call, a line with an arrowhead and a marker joins each call, and a call with a problem is dashed. A marker or a row in the list of calls selects a call, and Problem calls hides the healthy ones without moving a box. Its controls switch between a shop's calls, a fan-out whose column wraps, a cycle, and services with no calls. Calls are routed as the service map in Observability routes them: they cross columns through the gutters between boxes, and a cycle's call comes back below them. The header counts how many times the calls cross.

## Add a story

1. Write its view in `crates/freshkube-workbench/src/stories/<name>.rs` with a `build(window, cx) -> AnyView` and a root element with the id `<slug>-page`.
2. List it in `STORIES` in `stories/mod.rs`.
3. Draw it from `freshkube_ui` (`page`, `table`, `ui`) as a page would. `scripts/check-style.sh` checks the workbench as it checks pages.
4. Use invented names only (`app-a`, `shop-redis`, `cluster-a`, `registry.example`).
5. Give its own states controls in its header, with ids starting `<slug>-`, and test them in `src/tests.rs`. The shared test already draws every story in both themes at text sizes 13 and 20.

The crate depends on `freshkube-ui`, `freshkube-graph` and `freshkube-probe` only, never on `freshkube-desktop` or `freshkube-core`. A story needs nothing a page outside the shared components owns.
