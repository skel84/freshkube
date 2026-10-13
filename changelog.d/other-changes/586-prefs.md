- **Internal:** `navigation.json` is read and written through a new
  preferences crate. A save now syncs the file before the rename, keeps the
  file's permissions, writes through a symlink, and logs a failure once
  until a save works again. A file that can't be read or parsed is logged,
  without its contents. Nothing changes on screen or in what is saved.
