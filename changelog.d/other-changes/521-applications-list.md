- **Applications:** a new rail area lists what Kargo Projects, Argo CD and the
  `app.kubernetes.io/part-of` label say the cluster runs, grouped by the rule that
  found each application, with its evidence and notes in the Inspector. Argo CD is
  read in the namespaces its own workloads run in, and the page says which. A source
  that was refused, failed or stopped short says applications may be missing; it is
  never shown as "no applications" ([#521](https://github.com/skel84/freshkube/issues/521)).
