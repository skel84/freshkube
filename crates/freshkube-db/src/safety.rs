//! Guarantee 20 of the safety suite (docs/SAFETY.md): a database session never
//! writes and never leaves its read-only transaction. The tests drive the
//! Postgres session over the recording fake wire and look at what was sent,
//! and each fails when its guard is removed.
use crate::{
    fake::{FakeWire, text_rows},
    limits::Limits,
    postgres::Postgres,
    readonly::{FUNCTIONS, MAINTENANCE},
    session::{ConnectError, QueryError, Session},
};

const UNLOCK: &str = "SELECT pg_advisory_unlock_all()";

async fn open(wire: &FakeWire) -> Postgres<FakeWire> {
    Postgres::open(wire.clone(), Limits::default())
        .await
        .unwrap()
}

#[tokio::test]
async fn safety_g20_a_write_is_refused_by_the_read_only_transaction_alone_and_nothing_is_applied() {
    let wire = FakeWire::new();
    let mut session = open(&wire).await;
    // The connect-time setting is gone: only each request's own
    // `BEGIN READ ONLY` stands between the statement and the data.
    wire.turn_setting_off();
    for sql in [
        "INSERT INTO orders (id) VALUES (1)",
        "update orders set status = 'paid'",
        "DELETE FROM orders",
        "DROP TABLE orders",
        "TRUNCATE orders",
        "CREATE TABLE t (id int)",
    ] {
        let error = session.run(sql).await.unwrap_err();
        assert!(matches!(error, QueryError::Refused(_)), "{sql}: {error:?}");
        // Sent once and never again: a refusal is not retried.
        assert_eq!(wire.sent().iter().filter(|s| s.as_str() == sql).count(), 1);
    }
    assert!(wire.applied().is_empty(), "{:?}", wire.applied());
}

#[tokio::test]
async fn safety_g20_a_server_that_does_not_hold_read_only_is_never_used() {
    let wire = FakeWire::ignoring_read_only();
    let error = Postgres::open(wire.clone(), Limits::default())
        .await
        .err()
        .expect("refused");
    assert!(matches!(error, ConnectError::NotReadOnly(_)), "{error:?}");
    // The setting was asked for at connect, before anything else.
    assert_eq!(
        wire.sent().first().unwrap(),
        "SET default_transaction_read_only = on"
    );
}

#[tokio::test]
async fn safety_g20_statements_that_leave_the_transaction_or_change_the_session_are_not_sent() {
    let wire = FakeWire::new();
    let mut session = open(&wire).await;
    let before = wire.sent();
    for sql in [
        "SET default_transaction_read_only = off",
        "RESET ALL",
        "set role postgres",
        "BEGIN",
        "start transaction read write",
        "COMMIT",
        "ROLLBACK",
        "SAVEPOINT a",
        "select 1; commit",
        "select 1; select 2",
        "select set_config('default_transaction_read_only', 'off', false)",
    ] {
        let error = session.run(sql).await.unwrap_err();
        assert!(matches!(error, QueryError::NotSent(_)), "{sql}: {error:?}");
    }
    assert_eq!(wire.sent(), before);
}

#[tokio::test]
async fn safety_g20_maintenance_and_functions_that_act_on_the_server_are_not_sent() {
    let wire = FakeWire::new();
    let mut session = open(&wire).await;
    let before = wire.sent();
    for word in MAINTENANCE {
        let sql = format!("{word} orders");
        let error = session.run(&sql).await.unwrap_err();
        assert!(matches!(error, QueryError::NotSent(_)), "{sql}: {error:?}");
    }
    for name in FUNCTIONS {
        let sql = format!("select {name}(1)");
        let error = session.run(&sql).await.unwrap_err();
        assert!(matches!(error, QueryError::NotSent(_)), "{sql}: {error:?}");
    }
    let error = session
        .run("select dblink_exec('host=x', 'select 1')")
        .await
        .unwrap_err();
    assert!(matches!(error, QueryError::NotSent(_)), "{error:?}");
    assert_eq!(wire.sent(), before);
}

