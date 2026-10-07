- **Delivery join follow-ups:** a build read discovers Tekton's API once, not
  once per PipelineRun, and one build whose TaskRuns are refused leaves the
  other builds of the commit whole. An error page that isn't the API server's
  JSON loses the bare host names and addresses it names, as `host:port`
  already did ([#292](https://github.com/skel84/freshkube/issues/292)).
