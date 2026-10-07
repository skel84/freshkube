- **Loading tables cost less:** while Pods, Nodes or System services wait
  for their first answer, the moving loading rows no longer redraw the
  page, the header, the rail or the column on every frame. Refresh buttons
  show a read in flight with an accent icon and "Refreshing…" instead of a
  spinner. Nodes now draws its table's header over the loading rows from
  the first frame, before Talos or Kubernetes answers.
