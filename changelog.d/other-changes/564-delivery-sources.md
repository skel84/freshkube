- **Delivery sources:** Settings › Workspace has a Delivery sources list that says
  which entry Kargo, the builds and Argo CD are read from (by role, or pinned under
  `sources` in `workspace.json`) and which entries the Argo CD destinations' workloads
  are on. Its Check sources reads one item of each, saying whether it is readable,
  refused, not served, unreachable or not in the workspace. Until Check is pressed it
  reads nothing.
