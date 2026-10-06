# Freshkube Fog

Fog is the design direction chosen on 3 October 2026: calm grey content, a blue-slate frame, dense tables and pastel semantic colour. Figtree carries the interface; IBM Plex Mono carries resource names, numbers and code. Colour identifies status, selection or resource type. The visual language must remain distinct from Aptakube, Lens and Rubick.

The reference is the user's **Freshkube holistic layout-selection.pdf** (1280×880 fictional window comps), exported from the [Freshkube holistic layout canvas](https://claude.ai/artifact/U5UUnSURiNJJ3qtVxMRbi1), which holds every board named here and is private to its owner until shared. G7/G7c, G8 and H1–H7 are canonical. G2b/G3b only introduced bullet meters and quieter links; G–G6 are earlier palette studies. H and G7–G8 win over G2b/G3b, which win over G–G6. The written rules win when a mock disagrees; the comps were not pixel-QA'd. This supersedes the earlier Console direction.

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

Figtree Regular, SemiBold, Bold and Black are embedded as static faces (GPUI loads the variable face only at its default weight); IBM Plex Mono Regular and SemiBold are embedded with their OFL licences. Base UI 13 px, tables 12.5 px, secondary 12 px. Captions 11 px, uppercase bold with 0.06em tracking; a table's column labels 11.5 px semibold, as given (`ui::column_label`). Section titles 14 px bold; screen titles 20 px black (900). Numeric columns use monospace/tabular figures. The user's text-size preference scales layout through `ui::dp`; 13 is the new default, and existing saved sizes remain valid.

Use a 4 px grid, 16–20 px content padding, 12–14 px card padding and 10–12 px gaps. Radii: cards 12 px, controls 8 px, rail buttons 10 px, meters and other data marks (flame frames, heat cells, legend swatches) 3 px, chips fully rounded. Shadows belong only to tooltips and popovers. The previous light palette remains available until a Fog light theme is designed.

## Platforms: desktop grade, adapted at the edges

Freshkube is a desktop app and is meant to ship on macOS, Windows and Linux. GPUI draws every control itself, so the app imitates no operating system: it keeps one look of its own on all three, holds it to the conventions desktop apps share, and adapts each platform's differences in one place. Agreed with the user on 5 October 2026, and chosen the same day as the look for every screen over the dashboard style of the platform boards P1–P7, which remain the reference for what each screen holds ([PLATFORM.md](PLATFORM.md#desktop-grade)).

What desktop apps share, and new screens follow:

- **Lists, tables and outlines carry the content.** Selecting a row shows it in an inspector or pane. No dashboards of cards.
- **Actions act on the selection,** from the toolbar, a context menu or a key. An inspector or dialog has at most one default button; rows and cards don't carry their own.
- **The frame states the location.** Pages don't open on a hero heading or a greeting.
- **Desktop density.** Controls 24 dp, toolbars 38 dp, rows 26 dp, column headers in sentence case at 11.5 px semibold, panes padded 10–12 dp.
- **Space belongs to panes.** Splits are resizable and remembered; empty space sits inside a pane, not between cards.
- **Every action has a key,** and ⌘K reaches everything.

What differs, handled by one platform module and never by a screen:

| Concern | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Window controls | Traffic lights, leading | Caption buttons, trailing | The desktop's decorations; client-side when the compositor asks |
| Primary modifier | ⌘ | Ctrl | Ctrl |
| Default button in a group | Trailing | Leading | Trailing |
| App menu | Global menu bar | Menu button in the header | Menu button in the header |
| Credentials | Keychain | Credential Manager | Secret Service |
| Terminal-wide shortcuts | ⌘ | Ctrl-Shift | Ctrl-Shift |
| Packaging | DMG, Homebrew cask | MSI or winget | AppImage, Flatpak or deb |

Bind app keys with `secondary-` and show the platform's label (`⌘K`, `Ctrl+K`). Terminal copy, paste and leave use Command on macOS and explicit Ctrl-Shift bindings on Windows and Linux; the interceptor reserves plain Ctrl for the shell. The bundled faces keep the same metrics on every platform. [#81](https://github.com/skel84/freshkube/issues/81)'s advisory Linux and Windows CI checks compilation, linting and the terminal keymap; full runtime and packaging support remain separate work.

The comps are in the canvas's [Desktop grade](https://claude.ai/artifact/U5UUnSURiNJJ3qtVxMRbi1) section. P1d, P4d and P4w are pages, also in [`platform/`](platform/) as `home-desktop`, `app-delivery-desktop` and `app-delivery-windows`. N1, N1c and N2, the Native pass, are the workspace around a page: a source list, document tabs, the inspector and a bottom panel for logs, the terminal and agent activity. R is the app today, for comparison.

### From today's app to desktop grade

