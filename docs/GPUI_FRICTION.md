# GPUI Kit friction log

This log records GPUI Kit and GPUI friction found while building Freshkube, and what worked well. It is the evidence for judging GPUI Kit as Freshkube's toolkit. Add an entry when work needs a workaround, a toolkit source investigation, or explaining a surprising behavior. Give it the next `K` number and cite source at the pinned version where you can. Classify each entry as an **application bug**, **documentation gap**, **component limitation**, **framework issue** or **platform limitation**. An item later found to be an application mistake moves to [Reviewed and not recorded as friction](#reviewed-and-not-recorded-as-friction).

Source locations for K01–K10 are relative to a `gpui-kit` checkout at `201b55a431fb1b82a6047e908de63913db3d4354` (version 0.7.0). This application uses the published `gpui-kit` 0.7.0 from crates.io. GPUI is the crates.io snapshot `gpui-pre` 0.3.7, and its paths are relative to that crate.

**Where entries come from.** K01–K10 were found while building the first Freshkube prototype, a separate repository (archived locally as `freshkube-prototype`, `main` at `7706655`), now superseded by this one. Their Freshkube paths (`pods/table.rs`, `workspace/…`) refer to that repository. K11–K25 come from building this application's GPUI frontend, which started as talos-pilot's GPUI prototype; their paths are relative to `crates/freshkube-desktop/src/`. From K17 on, toolkit source is cited in the published crates in the Cargo registry, by crate name and version (`gpui-pre` 0.3.7, `gpui-base` 0.7.0, `gpui-component` 0.7.0, `gpui-kit` 0.7.0), with paths relative to each crate.

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
| [K17](#k17-keys-follow-the-last-drawn-frame) | A focus target that isn't drawn sends keys to the window root | Workflow pass | Documentation gap |
| [K18](#k18-bindings-run-before-key-listeners) | Bindings run before a focused element's key listeners | Terminal view | Framework issue |
| [K19](#k19-scroll_to_item-targets-direct-children-and-the-last-viewport) | `scroll_to_item` targets direct children and the last frame's viewport | Browsing step 2a | Documentation gap |
| [K20](#k20-popup-menu-items-are-identified-by-position) | Popup menu items are identified by position | Pod logs | Component limitation |
| [K21](#k21-a-dialog-returns-focus-to-what-had-it-at-open) | A dialog returns focus to what had it when it opened | Workflow pass | Documentation gap |
| [K22](#k22-no-terminal-widget-for-the-pinned-kit) | No terminal widget builds against the pinned Kit | Pod exec | Component limitation |
| [K23](#k23-covered-or-locked-windows-draw-nothing) | Covered or locked windows draw nothing | Live checks, performance pass | Platform limitation |
| [K24](#k24-path-vertices-are-copied-into-a-fresh-vector-every-frame) | Path vertices are copied into a fresh, unreserved vector every frame | Monitoring, #22 | Framework issue |
| [K25](#k25-a-path-past-65536-vertices-fails-to-build) | One path holds at most 65,536 vertices | Monitoring series cap | Framework issue |

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
- **Text size (this application):** the text-size work (`6dadfba` to `0090d8e`) answered the open question. Kit's own controls follow the theme's font size, which `Root` makes the window's rem size. Freshkube converts on every render with `ui::dp_px(n, window)` (`ui.rs`), so a dialog's width (`desktop/kind_switcher.rs`) and the detail split's limits (`resources/screen/mod.rs`) follow a size change. A Kit resizable panel uses its `size` only until its first layout, then keeps the measured pixels (`gpui-base` 0.7.0 `src/resizable/panel.rs:341-352`, `src/resizable/mod.rs:200-214`). An open split should therefore keep its width in pixels across a size change while its limits follow. That is read from source, not checked in the app.
- **Classification:** component limitation (API shape).
- **Reproduction:** no minimal repro yet. A one-line compile check with `Dialog::w(rems(..))` would be enough.
- **Upstream:** not reported.

## K04 Observed element type differs in test builds

- **Found in:** G02.
- **Symptom:** a helper that returned `Stateful<Div>` from `.id(..).test_support()` compiled in normal builds but failed under `cargo test`. With the `test-support` feature, which the dev-dependency enables, `.test_support()` returns `Observed<Stateful<Div>>`.
- **Kit source:** `crates/base/src/observe.rs:4-9` defines `ObservedElement<E>` as `Observed<E>` with `test-support` and as `E` otherwise. `observe.rs:27` has `fn test_support(self) -> ObservedElement<Self>`, and `crates/base/src/test_support.rs:237` has `Observed<E>`. `crates/kit/TESTING.md` does not mention the effect on return types.
- **Freshkube workaround:** helpers that call `.test_support()` return `impl IntoElement` (`PodTable::render_read_state` in `pods/table.rs`).
- **Builder order (this application):** `.test_support()` needs an id already set and must come before `.track_focus(..)`, or focus queries can't see the binding. Kit documents both (`gpui-base` 0.7.0 `src/observe.rs:25-26` and `src/test_support.rs:259-263`; `gpui-kit` 0.7.0 `TESTING.md:50-54`), but a missed binding on a custom element can still report focus as `None` without a panic (`TESTING.md:74-78`). Freshkube's focusable wrappers put it straight after `.id(..)`, as `resource-body` in `resources/screen/mod.rs` does.
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
  - Emitted events and notifications are queued as effects and run only when the outermost update ends (`gpui-pre` 0.3.7 `src/app.rs:1161-1180`; `emit` queues an `Effect::Emit`, `src/app/context.rs:760-770`). A test that emits and checks a subscriber's result inside one `update_window` closure sees nothing yet; it has to check after the closure returns. Found while building custom resources (step 4).
- **Freshkube workaround:** the rules under [UI tests](../AGENTS.md#ui-tests) in AGENTS.md.
- **Classification:** documentation gap (test harness behaviour that is not documented).

## K16 Synthetic input needs a focused window

- **Found in:** native visual checks of the desktop app.
- **Symptom:** macOS ignores synthetic keystrokes once the window loses focus, so driving the running app from another process is unreliable. Bringing the window to the front first (System Events `set frontmost`) worked during the Freshkube prototype's checks.
- **Freshkube workaround:** debug builds open a page from `FRESHKUBE_PAGE=<slug>`; interaction checks live in headless UI tests.
- **Classification:** platform limitation, not a GPUI defect.

## K17 Keys follow the last drawn frame

- **Found in:** the workflow pass (`2c9c278`).
- **Symptom:** while the Resources page showed a message in place of its rows, the list's keys and the shell's page keys stopped working. The focused list was not drawn, so key dispatch started from the window root, outside every context of the page. Nothing reports that the focus target is missing.
- **Source (GPUI):** `gpui-pre` 0.3.7 `src/window.rs:5807-5808` (`dispatch_key_event` builds the dispatch path from the focused node in the rendered frame) and `:6251-6259` (`focus_node_id_in_rendered_frame` falls back to `root_node_id()` when the focused handle is not in that frame). The same lookup makes `FocusHandle::dispatch_action` (`:627-636`) do nothing for a handle that wasn't drawn, while `Window::dispatch_action` (`:2441-2454`) is deferred to after the current update. Neither doc comment mentions this.
- **Freshkube workaround:** a page's key context and focus sit on a wrapper drawn in every state (`ResourcesScreen::keyed`, the `resource-body` element in `resources/screen/mod.rs`). See [Keys and focus](../AGENTS.md#keys-and-focus).
- **Classification:** documentation gap.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K18 Bindings run before key listeners

- **Found in:** pod exec step 1, the terminal view (`6fda2db`).
- **Symptom:** a terminal must pass Escape, Tab, Control-Tab and the arrows to the program. Any binding on the focus path takes such a key before the focused element's own key-down listener sees it, even a listener in the capture phase. Kit's `Root` binds Tab and Shift-Tab to focus traversal, and the app binds Escape and Control-Tab in enclosing contexts. Separately, the keypad can't be told apart from the main keyboard, so application keypad mode can't be honoured.
- **Source (GPUI):** `gpui-pre` 0.3.7 `src/window.rs:5860-5870` (keystroke interceptors run first, and stopping propagation there skips the rest), `:5944-5957` (bindings on the dispatch path), then `:5959` and `:6007-6016` into `dispatch_key_down_up_event` (`:6049-6075`), where capture and bubble key listeners run. `App::intercept_keystrokes` (`src/app.rs:2321-2345`) is the only hook ahead of bindings, and it is app-wide. Kit binds `tab` and `shift-tab` in `Root` (`gpui-base` 0.7.0 `src/root.rs:16-17`). `gpui-pre-macos` 0.3.7 `src/events.rs:368` maps the keypad's Enter to `enter`, and `Keystroke` (`gpui-pre` `src/platform/keystroke.rs:18`) has no keypad flag.
- **Freshkube workaround:** one app-wide interceptor (`register` and `intercept` in `freshkube-terminal`'s `input.rs`) finds the focused terminal, hands it every key without Command and stops propagation. Command shortcuts and typed text pass on to bindings and the input handler. Terminal-wide shortcuts use Command. The keypad types digits and Enter, as most terminals do by default. Binding every key in the terminal's own context would also work, since a deeper context wins, but it would need a binding for each key.
- **Classification:** framework issue (no per-element way to take keys ahead of bindings) and platform-layer limitation (no keypad flag).
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K19 scroll_to_item targets direct children and the last viewport

- **Found in:** browsing step 2a (`361b9d5`), revealing the current kind in the sidebar.
- **Symptom:** `ScrollHandle::scroll_to_item(ix)` counts only the scrolled element's direct children, so a row nested inside a group can't be named. On a window's first frame the reveal had no viewport size to work with.
- **Source (GPUI):** `gpui-pre` 0.3.7 `src/elements/div.rs:4315-4323` (`scroll_to_item` stores an index), `:1985-1996` (`child_bounds` holds the bounds of direct children only) and `:2009-2011` (the scroll is applied in prepaint against `state.bounds`). Those bounds are set later in the same prepaint (`:2482-2485`), so the scroll uses the previous frame's viewport, which on a first frame is empty. An index with no child stays pending (`:4344-4382`). `ScrollAnchor` (`:4186-4212`) can target a nested element on the next frame. Freshkube doesn't use it yet.
- **Freshkube workaround:** the sidebar flattens its rows into the scroll area's children and counts them (`FIRST_ROW + row` in `desktop/shell.rs`), and asks for another frame while `sidebar_scroll.bounds()` is still empty. Diagnostics and Security count the selected row's child index in the same way.
- **Classification:** documentation gap.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K20 Popup menu items are identified by position

- **Found in:** pod logs (`eec2418`).
- **Symptom:** UI tests can't pick a container or tail length from the menu by a domain id. A Kit `PopupMenuItem` takes no id. Each item's element id is its index in the menu, which counts separators and headings and so shifts with the pod's containers.
- **Kit source:** `gpui-component` 0.7.0 `src/menu/popup_menu.rs:1226` (`MenuItemElement::new(ix, ..)`), `:1494-1499` (every item, separators and labels included, is enumerated) and the item builders at `:71-214` (no id). `MenuItemElement::new` is `pub(crate)` (`src/menu/menu_item.rs:25`).
- **Freshkube workaround:** tests drive the choices through the view's methods (`choose_container`, `set_tail` in `logs/pod/mod.rs`). No UI test covers the menus' own click path.
- **Classification:** component limitation.
- **Reproduction:** not applicable; the id is positional by construction.
- **Upstream:** not reported.

## K21 A dialog returns focus to what had it at open

- **Found in:** the workflow pass, the Command-K kind palette (`f6c28d0`).
- **Symptom:** `open_dialog` records the focused element, then focuses a handle of its own, not the dialog's content. The content has to be focused after `open_dialog`: if it is focused before, the dialog records it as the place to return to on close. A close that the dialog requests itself can be deferred, and then focus comes back only after the close animation. That can undo a focus change the app made in the meantime. `window.close_dialog` restores focus at once.
- **Kit source:** `gpui-component` 0.7.0 `src/root.rs:235-264` (`open_dialog` records `window.focused` at `:243` and focuses its own handle at `:251-252`), `:275-281` (`close_dialog`), `:283-310` (`defer_close_dialog` restores after `ANIMATION_DURATION`) and `src/dialog/dialog.rs:650-655` (`request_close` chooses between them). `src/window_ext.rs:30-33` documents `open_dialog` as "Opens a Dialog." without mentioning focus.
- **Freshkube workaround:** `open_kind_switcher` (`desktop/kind_switcher.rs`) focuses the `Command` state after `open_dialog`. On confirm, it closes the dialog with `window.close_dialog` before opening the kind, so the page's focus change comes after the restore. Compare "Command palette query focus" under [Reviewed and not recorded](#reviewed-and-not-recorded-as-friction).
- **Classification:** documentation gap.
- **Reproduction:** no minimal repro yet.
- **Upstream:** not reported.

## K22 No terminal widget for the pinned Kit

- **Found in:** the pod exec feasibility check (`42830e1`).
- **Symptom:** no published GPUI terminal widget builds against GPUI Kit 0.7.0. Kit has none, `gpui-terminal` 0.1.0 depends on `gpui` 0.2.2 and `gpui_xterm` 0.2.1 on `gpui-kit` 0.6.0. `bezel-terminal` uses its own GPUI fork ([POD_EXEC.md](POD_EXEC.md#feasibility)).
- **Source:** `gpui-terminal-0.1.0/Cargo.toml:49-50` and `gpui_xterm-0.2.1/Cargo.toml:71-72`.
- **Freshkube workaround:** Freshkube draws its own grid over `alacritty_terminal`. The `terminal/` module is 2,305 lines, 517 of them tests. It holds 60 frames a second through a 5 MB/s stream ([PERFORMANCE.md](PERFORMANCE.md#terminal-floods)).
- **Classification:** component limitation (an ecosystem gap: third-party widgets trail Kit's versions).
- **Upstream:** not applicable. The view knows nothing about Kubernetes, so it could be offered as a widget.

## K23 Covered or locked windows draw nothing

- **Found in:** live checks of browsing steps 2 and 3, and the performance pass.
- **Symptom:** captures of a live page taken with the screen locked or the window covered showed only the first frames, before any read finished. A stress run on a locked screen draws nothing, so its drawing numbers mean nothing; the `table.*` spans still measure.
- **Source (GPUI):** intended and documented. `gpui-pre` 0.3.7 `src/platform.rs:85-112` (`WindowVisibility`: on macOS it comes from `NSWindow.occlusionState`, and the platform requests no frames while the window is hidden). The same comment says that Windows, and X11 with a compositor, keep reporting a covered window as visible.
- **Freshkube workaround:** `scripts/stress.sh` refuses to run on a locked screen, and smoke tests confirm that the screen is unlocked first ([Smoke tests](../AGENTS.md#smoke-tests)).
- **Classification:** platform limitation. GPUI documents it; this entry records the cost to checks and measurements.

## K24 Path vertices are copied into a fresh vector every frame

- **Found in:** the Monitoring hover profile for #22 ([PERFORMANCE.md](PERFORMANCE.md#a-pointer-move-redraws-no-panel-22)).
- **Symptom:** a frame that only moves a small overlay still pays for every path on screen. The test was a release build on an Intel Mac, sampled for 10 s on the main thread while the pointer moved over an invented dashboard: 30 line charts, a few dozen stroked series each, about a third of them on screen.
  - `MetalRenderer::draw` took 23% of the main thread's samples.
  - 12% was `RawVecInner::finish_grow` → `realloc` → `memmove` under `draw_paths_to_intermediate`.
  - 8% was the `map` that turns each `PathVertex` into a `PathRasterizationVertex`.
- **Source (GPUI):** `gpui-pre-apple` 0.3.7 `src/metal_renderer.rs:768-778` (`MetalRenderer::draw_paths_to_intermediate`).
  - Each path batch, in every frame, starts from `Vec::new()` with no capacity and `extend`s it with every vertex of every path in the batch, so it grows by doubling through `realloc`. It is then copied once more into the instance buffer (`writer.write(&vertices)`).
  - The wgpu and DirectX renderers build theirs the same way: `gpui-pre-wgpu` 0.3.7 `src/wgpu_renderer.rs:1842-1845` and `gpui-pre-windows` 0.3.7 `src/directx_renderer.rs:636-639`. Only the Metal one was measured.
- **Freshkube workaround:** draw fewer vertices.
  - Series that look the same unfocused share one path, so a crowded panel's grey series draw as one line and one area ([MONITORING.md](MONITORING.md)).
  - A cap on the series a chart draws: the 30 with the highest peaks, with the rest one click away ([MONITORING.md](MONITORING.md), The series cap).
  - Moving the cursor builds no path.
- **Measured again with the grey lines:** the profile above ran on the stress binary's `thirty.json`, whose crowded charts had lost their shared grey path to [K25](#k25-a-path-past-65536-vertices-fails-to-build). With the greys drawn and each chart capped at 30 series, `MetalRenderer::draw` took 52–61% of the main thread's samples, the vertex `map` 38–49% and `finish_grow` about 1%. The window drew about 8 frames a second while the pointer moved, down from 24 ([PERFORMANCE.md](PERFORMANCE.md#the-hover-with-the-grey-lines-drawn-22)).
- **Copies a replayed frame makes:** in the hover no plot paints, and every frame replays the panels from GPUI's view cache. A path vertex on screen is copied four times a frame, 272 B in all:
  1. `Scene::replay` clones the primitive (32 B, `gpui-pre` 0.3.7 `src/scene.rs:141-149`).
  2. `Scene::insert_primitive` clones it again into `paths`, keeping the first in `paint_operations` (32 B, `src/scene.rs:87-138`). Each scene therefore holds every path twice.
  3. The renderer's `map` into a fresh vector (104 B).
  4. `writer.write` into the instance buffer (104 B).

  On `thirty.json`, about 9 panels of 99,312 vertices reach the renderer: about 240 MB copied a frame. Most of the greys' extra memory is these copies and the instance buffers that hold them ([PERFORMANCE.md](PERFORMANCE.md#where-the-grey-lines-memory-goes-22)).
- **Classification:** framework issue.
- **Upstream:** not reported. Suggested fix, smallest first:
  1. Reserve the total up front: `Vec::with_capacity(paths.iter().map(|p| p.vertices.len()).sum())`.
  2. Keep the vector on the renderer and `clear()` it each frame, so its capacity survives between frames.
  3. Write the vertices straight into the instance buffer, with no intermediate vector.
  4. Share a path's vertices between the scene's copies (an `Arc<[PathVertex]>` in `Path`), so replay and `insert_primitive` copy a pointer.

  Fixes 1 and 2 remove the regrowth, now about 1% of the main thread. Fix 3 removes copy 4, about 38% of the bytes; the `map` stays. Fix 4 removes copies 1 and 2 and halves each scene's share of the heap.

## K25 A path past 65,536 vertices fails to build

- **Found in:** the series cap's captures (#245). After Show all, a `thirty.json` chart drew its two coloured lines and no grey ones.
- **Symptom:** a chart's grey series share one stroked path ([K24](#k24-path-vertices-are-copied-into-a-fresh-vector-every-frame)). With 65 of them at the 600 samples a panel asks for by default, `PathBuilder::build` returned `Too many vertices`. The plot turned the error into no path and drew nothing, with no message. The 28 grey series under the cap built, at 91,992 triangle vertices.
- **Source:**
  - `gpui-pre` 0.3.7 `src/path_builder.rs:262` and `:308`: `PathBuilder` tessellates into lyon's `VertexBuffers<Point<Pixels>, u16>`, then expands the indices into a triangle list.
  - `lyon_tessellation` 1.0.22 `src/geometry_builder.rs:338-346`: a new vertex whose index passes `u16::MAX` fails with `GeometryBuilderError::TooManyVertices`.
  - So one path holds at most 65,536 distinct vertices. The triangle list it builds can be longer, since triangles share vertices.
- **How much fits:** a stroked sample takes about 2 vertices on a flat straight line and about 56 on a smooth line that jumps 100 px at every sample. In one path that is from 32,768 samples down to 1,172.
- **Freshkube workaround:** `monitoring/panel/plot.rs` builds a group's lines a chunk of series at a time, about 4,000 samples each. It halves a chunk that still fails, and joins the chunks' triangles into one path, so the group still draws in one pass. Areas are halved only on failure, since chunks whose areas overlap would fill twice. A path that fails anyway is reported once per chart on stderr and fails a debug assertion.
- **Classification:** framework issue. The `u16` limit is lyon's choice of index type, and `build` does return the error; the cost is that nothing near `PathBuilder` says one path has a limit.
- **Upstream:** not reported. Possible fixes: tessellate with `u32` indices, since `build` expands them into a vertex list anyway, or document the limit on `PathBuilder`.

## Strengths observed

The evaluation weighs these against the friction above:

- **State model:** entities changed through `update`, `cx.notify()`, `observe` and `subscribe`, with weak handles in closures, fit Rust's ownership rules; there are few borrow-checker fights and no shared mutable state passed around.
- **Rendering model:** each frame rebuilds the element tree from current state, with no hidden reactive graph to debug.
- **Styling:** the builder API (`.px()`, `.gap()`, `v_flex()`) is quick to write, and GPUI Kit's components and theming produce a polished result.
- **UI tests:** headless tests render the real app, find elements by id and click or type into them; the desktop app's suite of about 168 tests runs in about 3 seconds.
- **No UI boundary:** screens call core logic directly with Rust types, unlike a WebView-based frontend.
- **Drawing our own:** where no Kit component fits, GPUI's lower layers carry the load: `uniform_list` for tables, `VirtualList` with measured heights for logs, and a canvas of quads and `shape_line` runs for the terminal. They handle a 50,000-row list, 10,000 log lines a second and a 60-frame terminal ([PERFORMANCE.md](PERFORMANCE.md)).
- **Scaling:** Kit sizes its controls from the theme's font size, which `Root` makes the rem size. Once the app's own lengths were rems, the text-size setting scaled the whole window like a zoom (`text_size.rs`).

## Observations without a verdict yet

- **Large single-line YAML documents (G04):** the ignored diagnostic `yaml_large_long_line_unicode_find_typing_and_undo` (`workspace/yaml.rs`) mounts the production YAML view with 1 MiB and 5 MiB single-line sources. Creating the view, drawing a frame, and making an end-of-buffer edit took 5.437 s and 27.233 s. The whole find/undo flows took 13.511 s and 64.072 s. These are unoptimized, headless, single runs, not native typing latency. Measure them in release mode on a native window under G11 before attributing the cost to the Kit editor, GPUI text layout, or the test harness.
- **Disabled state in UI queries (G02):** `ElementSnapshot::disabled()` returns `Some(true)` or `None`, never `Some(false)` (`crates/base/src/test_support.rs:106-109`, `:317`). The G02 tests could not assert the disabled state of fixture buttons. They check that an attempted action leaves the model and clipboard unchanged instead. Whether Kit `Button` exposes its disabled state to queries has not been determined.
- **Focus-in listeners with an inactive window (G03):** when Tab was delivered to an inactive native window by the automation tool, `on_focus_in` listeners that scroll a control into view did not run. `PodBrowser::reveal_focused_control` (`pods/table.rs`) scrolls the newly focused control directly. This was seen only with tool-injected input, not in foreground use.
- **Wrapping rows wrap early (browsing step 2a, text size):** a `flex_wrap` row of controls wrapped before it was full, and at 20 px text in the smallest window the Resources toolbar ran past the edge. The commit that replaced it (`79c7b76`) puts this down to GPUI measuring a wrapping row without its gaps. But taffy 0.13's flexbox adds the gaps when it sizes a wrapping container (`src/compute/flexbox.rs:979`, `:1005`, `:1180`), so the cause is unconfirmed. The Resources toolbar now chooses its layout from `content_width` in render. Reproduce it with a minimal row before recording it as friction.
- **Log floods keep the main thread busy (performance pass):** at 10,000 lines a second a log keeps up, but frames come about 15 times a second and a key press can wait about 130 ms. A profile put 35% of the main thread in `CAMetalLayer nextDrawable`, 23% in shaping new rows and about 27% in GPUI's layout and paint ([PERFORMANCE.md](PERFORMANCE.md#logs-keep-up-with-a-flood)). Before attributing the drawable wait to GPUI's Metal renderer, compare it with a minimal GPUI window that draws new text every frame.

## Reviewed and not recorded as friction

- **Command palette query focus (G03):** Kit's `Command` does not take focus when it is mounted in a `Dialog`. The Command story and `website/component/command.md:101-119` show focusing it explicitly. Freshkube's omission was an application bug, now fixed in `Workspace::open_palette`.
- **Measurement canvas origin (G03):** an `.absolute()` element without insets is placed at its static position, as in CSS. Freshkube's workspace measurement canvas now sets `top_0().left_0()`. This was an application bug.
- **Unclamped `set_dock_size` (G03):** `DockArea::set_dock_size` is documented as unclamped against the area, with only a minimum-size floor (`crates/base/src/dock/dock_placement.rs:152-160`). Restored and reopened docks are clamped by Freshkube with `DockSizing` (`Snapshot::clamp`, `Workspace::clamp_geometry`). This is expected application work.
