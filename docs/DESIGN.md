# Freshkube Fog

Fog is the design direction chosen on 3 October 2026: calm grey content, a blue-slate frame, dense tables and pastel semantic colour. Figtree carries the interface; IBM Plex Mono carries resource names, numbers and code. Colour identifies status, selection or resource type. The visual language must remain distinct from Aptakube, Lens and Rubick.

The reference is the user's **Freshkube holistic layout-selection.pdf** (1280×880 fictional window comps). G7/G7c, G8 and H1–H7 are canonical. G2b/G3b only introduced bullet meters and quieter links; G–G6 are earlier palette studies. H and G7–G8 win over G2b/G3b, which win over G–G6. The written rules win when a mock disagrees; the comps were not pixel-QA'd. This supersedes the earlier Console direction.

## Tokens and type

| Role | Dark value |
| --- | --- |
| Content | `#24272D` |
| Header, rail, status bar | `#1B2029` |
| Cards and tables | `#2C3037` |
| Raised, table headers, inputs | `#343943` |
| Hover | `#3C424D` |
| Tooltips and popovers | `#444A55` |
| Hairline / control border | `#393E47` / `#4A505B` |
| Primary / secondary text | `#EEF1F5` / `#CDD3DC` |
| Muted text | `#A0A7B2` |
| Decorative faint text | `#737A85` |
| Accent, selection, focus, prose links | `#8AB4F8` |
| Text on solid accent or status fills | `#14223B` |
| OK | `#82D4AB` |
| Warning, at risk | `#F2C46D` |
| Critical | `#F28B82`; text `#F5A097` |
| Integration required / memory resource | `#B7AAF7` |

Status tints use 14–16% opacity and light semantic text. Navy is for **solid** fills; navy on a translucent dark tint fails contrast. Muted text is about 5.5:1 on cards, but falls below AA on hover and tooltip surfaces: use secondary or primary text there. Faint is only for decorative suffixes and separators.

Figtree Regular, SemiBold, Bold and Black are embedded as static faces (GPUI loads the variable face only at its default weight); IBM Plex Mono Regular and SemiBold are embedded with their OFL licences. Base UI 13 px, tables 12.5 px, secondary 12 px. Captions 11 px, uppercase bold with 0.06em tracking. Section titles 14 px bold; screen titles 20 px black (900). Numeric columns use monospace/tabular figures. The user's text-size preference scales layout through `ui::dp`; 13 is the new default, and existing saved sizes remain valid.

Use a 4 px grid, 16–20 px content padding, 12–14 px card padding and 10–12 px gaps. Radii: cards 12 px, controls 8 px, rail buttons 10 px, meters 3 px, chips fully rounded. Shadows belong only to tooltips and popovers. The previous light palette remains available until a Fog light theme is designed.

## Frame

- Header 52 dp: context and “Talos + Kubernetes”, Dashboard / Nodes / Workloads / Events / Observability tabs, relevant time range, Search (⌘K), refresh and settings. Tabs become icons in narrow windows.
- Rail 64 dp: icon-only areas, tooltips, blue active wash; status dots indicate a problem. Existing Kubernetes groups, Custom Resources, Talos pages and Prometheus dashboards stay reachable.
- Contextual sidebar 208 dp or a 52 dp icon strip. ⌘B and the title's collapse button toggle it. The choice is remembered in `navigation.json` beside preferences; narrow windows auto-collapse but can be expanded manually. Workloads adds namespace counts from the existing summary. Collapsed namespaces become “all”; Observability data sources become one database button. Collapsed tooltips open to the right.
- Status bar 28 dp: connection, port forwards, last refresh.

## Status and links

| State | Glyph |
| --- | --- |
| OK | Filled mint dot |
| Warning / at risk | Amber outlined triangle |
| Critical / failing | Filled coral diamond |
| Pending / unknown | Hollow grey circle |
| Completed | Grey tick |
| Integration required | Lavender outlined square |
| Log errors | Blue dot |

Shape and colour communicate severity; words explain the cause when the group header has not already done so. Logs use neutral source identifiers. Prose links, breadcrumbs, selection and focus are blue. A chosen chip or outline toggle takes the primary outline (`ui::choice`); a chosen segment of a ghost segmented control on a `surface_2` track takes the accent tint (`ui::segment`), since Kit's own selected ghost colour matches that track. Dense cross-references use secondary text with dotted underlines and become blue on hover.

## G7: tables

