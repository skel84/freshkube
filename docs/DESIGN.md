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

## Platforms: desktop grade, adapted at the edges

Freshkube is a desktop app and is meant to ship on macOS, Windows and Linux. GPUI draws every control itself, so the app imitates no operating system: it keeps one look of its own on all three, holds it to the conventions desktop apps share, and adapts each platform's differences in one place. Agreed with the user on 5 October 2026.

What desktop apps share, and new screens follow:

- **Lists, tables and outlines carry the content.** Selecting a row shows it in an inspector or pane. No dashboards of cards.
- **Actions act on the selection,** from the toolbar, a context menu or a key. An inspector or dialog has at most one default button; rows and cards don't carry their own.
- **The frame states the location.** Pages don't open on a hero heading or a greeting.
- **Desktop density.** Controls 24 dp, toolbars 38 dp, rows 26–28 dp, column headers in sentence case at 11.5 px semibold, panes padded 10–12 dp.
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
| Terminal-wide shortcuts | ⌘ | Ctrl-Shift (proposed) | Ctrl-Shift (proposed) |
| Packaging | DMG, Homebrew cask | MSI or winget | AppImage, Flatpak or deb |

Bind keys with `secondary-` and show the platform's label (`⌘K`, `Ctrl+K`). A focused terminal takes every key without the modifier above, so on Windows and Linux its shortcuts can't use plain Ctrl, which the shell needs. The bundled faces keep the same metrics on every platform. [#81](https://github.com/skel84/freshkube/issues/81) adds Linux and Windows checks to CI and fixes the terminal's copy and paste keys there first.

The comps are P1d, P4d and P4w on the canvas, also in [`platform/`](platform/) as `home-desktop`, `app-delivery-desktop` and `app-delivery-windows`; [PLATFORM.md](PLATFORM.md#desktop-grade) compares them with P1 and P4.

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

Pods start on Problems when problems exist, grouped by cause, with healthy rows folded. The toolbar contains title, text filter, glyph/count filters, comfortable/compact density and Columns. A blue-tinted banner states how many rows are shown and provides Show all; selected rows expose bulk copy/clear actions. Comfortable rows are 34 dp, compact 26 dp, group headers 28 dp, table headers 30 dp.

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
| H3 Application | Report tabs carry each report's glyph. One evidence card lists the REST report first and, under *Also from Coroot MCP*, only what MCP adds; each source's freshness, error and Retry sit in the card's footer. Rows give a title, the source's own words and only the reported figures, with units (ms, s, rps). Dependencies and clients link to their report. Explicitly mapped subjects open Kubernetes objects through the shell guard; complete histories and live mutations are unavailable. |
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

## Open

- Whether integration-required lavender should become soft teal, to separate it from memory.
- Default row density; comfortable remains the initial value.
- More causal grouping beyond pod state and NotReady nodes.
- Fog light theme.
- Screen titles: the 20 px black heading above versus a toolbar label under [desktop grade](#platforms-desktop-grade-adapted-at-the-edges). Decide with the P1/P1d and P4/P4d comparison; existing pages move only when next touched.
- Whether Figtree stays the interface face at desktop density, or Inter replaces it.
- The Ctrl-Shift terminal shortcuts on Windows and Linux.
- Later Coroot destinations and a reviewed workflow for any fixes. The first live slice is read-only; [COROOT.md](COROOT.md) records its supported evidence and remaining API gaps.
