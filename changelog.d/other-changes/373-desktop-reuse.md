- **Less duplicated code:** the desktop app uses the shared helpers for
  plurals, digit grouping, bounded config reads, base64 and column widths
  instead of local copies, and drops unused icon entries, wrappers and
  port-table code. A packet capture's save dialog now starts in the
  Downloads folder: the XDG one on Linux (else the home folder) and the
  Downloads folder on Windows.
