//! A fake driver for example data and tests, with invented names only.
//!
//! [`FakeWire`] stands in for the server under [`crate::postgres::Postgres`]:
//! it records every statement sent and answers by matching text, so a test
//! sees what really goes out. [`FakeSession`] is a whole database in memory
//! (`shop`), which the `--fixture` explorer reads, and [`Shape`] says how a
//! connection to it ends.
use crate::{
    limits::Limits,
    readonly,
    session::{
        Canceller, ColumnEntry, ConnectError, Listed, Plan, QueryError, QueryResult, Session,
        TableEntry, TableKind, TablePage,
    },
    wire::{Rows, Wire},
};
use async_trait::async_trait;
use dbflux_core::{DbError, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// How a connection to the example database ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Connected,
    Refused,
    TimedOut,
    NotReadOnly,
}

/// Connects the example database the way `shape` says.
pub fn connect(shape: Shape, limits: Limits) -> Result<FakeSession, ConnectError> {
    match shape {
        Shape::Connected => Ok(FakeSession::shop(limits)),
        Shape::Refused => Err(ConnectError::Refused(
            "password authentication failed for user \"shop_reader\"".into(),
        )),
        Shape::TimedOut => Err(ConnectError::TimedOut),
        Shape::NotReadOnly => Err(ConnectError::NotReadOnly(
            "The server reports default_transaction_read_only = off".into(),
        )),
    }
}

#[derive(Clone)]
enum Answer {
    Rows(Rows),
    Timeout,
    Failed(String),
    /// Runs until the session is cancelled.
    Hold,
}

type Rule = (String, Answer);

/// A server that records what it is sent. Rules are tried in order; a
/// statement no rule matches answers with no rows.
#[derive(Clone, Default)]
pub struct FakeWire {
    sent: Arc<Mutex<Vec<String>>>,
    rules: Arc<Mutex<Vec<Rule>>>,
    /// What `SHOW default_transaction_read_only` answers; a server that
    /// ignores the setting says `off`.
    read_only_setting: Arc<Mutex<String>>,
    /// Texts the fake server's parser takes for several statements.
    multi: Arc<Mutex<Vec<String>>>,
    /// Inside `BEGIN READ ONLY`, until the next `ROLLBACK`.
    in_read_only: Arc<Mutex<bool>>,
    /// Writes the server accepted. Empty for any read-only session.
    applied: Arc<Mutex<Vec<String>>>,
    /// The rows the last `DECLARE` will `FETCH`.
    cursor: Arc<Mutex<Rows>>,
    cancel: Arc<Notify>,
    cancels: Arc<Mutex<usize>>,
}

impl FakeWire {
    pub fn new() -> Self {
        let wire = Self::default();
        *wire.read_only_setting.lock().unwrap() = "on".into();
        wire
    }

    pub fn ignoring_read_only() -> Self {
        let wire = Self::new();
        *wire.read_only_setting.lock().unwrap() = "off".into();
        wire
    }

    /// Answer statements containing `needle` with `rows`.
    pub fn answer(&self, needle: &str, rows: Rows) -> &Self {
        self.rules
            .lock()
            .unwrap()
            .push((needle.into(), Answer::Rows(rows)));
        self
    }

    /// Statements containing `needle` run until cancelled.
    pub fn hold(&self, needle: &str) -> &Self {
        self.rules
            .lock()
            .unwrap()
            .push((needle.into(), Answer::Hold));
        self
    }

    /// The writes the server accepted: none, for a read-only session.
    pub fn applied(&self) -> Vec<String> {
        self.applied.lock().unwrap().clone()
    }

    /// How many cancel requests reached the server.
    pub fn cancels(&self) -> usize {
        *self.cancels.lock().unwrap()
    }

    pub fn time_out(&self, needle: &str) -> &Self {
        self.rules
            .lock()
            .unwrap()
            .push((needle.into(), Answer::Timeout));
        self
    }

