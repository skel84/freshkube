- **Platforms:** what differs between macOS, Windows and Linux (the room the
  header leaves the window's controls, shortcut labels, the terminal's own
  keys, the dock's Close tab key and where preferences live) now comes from one
  module ([#244](https://github.com/skel84/freshkube/issues/244)). Nothing
  changes on macOS. On Windows and Linux, shortcut labels read `Ctrl+K`
  instead of `Ctrl K`, and the header's leading inset is 12 dp, scaling with
  the text size, instead of a fixed 12 px.
