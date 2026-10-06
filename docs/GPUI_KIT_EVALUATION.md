# GPUI Kit evaluation

Written on 2 October 2026, after browsing steps 1–5, the workflow pass, the performance pass, text size and the terminal view (`6fda2db`). The app pins `gpui-kit` =0.7.0 on GPUI `gpui-pre` 0.3.7 and has only been built and run on macOS. The evidence is the [friction log](GPUI_FRICTION.md) (K01–K23), [PERFORMANCE.md](PERFORMANCE.md), [LONG_LISTS.md](LONG_LISTS.md) and [POD_EXEC.md](POD_EXEC.md).

## Verdict

**Continue with GPUI Kit, limit its scope, and report a short list upstream.**

- Nothing in the log blocked a feature. Every entry has a workaround in the app, and the three targets in PERFORMANCE.md are met. Every performance fix was in our own code.
- The costs cluster in two places. One is Kit's data components (DataTable, DockArea), which this app no longer uses. The other is keyboard, focus and layout details that fail silently. Kit's controls, theme, dialogs and palette have served well.
- No entry has been reported upstream. A few are small, clear and worth sending. Contributing code is worth it mainly for the table fixes, and only if we want DataTable back.

## What served well

- **Headless UI tests.** They render the real app, find elements by id and click or type into them, without a window. The desktop crate's 309 tests, 209 of them UI tests, ran in 17 seconds in the last full run. The keyboard paths of the workflow pass and the terminal's keys, selection and resizing are checked there rather than by hand (K16).
- **Fast enough, once measured.** These are release-build figures from PERFORMANCE.md:
  - a filter keystroke at 50,000 rows rebuilds in 2.8 ms median;
  - a first list of 50,000 pods costs the main thread 1.1 ms;
  - watch bursts on 20,000 pods use 23–37% CPU;
  - logs keep up at 10,000 lines a second, about 6 ms behind;
  - the terminal holds 60 frames a second at 5.3 MB/s, painting in 5.8 ms median;
  - memory stays flat, between 74 and 116 MB through floods.
- **Primitives under the components.** Where Kit has nothing that fits, `uniform_list`, `VirtualList` with measured heights and a canvas of `shape_line` runs carried our tables, logs and terminal.
- **State model.** Entities with `update`, `notify`, `observe` and `subscribe` fit Rust's ownership rules. Screens call `freshkube-core` with Rust types and no serialization boundary.
- **Theme and scaling.** Kit sizes its controls from the theme's font size, so the text-size setting became a whole-window zoom in four commits (`6dadfba`–`0090d8e`).
- **Kit controls used as they are:** Button, Input, Select, DropdownMenu, Tooltip, Popover, Switch, Checkbox, resizable splits, Dialog and the Command palette. The only friction with these was K03, K08, K20 and K21.

## What it cost

The log records workarounds, not hours. Of the 23 entries, 9 are documentation gaps, 7 component limitations, 5 framework issues and 2 platform limitations. Each documentation gap is reasonable behaviour once understood, but each took a source read to find.

