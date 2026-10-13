- **Internal:** `preferences.json` (the text size) is read and written through
  `freshkube-prefs`, whose save is now atomic; every other key in the file is
  kept, unknown ones included. A new preferences file is created with the
  usual umask. Nothing changes on screen or in what is saved.
