- **Diagnostics as a table:** a node's Diagnostics tab lists its checks in the same edge-to-edge
  table as Pods, grouped by category, each group marked with its worst status and counting its
  checks and problems. Passing, Warnings, Failing and Unknown become chips in the header that
  filter to that status; P still shows only the problems. The CNI and the addons move into the
  line under the header, and a long result no longer runs out of the table: it ends with "…",
  and hovering the row or opening the check shows all of it.
