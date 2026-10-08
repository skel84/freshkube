- **One spelling for Docker Hub images:** the delivery trail now treats
  `acme/api`, `docker.io/acme/api` and `index.docker.io/acme/api` as one
  repository, and adds `library/` to a single-name image, as a container
  runtime does. A Freight now joins its Warehouse, Deployment, Rollout and pods
  across these spellings. A registry's host ignores case. Its port still
  counts, so `registry.example:5000` stays a different registry. A name that
  isn't a valid reference is compared as written.
