- **Workload logs:** a container whose log is refused is read again only when
  the workload's pods change or on Retry, not every 30 seconds, and other
  failed streams spread their retries so they don't all read again at once.
