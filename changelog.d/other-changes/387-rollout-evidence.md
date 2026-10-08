- **Rollouts confirmed by what runs:** a change's trail confirms a Rollout
  only from what the cluster reports. Its pods count when their owner
  references lead through a ReplicaSet to the Rollout and they carry its
  current pod hash, so a canary's stable pods and a sidecar's image are
  never judged. The Application's link to the Rollout needs Argo CD's image
  summary to list the digest and those pods to run it. A spec that only
  pins the digest is shown as claimed, with what is missing.
