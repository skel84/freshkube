- **Observability in its own crate.** The Coroot pages and their log source
  move unchanged from the desktop crate into `freshkube-observability`, so
  their tests build and run without building the rest of the app.