Pods start on Problems when problems exist, grouped by cause, with healthy rows folded. The toolbar contains title, text filter, glyph/count filters, comfortable/compact density and Columns. A blue-tinted banner states how many rows are shown and provides Show all; selected rows expose bulk copy/clear actions. Comfortable rows are 34 dp, compact 26 dp, table headers 30 dp; group headers take the row height, since the list is uniform.

The status is a glyph, replaced by a checkbox on a marked row. Ready and restarts share a cell (`0/1 ↻14`). Owner prefixes are muted (`deploy/`, `sts/`). Namespace prefixes are muted; random suffixes are faint. Remove a common node prefix only when detected and keep the full name in its tooltip. Sort arrows, a blue focus outline, ↑↓, Enter, X and L keep their existing behavior.

Resource meters are 44×10 dp capsules. A resource-coloured 28% band extends to the request; a 4 dp bar shows use; the end is the limit. CPU is blue and memory lavender. Above request, brighten that resource's bar; at ≥85% of a known limit, use amber. Stale data is diagonally striped in its resource colour. Values remain neutral. No request ticks. Tooltips name used/requested/limit, freshness and missing limits; the footer explains the states. Unknown limits must be identified, never represented as known capacities.

## G8: pod detail

Open on the cause: state, last exit, next restart, termination output and a Logs action. Continue with the restart timeline, containers, Runs on (node and Talos kubelet health), relationships and recent events. The retained resource pane keeps its watch and shell/forward protections. Previous-instance logs are read only through an explicit Logs action; unavailable history is not invented. The node memory bullet uses Talos used/physical capacity when available and explicitly identifies unavailable requests.

## H1–H7: observability

These use Coroot's concepts in Fog's visual language. Applications, the service map and supported application-report evidence have a read-only Coroot connection; [COROOT.md](COROOT.md) records transport, identity, limits and validation. The retained page owns requests and prepared display data. H4–H7 remain interactive fixture previews and show explicit limitations in live mode. Rollback/configuration previews never execute writes; threshold and mute choices remain local to example mode.

| Screen | Interaction |
| --- | --- |
| H1 Applications | Shared fixture/live projection, stable AppIds, 12 check columns including separate disk usage and I/O. Problems first, grouped by Coroot category, with text/namespace/category/status filters. A cell shows its glyph alone unless Coroot supplied a figure, so healthy, unknown and not reported stay distinct without truncated words; the tooltip carries the full state. |
| H2 Service map | Layered left to right: a node's column is its longest chain of callers, tall columns wrap at eight, each column is ordered by its callers' rows, and unconnected nodes share a last column. Curved links end in arrowheads; width follows traffic and problem links are dashed. The layout uses every link between the page's nodes, so the problem filter never moves a box. The Connections inspector sits beside the map only when the whole map fits, otherwise below it; the map scrolls inside its card. Link selection survives reorder; nodes open application reports. |
| H3 Application | Report tabs carry each report's glyph. One evidence card lists the REST report first and, under *More checks from Coroot*, only what its MCP endpoint adds; each source's freshness, error and Retry sit in the card's footer. Rows give a title, the source's own words and only the reported figures, with units (ms, s, rps). Dependencies and clients link to their report. Explicitly mapped subjects open Kubernetes objects through the shell guard; complete histories and live mutations are unavailable. |
| H4 Incident | Two incidents, SLO compliance and 1h/5m burn, root cause, cause chain, previewed fixes and ruled-out evidence. Burn threshold 14.4×. |
| H5 Deployments | Four releases, selectable revisions, actual diff of the fictional Deployment specs, full selected YAML and rollback preview. |
| H6 Profiling | Comparison toggle, CPU history, biggest increases, function search and a flame graph with zoom/reset. |
| H7 Traces | Latency/error heatmap with SLO line, selectable buckets and error causes, matching sample waterfalls and span inspection. |

The later fixture histories end at 15:00, with the worker deployment at 12:52 and node event at 14:48; header ranges regenerate that bounded example history. Live Refresh/reopen captures the current time interval, while report navigation and retries retain the displayed UTC interval. Applications are virtualized; maps and report evidence have explicit bounds. Hiding cancels ordinary requests, and changed provider/access/project/range generations reject old results.

Debug entry points:

```sh
FRESHKUBE_THEME=dark FRESHKUBE_TEXT_SIZE=13 FRESHKUBE_WINDOW_SIZE=1280x880 \
  FRESHKUBE_PAGE=observability cargo run -- --fixture
# observability-service-map, observability-application, observability-incidents,
# observability-deployments, observability-profiling, observability-traces
# FRESHKUBE_SIDEBAR=expanded|collapsed overrides only this debug session.
```

