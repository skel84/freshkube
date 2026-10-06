- **IPv6 nodes:** requests for a node addressed by IPv6, bare or as
  `[address]:port`, reach that node; the address was cut at its first colon
  before. A restart, configuration apply, reboot or shutdown with no node
  left to target, for example only `localhost`, is refused instead of
  reaching whichever machine the endpoint is.
