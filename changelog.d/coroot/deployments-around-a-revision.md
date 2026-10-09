- **Deployments:** Observability's Deployments lists the chosen application's
  revisions as Coroot keeps them (its last 100; the time range doesn't filter
  them), each with Coroot's own findings, and reads the application's charts in
  a window around the selected one's start (±30 min, ±1 h or ±3 h), saying how
  many samples fall before and after it. Coroot's 404 for a window reads as no
  data, never as a missing application, and only Kubernetes Deployments have
  revisions. It shows live too. The example preview's invented YAML diff,
  Compare with, Roll back and Synced by Argo CD are removed.
