- **Applications are read from a cluster:** Kargo's Projects with their
  Stages and Warehouses (at most 50 Projects), Argo CD's Applications and
  ApplicationSets, and the Deployments, StatefulSets and DaemonSets labelled
  `app.kubernetes.io/part-of`, all through the read-only, bounded delivery
  reader. The acme workspace of the platform mocks is available as example
  data. Nothing shows them yet.
