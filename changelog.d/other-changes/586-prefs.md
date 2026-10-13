- **Internal:** `navigation.json` is read and written through a new
  `freshkube-prefs` crate. Its save syncs the file, keeps an existing file's
  permissions and a symlink. Desktop logs a bad file or a failed save once;
  the crate itself logs nothing.
  Nothing changes on screen or in what is saved.
