//! The Postgres session: read-only, with timeouts, over any [`Wire`].
//!
//! At connect it sends `SET default_transaction_read_only = on` and the three
//! timeouts, then asks the server what it holds and refuses a session that
//! isn't read-only. Every request then runs as `BEGIN READ ONLY`, the
//! statement, `ROLLBACK`. A request that is `SET`, `BEGIN` or `COMMIT`
//! (`readonly.rs`) is refused before it reaches the wire. The server is what
//! enforces it; the check only explains.
use crate::{
    limits::Limits,
    readonly::{self, Refusal},
    session::{
        Canceller, ColumnEntry, ConnectError, Listed, MAX_PLAN_LINES, MAX_SCHEMAS, MAX_TABLES,
        Plan, QueryError, QueryResult, Session, TableEntry, TableKind, TablePage,
    },
    wire::{Rows, Wire},
};
use async_trait::async_trait;
use dbflux_core::{DbError, Value};
use std::sync::Arc;

pub struct Postgres<W: Wire> {
    wire: W,
    limits: Limits,
}

impl<W: Wire> Postgres<W> {
    /// Makes `wire` a read-only session under `limits`. Everything sent here
    /// is ours; nothing a caller typed goes through `setup`.
    pub async fn open(mut wire: W, limits: Limits) -> Result<Self, ConnectError> {
        let work = async {
            for sql in setup_statements(&limits) {
                wire.simple(&sql).await.map_err(not_read_only)?;
            }
            let held = wire
                .simple("SHOW default_transaction_read_only")
                .await
                .map_err(not_read_only)?;
            match held.rows.first().and_then(|r| r.first()) {
                Some(Some(v)) if v == "on" => Ok(()),
                other => Err(ConnectError::NotReadOnly(format!(
                    "The server reports default_transaction_read_only = {}",
                    other.and_then(|v| v.as_deref()).unwrap_or("nothing")
                ))),
            }
        };
        match tokio::time::timeout(limits.connect_timeout, work).await {
            Err(_) => return Err(ConnectError::TimedOut),
            Ok(result) => result?,
        }
        Ok(Self { wire, limits })
    }

    /// Ends whatever transaction the connection is in, and lets go of any
    /// session-level advisory lock. The refused functions are how a statement
    /// would take one; this is the second line.
    #[allow(
        clippy::result_large_err,
        reason = "DbError is DBFlux's type, and large; every read here returns it"
    )]
    async fn end_transaction(&mut self) -> Result<(), DbError> {
        self.wire.simple("ROLLBACK").await?;
        self.wire.simple("SELECT pg_advisory_unlock_all()").await?;
        Ok(())
    }

    /// One request of a caller's: checked, then run in a read-only transaction
    /// that is always rolled back. Reads at most `row_cap + 1` rows' worth is the
    /// caller's SQL (`LIMIT`); this cuts anything longer.
    #[allow(
        clippy::result_large_err,
        reason = "DbError is DBFlux's type, and large; every read here returns it"
    )]
    pub async fn read(&mut self, sql: &str) -> Result<Rows, DbError> {
        self.read_up_to(sql, self.limits.rows + 1).await
    }

    /// As [`read`](Self::read), keeping at most `keep` rows.
    #[allow(
        clippy::result_large_err,
        reason = "DbError is DBFlux's type, and large; every read here returns it"
    )]
    async fn read_up_to(&mut self, sql: &str, keep: usize) -> Result<Rows, DbError> {
        readonly::check_statement(sql).map_err(|r: Refusal| DbError::query_failed(r.explain()))?;
        // A request cut off earlier may have left the connection inside its
        // transaction; end any before starting this one.
        self.end_transaction().await?;
        self.wire.simple("BEGIN READ ONLY").await?;
        // The server's word that this is one statement, whatever our
        // splitter made of the text.
        if let Err(e) = self.wire.single_statement(sql).await {
            let _ = self.end_transaction().await;
            return Err(e);
        }
        let result = self.wire.simple(sql).await;
        // The rollback also ends an aborted transaction after a failed statement.
        let rolled = self.end_transaction().await;
        let mut rows = result?;
        rolled?;
        rows.rows.truncate(keep);
        Ok(rows)
    }
}

