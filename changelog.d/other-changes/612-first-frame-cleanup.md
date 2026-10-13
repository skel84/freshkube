- **Internal:** the packaged-app first-frame check no longer fails when
  Windows still holds the extracted executable while its temporary directory
  is removed; only a missing or slow first frame fails it. Nothing changes
  on screen.
