- **Delivery:** the delivery spike reads what Argo CD reports about
  reconciling an Application: auto-sync and self-heal, the retry limit and
  the operation's retry count and phase, single or multiple sources, the
  requested, compared and deployed revisions, each managed object's sync and
  health, and the Application's conditions. A missing inventory is unknown,
  not empty, and configuration alone never claims a retry is running.
