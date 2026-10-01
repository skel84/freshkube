# GPUI Kit friction log

This log records GPUI Kit and GPUI friction found while building Freshkube, and what worked well. It is the evidence for judging GPUI Kit as Freshkube's toolkit. Add an entry when work needs a workaround, a toolkit source investigation, or explaining a surprising behavior. Give it the next `K` number and cite source at the pinned version where you can. Classify each entry as an **application bug**, **documentation gap**, **component limitation**, **framework issue** or **platform limitation**. An item later found to be an application mistake moves to [Reviewed and not recorded as friction](#reviewed-and-not-recorded-as-friction).

Source locations for K01–K10 are relative to a `gpui-kit` checkout at `201b55a431fb1b82a6047e908de63913db3d4354` (version 0.7.0). This application uses the published `gpui-kit` 0.7.0 from crates.io. GPUI is the crates.io snapshot `gpui-pre` 0.3.7, and its paths are relative to that crate.

**Where entries come from.** K01–K10 were found while building the first Freshkube prototype, a separate repository (archived locally as `freshkube-prototype`, `main` at `7706655`), now superseded by this one. Their Freshkube paths (`pods/table.rs`, `workspace/…`) refer to that repository. K11–K16 come from building this application's GPUI frontend, which started as talos-pilot's GPUI prototype; their paths are relative to `crates/freshkube-desktop/src/`.

