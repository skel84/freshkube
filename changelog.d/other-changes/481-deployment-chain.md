- **Delivery trails follow a Deployment:** an Application that manages a
  Deployment is now joined through the Deployment's current ReplicaSet, which
  the controller reports by revision and owner UID, to that ReplicaSet's pods,
  instead of judging every pod in the namespace. A pod list or ReplicaSet list
  that stopped at its cap, a Deployment that has not seen its latest spec, or a
  pod not yet ready only claims; an unread side is unknown, with the reason.
