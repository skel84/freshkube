- **Secret store in core:** the interface Monitoring and Coroot use to
  remember a key in the system's credential store now lives in
  `freshkube-core`, with tests, so Monitoring can become a crate of its own.
  The Keychain, Credential Manager and Secret Service access stays in the
  app. Nothing on screen changes.
