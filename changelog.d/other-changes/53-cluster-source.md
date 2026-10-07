- Monitoring and Observability take the cluster connection through a small
  core type, `ClusterSource`, instead of the Resources page's own, so
  Monitoring can later move into a crate of its own. Nothing changes in
  the app.
