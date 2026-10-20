- **Observability's Kubernetes links follow the open entry:** the map's and the
  reports' links to Kubernetes objects now read the open entry's `clusters[].coroot`
  key in the setup file (its Coroot cluster, and its project or else the connection's),
  and the "Link Kubernetes cluster…" action is gone. An entry without the key is
  not linked, even alone in its workspace, and says to set `clusters[].coroot.cluster`
  in the setup file. Changing the entry or reloading an edited setup reads again for
  the new link. ([#269](https://github.com/skel84/freshkube/issues/269))
