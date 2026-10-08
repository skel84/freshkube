- **Applications are decided once, in core:** an application is a Kargo
  Project, else an Argo CD ApplicationSet or Application, else the
  `app.kubernetes.io/part-of` label, with a rename, merge, split and hide
  override on top. A lower claim is noted, never dropped, and an unread or
  refused source reports as unknown rather than as no applications. Nothing
  shows it yet.