/// What `run` sends for a statement that returns rows: a cursor, so the
/// server hands over only the cap and one more, never the whole result.
const CURSOR: &str = "freshkube_query";

fn returns_rows(word: &str) -> bool {
    matches!(word, "select" | "with" | "values" | "table")
}

impl<W: Wire> Postgres<W> {
    /// One statement of the query tab: checked, then run in a read-only
    /// transaction that is always rolled back. A statement that returns rows
    /// is read through a cursor, `row_cap + 1` rows at most. The database,
    /// not this function, refuses a write; nothing is retried.
    async fn run_statement(&mut self, sql: &str) -> Result<QueryResult, QueryError> {
        readonly::check_statement(sql).map_err(|r: Refusal| QueryError::NotSent(r.explain()))?;
        let started = std::time::Instant::now();
        let cap = self.limits.rows;
        let cursor = returns_rows(&readonly::leading_word(sql));
        self.end_transaction().await?;
        self.wire.simple("BEGIN READ ONLY").await?;
        let result = self.run_in_transaction(sql, cursor, cap).await;
        // Also ends an aborted transaction after a failed statement.
        let rolled = self.end_transaction().await;
        let mut rows = result?;
        rolled?;
        let capped = rows.rows.len() > cap;
        rows.rows.truncate(cap);
        Ok(QueryResult {
            columns: rows.columns,
            rows: rows
                .rows
                .into_iter()
                .map(|r| {
                    r.into_iter()
                        .map(|c| c.map_or(Value::Null, Value::Text))
                        .collect()
                })
                .collect(),
            capped,
            row_cap: cap,
            elapsed: started.elapsed(),
        })
    }

    #[allow(
        clippy::result_large_err,
        reason = "DbError is DBFlux's type, and large; every read here returns it"
    )]
    async fn run_in_transaction(
        &mut self,
        sql: &str,
        cursor: bool,
        cap: usize,
    ) -> Result<Rows, DbError> {
        // The server's word that this is one statement, before the text is
        // sent anywhere it could be read as more.
        self.wire.single_statement(sql).await?;
        if !cursor {
            return self.wire.simple(sql).await;
        }
        self.wire
            .simple(&format!("DECLARE {CURSOR} NO SCROLL CURSOR FOR {sql}"))
            .await?;
        self.wire
            .simple(&format!("FETCH {} FROM {CURSOR}", cap + 1))
            .await
    }
}

fn setup_statements(limits: &Limits) -> Vec<String> {
    vec![
        "SET default_transaction_read_only = on".to_string(),
        // The string literals we build rely on backslashes being plain.
        "SET standard_conforming_strings = on".to_string(),
        format!(
            "SET statement_timeout = '{}ms'",
            limits.statement_timeout.as_millis()
        ),
        format!("SET lock_timeout = '{}ms'", limits.lock_timeout.as_millis()),
        format!(
            "SET idle_in_transaction_session_timeout = '{}ms'",
            limits.idle_in_transaction_timeout.as_millis()
        ),
    ]
}

fn not_read_only(error: DbError) -> ConnectError {
    match error {
        DbError::Timeout => ConnectError::TimedOut,
        other => ConnectError::NotReadOnly(other.to_string()),
    }
}

pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn count(text: &Option<String>) -> Option<u64> {
    text.as_deref()?
        .parse::<i64>()
        .ok()
        .and_then(|n| u64::try_from(n).ok())
}

#[allow(
    clippy::double_must_use,
    reason = "the async_trait expansion marks its boxed futures must_use"
)]
#[allow(
    clippy::result_large_err,
    reason = "DbError is DBFlux's type, and large; every read here returns it"
)]
#[async_trait]
impl<W: Wire> Session for Postgres<W> {
    fn read_only(&self) -> bool {
        true
    }

