- **Compare a change in Observability:** a Deployment's hop on the change page
  shows its current ReplicaSet's pod-template hash, and Compare in
  Observability opens Deployments on the revision Coroot keeps under that
  hash. When it can't, the page says why: Coroot keeps no such revision, lists
  no such application, or no Coroot cluster is linked to the open one. An Argo
  Rollout's button is greyed out, since Coroot keeps revisions of Deployments
  only. Connected without a project, Observability now says "Choose a
  project" instead of "Connect Coroot", and a revision Coroot reports no image
  for no longer shows a bare "—" in the Inspector.
