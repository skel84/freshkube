- **Delivery trails no longer confirm a build from declared fields alone:**
  a build ties to a commit only when a result reports it (the run's, then its
  TaskRuns'); a PaC label and a revision parameter that agree are two declared
  fields, so Commit → PipelineRun, Pull request → PipelineRun and a Pull
  request → Freight joined through a head build are Claimed, and so is a link
  whose build reports other commits. A Freight that joins on the commit stays
  Confirmed only when a build reports it, becomes Claimed when the build's tie
  is declared, and with no build, the commit joins the Freight directly.
  Chains' `signed` annotation is now a claim, since nothing verifies the
  signature, so no supply-chain link is Confirmed. The Rollout links are
  unchanged.