    async fn schemas(&mut self) -> Result<Listed<String>, DbError> {
        let rows = self
            .read_up_to(
                &format!(
                    "SELECT nspname FROM pg_namespace \
                 WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' \
                 ORDER BY nspname LIMIT {}",
                    MAX_SCHEMAS + 1
                ),
                MAX_SCHEMAS + 1,
            )
            .await?;
        let mut items: Vec<String> = rows
            .rows
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .collect();
        let capped = items.len() > MAX_SCHEMAS;
        items.truncate(MAX_SCHEMAS);
        Ok(Listed { items, capped })
    }

    async fn tables(&mut self, schema: &str) -> Result<Listed<TableEntry>, DbError> {
        let sql = format!(
            "SELECT c.relname, c.relkind::text, c.reltuples::bigint::text, \
                    pg_total_relation_size(c.oid)::text \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {} AND c.relkind IN ('r','v','m','f','p') \
             ORDER BY c.relname LIMIT {}",
            quote_literal(schema),
            MAX_TABLES + 1
        );
        let rows = self.read_up_to(&sql, MAX_TABLES + 1).await?;
        let mut items: Vec<TableEntry> = rows
            .rows
            .into_iter()
            .filter_map(|r| {
                let name = r.first()?.clone()?;
                let kind = match r.get(1)?.as_deref()? {
                    "v" => TableKind::View,
                    "m" => TableKind::Materialized,
                    "f" => TableKind::Foreign,
                    "p" => TableKind::Partitioned,
                    _ => TableKind::Table,
                };
                // reltuples is -1 until the table is first analysed.
                let rows_estimate = count(r.get(2)?);
                Some(TableEntry {
                    name,
                    kind,
                    rows_estimate,
                    size_bytes: count(r.get(3)?),
                })
            })
            .collect();
        let capped = items.len() > MAX_TABLES;
        items.truncate(MAX_TABLES);
        Ok(Listed { items, capped })
    }

    async fn columns(&mut self, schema: &str, table: &str) -> Result<Vec<ColumnEntry>, DbError> {
        let sql = format!(
            "SELECT a.attname, format_type(a.atttypid, a.atttypmod), \
                    (NOT a.attnotnull)::text, pg_get_expr(d.adbin, d.adrelid), \
                    EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid \
                            AND i.indisprimary AND a.attnum = ANY (i.indkey))::text \
             FROM pg_attribute a \
             JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE n.nspname = {} AND c.relname = {} AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            quote_literal(schema),
            quote_literal(table)
        );
        let rows = self.read(&sql).await?;
        Ok(rows
            .rows
            .into_iter()
            .filter_map(|r| {
                Some(ColumnEntry {
                    name: r.first()?.clone()?,
                    type_name: r.get(1)?.clone()?,
                    nullable: r.get(2)?.as_deref() == Some("true"),
                    default: r.get(3)?.clone(),
                    primary_key: r.get(4)?.as_deref() == Some("true"),
                })
            })
            .collect())
    }

    async fn run(&mut self, sql: &str) -> Result<QueryResult, QueryError> {
        self.run_statement(sql).await
    }

    async fn explain(&mut self, sql: &str) -> Result<Plan, QueryError> {
        // A statement that is already a plan is not wrapped, and is not run
        // here: `explain analyze` would execute what it explains.
        if readonly::leading_word(sql) == "explain" {
            return Err(QueryError::NotSent("Already a plan; use Run".into()));
        }
        readonly::check_statement(sql).map_err(|r: Refusal| QueryError::NotSent(r.explain()))?;
        let text = format!("EXPLAIN (FORMAT TEXT) {sql}");
        let rows = self.read_up_to(&text, MAX_PLAN_LINES + 1).await?;
        let mut lines: Vec<String> = rows
            .rows
            .into_iter()
            .map(|r| r.into_iter().next().flatten().unwrap_or_default())
            .collect();
        let capped = lines.len() > MAX_PLAN_LINES;
        lines.truncate(MAX_PLAN_LINES);
        Ok(Plan { lines, capped })
    }

    fn canceller(&self) -> Arc<dyn Canceller> {
        self.wire.canceller()
    }

