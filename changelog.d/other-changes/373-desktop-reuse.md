- **Less duplicated code:** the desktop app uses the shared helpers for
  plurals, digit grouping, bounded config reads, base64 and column widths
  instead of local copies, and drops unused icon entries, wrappers and
  port-table code. No change in behaviour, except that a missing
  talosconfig now reads "Cannot read talosconfig: file is unreadable or
  does not exist".