The changes below take what Pods draws today (R) to desktop grade. They are in order, and each is one PR in `freshkube-ui`'s page and table components, except that changes 6 and 12 land together, since both rebuild `PageHeader`'s row, and change 13 can land at any point. Two pages draw with those components today: **Resources** (Pods and every other kind, with its detail pane) and **System services**. **Applications** (Observability, [#68](https://github.com/skel84/freshkube/pull/68)) and **Nodes** ([#69](https://github.com/skel84/freshkube/pull/69)) join them when those PRs merge, and each change below names them where it reaches them. Overview, the Talos screens, Monitoring and Observability's other screens take each change when they migrate; no PR here restyles them by hand.

Each PR updates the rows of [Components](#components) it changes, since that section describes the app as it is, and the numbers in `layout_check` and `scripts/check-style.sh` in the same change.

1. **Sentence-case column headers.** The header cells use a new `ui::column_label` (11.5 px semibold, muted, the label as given) instead of `ui::caption`, which keeps uppercase for section and field labels for now. Pages give labels in sentence case: `Ready`, not `READY`. Resources' server-printed columns already arrive as `Name`, `Ready`. *Resources, System services, Applications, Nodes.*
2. **A 26 header row.** `HEADER_HEIGHT` 30 → 26, still on `surface_2` with a bottom hairline. *Resources, System services, Applications, Nodes.*
3. **One row height, 26.** `ROW_HEIGHT` 34 → 26 and group rows follow it. The Density control, `COMPACT_ROW_HEIGHT` and the saved density go in the same change, and `layout_check` measures one height; the text size scales rows for anyone who wants them larger. Cells pad 10 horizontally instead of 12. *Resources, System services, Applications, Nodes.*
4. **No card around the table.** `data_table` draws the table bare by default, edge to edge across its pane, with hairlines where it meets the toolbar, the inspector and the status bar. A table inside a card of its own asks for `.carded()`, which replaces `.bare()`. *Resources, System services, Applications, Nodes.*
5. **A page frame without margins.** `page::page` drops the 26 / 22 / 18 padding and the 14 gaps: toolbar, any banner and panes stack edge to edge, divided by hairlines, and content inside a pane is padded 10–12. *Resources, System services, Applications, Nodes.*
6. **`PageHeader` as a toolbar.** One 38 row with a bottom hairline: the filter, the segment and `StatusChips` at the left, the controls after the spacer, every control 24 high. A page's `.secondary(row)` becomes a second toolbar row. In a narrow window the controls fold into an overflow menu instead of wrapping onto more lines. The title becomes the toolbar's leading label in the same PR, change 12. *Resources, System services, Applications, Nodes.*
7. **The meta line in the status bar.** The context, count, state and refresh time move from under the header to the status bar's page segment, which the page fills. *Resources, System services, Applications, Nodes, etcd, Security, Health (workloads), and the shell's status bar.* The node pane's tabs keep their line under their header, since the bar there shows Nodes'.
8. **Counts in the table's footer.** `ShowingBar`'s `Showing 40 of 212` with Show all, and `SelectionBar`'s `3 selected` with its actions, move into the `Legend` row: 26 high, on the frame's fill, with a top hairline. The blue strip above the rows goes. *Resources, System services, Applications.*
9. **An inspector instead of the detail card.** A shared `Inspector` takes the pane's frame: the split's trailing pane, with no card, divided by a hairline, padded 12, tabs 28 high. The split stays resizable, and its width is remembered per page in `navigation.json`. Below 900 content width it still opens under the table. *Resources (the detail pane of every kind), Nodes (the node pane).*
10. **Actions on the selection, not the row.** A shared row context menu is built from the same actions as the toolbar, each shown with its key in the platform's label. `GroupRow`'s Select all, Expand and Open node move to it and to keys; a group row keeps its chevron, glyph, label and count. *Resources (Pods grouped by cause), Nodes (Expand and Collapse on Healthy).*
11. **System services without an actions column.** Its row actions, such as Health check, act on the selected row from the toolbar, the context menu and keys, and the column goes. *System services.*

12. **The page title in the toolbar,** in the same PR as change 6. `PageHeader`'s 20 px black title becomes the toolbar's leading label, 13 semibold in `ink`, before the filter in change 6's 38 row. A `Breadcrumb` folds the same way: its parent muted, a faint `/`, then the label. The id `<page>-title` stays, and `layout_check` measures the label instead of the 20 title on its 28 line. *Resources, System services, Applications, Nodes.*
13. **G6 glyphs and the skull,** at any point in the order, since it touches nothing else in this list. `ui::status_glyph`, `status_mark` and `health_mark` draw the round G6 set in [Status and links](#status-and-links) instead of the dot, triangle and diamond. A new `Tone::Died` draws the K1 skull in the critical colour and counts with critical in `StatusChips`. Pods map a container that ran and stopped (`CrashLoopBackOff`, `Error`, `OOMKilled`) to it; a pod that can't start because of an error (`ImagePullBackOff`) stays critical, the disc with a bar, and an unschedulable pod, which prints `Pending`, keeps the dashed pending ring. The style check already keeps these three functions the only places glyphs are drawn, so no page changes its own code apart from Pods' mapping. *Every page that shows a status: Resources (rows, group rows, chips and the pod's cause card), System services, Applications, Nodes, Overview, etcd, Monitoring, Observability, Kubernetes-only, the shell's header and status bar, and every `ui::tag`.*

The user settled the open questions on these changes on 5 October 2026:

- **Page titles:** a label in the toolbar, change 12.
- **The interface face:** Figtree stays, at desktop density. No font change.
- **Status glyphs:** G6 Round, with the K1 skull for a container that died, change 13.
- **Density:** no toggle. Rows of 28 and 26 differ too little to earn a control, and desktop apps don't offer one; change 3 sets one height of 26.
- **Integration required:** stays lavender. G6 draws it as a ring with a plus, so its shape already tells it from the memory meters.
- **Terminal shortcuts:** Ctrl-Shift on Windows and Linux, so the shell keeps plain Ctrl. They belong to [#81](https://github.com/skel84/freshkube/issues/81), not to this list.

## Frame

- Header 52 dp: context and “Talos + Kubernetes”, Dashboard / Nodes / Workloads / Events / Observability tabs, relevant time range, Search (⌘K), refresh and settings. Tabs become icons in narrow windows.
- Rail 64 dp: icon-only areas, tooltips, blue active wash; status dots indicate a problem. Existing Kubernetes groups, Custom Resources, Talos pages and Prometheus dashboards stay reachable.
- Contextual sidebar 208 dp or a 52 dp icon strip. ⌘B and the title's collapse button toggle it. The choice is remembered in `navigation.json` beside preferences; narrow windows auto-collapse but can be expanded manually. Workloads adds namespace counts from the existing summary. Collapsed namespaces become “all”; Observability data sources become one database button. Collapsed tooltips open to the right.
- Status bar 28 dp: the shell's status (the connection, a running operation or a node's log collection), then, behind a 14 dp hairline, the visible page's segment, then port forwards and the last refresh. Below 1080 dp the shell's status is its glyph alone, with its text in the tooltip, so the page's segment keeps the room.

## Status and links

Every glyph is the G6 Round set, chosen on 5 October 2026: one round silhouette for every state, drawn 10 dp, whose inside carries the meaning. `ui::status_glyph` draws it ([change 13](#from-todays-app-to-desktop-grade)): each drawing is a single-colour SVG on a 16 grid in `crates/freshkube-ui/assets/glyphs/`, compiled in and painted as an alpha mask in the tone's colour, so a halo keeps its transparency and a cut-out shows what lies behind it.

| State | Glyph |
| --- | --- |
| OK | Mint dot in a soft mint halo |
| Warning / at risk | Amber ring, half filled |
| Died | Coral skull, `Tone::Died`: a container that ran and stopped (`CrashLoopBackOff`, `Error`, `OOMKilled`). Counted with critical. Pods, Health and Overview's Needs attention use it, from core's `PodIssue::died` where a page reads the summary. The 10 dp skull is the simplified drawing B: K1 with three teeth and no nose, which hold at 10 px where K1's nose and thin teeth blur. K1 itself is kept for a larger size, if one ever appears |
| Critical / can't run | Coral disc with a bar cut out: an image that won't pull, a node down, an etcd alarm |
| Pending / unknown | Dashed grey ring |
| Completed | Grey tick in a faint ring (not drawn yet: Pods shows a tick icon) |
| Integration required | Lavender ring with a plus |
| Information, such as log errors | Small blue dot, `Tone::Info`: something to know that isn't a fault |

Shape and colour communicate severity; words explain the cause when the group header has not already done so. Logs use neutral source identifiers. Prose links, breadcrumbs, selection and focus are blue. A chosen chip or outline toggle takes the primary outline (`ui::choice`); a chosen segment of a ghost segmented control on a `surface_2` track takes the accent tint (`ui::segment`), since Kit's own selected ghost colour matches that track. Dense cross-references use secondary text with dotted underlines and become blue on hover.

## G7: tables

Pods start on Problems when problems exist, grouped by cause, with healthy rows folded. The toolbar contains title, text filter, glyph/count filters and Columns. A blue-tinted banner states how many rows are shown and provides Show all; selected rows expose bulk copy/clear actions. Rows are 26 dp and table headers 26 dp; group headers take the row height, since the list is uniform. There is no density control: the text size scales rows for anyone who wants them larger.

The status is a glyph, replaced by a checkbox on a marked row. Ready and restarts share a cell (`0/1 ↻14`). Owner prefixes are muted (`deploy/`, `sts/`). Namespace prefixes are muted; random suffixes are faint. Remove a common node prefix only when detected and keep the full name in its tooltip. Sort arrows, a blue focus outline, ↑↓, Enter, X and L keep their existing behavior.

Resource meters are 44×10 dp capsules. A resource-coloured 28% band extends to the request; a 4 dp bar shows use; the end is the limit. CPU is blue and memory lavender. Above request, brighten that resource's bar; at ≥85% of a known limit, use amber. Stale data is diagonally striped in its resource colour. Values remain neutral. No request ticks. The tooltip is `meters::tip`, for Pods and Nodes alike: use and its share of the end, the request and its share, the end (`limit none` on a pod without one, `allocatable unknown` on a node), `(last known)` on stale use, and the state in words, `above request` or `at least 85% of limit` (or allocatable), from the same emphasis the bar is coloured by; percentages round down. A page appends its own provenance after it. The footer explains the states. Unknown limits must be identified, never represented as known capacities. Every per-row CPU and memory value uses these shared resource meters.

## G8: pod detail

Open on the cause: state, last exit, next restart, termination output and a Logs action. Continue with the restart timeline, containers, Runs on (node and Talos kubelet health), relationships and recent events. The retained resource pane keeps its watch and shell/forward protections. Previous-instance logs are read only through an explicit Logs action; unavailable history is not invented. The node memory bullet uses Talos used/physical capacity when available and explicitly identifies unavailable requests.

## H1–H7: observability

These use Coroot's concepts in Fog's visual language. Applications, the service map and supported application-report evidence have a read-only Coroot connection; [COROOT.md](COROOT.md) records transport, identity, limits and validation. The retained page owns requests and prepared display data. H4–H7 remain interactive fixture previews and show explicit limitations in live mode. Rollback/configuration previews never execute writes; threshold and mute choices remain local to example mode.

| Screen | Interaction |
| --- | --- |
| H1 Applications | Shared fixture/live projection, stable AppIds, 12 check columns including separate disk usage and I/O. Problems first, grouped by namespace, with text/namespace/category/status filters. Healthy figures are plain, and a healthy check without a figure reads “ok”. Warning, critical and unknown reports carry the shared glyph. Missing reports show an em dash. The accessible label and tooltip name the full state. |
| H2 Service map | Layered left to right: a node's column is its longest chain of callers, tall columns wrap at eight, each column is ordered by its callers' rows, and unconnected nodes share a last column. Curved links end in arrowheads; width follows traffic and problem links are dashed. The layout uses every link between the page's nodes, so the problem filter never moves a box. The Connections inspector sits beside the map only when the whole map fits, otherwise below it; the map scrolls inside its card. Link selection survives reorder; nodes open application reports. |
| H3 Application | Coroot's own application view (`application/`). The header is the breadcrumb, *Applications / namespace / ● name* (`PageHeader::crumb` and `glyph`), over the meta line. A map strip: clients left, the application with its instances in the middle, dependencies right, each link coloured by its status and dashed when not OK; a box opens that application on the same report. Coroot's report tabs, with a glyph only on a report that has something to judge by. The chosen report's checks card gives `Title: verdict` (`ok` when it passes) and the condition with its threshold set apart. Its widgets follow in Coroot's widths: tables in the shared `DataTable`, one entity each, whose application links open their report; Tracing and Profiling embed those pages' views for that application. Until the evidence card is retired, it adds only what the checks don't say (calls, clients, charts, log patterns): the REST report first and, under *More checks from Coroot*, only what its MCP endpoint adds; with nothing to add it isn't drawn. Explicitly mapped subjects open Kubernetes objects through the shell guard; complete histories and live mutations are unavailable. |
| H4 Incident | Two incidents, SLO compliance and 1h/5m burn, root cause, cause chain, previewed fixes and ruled-out evidence. Burn threshold 14.4×. |
| H5 Deployments | Four releases, selectable revisions, actual diff of the fictional Deployment specs, full selected YAML and rollback preview. |
| H6 Profiling | Comparison toggle, CPU history, biggest increases, function search and a flame graph with zoom/reset. |
| H7 Traces | Latency/error heatmap with SLO line, selectable buckets and error causes, matching sample waterfalls and span inspection. |

The later fixture histories end at 15:00, with the worker deployment at 12:52 and node event at 14:48; header ranges regenerate that bounded example history. Live Refresh/reopen captures the current time interval, while report navigation and Retry retain the captured absolute interval, shown with local dates and times. Applications are virtualized; maps and report evidence have explicit bounds. Hiding cancels ordinary requests, and changed provider/access/project/range generations reject old results.

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
| Table page | Pods, every Resources kind | `PageHeader`, then any banner, then one bare `DataTable`, with a hairline above and below. A detail pane opens beside it (content ≥ 900 wide: 460 default, 340 for a detail of a few short fields such as Storage's and Diagnostics', 320 minimum, 14 gap) or below it (190 list, 380 pane). |
| Dashboard | Monitoring | `PageHeader` with a `Breadcrumb`, a row of variable chips, then a grid of `StatCard` and `ChartCard` with 8 between them. Section rows are 36 high, with a chevron and a 14 bold title. |
| Canvas | Service map, flame graph, heatmap | The page draws its own picture inside a card, with explicit bounds, under the same header, padding, status glyphs and states. It never draws its own title or toolbar. |
| Detail pane | The pod pane | A card: kind caption, 13.5 monospace title, a state tag, actions (xsmall outline), then 32-high tabs with a 2 px accent underline on the chosen one and muted text on the rest. |

**The page frame.** A table page has no margins (`page::page`): its header, any banner and its panes stack edge to edge on the content background, and the table runs from the column to the window's edge. The header is `page::toolbar(cx)`, padded 12 at the sides with a hairline under it. What else isn't a pane sits in `page::inset()`, padded 12 at the sides and 10 above and below (`PANE_PADDING`, `PANE_PADDING_Y`): a banner, a state in the table's place, a grid of cards, and until change 9 the detail pane's card on its outer edges. When the window is short (`page::is_short`, under 620 dp), a table page scrolls its frame and its list keeps at least 180 dp (`SHORT_LIST_HEIGHT`), so the header can't squeeze the list to a row; Pods, Nodes and System services do. A page of cards (Overview, Observability's destinations other than Applications) keeps the padded frame, `page::padded`: 26 left and right (`PAGE_PADDING`), 22 top, 18 bottom and 14 between the header, banners and body. Monitoring's 20/14/10 on `surface_2` moves to this when it migrates.

**Cards.** 12 px radius, a `line` hairline, `surface` fill, no shadow (`page::card`, which desktop's `screens::panel` calls). Card padding 12–14.

### Components

| Component | What it draws | Sizes and ids |
| --- | --- | --- |
| `PageHeader` | A toolbar. Left: the title as its label (13 semibold, `ink`), filter input, segment (`Problems` / `All`), `StatusChips`. Right, after a spacer: the source label or namespace picker, Columns, the time picker where the page has a range, and refresh. Below: a `.secondary` row, then, on a page of cards (Overview, Monitoring, etcd), the meta line. | Rows 38 high, items centred on them, 8 between items; every control 24 high (`ui::CONTROL_HEIGHT`, as `dp`). On a table page the rows sit in `page::toolbar`, with its hairline; a page of cards draws the same rows in its padding, without one. The row never wraps: controls that don't fit fold into a 24 `…` menu at its right, the rightmost first, where each shows as its menu form (Refresh an item, Columns, namespace and time range submenus). A submenu that picks a value names it, `Namespace · payments ›`, cut at 24 characters. While a folded control isn't at its default (a namespace or node picked, columns hidden other than the page's default, a range other than 3 hours, Failed requests on, another trace source), the `…` carries an accent `badge_dot` at its top right, `<page>-more-dot`, and its tooltip and accessibility label list them: `More · Namespace payments · 1 column hidden`. Refresh never marks it, and Project shows its value without one. When even the title, filter and `…` leave no room for the chips, the segment and chips take a row of their own under it, and the controls unfold as far as the row then allows. When even that leaves controls that don't fold no room beside the title, as on Monitoring, the controls take a row of their own under the chips, right-aligned, folding only as that row needs. Filter 150, shrinking to 96 on a full row; namespace 132 with a 260 menu. Meta line 11 muted, wrapping between whole parts on a narrow page: `context · 12 pods · state · 14:02:11`, with `Example data` or `Not connected` in place of the context. Table pages (Resources, System services, Nodes, Observability) put that line in the status bar instead. Ids `<page>-title`, `<page>-filter`, `<page>-namespace`, `<page>-columns`, `<page>-refresh`, `<page>-more`, `<page>-scope`. |
| `status::Segment` | A table page's line in the status bar, after the shell's status and a hairline: `12 pods · reconnecting · 14:02:11`. The page derives it when its data changes; the shell reads the visible page's, and a page drawn by a screen hands it over with `ScreenPanel::status`. It leaves out the context when the shell's status already names it (`Connected to prod-fra`) and keeps it when the page reads another. A stale or reconnecting part is drawn in `warn_ink`, a failed or refused one in `crit_ink`. | 11.5, the bar's muted text, one line. A bar too narrow for the whole line drops parts rather than cutting the end: minor parts first (the refresh time and `example data`, which the bar shows already, and counts of none), then ordinary ones from the end, then the first part when a warning is left. The context and warning or failed parts always stay; only when even they don't fit is the line cut at its end. The tooltip and accessibility label hold the whole line, and the tooltip adds the page's note (how Incidents samples, Traces' limit, etcd's quorum tolerance). Id `<page>-scope`, inside `status-bar`. |
| `Breadcrumb` | Parent, `/`, then the title, for a page inside a collection (a dashboard, an application report). `PageHeader::parent` draws it in the label's place, on the toolbar's first row. | Parent 13 muted, a link that turns blue on hover; separator faint; title as `PageHeader`'s label, truncating, at least 120 wide. Ids `<page>-<part>` for the parent, `<page>-title` for the title. |
| `StatusChips` | One ghost chip per status: its glyph and count. Pressing one filters to it; pressing it again clears. | Chips `small`, 6 horizontal padding, 2 apart; 12 monospace counts in `ink_2`. Tooltip `3 failing · click to filter`; accessibility label `3 failing`. Order: critical, warning, waiting, OK. Ids `<page>-tally-<what>`. |
| `DataTable` | The table, bare by default: on its pane's background with a hairline above and below, no radius and no side borders; `.carded()` puts it in a card for a page of cards. Inside: any notes (`SelectionBar`, `ShowingBar`), the header row, the virtualised rows, the footer `Legend`. Scrolls horizontally inside its frame when its columns are wider than the view; its group rows are as wide as the view, so their details truncate before their actions, and stay in view when it scrolls; the leading pinned columns (a row's glyph and name) stay at the left edge with a hairline on their right, unless they would take more than two thirds of the view. | Header 26 on `surface_2` with a bottom hairline; column labels (`ui::column_label`: 11.5 semibold muted, as given, so pages write them in sentence case and Kubernetes' printed columns keep theirs) with a 12 sort arrow. Rows 26, monospace 12.5. Cells pad 10 (`CELL_PAD`) horizontally and truncate; each column has a fixed width or is the one flexible column. Widths are derived when the data changes: 7.5 a character plus 24, between 64 and 280 (440 for the flexible one). A glyph column is 34 (`GLYPH_WIDTH`), first, after the row's 1 px border, and draws with `glyph_cell`: unpadded, its glyph or a marked row's box centred, on every page. Selected row: `accent_soft` with a 1 px accent border; marked: `hover`; hover: `hover`. Selection follows a row's key, never its position. An empty table shows one 12.5 muted line, padded 12 × 14. Ids `<page>-list`, `<page>-rows`, `<page>-table-scroll`, `<page>-sort-<n>`, rows from their key. |
| `GroupRow` | A group's status glyph, its label (semibold, in the status's text colour), an optional monospace subject (a node), then `· 12 pods · collapsed` muted and truncating, then xsmall ghost actions (Select all, Expand / Collapse, Open node). | The row height, so the list stays uniform. Its glyph sits in a 34 slot after a 1 px inset (`glyph_slot`), centred on the rows' glyph column; with the 10 gap its label starts where the first text column's text does. 12 right padding, 12 text, `track` at 45 % with a bottom hairline. Role heading. Ids `<page>-group-<key>`. |
| `ShowingBar` | `Showing 40 of 212`, and `Show all 212` when rows are folded. | `accent_soft`, 12 horizontal × 5 vertical padding, bottom hairline, 12.5 `ink_2`, xsmall ghost action. Role status. Id `<page>-collapsed`. |
| `SelectionBar` | `3 selected` (semibold), Copy names, Clear. | As `ShowingBar`. Id `<page>-marks`. |
| `Legend` | The table's footer: what its marks mean, with live examples (meters, stripes). | Top hairline, 12 × 7 padding, 12 gap, 11 muted, wrapping. Under 600 content width it becomes one 26-high line ending in ⓘ, with the full legend as its tooltip. Id `<page>-meter-legend`. |
| `Document` | A text document's lines (`document::lines`), such as a generated configuration under review. They are virtualised and never wrapped; the list is as wide as its widest line and scrolls sideways rather than clipping one. The page draws each line from `document::line` and adds what it needs: a gutter, highlights, a selection. | Lines 20 high (`document::LINE_HEIGHT`), monospace 12; the frame is the page's. Ids `<page>-<what>-lines` for the list and `<page>-<what>-line-<n>` for a line. |
| `CardGrid` | Items `columns` to a row (`grid::cards`), virtualised by row, so a long list of cards draws only the rows in view. The page draws each item, all the same height and a column's share of the row, even in a short last row; `grid::row_of` finds an item's row to scroll it into view. | The gap the page gives, between items and below each row. |
| `StatCard` | A muted 12 bold title, then one figure (22, weight 900) or a wrap of named figures (18, each at least 96 wide, 20 × 8 apart), each with an optional tag, gauge, meter of parts (one per item in its tone, such as a cluster's nodes; above 24 items, a run per tone, problems first, each at least 4 wide) or sparkline, and an optional 12 muted detail line that wraps. A figure that is the status itself (Overview) is tinted: its value takes the tone's text colour instead of ink. | Card; header 28 with 14 left padding; body 14 × 10. |
| `ChartCard` | A 13 bold title, the unit, an info mark with the query, the stale mark at the right; the plot, then its legend. | Card; header 32 with 12 left padding; plot padded 12, at least 64 high. |

A virtualised grid of cards draws on `freshkube_ui::grid`: Nodes' Cards view, and its compact two-line pane list as a grid of one column.

### States

| State | Where | How it looks |
| --- | --- | --- |
| Empty | Instead of a page's body | `ui::empty_state`: centred, at most 480 wide, a 20 icon, a 19 bold title, the description, then actions. Say what was looked for and where. |
| Loading | First read of a table or card | Skeleton bars (5 px radius) in the card's shape: a table card shows nine bars of 70 % × 12, 12 apart; a card shows a 40 % title bar and a body bar. Never a spinner alone. |
| Stale | Data kept after a failed refresh | A warning banner above the body (`ui::warning_banner`): what happened, `Showing pods as last seen at 14:02:11.`, the reason, Retry. In a card: the warning glyph at the header's right with the reason as its tooltip. Stale meters are striped. |
| Refused | 403 | An empty state with the shield icon: `Not permitted to list pods`, that this says nothing about whether any exist, and the server's reason in a monospace `crit_soft` box. |
| Failed | Nothing known yet | An empty state: `Couldn't list pods`, that nothing is shown as missing, the reason, and Retry. In a card: one centred 12 line with the critical glyph. |

Each state's element has role status and an id `<page>-<state>`.

### Tooltips

Kit's `Tooltip` is the only tooltip: `.tooltip(…)` on an element or a button, `tooltip_with_action` on a button with a shortcut. A tooltip says what the element can't show; it never repeats a label shown whole. One is required on:

- **Truncated text:** the full value, such as a name cut at its column or shown without its common node prefix.
- **Compact values:** the full text behind `2.8%`, `1.9k` or `refused`: the exact figure with its unit, or what was refused and why.
- **Icon-only controls:** the action's name and its shortcut, if it has one: `Refresh pods`.
- **Resource meters:** used, requested and the limit (allocatable on a node), how fresh the reading is, and any source that is missing.
- **Status glyphs:** the state in words and its reason, unless the group header already gives them.

### In code

[#47](https://github.com/skel84/freshkube/issues/47) moved what Pods draws into `freshkube-ui`, with ui.rs, the palette, theme, text size and meters; the app still reaches those by their old `crate::` paths.

- `freshkube_ui::page`: `page(id)` is the frame without margins, `toolbar(cx)` the header's strip in it, and `inset()` the padding inside it (`PANE_PADDING`, `PANE_PADDING_Y`); `padded(id)` is a page of cards' frame, with `PAGE_PADDING`, `PAGE_TOP`, `PAGE_BOTTOM` and `PAGE_GAP`. Also `card(cx)` and `meta_line(id, cx)`.
- `freshkube_ui::status`: `Segment::new(context, parts)` with `Part::new(text).tone(tone)`, `.minor()` for a part a narrow bar drops first, and `.note(text)`; a `Warn`, `Crit` or `Died` part is never dropped, so tone a count that warns rather than only naming it; `status::segment(id, &segment, named, cx)` draws it, leaving out the context the shell's status names. A page keeps its segment on the entity and derives it again only when its inputs change (Resources and Observability compare them with what they last derived from), so the shell can read it on every frame.
  - `PageHeader::new(prefix, title)` draws `<prefix>-title` on its row, `<prefix>-toolbar`, each control in a box `<prefix>-slot-<n>` (n counts the controls the page added, folded or not), `<prefix>-controls` around them, `<prefix>-more` when any fold and, from `.meta(…)`, `<prefix>-scope`. `header.id("columns")` gives the caller its other ids. Then `.filter(div)`, `.chips(…)`, one call per control in order, and `.render(window, cx)`. `.control(…)` adds one that never folds; `.foldable(control, items)` one that folds into `…` as `items`, a `page::MenuItems`. Build them so the control and its menu form share one handler: `page::handler(cx, f)` gives a button's `Handler`, `page::item(label, handler)` (or `checked_item`, `disabled_item`) its form, and a control that opens a menu builds its own menu and its `page::submenu(label, items)` from the same `MenuItems`. `page::Fold::from(items).changed(note)` adds what the control is set to while it isn't at its default; `page::submenu_value(label, value, items)` and `page::columns_fold(items, hidden, at_default)` build the usual forms. A second row, such as category segments, is `.secondary(row)`, `<prefix>-secondary`, left-aligned under the toolbar's, before the meta line. The header measures its parts as it draws and keeps the widths under its prefix; the first frame folds nothing, and a new placement draws on the next frame. `.parent(part, label, on_click)`, the breadcrumb before the label: the parent a link with the id `<prefix>-<part>`, then a faint `/`, then the label, which keeps its id and truncates.
- `freshkube_ui::table`: `data_table(source, window, cx)` draws the table bare, filling the room it is given, for any page entity that implements `TableSource`. `DataTable::new()` chooses otherwise: `.carded()` puts it in a card, for a table among cards on a scrolling page (Incidents and Traces, until change 9's inspector); `.fit(max_lines)` makes it as tall as its header and lines, for a table in a scrolling page; then `.render(source, window, cx)`. The page keeps a `TableState::new(prefix)` (its scrolls; the ids `<prefix>-list`, `-rows`, `-table-scroll` and `-empty`, each labelled header cell `(<prefix>-sort, column)`, and `id(part)` for the rest), so several tables can share a page. The page supplies:
  - its `columns()` (any `TableColumn`, whose `pinned()` keeps the leading columns, such as the glyph and the name, in view when the table scrolls sideways) and `width()`;
  - `line_count()` and `line(n)`, either a `TableRow` (a `Key: Hash + Eq + Clone`, an element id, a label, an optional tooltip, marked and muted) or a `Group(n)`;
  - `cell(row, style, column)`, where `RowStyle` says whether the row is selected and carries its palette, muted text brightened on a selected or marked row;
  - `selected_key()` and `line_of(key)`: selection is by key, never by position, and `table::step(source, delta, cx)` and `table::reveal(source, strategy)` move to and show it;
  - `group(n)`, `sorting(column)` and `sort(..)`, `click(key, event)` (or `clickable() -> false` for rows that don't select), `empty()` as an element, `notes()` and `footer()`.

  It is generic, not `dyn`, so 20,000 rows cost what hand-written ones did. A table scrolled sideways moves its group lines back by the scroll (a table wider than its view draws them as wide as the view), and draws a row's pinned cells last, inside `(<prefix>-pinned, line)` (`<prefix>-pinned-header` for the header), over empty cells that keep their place, opaque in the row's state; both move while the frame is laid out, after the scroll is clamped, so they never trail it. Every cell is drawn once, so its id finds one element. While the run pins, the cells passing under it are clipped at its right edge, so there only the row and the pinned cells take hover, clicks and tooltips. A header cell passing under the run moves its label right to stay beside the run, clipped to its own place, while any of its column shows: a short label would otherwise vanish while a long cell's tail still shows. The table builds pinned only when the run pins, scrolled and at most two thirds of the view; otherwise it builds the unpinned tree, and the scrolled header draws the table again if a frame's scroll disagrees with what was built. Unscrolled, the rows draw as they did before pinning. The pieces are `ROW_HEIGHT`, `HEADER_HEIGHT`, `ROW_GROUP` (the row's hover group), `cell(column)`, `CELL_PAD`, `GLYPH_WIDTH`, `glyph_cell(column)`, `glyph_slot`, `GroupRow`, `selection_bar`, `showing_bar`, `legend`, `legend_item`, `legend_line`, `status_chip` and `status_chips`. `ui::Tone::Integration` is the integration-required ring with a plus, from the palette's `integration` token, and `ui::Tone::Died` the skull in the critical colour, in `status_glyph`, `status_mark`, tags and chips. `status_mark` names its state to assistive technology with its tooltip; a bare `status_glyph` is decorative, named by what holds it.
- `freshkube_ui::card` ([#62](https://github.com/skel84/freshkube/issues/62), moved from `monitoring/panel`): `CardHeader::new(id, title)` with `.unit(…)`, `.about(text)` (the info mark's tooltip), `.copy(text, hint)` (what a click on the mark copies) and `.stale(…)`; its parts are `<id>-title`, `-query` and `-stale`. `StatCard::new(header).render(body, cx)` and `ChartCard::new(header).render(body, cx)` draw the card with id `<id>`, filling its grid cell. A stat card's body is `figures(&[Figure], cx)`; each `Figure` borrows its name, value and note, with an optional `Tone`, gauge and `Spark` (whose last stretch takes the colour the caller gives). `gauge(fraction, tone, cx)` is the fill bar. The page derives its figures when its data changes; the chart's plot, cursor, legend and markers stay with the page.
- Not in the crate yet: the states, which stay `ui::empty_state` and `ui::warning_banner`.

### Settled values

- **Padding:** none on a table page, 12 × 10 inside its insets (`PANE_PADDING`); 26 on a page of cards (`PAGE_PADDING`), with 22 / 18 / 14 as above.
- **Card radius:** 12 px, including Monitoring's cards and Overview's.
- **Time range:** one picker in the page header's right group, before refresh, as Monitoring's: an outline small button with a clock, the range and a caret, opening a checked menu. Observability uses this picker; its shell time chips are removed.
- **Refresh:** one per page, the page header's ghost icon button, tooltip `Refresh pods`. A page with auto-refresh puts its interval menu beside it.
- **Group rows:** the row height (26), not a separate 28, so every list stays a uniform list.
- **Headers on other Resources kinds:** every kind draws Pods' `PageHeader`, with its filter, the namespace picker on namespaced kinds and an icon Refresh; the scope moved to the meta line with change 6, and into the status bar with change 7.

### Checks

Two checks keep pages on these components ([#48](https://github.com/skel84/freshkube/issues/48)).

- **Layout tests.** A table page's UI test calls `desktop::layout_check::assert_table_page` with its page, title, table and list ids. It measures a headless render's painted bounds: the 26 header, 26 rows, group rows at the row height, the table edge to edge across the page with the title 12 in and a hairline under the toolbar (`assert_edge_frame`), the toolbar's 38 row with the title as its 13 label, centred, and every control 24 high (`assert_toolbar`), and `assert_bare` fails if a card frames the table. Pods' test runs it at the default text size and at 20 px; every migrated table page adds its own. A page whose table isn't the whole page uses the two halves: `assert_edge_frame`, or `assert_page_frame` on a padded page of cards (page, title and content ids: padding and toolbar), and `assert_table` (an optional header frame and the list: header, rows and group rows), once per table.
- **Style check.** `scripts/check-style.sh` runs in CI beside Clippy. Outside the shared components (`crates/freshkube-ui`) a page may not run its own `uniform_list` or `VirtualList` (logs and the terminal measure their own rows; a document draws on `document::lines`, a grid of cards on `grid::cards`), round a corner with `px` radii other than 3, 8, 10 or 12, set text 20 or larger with a literal, or draw a status glyph itself (a `●` `◆` `▲` `○` `✓` string, or a small `rounded_full` dot) instead of `ui::status_glyph`, `ui::status_mark` or `ui::health_mark`, give a button an icon and no label, child or tooltip ([Tooltips](#tooltips)), or scroll vertically without `.restrict_scroll_to_axis()`: unrestricted, GPUI hands a vertical-only scroll a sideways wheel's movement too, so a sideways swipe over a table inside it also scrolls the page. `scripts/style-allowlist.txt` names, per rule, the files that broke a rule when the check arrived. It only shrinks: the check fails when an unlisted file breaks a rule and when a listed one no longer does, so a migration removes its entries in the same change. `--list` prints every offence with its line.

## Open

Page titles, the interface face, status glyphs, density, the integration colour and the terminal shortcuts were [decided on 5 October 2026](#from-todays-app-to-desktop-grade). Still open:

- More causal grouping beyond pod state and NotReady nodes.
- Fog light theme.
- Whether a last-known problem keeps a muted rail dot. Today a card whose evidence is stale shows Unknown with "Last known ·" and leaves no dot, Kubernetes and Talos cards alike ([#65](https://github.com/skel84/freshkube/issues/65)).
- Later Coroot destinations and a reviewed workflow for any fixes. The first live slice is read-only; [COROOT.md](COROOT.md) records its supported evidence and remaining API gaps.
