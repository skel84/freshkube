# Database explorer

Code: `freshkube-db` (`crates/freshkube-db/src/`) is the session, with no GPUI and nothing of desktop: `postgres.rs` (the read-only Postgres session), `readonly.rs` (the statement check), `limits.rs` (timeouts and row cap), `wire.rs` (the `tokio-postgres` connection under it), `session.rs` (the `Session` trait and its types), `credentials.rs` (the in-memory password) and `fake.rs` (the example `shop` database and a recording fake server). `freshkube-core`'s `databases/explore.rs` picks the pod and reads the login. `freshkube-desktop/src/screens/databases/explore/` is the tab: `mod.rs` (the connection), `browser.rs` (schemas and tables), `data.rs` (a table's rows and the record Inspector), `view.rs` and `fail.rs` (the connect step's states). Design and slices: [#948](https://github.com/skel84/freshkube/issues/948). Tests: the unit tests in each `freshkube-db` module, core's `databases::explore::tests` and desktop's `explore/tests.rs`.

This is slice E1: PostgreSQL by CloudNativePG, read-only, through a port forward. The query tab (E2), Teleport (E3), the `databases:` block of Team setup (E4), MySQL and MariaDB (E5) and writes (E7) are later; until E4 the row cap (200) and the timeouts are built in.

## Opening it

**Explore** on a Databases row (the toolbar, the row menu, `E`) opens one tab per database beside the inventory's Clusters, Backups and Schedules views; it is not a rail area. Only a CloudNativePG database can be explored; MariaDB says so. Closing the tab closes the connection and stops the forward it started. Changing the connection (cluster) closes every explorer tab.

## Connecting

Four steps, each a state the tab can stay in, with words that say what to do:

1. **Choose a pod** from the Cluster's instances: a ready replica by default (first by name), else the ready primary. *No pod is ready* otherwise.
2. **Forward to it** through the app's `ForwardList` ([PORT_FORWARD.md](../PORT_FORWARD.md)): a pod forward tied to the pod's UID, on an automatic local port, port 5432. It appears in the status bar like any forward, and ends with the tab. *The forward failed* carries the forward's reason.
3. **Read the login and connect.** The team's reader Secret comes first (the name arrives with E4; until then none is named), then the operator's application user (`<cluster>-app`). The superuser Secret is never read or offered. *No reader Secret* when neither exists; *The Secret has no login* when it lacks a username or password.
4. **Make the session read-only** and verify it. *The database refused the connection*, *did not answer in time*, *could not be reached*, and *The session could not be made read-only* are the other outcomes. A failed connection offers Try again; nothing retries by itself.

### The Secret

This is the one place Databases reads a Secret. It is read **once per connection**, inside the connect job after the forward listens, through core's `databases::explore::read_login` (the explicit call site), and goes straight into a zeroizing in-memory `Password` (`Login` in core): no `Display`, `Clone` or `Serialize`, and a `Debug` that prints `<redacted>`. Only the username, password and database name are taken from the Secret; its other keys (connection strings that embed the password) are dropped at once. The password is never logged, never in an error, history, the audit log, the document, Search, a deep link or a saved file, and it is dropped (wiped) when the connection is made. Tests assert it is absent from the `Debug`/`to_string()` of a failed connect, the connect states and `Connected`.

**The superuser is refused, not just not offered.** A reader input or app Secret named `<cluster>-superuser`, or the Secret the Cluster's `spec.superuserSecret` names, is refused before any Secret is read; so is any login whose username is `postgres`, whatever its Secret is called. (The Cluster object is read for that spec field, as it is for `serverCASecret`.)

**One more read: the CA's certificate.** To check the server's certificate, core reads the cluster's CA Secret (`<cluster>-ca`, or `spec.certificates.serverCASecret`) and takes only `ca.crt`; the rest of that Secret, which holds the CA's private key, is dropped unread. When it can't be read the connection is encrypted without being verified, and says so.

## TLS

CloudNativePG enables TLS by default (`ssl = 'on'`, operator-managed server certificate) and its generated `pg_hba.conf` ends with `host all all all scram-sha-256` (`md5` on PostgreSQL 13 and older), so both TLS and plain logins are admitted for ordinary users; `hostssl … cert` rules cover only `streaming_replica` and the pooler. The server certificate covers the `<cluster>-rw`, `-r` and `-ro` Services (also with `.<namespace>` and `.<namespace>.svc`) ([postgresql_conf](https://cloudnative-pg.io/docs/current/postgresql_conf), [certificates](https://cloudnative-pg.io/docs/current/certificates)).

The session behaves like `sslmode=prefer` with rustls (`tokio-postgres-rustls`, `ring`; no OpenSSL): it tries TLS and falls back to plain **only when the server doesn't offer it**; a server that offers TLS and fails the handshake or the certificate check is an error (*The server's certificate was not accepted*), never a downgrade. Through a port forward the address is loopback, so the name checked is the Service's: `<cluster>-ro.<namespace>.svc` for a replica, `-rw` for the primary (`hostaddr` is the forward, `host` the name). The strip says which happened: *encrypted, certificate verified* (checked against `ca.crt`), *encrypted, certificate not verified* (no CA could be read), or *not encrypted* (the server offered no TLS), found by asking the server (`pg_stat_ssl`).

## Read-only, enforced by the session

`freshkube_db::postgres::Postgres::open` sends `SET default_transaction_read_only = on`, `SET standard_conforming_strings = on` (the string literals we build rely on it) and our timeouts, then asks the server what it holds; a server that does not report `on` is refused, never used. Every request then runs as `ROLLBACK` (ends a transaction a cut-off request left open), `BEGIN READ ONLY`, a **server-side single-statement check**, the statement, `ROLLBACK` (also after a failure). The check prepares the text with the extended protocol, which the server refuses unless it is exactly one statement, so a splitter bypass can't reach `simple_query`. A request that is `SET`, `RESET`, `BEGIN`, `COMMIT`, `ROLLBACK` or more than one statement is also refused before it is sent (`readonly.rs`; a `$` after an identifier character belongs to the identifier, not to a dollar quote); that check only explains, the session is what enforces. Our own timeouts go out at connect, since DBFlux's drivers set none: `statement_timeout` (30 s), `lock_timeout` (5 s), `idle_in_transaction_session_timeout` (60 s), and a 10 s connect timeout. A table page asks for the cap plus one row to know there were more; at most 200 are shown, marked *Capped*. The schema and table listings are bounded too (500 schemas, 2,000 tables per schema, marked capped).

## The tab

The strip shows *Read-only session*, what protects the connection, who the session is (user and Secret kind, never a password), the pod and local port, the table summary, *Capped* when it is, Read again, and the writes gate: a disabled **Edit data** labelled *later slice*, with nothing behind it.

The browser sits beside the rows from 900 dp of page width and above them below it; the failure states scroll. The schema browser is the shared table: each schema is a group row, opened by its chevron or a click; the first (`public`, else the first) opens on its own and the others read their tables only when opened. Rows carry the kind, the planner's row estimate (`–` before the table is analysed) and the total size. Choosing a table reads its columns and its first rows; a row opens its record in the Inspector (column, type, key and nullability, value; `NULL` as such).

## Lifetime

The connection is user-started and belongs to its tab ([STATE_LIFETIMES.md](../STATE_LIFETIMES.md#database-explorer)). Reads (pick, connect, schemas, tables, columns, rows) are `Loader`s: a Tokio job plus a delivery task, dropped when the tab hides or connects again, landing only for the epoch they were asked in. A hidden tab reads nothing; showing it resumes what was cut short. The connected session is kept while hidden.

## Example data

`--fixture` uses `freshkube_db::fake` and no forward or Secret. `FRESHKUBE_DB_EXPLORE` (AGENTS.md) opens the first PostgreSQL database in the explorer and shapes how the connection ends. The example `shop` database has the schemas `public` and `audit`; `orders` has 250 rows (more than the cap), `customers` 12, `products` none.

## Dependency

`freshkube-db` depends on DBFlux's `dbflux_core` ([0xErwin1/dbflux](https://github.com/0xErwin1/dbflux), MIT OR Apache-2.0), pinned by `rev`, for `Value` and `DbError`. DBFlux's Postgres driver is not used: it needs a system OpenSSL through `native-tls` and `dbflux_ssh`, with no feature to drop them, and Freshkube ships none. `dbflux_core`'s Linux keyring needs libdbus, which `freshkube-db` builds from source (`libdbus-sys` with `vendored`), so nothing is installed on the build machine. The session is therefore ours, on `tokio-postgres`. `dbflux_core` brings `keyring` 3 (not optional), the dbus crates and `tree-sitter`/`zstd-sys` for two types; on a clean Linux debug build of `freshkube-db` that is about 190 CPU-seconds of the crate's roughly 575 (`zstd-sys` 64 s, `dbflux_core` 33 s, `dbus` and `libdbus-sys` 34 s); roughly a minute of wall-clock time on a multi-core runner. E2 may define `Value` and `DbError` locally until it needs the `Connection` trait. See the NOTICE file.
