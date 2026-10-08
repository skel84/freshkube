- **Settings › Workspace edits `workspace.json`:** Add, Edit, Remove (after a
  confirmation) and Move up and down act on the selected cluster, each saved at
  once. A file the app couldn't use is set aside as `workspace.json.bak` (or
  `workspace.<time>.bak`, never over an earlier backup) before the first save
  writes a new one, keys the app doesn't know are kept and named, and example
  data is never saved.