## Charts

Use 2 px series lines, solid hairline grids, recessive axes and legends for multiple series. Deploy markers are blue and node events coral: a fine vertical line and bottom triangle. Text stays neutral.

Two series use `#5E93E6` and `#CC7C4A`. Additional dashboard series remain neutral until focused in the existing legend; this avoids introducing an unreviewed larger categorical palette. Ordered quantiles use a blue ramp (`#5379BB`, `#7AA0E6`, `#B3CEFA`).

Heatmap levels: `#323845`, `#33466A`, `#3D5C92`, `#5379BB`, `#7AA0E6`, `#B3CEFA`; errors use coral steps from `#3A3036` to `#F28B82`. Flame differences: less `#3F65A0` / `#3F5A80`, same `#454B56`, more `#7E4A49` / `#A64B46`, with white labels. Status colours never stand in for ordinary chart series.

## Components

Every page looks like Pods. The pieces Pods and Monitoring draw are shared components in the `freshkube-ui` crate, which depends on neither the app nor core, so a page uses only what it exports ([#46](https://github.com/skel84/freshkube/issues/46)). Components are render helpers and small state structs owned by the page's existing entity; none is an entity of its own. Sizes below are `dp` unless marked `px`, measured from what Pods and Monitoring draw today.

### Page types

| Type | Example | Body |
| --- | --- | --- |
| Table page | Pods, every Resources kind | `PageHeader`, then any banner, then one `DataTable` in a card. A detail pane opens beside it (content ≥ 900 wide: 460 default, 320 minimum, 14 gap) or below it (190 list, 380 pane). |
| Dashboard | Monitoring | `PageHeader` with a `Breadcrumb`, a row of variable chips, then a grid of `StatCard` and `ChartCard` with 8 between them. Section rows are 36 high, with a chevron and a 14 bold title. |
| Canvas | Service map, flame graph, heatmap | The page draws its own picture inside a card, with explicit bounds, under the same header, padding, status glyphs and states. It never draws its own title or toolbar. |
| Detail pane | The pod pane | A card: kind caption, 13.5 monospace title, a state tag, actions (xsmall outline), then 32-high tabs with a 2 px accent underline on the chosen one and muted text on the rest. |

**The page frame.** Padding 26 left and right (`PAGE_PADDING`), 22 top, 18 bottom, and 14 between the header, banners and body, on the content background. Monitoring's 20/14/10 on `surface_2` moves to this when it migrates.

**Cards.** 12 px radius, a `line` hairline, `surface` fill, no shadow (`screens::panel` today). Card padding 12–14.

### Components

| Component | What it draws | Sizes and ids |
| --- | --- | --- |
| `PageHeader` | Left: title (20/28, weight 900), filter input, segment (`Problems` / `All`), `StatusChips`. Right, after a spacer: the source label or namespace picker, Density, Columns, the time picker where the page has a range, and refresh. Below: the meta line. | One row, 8 between items; below 920 content width it stacks title and filter, chips, then controls. Filter 150 (flex when stacked), namespace 132 with a 260 menu, all controls `small`. Meta line 11 muted: `context · 12 pods · state · 14:02:11`, with `Example data` or `Not connected` in place of the context. Ids `<page>-title`, `<page>-filter`, `<page>-namespace`, `<page>-density`, `<page>-columns`, `<page>-refresh`, `<page>-scope`. |
| `Breadcrumb` | Parent, `/`, then the title, for a page inside a collection (a dashboard, an application report). | Parent 12.5 muted, a link that turns blue on hover; separator faint; title as `PageHeader`'s, truncating, at least 120 wide. |
| `StatusChips` | One ghost chip per status: its glyph and count. Pressing one filters to it; pressing it again clears. | Chips `small`, 6 horizontal padding, 2 apart; 12 monospace counts in `ink_2`. Tooltip `3 failing · click to filter`; accessibility label `3 failing`. Order: critical, warning, waiting, OK. Ids `<page>-tally-<what>`. |
| `DataTable` | The table card: any notes (`SelectionBar`, `ShowingBar`), the header row, the virtualised rows, the footer `Legend`. Scrolls horizontally inside the card when its columns are wider than the view. | Header 30 on `surface_2` with a bottom hairline; captions (11 bold uppercase muted) with a 12 sort arrow. Rows 34 comfortable, 26 compact, monospace 12.5. Cells pad 12 horizontally and truncate; each column has a fixed width or is the one flexible column. Widths are derived when the data changes: 7.5 a character plus 24, between 64 and 280 (440 for the flexible one); a glyph column is 34. Selected row: `accent_soft` with a 1 px accent border; marked: `hover`; hover: `hover`. Selection follows a row's key, never its position. An empty table shows one 12.5 muted line, padded 12 × 14. Ids `<page>-list`, `<page>-rows`, `<page>-table-scroll`, `<page>-sort-<n>`, rows from their key. |
| `GroupRow` | A group's status glyph, its label (semibold, in the status's text colour), an optional monospace subject (a node), then `· 12 pods · collapsed` muted and truncating, then xsmall ghost actions (Select all, Expand / Collapse, Open node). | The row height, so the list stays uniform. 12 horizontal padding, 10 gap, 12 text, `track` at 45 % with a bottom hairline. Role heading. Ids `<page>-group-<key>`. |
| `ShowingBar` | `Showing 40 of 212`, and `Show all 212` when rows are folded. | `accent_soft`, 12 horizontal × 5 vertical padding, bottom hairline, 12.5 `ink_2`, xsmall ghost action. Role status. Id `<page>-collapsed`. |
| `SelectionBar` | `3 selected` (semibold), Copy names, Clear. | As `ShowingBar`. Id `<page>-marks`. |
| `Legend` | The table's footer: what its marks mean, with live examples (meters, stripes). | Top hairline, 12 × 7 padding, 12 gap, 11 muted, wrapping. Under 600 content width it becomes one 26-high line ending in ⓘ, with the full legend as its tooltip. Id `<page>-meter-legend`. |
| `StatCard` | A muted 12 bold title, then one figure (22, weight 900) or a wrap of named figures (18, each at least 96 wide, 20 × 8 apart), each with an optional tag, gauge or sparkline. | Card; header 28 with 14 left padding; body 14 × 10. |
| `ChartCard` | A 13 bold title, the unit, an info mark with the query, the stale mark at the right; the plot, then its legend. | Card; header 32 with 12 left padding; plot padded 12, at least 64 high. |

### States

| State | Where | How it looks |
| --- | --- | --- |
| Empty | Instead of a page's body | `ui::empty_state`: centred, at most 480 wide, a 20 icon, a 19 bold title, the description, then actions. Say what was looked for and where. |
| Loading | First read of a table or card | Skeleton bars (5 px radius) in the card's shape: a table card shows nine bars of 70 % × 12, 12 apart; a card shows a 40 % title bar and a body bar. Never a spinner alone. |
| Stale | Data kept after a failed refresh | A warning banner above the body (`ui::warning_banner`): what happened, `Showing pods as last seen at 14:02:11.`, the reason, Retry. In a card: the warning glyph at the header's right with the reason as its tooltip. Stale meters are striped. |
| Refused | 403 | An empty state with the shield icon: `Not permitted to list pods`, that this says nothing about whether any exist, and the server's reason in a monospace `crit_soft` box. |
| Failed | Nothing known yet | An empty state: `Couldn't list pods`, that nothing is shown as missing, the reason, and Retry. In a card: one centred 12 line with the critical glyph. |

Each state's element has role status and an id `<page>-<state>`.

### Settled values

- **Padding:** 26 on every page (`PAGE_PADDING`), with 22 / 18 / 14 as above.
- **Card radius:** 12 px, including Monitoring's cards and Overview's.
- **Time range:** one picker in the page header's right group, before refresh, as Monitoring's: an outline small button with a clock, the range and a caret, opening a checked menu. The shell's 1h / 3h / 24h / 7d chips go when Observability migrates.
- **Refresh:** one per page, the page header's ghost icon button, tooltip `Refresh pods`. A page with auto-refresh puts its interval menu beside it.
- **Group rows:** the row height (34 or 26), not a separate 28, so every list stays a uniform list.
- **Headers on other Resources kinds:** they keep today's header (title with an inline scope line, Refresh with a label) through the extraction, which changes nothing visible, and take `PageHeader`'s form in their own change.

## Open

- Whether integration-required lavender should become soft teal, to separate it from memory.
- Default row density; comfortable remains the initial value.
- More causal grouping beyond pod state and NotReady nodes.
- Fog light theme.
- Later Coroot destinations and a reviewed workflow for any fixes. The first live slice is read-only; [COROOT.md](COROOT.md) records its supported evidence and remaining API gaps.
