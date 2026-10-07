- **Less duplicated code:** the desktop app uses the shared helpers for
  plurals, digit grouping, bounded config reads, base64 and column widths
  instead of local copies, and drops unused icon entries, wrappers and
  port-table code. Two things you can see: a missing or unreadable
  talosconfig now reads "Cannot open talosconfig: file not found" or
  "Cannot open talosconfig: permission denied" in place of the operating
  system's text, and a packet capture's save dialog starts in the Downloads
  folder: the XDG one on Linux, else the home folder, and the Downloads
  folder on Windows, instead of the temp folder.
