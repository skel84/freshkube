- **Argo CD destinations:** `workspace.json`'s `destinations` maps an Argo CD
  destination, by server URL or cluster name, to a workspace cluster. On the
  change page a Stage deploying to a mapped cluster names it and lists the
  workloads Argo CD reports there, as Argo CD's claim; an unmapped destination
  stays Unknown and says so. A malformed row refuses the file and is named by
  its index.
