- **Internal:** the debug knobs startup reads (page, kind, theme, sidebar,
  dashboard, log search, motion) and the window size are parsed once into
  `DebugOptions` and passed in, so startup no longer reads the environment.
  Every value opens what it opened before; release builds still ignore them.