#[tokio::test]
async fn safety_g20_sql_built_at_run_time_or_spelled_in_escapes_is_not_sent() {
    let wire = FakeWire::new();
    let mut session = open(&wire).await;
    let before = wire.sent();
    for sql in [
        // A block that builds the name of a refused function when it runs.
        "do $$ begin execute 'select pg_cancel' || '_backend(1)'; end $$",
        // Functions that run the text they are given.
        "select query_to_xml('select pg_term'||'inate_backend(1)', true, false, '')",
        "select * from ts_stat('select to_tsvector(''x'')')",
        // An E-string whose escaped quote used to hide the call from the check.
        "select E'\\'', 'x--', pg_terminate_backend(1), U&\"x\"\nfrom pg_stat_activity",
        "select ts_rewrite('a'::tsquery, 'select pg_term'||'inate_backend(1)')",
        // A name spelled with Unicode escapes.
        "select U&\"pg_terminate_backen\\0064\"(pid) from pg_stat_activity",
        // Files and programs on the server.
        "copy orders to '/tmp/orders.csv'",
        "COPY (select 1) TO PROGRAM 'id'",
    ] {
        let error = session.run(sql).await.unwrap_err();
        assert!(matches!(error, QueryError::NotSent(_)), "{sql}: {error:?}");
    }
    assert_eq!(wire.sent(), before);
}

#[tokio::test]
async fn safety_g20_a_plan_never_analyzes() {
    let wire = FakeWire::new();
    wire.answer(
        "EXPLAIN",
        text_rows(&["QUERY PLAN"], &[&[Some("Seq Scan")]]),
    );
    let mut session = open(&wire).await;
    session.explain("select * from orders").await.unwrap();
    for sql in [
        "explain analyze delete from orders",
        "EXPLAIN (ANALYSE) select 1",
        "explain verbose analyze select 1",
    ] {
        // As a plan and as a run: neither executes what it explains.
        assert!(
            matches!(session.explain(sql).await, Err(QueryError::NotSent(_))),
            "{sql}"
        );
        assert!(
            matches!(session.run(sql).await, Err(QueryError::NotSent(_))),
            "{sql}"
        );
    }
    // A statement that is already a plan is never wrapped or sent by Plan.
    assert!(matches!(
        session.explain("explain select 1").await,
        Err(QueryError::NotSent(_))
    ));
    let sent = wire.sent();
    assert!(
        !sent
            .iter()
            .any(|s| s.to_uppercase().contains("ANALYZ") || s.to_uppercase().contains("ANALYS")),
        "{sent:?}"
    );
}

#[tokio::test]
async fn safety_g20_every_request_ends_rolled_back_with_its_locks_let_go_and_nothing_is_committed()
{
    let wire = FakeWire::new();
    wire.fail("nope", "relation \"nope\" does not exist");
    let mut session = open(&wire).await;
    let before = wire.sent().len();
    session.run("select 1").await.unwrap();
    session.run("select * from nope").await.unwrap_err();
    session.run("delete from orders").await.unwrap_err();
    session.tables("public").await.unwrap();
    let sent = wire.sent()[before..].to_vec();
    // Each request: rolled back first and last, with the unlock after each.
    let begins = sent.iter().filter(|s| *s == "BEGIN READ ONLY").count();
    let rollbacks = sent.iter().filter(|s| *s == "ROLLBACK").count();
    let unlocks = sent.iter().filter(|s| *s == UNLOCK).count();
    assert_eq!((begins, rollbacks, unlocks), (4, 8, 8), "{sent:?}");
    for (ix, line) in sent.iter().enumerate() {
        if line == "ROLLBACK" {
            assert_eq!(sent[ix + 1], UNLOCK, "{sent:?}");
        }
    }
    assert_eq!(sent.last().unwrap(), UNLOCK);
    assert!(!sent.iter().any(|s| s.to_uppercase().contains("COMMIT")));
}
