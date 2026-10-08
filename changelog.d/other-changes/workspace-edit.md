- **Settings › Workspace edits `workspace.json`:** Add opens a form, and Edit,
  Remove (after a confirmation) and Move up and down act on the selected
  cluster, each saved at once. A file the app couldn't use is set aside as
  `workspace.json.bak` (or `workspace.<time>.bak`, never over an earlier
  backup) before the first save writes a new one. A file changed by hand
  since it was read is never overwritten (Reload reads it again), a file the
  next launch would refuse for its size is not written, keys the app doesn't
  know are kept and named, and example data is never saved.