| Kind | Entries | Cost |
| --- | --- | --- |
| Kit data components | K01, K02, K09, K10 (DataTable); K06, K07 (DockArea) | Found in the prototype, whose table needed key overrides, a copy of the column widths, an identity layer over positional selection and its own click listener. K10 was never solved. This app uses neither component: resource lists are hand-built `uniform_list` rows selected by identity. |
| Rendering model | K11, K12, K24; the log-flood observation | Slow is the default: every notify redraws the window, caching is opt-in, and debug builds draw about 14× slower. The answer is the rules in AGENTS.md's [Keep render cheap](../AGENTS.md#keep-render-cheap), render probes in tests and a stress harness. Laying out a log row cost 0.27 ms, which capped logs at about 3,000 lines a second until only on-screen rows were measured. A flood still keeps the main thread busy. Every path costs vertex copies each frame, so crowded charts draw fewer series. |
| Keyboard and focus | K05, K17, K18, K21 | Mostly silent failures. Quit did nothing (K05). Page keys stopped working while a placeholder showed (K17). A terminal needs an app-wide keystroke interceptor (K18). A dialog's focus return depends on call order (K21). |
| Layout and sizing | K03, K08, K19; the `flex_wrap` observation | Pixel-only APIs are converted at every render (`ui::dp_px`), and Selects are wrapped in fixed-width containers. A scroll reveal flattens rows and waits a frame. The Resources toolbar chooses its layout in render. |
| Tests and checks | K04, K15, K16, K20, K23 | The harness rules in AGENTS.md's [UI tests](../AGENTS.md#ui-tests): reduced motion (two dialog tests failed every run without it), render probes, outcome checks instead of the disabled state, real input events, and menus driven through view methods. Native checks need a window that is in front and a screen that is unlocked. |
| Gaps and docs | K13, K22 | We learn the API from registry source and keep exact pins. No terminal widget builds against Kit 0.7.0, so we wrote `terminal/`: 2,305 lines, 517 of them tests. |
| Integration | K14 | GPUI's executor and Tokio both run; requests use `OwnedJob` and target epochs. |

## Recommendations

### Report or contribute upstream

Kit items go to `longbridge/gpui-kit`, the Kit crates' repository. Send GPUI items there too until it is clear who takes changes to the `gpui-pre` snapshot, whose manifest names the Zed repository. Each needs a minimal repro first; none has one yet.

| Entry | What to send | Why |
| --- | --- | --- |
| K01 | Issue and a small PR: make the column key handlers check `col_selectable` | A plain bug with a one-line check |
| K10 | Issue or PR: a selection setter that neither scrolls nor emits | Any table with live data needs it |
| K02 | Issue: `TableState::refresh` discards resized widths | Any table that sorts from its own header loses what the user set |
| K03 | Issue: `Dialog::w` shadows `Styled::w` and takes only pixels | One-line compile repro; at odds with Kit's own rem guidance |
| K20 | Issue or PR: an id on `PopupMenuItem` | Testability, which Kit's TESTING.md promotes |
| K05 | Issue: a global action that updates the dispatching window fails silently with "window not found" | Silent and misleading |
| K04, K06, K07, K08, K17, K19, K21 | One documentation issue or PR | Each cost a source read, and each fix is a sentence or two |

K18 and the drawable wait during log floods are for discussion once a minimal repro shows them. They are not issues yet.

### Keep app-side

- **K09:** selecting by identity is a resource browser's requirement, not a Kit defect.
- **K11, K12, K14:** these are GPUI's model. AGENTS.md's rules cover them.
- **K13:** pin exact versions and read the source.
- **K16, K23:** these are platform behaviour. Keep interaction checks in UI tests.
- **K22:** the terminal stays ours. It knows nothing about Kubernetes, so it could be offered as a widget after pod exec's live check.

### Limit scope

- Use Kit for controls, theme, dialogs, menus, the Command palette and resizable splits.
- Draw our own data views: tables with `uniform_list`, logs with a measured `VirtualList`, the YAML view and the terminal, as [LONG_LISTS.md](LONG_LISTS.md) describes. Don't move resource lists to DataTable unless K01, K02 and K10 are fixed upstream.
- Don't adopt DockArea for a multi-pane workspace without checking K06 and K07 again.
- Don't wrap rows of controls with `flex_wrap`. Choose their layout in render from `content_width`.
- Keep every pixel-only Kit API behind `ui::dp_px`.

### Re-evaluate when

- **The pins move off `gpui-kit` =0.7.0 or `gpui-pre` 0.3.7.** Run the UI suite and the `scripts/stress.sh` workloads again, and recheck each entry's source citations. K13 expects breaking changes.
- **Linux or Windows builds start** (a roadmap follow-up). The toolkit has not been built or run on another platform here, and GPUI documents that covered windows behave differently there (K23).
- **The busy main thread during a log flood becomes a blocker,** or the drawable wait turns out to be inside GPUI's renderer.
- **A planned feature needs a Kit component with recorded friction,** such as DataTable or DockArea.
- **Upstream answers the reports above,** which may let us drop workarounds.
