- **Internal:** only the binaries' mains read the environment now. The stress
  options, the workbench's knobs and `TALOSCONFIG` are read at launch and
  handed in as plain values, with the same defaults and precedence as before.
