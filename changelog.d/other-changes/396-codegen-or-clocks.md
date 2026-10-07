- **Performance notes:** [docs/PERFORMANCE.md](docs/PERFORMANCE.md) records
  why the log view's spans ran slower once the header, rail and column were
  cached: the lighter main thread runs at lower clocks, not different code.