    async fn table_rows(&mut self, schema: &str, table: &str) -> Result<TablePage, DbError> {
        let cap = self.limits.rows;
        let sql = format!(
            "SELECT * FROM {}.{} LIMIT {}",
            quote_ident(schema),
            quote_ident(table),
            cap + 1
        );
        let mut rows = self.read(&sql).await?;
        let capped = rows.rows.len() > cap;
        rows.rows.truncate(cap);
        Ok(TablePage {
            columns: rows.columns,
            rows: rows
                .rows
                .into_iter()
                .map(|r| {
                    r.into_iter()
                        .map(|c| c.map_or(Value::Null, Value::Text))
                        .collect()
                })
                .collect(),
            capped,
            row_cap: cap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeWire, text_rows};
    use std::time::Duration;

    const UNLOCK: &str = "SELECT pg_advisory_unlock_all()";

    /// The request ended rolled back, with the session's advisory locks let go.
    fn assert_ended(wire: &FakeWire) {
        let sent = wire.sent();
        assert_eq!(sent[sent.len() - 2..], ["ROLLBACK", UNLOCK], "{sent:?}");
    }

    async fn open(wire: &FakeWire, limits: Limits) -> Postgres<FakeWire> {
        Postgres::open(wire.clone(), limits).await.unwrap()
    }

    #[tokio::test]
    async fn connect_sends_read_only_and_every_timeout_before_anything_else() {
        let wire = FakeWire::new();
        let limits = Limits {
            statement_timeout: Duration::from_millis(1500),
            lock_timeout: Duration::from_secs(2),
            idle_in_transaction_timeout: Duration::from_secs(45),
            ..Limits::default()
        };
        open(&wire, limits).await;
        assert_eq!(
            wire.sent(),
            vec![
                "SET default_transaction_read_only = on",
                "SET standard_conforming_strings = on",
                "SET statement_timeout = '1500ms'",
                "SET lock_timeout = '2000ms'",
                "SET idle_in_transaction_session_timeout = '45000ms'",
                "SHOW default_transaction_read_only",
            ]
        );
    }

    #[tokio::test]
    async fn a_server_that_keeps_the_setting_off_is_not_used() {
        let wire = FakeWire::ignoring_read_only();
        let error = Postgres::open(wire, Limits::default()).await.err().unwrap();
        assert!(matches!(error, ConnectError::NotReadOnly(_)), "{error:?}");
    }

    #[tokio::test]
    async fn every_request_is_a_read_only_transaction_that_is_rolled_back() {
        let wire = FakeWire::new();
        let mut session = open(&wire, Limits::default()).await;
        let before = wire.sent().len();
        session.read("SELECT 1").await.unwrap();
        assert_eq!(
            wire.sent()[before..],
            [
                "ROLLBACK",
                UNLOCK,
                "BEGIN READ ONLY",
                "PARSE SELECT 1",
                "SELECT 1",
                "ROLLBACK",
                UNLOCK
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_request_is_still_rolled_back_and_keeps_its_error() {
        let wire = FakeWire::new();
        wire.time_out("boom");
        let mut session = open(&wire, Limits::default()).await;
        let before = wire.sent().len();
        let error = session.read("SELECT boom").await.unwrap_err();
        assert!(matches!(error, DbError::Timeout));
        assert_ended(&wire);
        assert_eq!(wire.sent().len() - before, 7);
    }

    #[tokio::test]
    async fn the_server_refusing_several_statements_stops_a_request_our_splitter_passed() {
        let wire = FakeWire::new();
        wire.refuse_as_several("hidden");
        let mut session = open(&wire, Limits::default()).await;
        let error = session.read("SELECT 1 /* hidden */").await.unwrap_err();
        assert!(error.to_string().contains("multiple commands"), "{error}");
        // The statement itself was never sent, and the transaction was ended.
        let sent = wire.sent();
        assert!(
            !sent.iter().any(|s| s == "SELECT 1 /* hidden */"),
            "{sent:?}"
        );
        assert_ended(&wire);
    }

    #[tokio::test]
    async fn catalog_listings_stop_at_their_bound_and_say_so() {
        let wire = FakeWire::new();
        let many: Vec<Vec<Option<&str>>> = (0..=MAX_SCHEMAS).map(|_| vec![Some("s")]).collect();
        let refs: Vec<&[Option<&str>]> = many.iter().map(|r| r.as_slice()).collect();
        wire.answer("FROM pg_namespace", text_rows(&["nspname"], &refs));
        let mut session = open(&wire, Limits::default()).await;
        let listed = session.schemas().await.unwrap();
        assert_eq!((listed.items.len(), listed.capped), (MAX_SCHEMAS, true));
        assert!(
            wire.sent()
                .iter()
                .any(|s| s.ends_with(&format!("LIMIT {}", MAX_SCHEMAS + 1)))
        );
    }

    #[tokio::test]
    async fn set_begin_and_commit_never_reach_the_wire() {
        let wire = FakeWire::new();
        let mut session = open(&wire, Limits::default()).await;
        let before = wire.sent();
        for sql in [
            "SET default_transaction_read_only = off",
            "BEGIN",
            "COMMIT",
            "SELECT 1; COMMIT",
            "set role postgres",
        ] {
            assert!(session.read(sql).await.is_err(), "{sql}");
        }
        assert_eq!(wire.sent(), before);
    }

    #[tokio::test]
    async fn a_table_page_stops_at_the_row_cap_and_says_so() {
        let wire = FakeWire::new();
        let many: Vec<Vec<Option<&str>>> = (0..4).map(|_| vec![Some("a")]).collect();
        let refs: Vec<&[Option<&str>]> = many.iter().map(|r| r.as_slice()).collect();
        wire.answer("FROM \"public\".\"orders\"", text_rows(&["id"], &refs));
        let limits = Limits {
            rows: 3,
            ..Limits::default()
        };
        let mut session = open(&wire, limits).await;
        let page = session.table_rows("public", "orders").await.unwrap();
        assert_eq!((page.rows.len(), page.capped, page.row_cap), (3, true, 3));
        assert!(
            wire.sent().iter().any(|s| s.ends_with("LIMIT 4")),
            "{:?}",
            wire.sent()
        );
    }

    #[tokio::test]
    async fn a_table_under_the_cap_is_not_capped() {
        let wire = FakeWire::new();
        wire.answer(
            "FROM \"public\".\"t\"",
            text_rows(&["id"], &[&[Some("1")], &[None]]),
        );
        let mut session = open(
            &wire,
            Limits {
                rows: 3,
                ..Limits::default()
            },
        )
        .await;
        let page = session.table_rows("public", "t").await.unwrap();
        assert!(!page.capped);
        assert_eq!(
            page.rows,
            vec![vec![Value::Text("1".into())], vec![Value::Null]]
        );
    }

    #[tokio::test]
    async fn names_are_quoted_and_a_quote_in_a_name_cannot_end_the_identifier() {
        let wire = FakeWire::new();
        let mut session = open(&wire, Limits::default()).await;
        session.table_rows("sh\"op", "o'rders").await.unwrap();
        assert!(
            wire.sent()
                .iter()
                .any(|s| s.contains("\"sh\"\"op\".\"o'rders\""))
        );
        session.columns("sh'op", "t").await.unwrap();
        assert!(wire.sent().iter().any(|s| s.contains("'sh''op'")));
    }

    #[tokio::test]
    async fn tables_read_an_unanalysed_estimate_as_unknown() {
        let wire = FakeWire::new();
        wire.answer(
            "FROM pg_class",
            text_rows(
                &["relname", "relkind", "reltuples", "size"],
                &[
                    &[Some("orders"), Some("r"), Some("-1"), Some("8192")],
                    &[Some("v"), Some("v"), Some("0"), Some("0")],
                ],
            ),
        );
        let mut session = open(&wire, Limits::default()).await;
        let tables = session.tables("public").await.unwrap().items;
        assert_eq!(tables[0].rows_estimate, None);
        assert_eq!(tables[0].size_bytes, Some(8192));
        assert_eq!(tables[1].kind, TableKind::View);
    }

    // --- the query tab (E2) ---

    #[tokio::test]
    async fn a_write_is_sent_refused_by_the_database_and_nothing_is_committed() {
        let wire = FakeWire::new();
        let mut session = open(&wire, Limits::default()).await;
        for sql in [
            "INSERT INTO orders (id) VALUES (1)",
            "update orders set status = 'paid'",
            "DELETE FROM orders",
            "DROP TABLE orders",
            "TRUNCATE orders",
        ] {
            let error = session.run(sql).await.unwrap_err();
            assert!(matches!(error, QueryError::Refused(_)), "{sql}: {error:?}");
            assert!(error.to_string().contains("read-only"), "{error}");
            // Sent once, never again: no retry.
            let sent = wire.sent();
            assert_eq!(
                sent.iter().filter(|s| s.as_str() == sql).count(),
                1,
                "{sql}"
            );
            assert_ended(&wire);
        }
        assert!(wire.applied().is_empty(), "{:?}", wire.applied());
        assert!(!wire.sent().iter().any(|s| s.contains("COMMIT")));
    }

    #[tokio::test]
    async fn a_server_that_ignored_the_setting_would_have_applied_the_write() {
        // The control for the test above: the refusal comes from the
        // session's read-only transaction, which this fake does not have.
        let wire = FakeWire::ignoring_read_only();
        wire.fail("never", "x");
        let mut raw = wire.clone();
        raw.simple("DELETE FROM orders").await.unwrap();
        assert_eq!(wire.applied(), vec!["DELETE FROM orders"]);
    }

    #[tokio::test]
    async fn a_request_that_changes_the_session_is_not_sent_and_says_so() {
        let wire = FakeWire::new();
        let mut session = open(&wire, Limits::default()).await;
        let before = wire.sent();
        for sql in [
            "SET default_transaction_read_only = off",
            "COMMIT",
            "select 1; select 2",
            " ",
        ] {
            let error = session.run(sql).await.unwrap_err();
            assert!(matches!(error, QueryError::NotSent(_)), "{sql}: {error:?}");
        }
        assert_eq!(wire.sent(), before);
    }

    #[tokio::test]
    async fn a_query_reads_through_a_cursor_and_stops_at_the_row_cap() {
        let wire = FakeWire::new();
        let many: Vec<Vec<Option<&str>>> = (0..6).map(|_| vec![Some("a")]).collect();
        let refs: Vec<&[Option<&str>]> = many.iter().map(|r| r.as_slice()).collect();
        wire.answer("from big", text_rows(&["id"], &refs));
        let limits = Limits {
            rows: 5,
            ..Limits::default()
        };
        let mut session = open(&wire, limits).await;
        let before = wire.sent().len();
        let result = session.run("select id from big").await.unwrap();
        assert_eq!(
            (result.rows.len(), result.capped, result.row_cap),
            (5, true, 5)
        );
        assert_eq!(
            wire.sent()[before..],
            [
                "ROLLBACK",
                UNLOCK,
                "BEGIN READ ONLY",
                "PARSE select id from big",
                "DECLARE freshkube_query NO SCROLL CURSOR FOR select id from big",
                "FETCH 6 FROM freshkube_query",
                "ROLLBACK",
                UNLOCK
            ]
        );
    }

    #[tokio::test]
    async fn a_statement_that_returns_no_cursor_rows_runs_as_it_is_and_is_cut_at_the_cap() {
        let wire = FakeWire::new();
        wire.answer("SHOW", text_rows(&["v"], &[&[Some("16.2")]]));
        let mut session = open(&wire, Limits::default()).await;
        let result = session.run("SHOW server_version").await.unwrap();
        assert_eq!(result.rows, vec![vec![Value::Text("16.2".into())]]);
        assert!(!result.capped);
        assert!(!wire.sent().iter().any(|s| s.starts_with("DECLARE")));
    }

    #[tokio::test]
    async fn a_database_error_keeps_its_words_and_the_transaction_is_ended() {
        let wire = FakeWire::new();
        wire.fail("nope", "relation \"nope\" does not exist");
        let mut session = open(&wire, Limits::default()).await;
        let error = session.run("select * from nope").await.unwrap_err();
        assert_eq!(
            error,
            QueryError::Failed("relation \"nope\" does not exist".into())
        );
        assert_ended(&wire);
        wire.time_out("slow");
        let error = session.run("select * from slow").await.unwrap_err();
        assert_eq!(error, QueryError::TimedOut);
    }

    #[tokio::test]
    async fn cancel_stops_the_running_statement_without_the_session() {
        let wire = FakeWire::new();
        wire.hold("pg_sleep");
        let session = open(&wire, Limits::default()).await;
        let cancel = {
            let session: &dyn Session = &session;
            session.canceller()
        };
        let mut session = session;
        let running = tokio::spawn(async move { session.run("select pg_sleep(60)").await });
        // The statement is out and waiting; the session itself is busy.
        while !wire.sent().iter().any(|s| s.contains("pg_sleep")) {
            tokio::task::yield_now().await;
        }
        assert!(!running.is_finished());
        cancel.cancel().await.unwrap();
        let error = running.await.unwrap().unwrap_err();
        assert_eq!(error, QueryError::Cancelled);
        assert_eq!(wire.cancels(), 1);
        // The cancelled statement's transaction was rolled back.
        assert_ended(&wire);
    }

    #[tokio::test]
    async fn a_plan_is_explain_without_analyze_and_a_statement_that_is_a_plan_is_not_sent() {
        let wire = FakeWire::new();
        wire.answer(
            "EXPLAIN",
            text_rows(
                &["QUERY PLAN"],
                &[&[Some("Seq Scan on orders")], &[Some("  Filter: x")]],
            ),
        );
        let mut session = open(&wire, Limits::default()).await;
        let plan = session.explain("select * from orders").await.unwrap();
        assert_eq!(plan.lines, vec!["Seq Scan on orders", "  Filter: x"]);
        assert!(
            wire.sent()
                .iter()
                .any(|s| s == "EXPLAIN (FORMAT TEXT) select * from orders")
        );
        assert!(
            !wire
                .sent()
                .iter()
                .any(|s| s.to_uppercase().contains("ANALYZE"))
        );
        // Anything that already starts with EXPLAIN is not sent at all, so
        // `explain analyze` never runs the statement it explains.
        let before = wire.sent();
        for sql in [
            "explain select 1",
            "explain analyze select 1",
            "EXPLAIN (ANALYSE) select 1",
            "explain (analyse true) delete from orders",
        ] {
            assert_eq!(
                session.explain(sql).await.unwrap_err(),
                QueryError::NotSent("Already a plan; use Run".into()),
                "{sql}"
            );
        }
        assert_eq!(wire.sent(), before);
        // A request that would change the session is not explained either.
        assert!(matches!(
            session.explain("set role x").await.unwrap_err(),
            QueryError::NotSent(_)
        ));
    }

    #[tokio::test]
    async fn a_real_wire_session_runs_a_statement_and_is_cancelled_through_its_handle() {
        use crate::{
            credentials::Password,
            fake::FakeServer,
            wire::{Target, TlsPlan, TokioPostgres},
        };
        let server = FakeServer::start().await;
        let plan = TlsPlan {
            server_name: "orders-db-ro.staging.svc".into(),
            ca_pem: None,
        };
        let (wire, _) = TokioPostgres::connect(
            Target {
                address: "127.0.0.1".parse().unwrap(),
                port: server.port(),
                user: "shop_reader",
                database: "shop",
            },
            &Password::new("hunter2"),
            &Limits::default(),
            &plan,
        )
        .await
        .map_err(|e| e.to_string())
        .unwrap();
        let mut session = Postgres::open(wire, Limits::default()).await.unwrap();
        let result = session.run("select 1").await.unwrap();
        assert_eq!(result.rows.len(), 1);
        let cancel = session.canceller();
        let running = tokio::spawn(async move { session.run("select pg_sleep(60)").await });
        // The statement is with the server; the session is busy, and Cancel
        // does not need it.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        cancel.cancel().await.unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(10), running)
            .await
            .expect("the statement ended")
            .unwrap()
            .unwrap_err();
        assert_eq!(error, QueryError::Cancelled);
        assert_eq!(server.cancels(), 1);
    }
}
