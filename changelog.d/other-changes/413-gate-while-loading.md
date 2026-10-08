- **No false "No node selected":** while the Talos overview is still loading,
  etcd, Security, Lifecycle, Health, Operations and the node pane's Processes,
  Storage, Network and Diagnostics tabs show their loading state instead. If
  the first read fails they say so, with a Retry. "No node selected" now
  appears only once the overview has answered with no node to pick, and its
  hint points to the Nodes page.