    pub fn fail(&self, needle: &str, message: &str) -> &Self {
        self.rules
            .lock()
            .unwrap()
            .push((needle.into(), Answer::Failed(message.into())));
        self
    }

    /// The server refuses to prepare statements containing `needle`.
    pub fn refuse_as_several(&self, needle: &str) -> &Self {
        self.multi.lock().unwrap().push(needle.into());
        self
    }

    pub fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

pub fn text_rows(columns: &[&str], rows: &[&[Option<&str>]]) -> Rows {
    Rows {
        columns: columns.iter().map(|c| c.to_string()).collect(),
        rows: rows
            .iter()
            .map(|r| r.iter().map(|c| c.map(str::to_string)).collect())
            .collect(),
    }
}

struct FakeCanceller {
    cancel: Arc<Notify>,
    cancels: Arc<Mutex<usize>>,
}

#[async_trait]
impl Canceller for FakeCanceller {
    async fn cancel(&self) -> Result<(), String> {
        *self.cancels.lock().unwrap() += 1;
        // A permit, so a cancel that lands just before the wait still counts.
        self.cancel.notify_one();
        Ok(())
    }
}

/// Whether a statement changes data or the schema, as the fake server's
/// read-only transaction sees it.
fn writes(sql: &str) -> bool {
    matches!(
        readonly::leading_word(sql).as_str(),
        "insert"
            | "update"
            | "delete"
            | "drop"
            | "create"
            | "alter"
            | "truncate"
            | "grant"
            | "revoke"
            | "merge"
            | "copy"
    )
}

impl FakeWire {
    /// What the server answers for `sql`, by the first rule that matches.
    async fn answer_for(&self, sql: &str) -> Result<Rows, DbError> {
        let found = self
            .rules
            .lock()
            .unwrap()
            .iter()
            .find(|(needle, _)| sql.contains(needle.as_str()))
            .map(|(_, answer)| answer.clone());
        match found {
            Some(Answer::Rows(rows)) => Ok(rows),
            Some(Answer::Timeout) => Err(DbError::Timeout),
            Some(Answer::Failed(message)) => Err(DbError::query_failed(message)),
            Some(Answer::Hold) => {
                self.cancel.notified().await;
                Err(DbError::Cancelled)
            }
            None => Ok(Rows::default()),
        }
    }
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
impl Wire for FakeWire {
    fn canceller(&self) -> Arc<dyn Canceller> {
        Arc::new(FakeCanceller {
            cancel: self.cancel.clone(),
            cancels: self.cancels.clone(),
        })
    }

    async fn simple(&mut self, sql: &str) -> Result<Rows, DbError> {
        self.sent.lock().unwrap().push(sql.to_string());
        if sql == "BEGIN READ ONLY" {
            *self.in_read_only.lock().unwrap() = true;
        } else if sql == "ROLLBACK" || sql == "COMMIT" {
            *self.in_read_only.lock().unwrap() = false;
        }
        if let Some(inner) = sql
            .strip_prefix("DECLARE ")
            .and_then(|rest| rest.split_once(" CURSOR FOR "))
        {
            let rows = self.answer_for(inner.1).await?;
            *self.cursor.lock().unwrap() = rows;
            return Ok(Rows::default());
        }
        if let Some(n) = sql
            .strip_prefix("FETCH ")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse::<usize>().ok())
        {
            let mut rows = self.cursor.lock().unwrap().clone();
            rows.rows.truncate(n);
            return Ok(rows);
        }
        // The server's read-only session and transaction refuse a write,
        // whatever is typed; a server that ignores the setting applies it.
        if writes(sql) {
            let read_only = *self.in_read_only.lock().unwrap()
                || *self.read_only_setting.lock().unwrap() == "on";
            if read_only {
                return Err(DbError::permission_denied(format!(
                    "cannot execute {} in a read-only transaction",
                    readonly::leading_word(sql).to_uppercase()
                )));
            }
            self.applied.lock().unwrap().push(sql.to_string());
            return Ok(Rows::default());
        }
        if sql.starts_with("SHOW default_transaction_read_only") {
            let value = self.read_only_setting.lock().unwrap().clone();
            return Ok(text_rows(
                &["default_transaction_read_only"],
                &[&[Some(&value)]],
            ));
        }
        self.answer_for(sql).await
    }

