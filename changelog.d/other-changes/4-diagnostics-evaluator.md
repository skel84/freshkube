- **Diagnostics judged from what was read:** the Cilium operator, Hubble
  Relay and cert-manager states are now read with the rest of a node's
  diagnostics, and one function derives every check from them, with tests
  for refused and failed reads. Nothing on screen changes.
