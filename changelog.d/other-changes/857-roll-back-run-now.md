- **Roll back and Run now:** a Deployment can be rolled back to an earlier revision
  (B), with the dry run's diff of its pod template shown at every writable level,
  and a CronJob can be run now (J), which creates a Job from its template after a
  preview and a confirmation at every writable level. A read-only cluster sends
  neither, and a note says when Argo CD or Flux manages the object.
