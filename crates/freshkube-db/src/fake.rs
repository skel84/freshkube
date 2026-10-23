//! A fake driver for example data and tests, with invented names only.
//!
//! [`FakeWire`] stands in for the server under [`crate::postgres::Postgres`]:
//! it records every statement sent and answers by matching text, so a test
//! sees what really goes out. [`FakeSession`] is a whole database in memory
//! (`shop`), which the `--fixture` explorer reads, and [`Shape`] says how a
//! connection to it ends.
use crate::{
    limits::Limits,
    session::{ColumnEntry, ConnectError, Session, TableEntry, TableKind, TablePage},
    wire::{Rows, Wire},
};
use async_trait::async_trait;
use dbflux_core::{DbError, Value};
use std::sync::{Arc, Mutex};

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
            "the server reports default_transaction_read_only = off".into(),
        )),
    }
}

#[derive(Clone)]
enum Answer {
    Rows(Rows),
    Timeout,
    Failed(String),
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

#[async_trait]
impl Wire for FakeWire {
    async fn simple(&mut self, sql: &str) -> Result<Rows, DbError> {
        self.sent.lock().unwrap().push(sql.to_string());
        if sql.starts_with("SHOW default_transaction_read_only") {
            let value = self.read_only_setting.lock().unwrap().clone();
            return Ok(text_rows(
                &["default_transaction_read_only"],
                &[&[Some(&value)]],
            ));
        }
        for (needle, answer) in self.rules.lock().unwrap().iter() {
            if sql.contains(needle.as_str()) {
                return match answer.clone() {
                    Answer::Rows(rows) => Ok(rows),
                    Answer::Timeout => Err(DbError::Timeout),
                    Answer::Failed(message) => Err(DbError::query_failed(message)),
                };
            }
        }
        Ok(Rows::default())
    }
}

/// The example `shop` database: two schemas, a view, and an `orders` table
/// with more rows than a page holds so the cap shows.
pub struct FakeSession {
    limits: Limits,
}

const ORDERS: usize = 250;

impl FakeSession {
    pub fn shop(limits: Limits) -> Self {
        Self { limits }
    }
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

#[async_trait]
impl Session for FakeSession {
    fn read_only(&self) -> bool {
        true
    }

    async fn schemas(&mut self) -> Result<Vec<String>, DbError> {
        Ok(vec!["audit".into(), "public".into()])
    }

    async fn tables(&mut self, schema: &str) -> Result<Vec<TableEntry>, DbError> {
        Ok(match schema {
            "public" => vec![
                entry("customers", TableKind::Table, 12, 16_384),
                entry("order_items", TableKind::Table, 740, 65_536),
                entry("orders", TableKind::Table, ORDERS as u64, 49_152),
                entry("products", TableKind::Table, 0, 8_192),
                entry("recent_orders", TableKind::View, 0, 0),
            ],
            "audit" => vec![entry("events", TableKind::Table, 3, 8_192)],
            _ => Vec::new(),
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
