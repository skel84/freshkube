- **Internal:** `connection.json`, `monitoring.json` and `coroot.json` are read
  and written through `freshkube-prefs`, and a preferences update no longer
  overwrites a file it could not read. Each still logs a file it cannot use.
  An existing preferences file now keeps its permissions on save (before, each save
  replaced `connection.json` with a fresh 0600 file). Nothing changes on screen
  or in what is saved.
