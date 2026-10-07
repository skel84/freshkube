- **Status messages redacted further:** a Kubernetes or object status message
  loses credentials and email addresses whole (`user:pass@`, `git@host:org/…`
  with its owner), keeps the API groups it names, and is cut to 4 KiB. Error
  pages also lose IPv6 addresses right after a field's `key:`, userinfo with an
  `=` or a `%40`, and host names between dashes, and the 4 KiB cap now holds on
  the final message ([#311](https://github.com/skel84/freshkube/issues/311)).
