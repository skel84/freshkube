- **Internal:** `connection.json`, `monitoring.json` and `coroot.json` are read
  and written through `freshkube-prefs`, and a preferences update no longer
  overwrites a file it could not read. Each still logs a file it cannot use.
  Nothing changes on screen or in what is saved.
