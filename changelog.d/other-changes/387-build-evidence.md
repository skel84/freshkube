- **Delivery trails no longer confirm a build from declared fields alone:**
  a build ties to a commit only when a result reports it (the run's, then its
  TaskRuns'); a PaC label and a revision parameter that agree are two declared
  fields, so Commit → PipelineRun and Pull request → PipelineRun are Claimed,
  and a result that names another commit also makes them Claimed. A Freight
  that joins on the commit stays Confirmed only when a build reports it,
  becomes Claimed when the build's tie is its label, and with no build joins
  the commit itself. Chains' `signed` annotation is a claim that the signature
  was not verified, so no supply-chain link is Confirmed. The Rollout links
  are unchanged.
