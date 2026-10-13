- **Internal:** `navigation.json` is read and written through a new
  `freshkube-prefs` crate. Its save syncs the file, keeps an existing file's
  permissions and a symlink, and logs a bad file or a failed save once.
  Nothing changes on screen or in what is saved.
