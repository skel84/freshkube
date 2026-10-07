- **Smaller Talos client:** the `talos-rs` crate drops the client methods, talosctl
  wrappers, display helpers, generated protos and TLS helpers nothing called, and
  four dependencies with them. The processes view formats memory sizes with the
  shared formatter, so a size of 1 TiB or more now reads as TB instead of GB.
