# Workbench

`freshkube-workbench` shows the shared components from `freshkube-ui` one story at a time, on invented data. A component can be checked in both themes, at every text size and at both window widths without starting the app or reaching a cluster. It is a developer tool: the app bundle never holds it, and `scripts/package-macos.py` fails a bundle with any executable other than `freshkube`. Issue [#53](https://github.com/skel84/freshkube/issues/53) tracks it.

## Run it

```sh
cargo run -p freshkube-workbench
FRESHKUBE_STORY=data-table cargo run -p freshkube-workbench   # open one story
scripts/smoke.sh start --story data-table --theme dark --size 760x560
```

The window has the story list at the left, and a strip above the story with the theme (Light, Dark), the text size (12 to 20), the window's width (1280, 760) and motion (System, Reduced, Full). Nothing is saved. Debug builds also take the app's `FRESHKUBE_THEME`, `FRESHKUBE_WINDOW_SIZE` and `FRESHKUBE_TEXT_SIZE`, so `smoke.sh start --story` works like `--page` for theme and size. Release builds ignore them.

`FRESHKUBE_FIRST_FRAME=1` prints the time from process start to a story's first frame with rows, in debug builds. The app prints the same for its first Pods frame with rows (`FRESHKUBE_KIND=pods FRESHKUBE_PAGE=resources cargo run -- --fixture`). Every build also prints when its window first draws (`first frame: window after N ms`), release builds only that; the release smoke check waits for it.

## Stories

- **`data-table`:** `DataTable` on invented pods (`shop-api`, `app-a`, `cluster-a`, `registry.example`). It shows group rows with status glyphs, a pinned glyph and name while the wider columns scroll sideways, and sorting by name, restarts or age. The header's chips filter by status. Its controls switch between rows, loading, empty and failed, and between 24 and 2,000 rows.

- **`graph`:** `GraphView` (`freshkube_ui::graph`) on invented services, laid out by `freshkube-graph`. Callers sit left of what they call, a line with an arrowhead and a marker joins each call, and a call with a problem is dashed. The list of calls is the shared inspector, beside the graph in its card when the graph fits, otherwise under it. A marker or a row in the list selects a call, and Problem calls hides the healthy ones without moving a box. Its controls switch between a shop's calls, a fan-out whose column wraps, a cycle, and services with no calls. Calls are routed as the service map in Observability routes them: they cross columns through the gutters between boxes, and a cycle's call comes back below them. The header counts how many times the calls cross.

- **`drawer`:** `freshkube_ui::drawer` over a list of invented objects. A row opens it and swaps what it shows, a click beside the rows closes it, and its left edge resizes it within its bounds; under 600 dp it takes the whole page.

- **`dock`:** `freshkube_ui::dock` under a page, with invented log tabs on invented objects. Its bar holds the closable tabs and the chrome; its top edge drags the height, and a drag below 100 dp minimizes it to its bar. Its controls switch between Open, Minimized and Fit to window, which takes the page's room, and Add a tab adds one on the next invented object.

- **`squares`:** the container squares of Pods' Containers column (`freshkube_ui::squares`) on invented pods, a row per state with the words its tooltip gives: running, restarted, not ready, waiting, stuck, failed, exited 0, no state, and an init container first and dimmed. Many shows a pod with twelve containers, eight squares and `+4`.

- **`loading`:** the loading state ([DESIGN.md](DESIGN.md#motion)). Above, today's card of `ui::skeleton` bars, as each page draws its own; below, the one state every table would share: the table's real header over `table::LoadingRows`, skeleton rows at the row height with a bar in each column, a dot in the glyph column. The table draws them still and a `table::LoadingMotion` over it moves them, so its frames leave the table alone. Its control switches the rows between Pulse (Kit's 2 s fade, every bar together) and Shimmer (a light band sweeping across the rows, each row a little behind the one above).

- **`change-flash`:** the change flash on 40 invented pods by name. A changed row's status or restarts are tinted and fade over 1.5 s (`table::FlashLayer`, drawn over the cached table). The timer changes a pod once or four times a second, or not at all; Change one and Burst of 40 change rows by hand, and a batch past 8 changes in 1.5 s doesn't flash. Its second row picks what reduced motion shows, No tint or a Held tint cleared without a fade, and a Heavy root that draws 1,200 cells above the table, since each flash frame renders the views above the layer. The meta line counts the root's renders, the time its last one took and the table's renders, which a flash leaves alone.

The strip's Motion group applies to every story: System follows the OS (and names what it last said), Reduced and Full override it.

## Add a story

1. Write its view in `crates/freshkube-workbench/src/stories/<name>.rs` with a `build(window, cx) -> AnyView` and a root element with the id `<slug>-page`.
2. List it in `STORIES` in `stories/mod.rs`.
3. Draw it from `freshkube_ui` (`page`, `table`, `ui`) as a page would. `scripts/check-style.sh` checks the workbench as it checks pages.
4. Use invented names only (`app-a`, `shop-redis`, `cluster-a`, `registry.example`).
5. Give its own states controls in its header, with ids starting `<slug>-`, and test them in `src/tests.rs`. The shared test already draws every story in both themes at text sizes 13 and 20.

The crate depends on `freshkube-ui`, `freshkube-graph` and `freshkube-probe` only, never on `freshkube-desktop` or `freshkube-core`. A story needs nothing a page outside the shared components owns.
