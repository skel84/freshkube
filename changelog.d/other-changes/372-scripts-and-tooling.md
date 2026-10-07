- **Tooling cleanup:** the unmaintained `dirs-next` dependency is gone (the home
  folder now comes from `std::env::home_dir`, the Downloads folder from `dirs`;
  on Windows the home folder is read from `USERPROFILE` first), four one-off
  test-cluster scripts are deleted, and the stress binary has one query parser.
