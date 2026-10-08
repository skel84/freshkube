- **The header switches between the clusters of `workspace.json`:** the
  context switcher lists them under Clusters, and picking one opens it
  through its talosconfig (and Talos context, a new field in the form) or its
  kubeconfig context (never a file's current context). Switching asks before ending a running shell, keeps
  nothing reading for the cluster left, and shows its last summary as last
  known, with its age, when you come back (only if the cluster's files are unchanged). The app starts on the entry you used last (else
  the first core cluster) when the command line names no source.
