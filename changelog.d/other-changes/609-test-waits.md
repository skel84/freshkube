- **Internal:** UI tests that wait on a fake server or another real runtime
  now wait through one shared helper, which allows 30 s of real time instead
  of a few seconds of test clock, so a slow machine no longer fails them.
  Nothing changes on screen.
