- **Back to Talos keeps your context:** going from Kubernetes-only mode back
  to Talos now uses the kubeconfig context you picked, not the file's current
  one. If that context belongs to another cluster than the Talos one, or is no
  longer in the file, it is refused with a message, and no other context is
  tried in its place.
