- **Applications across the workspace:** Kargo, Argo CD and each destination's
  labelled workloads are read from their own workspace entries, so the same
  applications show whichever cluster is open, and an entry that can't be
  reached, refuses, doesn't finish in time or isn't in the workspace is named
  in the page's banner. Each entry is read under its own deadline, Argo CD's
  inventories are matched to the workloads on the entries its destinations
  map to, and an application whose destination wasn't read is marked may be
  incomplete.
