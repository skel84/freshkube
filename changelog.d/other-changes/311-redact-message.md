- **Status messages redacted further:** a Kubernetes or object status message
  loses credentials and email addresses whole (`user:pass@`, base64 and
  percent-encoded userinfo, `git@host:org/…` with its owner), IPv6 addresses
  wherever they stand, and IPv4 addresses in fields with their port or prefix
  (`ip=192.0.2.1`, `10.0.0.0/24`). It keeps the API groups and versions it
  names, and is cut to 4 KiB after redaction. Error pages lose the same, and
  host names between dashes, and their 4 KiB cap now holds on the final
  message ([#311](https://github.com/skel84/freshkube/issues/311)).
