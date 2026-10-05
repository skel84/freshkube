- **etcd without a leader no longer crashes the app:** when members answer but none reports a
  leader, as after a lost quorum, the etcd page crashed; it now shows the leader as not reported.
