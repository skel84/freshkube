# Tabs

Several places of the open cluster at once, under the header, each with a history of its own (#877, slice 1 of #875). A tab is a `ViewState` ([deep links](deep-links.md)): a page, and for Resources a kind, a namespace, an object and a tab, for Nodes a node and its tab. Its title is derived from the view when the strip is, never stored. One tab is the window as it always was; the strip shows from two.

Code:

- `crates/freshkube-desktop/src/desktop/tabs/set.rs`: `TabSet`, `OpenIn` and the rules (open, close, reopen, step, history), with no GPUI types.
- `tabs/strip.rs`: `ViewTabs`, the strip as a cached view of its own; `tabs/commands.rs`: what `Pilot` does with the set, the keys and the click's gesture; `tabs/restore.rs`: the file.
- `crates/freshkube-ui/src/tab_strip.rs`: the row of closable tabs the dock and the strip share, and the bar the strip sits in.
- `crates/freshkube-resources/src/frozen/`: `FrozenRows` and `FrozenSet`, the bounds of frozen rows; `screen/frozen.rs`: the page's side of them.
- `crates/freshkube-desktop/src/desktop/pages.rs`: `Pilot::open_object_in`.

Tests that prove it: `tabs/set_tests.rs` (the rules), `tabs/tests.rs` (the window: keys, gestures, the guard, saving, the strip's redraws, frozen rows), `nodes/tests/events.rs` (the brackets inside Nodes), `freshkube-resources`' `screen/tests/frozen.rs` and `frozen/tests.rs`, and `desktop/tests/keys.rs` (the shell's keys).

## The set

A `TabSet` is on `Pilot` and belongs to the cluster that is open: a tab is one cluster's, and a context or workspace switch leaves one tab at the view shown. At most 16 tabs; a tab remembers 64 steps back and 64 forward. A background tab opens right after the shown one and changes nothing on screen; a front tab is shown at once. The last tab does not close. Closing the shown tab shows its right neighbour, else the left; the last ten closed tabs can be reopened.

The shown tab follows the window: whenever the shell or the Resources or Nodes page changes, the tab records the view the window shows (`Pilot::current_view`), pushing the old view onto its history. A tab just shown waits for its page to arrive (two seconds at most) before it follows the page again. Pages a link cannot name (Settings, Operations) leave the tab where it was.

## Opening links

`Pilot::open_object` still opens here. `open_object_in` takes an `OpenIn` and runs the cluster guard (`divert_link`) first, so a link made in another cluster is sent to it or refused, with the question about a running shell, whatever the click says; only then does it open a tab.

- Command-click opens a background tab, Shift-Command-click a tab in front. The shell notes how the last press or release was modified (`capture_any_mouse_down` and `capture_any_mouse_up` on its root, since some links act on the press) and the link that click makes, which arrives once the click's update ends, takes it (`take_gesture`, for half a second). Every link that reaches `resource_link` takes it, so a new kind of link needs no code of its own.
- A middle-click does the same where the link takes it: today the Resources list's Owner and Node cells. The other links (the pane's relation buttons, the Applications page's) act on a left click only, so Command-click is what reaches them all.
- In ⌘K, Command-Return opens the result in a new tab in front.
- A link whose names do not fit the link's closed types opens here.

## Keys

| Key | Does |
|-----|------|
| Command-T | a new tab at the view shown |
| Command-W (Ctrl-Shift-W elsewhere) | closes the shown tab; the dock's own binding wins while the dock has the keyboard |
| Shift-Command-T | reopens the last closed tab |
| Command-Shift-] and [ | the next and previous tab, everywhere (bound as `secondary-}` and `secondary-{`, which is what macOS reports) |
| Control-Tab and Control-Shift-Tab | the same, once there are two tabs; with one they still step the screens. A focused terminal gets them first |
| Command-[ and ] | the shown tab's previous and next view |
| Option-Command-] and [ | the tabs of a Resources pane or a node, which were Command-Shift-] and [ |

Command-1 to 9 stay pages. Each action's handler is on the shell's root only while it can act, so the menus grey it and its key goes on to whatever else binds it. Option-Command-[ is bound as `secondary-alt-[`; the headless test presses it, and the macOS keystroke is the same one Zed binds as `cmd-alt-[`. The running app on macOS is the check that cannot be made headless.

## The strip

32 dp under the header: 28 for the tab and the rest above it. It is a cached view of its own beside the page (`ViewTabs`), derived when a tab changes and drawn from that. A page's redraw and a bare notify of the shell leave it alone, and a hover over it draws only it; the render probe `tabs.strip` proves both. The shell draws again only when what it lays out or answers changes: the strip appearing, or Reopen, Back or Forward gaining or losing their handler, not at each navigation.

A tab's menu is the dock's: Close, Close others, Close to the right, Close all (which leaves the shown tab). A middle-click on a tab closes it.

## Saving

The views are saved in `navigation.json` under `tabs` (`selected`, and `views` as each view's query text), shortly after the last change. Nothing else: no title, history, rows or cluster. The first cluster to open in a run takes them back, each as loading rows until its first list; a view that no longer parses is dropped, and the selection follows the rest.

## Frozen rows

A Resources tab that stops showing keeps its last list, so coming back shows it at once. `FrozenSet` is the owner and holds the bounds: at most 8 tabs keep rows, a list of more than 5,000 rows keeps none, and every tab's rows together are kept within 64 MiB (estimated; each tab gets an eighth, and a list too big for its share keeps its first rows). The tab that froze longest ago loses its rows first, and a closed tab keeps none. Nothing is saved.

Coming back, the page shows them muted with "200 of 1,204 kept · last known 4 min ago", while it lists again, and the status bar says it. When the list lands the rows are replaced and the ones whose version changed flash. If the read fails the rows stay, marked stale with the page's banner and Retry, never an empty list; Retry keeps them until a list answers.

`ResourcesScreen::rows_are_frozen` says when the rows shown are frozen ones. The Resources page offers no Restart or Delete today (no page offers an action through the write framework yet, [the write model](../WRITE_MODEL.md)); when one does, it must stay disabled while rows are frozen, with a tooltip saying why, and Delete goes through the changed-since check of #853 once the rows are live.
