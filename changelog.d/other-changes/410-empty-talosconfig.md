- **Opening from Finder with only a kubeconfig:** a default talosconfig
  that lists no contexts, such as an empty stub, now counts as absent, so
  the app starts in Kubernetes-only mode as it does without the file. A
  named talosconfig, or a default file that can't be read or parsed, stays in
  Talos mode and keeps saying what's wrong.
