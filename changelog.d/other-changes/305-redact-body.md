- **Error pages redacted further:** an error page that isn't the API server's
  `Status` also loses wildcard and suffix domains (`*.apps.cluster.example`,
  `.svc.cluster.local`), host names whose last label has a digit, IPv6
  addresses inside markup and fields with their zones, and credentials and
  email addresses whole. A proxy's JSON no longer passes for the API server's
  error, and the message is cut to 4 KiB
  ([#305](https://github.com/skel84/freshkube/issues/305)).