| ID | Summary | Found in | Classification |
| --- | --- | --- | --- |
| [K01](#k01-datatable-keys-ignore-row-only-mode) | DataTable keys select columns in a row-only table | G02 | Component limitation |
| [K02](#k02-table-refresh-discards-resized-widths) | `TableState::refresh` discards user-resized column widths | G02 | Component limitation |
| [K03](#k03-pixel-only-sizing-apis) | Dialog, column, and dock sizes take pixels, not rems | G01–G03 | Component limitation |
| [K04](#k04-observed-element-type-differs-in-test-builds) | `.test_support()` changes the element type in test builds | G02 | Documentation gap |
| [K05](#k05-global-action-cannot-update-the-dispatching-window) | A global action cannot update the window that dispatched it | G04 | Framework issue |
| [K06](#k06-panel-selection-does-not-reveal-a-hidden-dock) | `DockArea::select_panel` neither opens a hidden dock nor focuses | G03 | Documentation gap |
| [K07](#k07-dock-resize-emits-layoutchanged-only-on-release) | Dock pointer resize emits `LayoutChanged` only on release | G03 | Documentation gap |
| [K08](#k08-select-fills-its-container) | `Select` fills its container; `.w()` sizes only the inner control | G03 | Documentation gap |
| [K09](#k09-table-selection-is-positional) | Table selection is a row index, not an object | G02, H01 | Component limitation |
| [K10](#k10-positional-selection-always-scrolls) | Setting the table selection always scrolls to it | H01 | Component limitation |
| [K11](#k11-every-notify-redraws-the-window) | Every notify redraws the whole window; caching is opt-in | Desktop app | Framework issue |
| [K12](#k12-unoptimised-builds-draw-about-14-slower) | Unoptimised builds draw about 14× slower | Desktop app | Framework issue |
| [K13](#k13-thin-documentation-and-unstable-api-shapes) | Thin documentation and unstable API shapes | Desktop app | Documentation gap |
| [K14](#k14-two-executors) | GPUI's executor and Tokio both have to run | Desktop app | Framework issue |
| [K15](#k15-ui-test-harness-traps) | UI test harness traps | Desktop app | Documentation gap |
| [K16](#k16-synthetic-input-needs-a-focused-window) | Synthetic input needs a focused window | Desktop app | Platform limitation |

## K01 DataTable keys ignore row-only mode

- **Found in:** G02. Tab was found in a native check, and Left/Right/Home/End in the independent review.
- **Symptom:** in a table configured with `row_selectable(true).col_selectable(false)`, Tab and Shift-Tab select a column instead of moving focus. Left/Right and Home/End also switch the table into column selection and drop the selected row. A production test timed out waiting for a row selection after Left.
- **Kit source:** `crates/component/src/table/data_table.rs:21-28` binds `left`/`right` to `SelectPrevColumn`/`SelectNextColumn`, `home`/`end` to `SelectFirst`/`SelectLast`, and `tab`/`shift-tab` to the column actions in the `DataTable` context. The handlers `crates/component/src/table/state.rs:979-1019` (`action_select_first_column`, `action_select_last_column`) and `:1088-1163` (`action_select_prev_col`, `action_select_next_col`) call `set_selected_col` (`state.rs:550`) without checking `col_selectable`. That flag is only consulted for header clicks (`state.rs:823`) and header rendering (`state.rs:1404`). `website/component/data-table.md:633-640` documents Left/Right as "Navigate columns" and Home/End as "first/last row/column" in row mode. It does not mention Tab.
- **Freshkube workaround:** `PodBrowser::new` in `pods/table.rs` overrides the bindings under `FreshkubePods > DataTable`. Tab and Shift-Tab do normal focus traversal, Left/Right are consumed without changing the selection, Home/End select the first or last row of the current projection (`PodBrowser::select_boundary`), and Enter activates. Regressions: `production_row_table_tab_moves_focus_without_selecting_columns` and `production_row_table_column_keys_preserve_or_navigate_rows`.
- **Classification:** component limitation (keyboard handling ignores the configured selection mode). The keyboard documentation for row mode is also incomplete.
- **Reproduction:** no minimal repro yet. The behavior is reproduced through Freshkube's production UI tests.
- **Upstream:** not reported.

## K02 Table refresh discards resized widths

- **Found in:** G02 (independent review).
- **Symptom:** after the Name column was pointer-resized to 428 px, sorting through Freshkube's labeled header button reset it to its initial 352 px.
- **Kit source:** pointer resizing changes only the state's cached `col_groups` (`crates/component/src/table/state.rs:1194-1213`, `resize_cols`) and emits `TableEvent::ColumnWidthsChanged` on release (`state.rs:1550-1561`). `TableState::refresh` (`state.rs:407-409`) calls `prepare_col_groups` (`state.rs:691-704`), which rebuilds every width from `TableDelegate::column`. Kit's built-in header-click sort (`state.rs:1216-1246`) updates `col_groups` in place. A custom `render_th` control that changes sort state in the delegate must call `refresh` so the sort indicators update, which resets the widths. The `TableDelegate::column` docs (`crates/component/src/table/delegate.rs:23-26`) say only that it is called on prepare and refresh.
- **Freshkube workaround:** the `ColumnWidthsChanged` subscription in `PodBrowser::new` (`pods/table.rs`) copies the new widths into `PodTable::columns`, so a later `refresh` from `PodTable::render_th` keeps them. Regression: `production_labeled_sort_preserves_dragged_column_width`.
- **Classification:** component limitation. No API updates the sort state without rebuilding column layout. The docs also do not say that the delegate must own any widths it wants to keep.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K03 Pixel-only sizing APIs

- **Found in:** G01 (`Dialog::w`), G02 (`Column::width`), and G03 (`DockArea::set_dock_size`).
- **Symptom:** the first G01 build failed because `Dialog::w(rems(28.))` does not compile. `Dialog` has an inherent `w(impl Into<Pixels>)` that shadows `Styled::w`, which would accept a rem length. Table column widths and dock sizes also take `Pixels`. The Kit design skill requires rem-based sizing.
- **Kit source:** `crates/component/src/dialog/dialog.rs:452` (`w`), `:460` (`width`), and `:513-517` (`impl Styled for Dialog`); `crates/component/src/table/column.rs:151` (`Column::width`); `crates/base/src/dock/dock_area.rs:348` (`set_dock_size`). The rem guidance is in `skills/gpui-kit-design-guides/SKILL.md:50-52`.
- **Freshkube workaround:** each call site converts with `rems(x).to_pixels(window.rem_size())`. The call sites are `open_about` (`shell.rs`), `PodTable::new` (`pods/table.rs`), and the default right-dock size in `Workspace::new` (`workspace/mod.rs`). These values are resolved once. They are not expected to follow a later theme font-size change, which has not been checked separately.
- **Classification:** component limitation (API shape).
- **Reproduction:** no minimal repro yet. A one-line compile check with `Dialog::w(rems(..))` would be enough.
- **Upstream:** not reported.

## K04 Observed element type differs in test builds

- **Found in:** G02.
- **Symptom:** a helper that returned `Stateful<Div>` from `.id(..).test_support()` compiled in normal builds but failed under `cargo test`. With the `test-support` feature, which the dev-dependency enables, `.test_support()` returns `Observed<Stateful<Div>>`.
- **Kit source:** `crates/base/src/observe.rs:4-9` defines `ObservedElement<E>` as `Observed<E>` with `test-support` and as `E` otherwise. `observe.rs:27` has `fn test_support(self) -> ObservedElement<Self>`, and `crates/base/src/test_support.rs:237` has `Observed<E>`. `crates/kit/TESTING.md` does not mention the effect on return types.
- **Freshkube workaround:** helpers that call `.test_support()` return `impl IntoElement` (`PodTable::render_read_state` in `pods/table.rs`).
- **Classification:** documentation gap.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K05 Global action cannot update the dispatching window

- **Found in:** G04 (independent review, P1).
- **Symptom:** Command-Q and native Quit did nothing. The global `Quit` handler (`App::on_action`) called `handle.update(..)` on the window whose key event was being dispatched. GPUI had taken that window out of its window map for the duration of the dispatch, so the update returned `Err("window not found")`. The handler discarded the error, so nothing was reported.
- **Source (GPUI):** `src/window.rs:1938-1942` (platform input is dispatched inside a window update), `src/app.rs:2882-2890` (`update_window_erased` takes the window out of `cx.windows` while it is being updated and reports it as absent otherwise), `src/app.rs:1987-2003` (`update_window_id` turns that into "window not found"), and `src/window.rs:6362-6388` (`dispatch_action_on_node_inner`, which starts at `:6277`, calls the bubble-phase global listeners registered by `App::on_action` at `src/app.rs:2367-2381` while the window is still taken). Kit's GPUI references document the panic for nested entity updates (`skills/gpui-kit/references/gpui/entity.md:122-130`). They do not cover this silent window case.
- **Freshkube workaround:** `Workspace::new` (`workspace/mod.rs`) registers the Quit handler with `cx.defer` around `handle.update`, and holds a weak workspace owner. Regression: `yaml_undo_redo_and_dirty_close_are_real_editor_interactions` presses Command-Q (`workspace/tests.rs`).
- **Classification:** framework issue. The window lease is intended behavior, but the failure is silent, the error message is misleading, and neither Kit nor GPUI guidance documents it. Discarding the `Result` was the application-side part of the bug.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K06 Panel selection does not reveal a hidden dock

- **Found in:** G03 (design and source inspection).
- **Symptom:** `DockArea::select_panel` on a panel in a closed dock changes that tab group's active index only. The panel stays off screen and keyboard focus does not move. Clicking a tab does focus the panel, because `TabGroup::select_tab` calls `focus_active_panel`.
- **Kit source:** `crates/base/src/dock/dock_area.rs:555-582`. The doc comment says "Display `panel` in the tab group that holds it". The implementation calls `tree.set_active` and `commit`, and reconciliation applies the index through `TabGroup::sync_from_tree` (`crates/base/src/dock/tab_group.rs:329-347`) without focusing. Compare `TabGroup::select_tab` (`tab_group.rs:231-242`).
- **Freshkube workaround:** `Workspace::focus_pane` (`workspace/mod.rs`) opens any closed dock that contains the panel with `toggle_dock`, calls `select_panel`, and then focuses the pane or the pod table. Regression: `reopening_hidden_dock_reclamps_and_closing_last_optional_document_stays_restorable`.
- **Classification:** documentation gap ("Display" suggests the panel becomes visible). Programmatic selection and tab clicks also differ in whether they focus the panel.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K07 Dock resize emits LayoutChanged only on release

- **Found in:** G03 (design risk review, then implementation).
- **Symptom:** during an outer-dock pointer resize, `DockArea` notifies on every pointer move but emits no `DockEvent::LayoutChanged`. At this revision the event arrives once, on mouse-up, and only because the `DockSkin` renderer emits it. A subscriber that re-clamps or persists layout only on `LayoutChanged` falls behind until release, and never catches up with a renderer that does not emit the event.
- **Kit source:** `crates/base/src/dock/dock_area.rs:1219-1263` (`resize_dock` calls only `cx.notify()`) and `crates/component/src/dock/dock.rs:235-246` (the skin's mouse-up handler emits `LayoutChanged`, with the comment "The size lives on the dock, not in the layout tree, so nothing else tells a subscriber to persist it"). The module docs present `LayoutChanged` as the change signal for tree edits (`crates/base/src/dock/mod.rs:35-40`). Dock size is not part of the tree.
- **Freshkube workaround:** `Workspace::new` (`workspace/mod.rs`) subscribes to `DockEvent` and also calls `cx.observe_in` on the dock. Both paths lead to `Workspace::dock_changed`.
- **Classification:** documentation gap. The code comment in `Workspace::new` is correct about the drag itself. It does not mention the event on release.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K08 Select fills its container

- **Found in:** G03 (native check).
- **Symptom:** the connection and namespace `Select`s placed directly in the workspace toolbar row were unconstrained. Wrapping each one in a fixed-width container fixed the layout. The original rendering was not recorded in detail.
- **Kit source:** the `Select` root element is `size_full()` (`crates/component/src/select.rs:858`), and so is the `SelectState` root (`select.rs:527`). `impl Styled for Select` (`select.rs:795-802`) stores the style in `options.style`, which is applied only to the inner input element (`select.rs:818`, `:553`). `website/component/select.md:194` shows `.w(px(320.))` as "Set dropdown width". The Select story wraps every Select in a fixed-width container (`crates/story/src/stories/select_story.rs:223-281`).
- **Freshkube workaround:** `Workspace::render` (`workspace/mod.rs`) wraps each Select in `div().w(rems(..)).input_h(Size::Small).flex_none()`.
- **Classification:** documentation gap.
- **Reproduction:** no minimal repro yet. Confirm the unwrapped behavior before reporting.
- **Upstream:** not reported.

## K09 Table selection is positional

- **Found in:** G02 (design requirement from V02).
- **Symptom:** `TableState` selection is a row index. When the delegate sorts, filters, or receives updates, the index stays the same and the highlight moves to a different object. Kit's own sort path does not remap the selection.
- **Kit source:** `crates/component/src/table/state.rs:480-510` (`selection`, `selected_row`, and `set_selected_row` are positional) and `state.rs:1216-1246` (`perform_sort` updates sort state and delegates without remapping the selection). `website/component/data-table.md:328-356` describes positional selection. During `TableDelegate::perform_sort`, the delegate is borrowed by `TableState` (`state.rs:1243`), so it cannot call selection APIs directly.
- **Freshkube workaround:** `PodProjection` (`pods/projection.rs`) holds the selected identity outside Kit's table state and caches its current visible index on every rebuild. `sync_selection` (`pods/table.rs`) maps it to the current projection index after sort, filter, or updates. Inside `PodTable::perform_sort` the call is deferred with `cx.defer_in`. `SelectRow` and `DoubleClickedRow` events are accepted only if the index still maps to the selected identity. An object that is filtered out, deleted, or recreated clears the selection.
- **Pointer events carry the painted index (H01):** GPUI hit-tests mouse input against the last painted frame without drawing first (`gpui-pre` `src/window.rs:5744-5758`), while key input draws first when the window is dirty (`:5803-5806`). Kit captures `row_ix` in the row's click listener at paint time (`crates/component/src/table/state.rs:2270-2272`) and `on_row_left_click` emits `SelectRow(row_ix)` and `DoubleClickedRow(row_ix)` from it (`state.rs:804-820`). When the delegate's rows change between paint and click, that index names a different object. Freshkube's rows register their own click listener before Kit's (`PodTable::render_tr`), recording the painted identity; `SelectRow` and `DoubleClickedRow` handling resolve through it. Regression: `a_click_on_a_frame_painted_before_a_store_batch_targets_the_painted_pod` (`pods/table_tests.rs`), which fails without the listener.
- **Classification:** component limitation. This is a reasonable default for a generic table, but a resource browser has to add an identity layer. Row events have no frame or data generation, so a delegate with live data cannot tell a stale pointer index from a current one without its own listener.
- **Reproduction:** not applicable. This is documented positional behavior.
- **Upstream:** not reported.

## K10 Positional selection always scrolls

- **Found in:** H01 (independent review).
- **Symptom:** keeping Kit's selected row in step with an identity after data changes requires `TableState::set_selection` or `set_selected_row`, and both scroll the selected row into view. When a batch shifts the selected pod's index, the table scrolls back to it even if the user had scrolled elsewhere. With fixtures this happens only on explicit fixture commands; with a live watch it would happen on every batch that moves the selection.
- **Kit source:** `crates/component/src/table/state.rs:501-524` (`set_selected_row` always calls `vertical_scroll_handle.scroll_to_item` and emits `SelectRow`), `:607-614` (`set_selection` delegates to it). There is no setter that updates the selected index without scrolling or emitting.
- **Freshkube workaround:** none yet. `sync_selection` (`pods/table.rs`) calls `set_selection` only when the index changed. G06 must decide between a no-scroll path (for example restoring the scroll offset for data-driven syncs) and an upstream API.
- **Classification:** component limitation.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K11 Every notify redraws the window

- **Found in:** the desktop app, while investigating why it felt slow.
- **Symptom:** `render` runs on every frame for every dirty view, and a notify on any entity redraws the whole window. Lists were sorted twice per frame, and a 1-second clock timer on the shell redrew everything once a second. Even with caching, a hover still forces a full redraw.
- **Source:** view caching is opt-in (`.cached()` on a view, or hand-made row caches). Exact GPUI source locations for the redraw path are not yet cited.
- **Freshkube workaround:** derive display data when the data changes, not in `render`; cache heavy views; give periodic timers to the smallest entity that shows them (see [AGENTS.md](../AGENTS.md#keep-render-cheap)).
- **Classification:** framework issue (performance model). It is workable, but slow is the default.
- **Reproduction:** none yet.

## K12 Unoptimised builds draw about 14× slower

- **Found in:** the desktop app.
- **Symptom:** in an unoptimised debug build, GPUI draws a frame roughly 14 times slower than in an optimised one, which makes debug builds misleading for performance and unpleasant to use.
- **Freshkube workaround:** the workspace manifest builds dependencies at `opt-level = 3` in the dev profile (`[profile.dev.package."*"]`) while workspace crates stay unoptimised for fast rebuilds.
- **Classification:** framework issue (build configuration every application needs).
- **Reproduction:** remove the profile override and compare frame times.

## K13 Thin documentation and unstable API shapes

- **Found in:** the desktop app and the Freshkube prototype.
- **Symptom:** much of the API has to be learned by reading GPUI and GPUI Kit source in the Cargo registry. Some shapes are surprising: `Window::root::<E>()` returns `Option<Option<Entity<E>>>` (`gpui-pre` 0.3.7 `src/window.rs:2257`). The pinned versions are pre-1.0, so APIs will change.
- **Freshkube workaround:** read the pinned sources before using an API; keep exact version pins (`gpui-kit = "=0.7.0"`).
- **Classification:** documentation gap.

## K14 Two executors

- **Found in:** the desktop app (and the Freshkube prototype's backend).
- **Symptom:** GPUI runs its own executor, while tonic (Talos gRPC) and kube need Tokio. Every data load starts on Tokio and hops back to a GPUI task to update entities.
- **Freshkube workaround:** the binary owns a Tokio runtime and passes its handle to the app; requests are tied to a target epoch and owned by `OwnedJob` handles that cancel on drop (`backend.rs`).
- **Classification:** framework issue (integration cost, not a defect).

## K15 UI test harness traps

- **Found in:** the desktop app's UI tests.
- **Symptoms:**
  - Dialogs animate on the real clock; advancing the test clock does not finish the animation, which made dialog tests flaky. Turning on reduced motion (`cx.set_reduce_motion(true)`, `gpui-pre` 0.3.7 `src/app.rs:1147`) in the shared test setup fixed two confirmation-dialog tests that failed on every run without it; the Freshkube prototype's dialog tests used it from the start.
  - `Window::render_frame` bypasses view caches, so it cannot show whether a cached view was redrawn. The app counts renders with its own probes (`desktop::probe`).
  - Element snapshots cannot tell whether a button is disabled (see the observation below).
  - Setting an input's value from code does not emit its change event. One test that relied on it ended up writing files into the crate directory.
- **Freshkube workaround:** the rules under [UI tests](../AGENTS.md#ui-tests) in AGENTS.md.
- **Classification:** documentation gap (test harness behaviour that is not documented).

## K16 Synthetic input needs a focused window

- **Found in:** native visual checks of the desktop app.
- **Symptom:** macOS ignores synthetic keystrokes once the window loses focus, so driving the running app from another process is unreliable. Bringing the window to the front first (System Events `set frontmost`) worked during the Freshkube prototype's checks.
- **Freshkube workaround:** debug builds open a page from `FRESHKUBE_PAGE=<slug>`; interaction checks live in headless UI tests.
- **Classification:** platform limitation, not a GPUI defect.

## Strengths observed

The evaluation weighs these against the friction above:

- **State model:** entities changed through `update`, `cx.notify()`, `observe` and `subscribe`, with weak handles in closures, fit Rust's ownership rules; there are few borrow-checker fights and no shared mutable state passed around.
- **Rendering model:** each frame rebuilds the element tree from current state, with no hidden reactive graph to debug.
- **Styling:** the builder API (`.px()`, `.gap()`, `v_flex()`) is quick to write, and GPUI Kit's components and theming produce a polished result.
- **UI tests:** headless tests render the real app, find elements by id and click or type into them; the desktop app's suite of about 168 tests runs in about 3 seconds.
- **No UI boundary:** screens call core logic directly with Rust types, unlike a WebView-based frontend.

## Observations without a verdict yet

- **Large single-line YAML documents (G04):** the ignored diagnostic `yaml_large_long_line_unicode_find_typing_and_undo` (`workspace/yaml.rs`) mounts the production YAML view with 1 MiB and 5 MiB single-line sources. Creating the view, drawing a frame, and making an end-of-buffer edit took 5.437 s and 27.233 s. The whole find/undo flows took 13.511 s and 64.072 s. These are unoptimized, headless, single runs, not native typing latency. Measure them in release mode on a native window under G11 before attributing the cost to the Kit editor, GPUI text layout, or the test harness.
- **Disabled state in UI queries (G02):** `ElementSnapshot::disabled()` returns `Some(true)` or `None`, never `Some(false)` (`crates/base/src/test_support.rs:106-109`, `:317`). The G02 tests could not assert the disabled state of fixture buttons. They check that an attempted action leaves the model and clipboard unchanged instead. Whether Kit `Button` exposes its disabled state to queries has not been determined.
- **Focus-in listeners with an inactive window (G03):** when Tab was delivered to an inactive native window by the automation tool, `on_focus_in` listeners that scroll a control into view did not run. `PodBrowser::reveal_focused_control` (`pods/table.rs`) scrolls the newly focused control directly. This was seen only with tool-injected input, not in foreground use.

## Reviewed and not recorded as friction

- **Command palette query focus (G03):** Kit's `Command` does not take focus when it is mounted in a `Dialog`. The Command story and `website/component/command.md:101-119` show focusing it explicitly. Freshkube's omission was an application bug, now fixed in `Workspace::open_palette`.
- **Measurement canvas origin (G03):** an `.absolute()` element without insets is placed at its static position, as in CSS. Freshkube's workspace measurement canvas now sets `top_0().left_0()`. This was an application bug.
- **Unclamped `set_dock_size` (G03):** `DockArea::set_dock_size` is documented as unclamped against the area, with only a minimum-size floor (`crates/base/src/dock/dock_placement.rs:152-160`). Restored and reopened docks are clamped by Freshkube with `DockSizing` (`Snapshot::clamp`, `Workspace::clamp_geometry`). This is expected application work.