    async fn single_statement(&mut self, sql: &str) -> Result<(), DbError> {
        self.sent.lock().unwrap().push(format!("PARSE {sql}"));
        // The server's rule: more than one statement can't be prepared.
        if self
            .multi
            .lock()
            .unwrap()
            .iter()
            .any(|n| sql.contains(n.as_str()))
        {
            return Err(DbError::query_failed(
                "cannot insert multiple commands into a prepared statement",
            ));
        }
        Ok(())
    }
}

/// The example `shop` database: two schemas, a view, and an `orders` table
/// with more rows than a page holds so the cap shows.
pub struct FakeSession {
    limits: Limits,
    cancel: Arc<Notify>,
    cancels: Arc<Mutex<usize>>,
}

const ORDERS: usize = 250;

impl FakeSession {
    pub fn shop(limits: Limits) -> Self {
        Self {
            limits,
            cancel: Arc::new(Notify::new()),
            cancels: Arc::new(Mutex::new(0)),
        }
    }

    /// How many cancel requests this session received.
    pub fn cancels(&self) -> usize {
        *self.cancels.lock().unwrap()
    }

    /// A handle on the cancel count, for a test that has boxed the session.
    pub fn cancel_counter(&self) -> Arc<Mutex<usize>> {
        self.cancels.clone()
    }
}

/// The table a statement of the example reads, after `from`.
fn from_table(sql: &str) -> Option<String> {
    let lower = sql.to_ascii_lowercase();
    let after = lower.split(" from ").nth(1)?;
    let name = after
        .split(|c: char| c.is_whitespace() || c == ';' || c == ')')
        .next()?
        .trim_matches('"');
    Some(name.rsplit('.').next().unwrap_or(name).to_string())
}

fn entry(name: &str, kind: TableKind, rows: u64, size: u64) -> TableEntry {
    TableEntry {
        name: name.into(),
        kind,
        rows_estimate: Some(rows),
        size_bytes: Some(size),
    }
}

fn column(name: &str, type_name: &str, nullable: bool, pk: bool) -> ColumnEntry {
    ColumnEntry {
        name: name.into(),
        type_name: type_name.into(),
        nullable,
        default: pk.then(|| "nextval('seq'::regclass)".to_string()),
        primary_key: pk,
    }
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
impl Session for FakeSession {
    fn read_only(&self) -> bool {
        true
    }

    async fn schemas(&mut self) -> Result<Listed<String>, DbError> {
        Ok(Listed {
            items: vec!["audit".into(), "public".into()],
            capped: false,
        })
    }

    async fn tables(&mut self, schema: &str) -> Result<Listed<TableEntry>, DbError> {
        let items = match schema {
            "public" => vec![
                entry("customers", TableKind::Table, 12, 16_384),
                entry("order_items", TableKind::Table, 740, 65_536),
                entry("orders", TableKind::Table, ORDERS as u64, 49_152),
                entry("products", TableKind::Table, 0, 8_192),
                entry("recent_orders", TableKind::View, 0, 0),
            ],
            "audit" => vec![entry("events", TableKind::Table, 3, 8_192)],
            _ => Vec::new(),
        };
        Ok(Listed {
            items,
            capped: false,
        })
    }

    async fn columns(&mut self, schema: &str, table: &str) -> Result<Vec<ColumnEntry>, DbError> {
        match (schema, table) {
            ("public", "orders") => Ok(vec![
                column("id", "bigint", false, true),
                column("customer_id", "bigint", false, false),
                column("status", "text", false, false),
                column("total", "numeric(12,2)", true, false),
                column("note", "text", true, false),
            ]),
            ("public", "customers") => Ok(vec![
                column("id", "bigint", false, true),
                column("email", "text", false, false),
                column("name", "text", true, false),
            ]),
            ("public", "products") => Ok(vec![
                column("id", "bigint", false, true),
                column("title", "text", false, false),
            ]),
            ("public", "order_items") | ("public", "recent_orders") | ("audit", "events") => {
                Ok(vec![
                    column("id", "bigint", false, true),
                    column("detail", "text", true, false),
                ])
            }
            _ => Err(DbError::object_not_found(format!("{schema}.{table}"))),
        }
    }

    async fn run(&mut self, sql: &str) -> Result<QueryResult, QueryError> {
        readonly::check_statement(sql).map_err(|r| QueryError::NotSent(r.explain().to_string()))?;
        let started = std::time::Instant::now();
        let word = readonly::leading_word(sql);
        if writes(sql) {
            // What the database answers a write in a read-only session.
            return Err(QueryError::Refused(format!(
                "cannot execute {} in a read-only transaction",
                word.to_uppercase()
            )));
        }
        if sql.to_ascii_lowercase().contains("pg_sleep") {
            self.cancel.notified().await;
            return Err(QueryError::Cancelled);
        }
        if let Some(table) = from_table(sql) {
            let page = match self.table_rows("public", &table).await {
                Ok(page) => page,
                Err(_) => {
                    return Err(QueryError::Failed(format!(
                        "relation \"{table}\" does not exist"
                    )));
                }
            };
            return Ok(QueryResult {
                columns: page.columns,
                rows: page.rows,
                capped: page.capped,
                row_cap: page.row_cap,
                elapsed: started.elapsed(),
            });
        }
        Ok(QueryResult {
            columns: vec!["?column?".into()],
            rows: vec![vec![Value::Int(1)]],
            capped: false,
            row_cap: self.limits.rows,
            elapsed: started.elapsed(),
        })
    }

    async fn explain(&mut self, sql: &str) -> Result<Plan, QueryError> {
        readonly::check_statement(sql).map_err(|r| QueryError::NotSent(r.explain().to_string()))?;
        let table = from_table(sql).unwrap_or_else(|| "orders".into());
        let rows = if table == "orders" { ORDERS } else { 12 };
        Ok(Plan {
            lines: vec![
                format!("Seq Scan on {table}  (cost=0.00..5.50 rows={rows} width=64)"),
                "  Filter: (status = 'paid'::text)".into(),
            ],
            capped: false,
        })
    }

    fn canceller(&self) -> Arc<dyn Canceller> {
        Arc::new(FakeCanceller {
            cancel: self.cancel.clone(),
            cancels: self.cancels.clone(),
        })
    }

    async fn table_rows(&mut self, schema: &str, table: &str) -> Result<TablePage, DbError> {
        let cap = self.limits.rows;
        let columns: Vec<String> = self
            .columns(schema, table)
            .await?
            .into_iter()
            .map(|c| c.name)
            .collect();
        let total = match table {
            "orders" => ORDERS,
            "customers" => 12,
            "order_items" => 40,
            "products" => 0,
            _ => 3,
        };
        let shown = total.min(cap);
        let rows = (0..shown)
            .map(|i| {
                columns
                    .iter()
                    .map(|c| match c.as_str() {
                        "id" => Value::Int(i as i64 + 1),
                        "customer_id" => Value::Int((i % 12) as i64 + 1),
                        "status" => Value::Text(["paid", "shipped", "pending"][i % 3].into()),
                        "total" => Value::Decimal(format!("{}.{:02}", 10 + i % 90, i % 100)),
                        "note" | "name" if i % 4 == 0 => Value::Null,
                        "email" => Value::Text(format!("customer{}@example.test", i + 1)),
                        other => Value::Text(format!("{other} {}", i + 1)),
                    })
                    .collect()
            })
            .collect();
        Ok(TablePage {
            columns,
            rows,
            capped: total > cap,
            row_cap: cap,
        })
    }
}
