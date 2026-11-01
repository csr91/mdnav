//! SQL execution domain: open a connection, run one statement, format one
//! page of its result. Pure and synchronous — no UI state lives here.
//! Threading and cancellation are the only impure bits (`spawn_query`,
//! `InFlight::cancel`), mirroring `http::spawn_request`.
//!
//! Three engines are supported behind the same `QueryResult` shape: sqlite
//! (via `rusqlite`), postgres (via the sync `postgres` crate) and mysql (via
//! the sync `mysql` crate). Each engine connects fresh for every query —
//! simple to reason about from a one-shot worker thread, and cheap enough
//! for a TUI helper that is not a connection-pooled server.
//!
//! A query that returns rows is never collected whole into memory: `run_*`
//! skips to `offset`, reads at most `PAGE_SIZE` rows and peeks one more to
//! set `has_more`, so a result with millions of rows costs one page, not all
//! of them. The studio re-runs the same query at a later `offset` to fetch
//! the next page — a re-scan, not a resumed cursor, which is simpler to run
//! from a one-shot worker thread and cheap enough for a TUI helper.
//!
//! Postgres/mysql connections are plain (no TLS) for now — fine for local
//! and most dev/staging databases, not yet wired for `sslmode=require` or
//! similar. Postgres column rendering covers the common scalar types (bool,
//! int2/4/8, float4/8, numeric, text/varchar, uuid, json/jsonb, date/time/
//! timestamp[tz]); anything else renders as `<tipo no soportado>` rather
//! than guessing. Mysql renders every value through its own untyped
//! `Value`, which already covers every column type the wire protocol can
//! send, so it has no such gap.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result as AnyResult};
use mysql::prelude::Queryable;
use postgres::fallible_iterator::FallibleIterator;
use rusqlite::{types::ValueRef, Connection, InterruptHandle};

/// Rows fetched per page. Bounds memory and worker time regardless of how
/// many rows the query would actually return.
pub const PAGE_SIZE: usize = 200;

/// Which driver a connection's value (path or connection string) is read
/// with. Stored per connection, shared by all of its environments.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Engine {
    #[default]
    Sqlite,
    Postgres,
    Mysql,
}

impl Engine {
    /// Strict parse, for the "new connection" prompt: unrecognized text is
    /// rejected rather than silently defaulting to sqlite.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "sqlite" | "sqlite3" => Some(Self::Sqlite),
            "postgres" | "postgresql" | "pg" => Some(Self::Postgres),
            "mysql" | "maria" | "mariadb" => Some(Self::Mysql),
            _ => None,
        }
    }

    /// Lenient parse, for reading a saved `connection.toml`: an unknown or
    /// missing value falls back to sqlite rather than failing to load.
    pub fn from_stored(text: &str) -> Self {
        Self::parse(text).unwrap_or(Self::Sqlite)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
        }
    }

    /// Shown as the prompt title when asking for a connection's (or
    /// environment's) value, so the user knows the shape expected.
    pub fn value_hint(&self) -> &'static str {
        match self {
            Self::Sqlite => "Ruta del archivo .sqlite",
            Self::Postgres => "postgres://usuario:clave@host:5432/base",
            Self::Mysql => "mysql://usuario:clave@host:3306/base",
        }
    }
}

/// Creates a fresh sqlite file with a tiny demo table, for the starter
/// connection the library seeds the first time `:@sql` runs empty.
pub fn init_example_db(path: &Path) -> AnyResult<()> {
    let conn =
        Connection::open(path).with_context(|| format!("No se pudo crear {}", path.display()))?;
    conn.execute_batch(
        "CREATE TABLE notas (id INTEGER PRIMARY KEY, titulo TEXT, contenido TEXT);
         INSERT INTO notas (titulo, contenido) VALUES
            ('Bienvenida', 'Esta es una conexion de ejemplo, podes borrarla.'),
            ('Tip', 'n crea una query nueva, c crea una conexion nueva.');",
    )
    .with_context(|| format!("No se pudo poblar {}", path.display()))?;
    Ok(())
}

/// One page of a column/row grid, ready to render in the results table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Set instead of `rows`/`columns` for statements that return no result
    /// set (INSERT/UPDATE/DELETE/DDL).
    pub rows_affected: Option<usize>,
    /// Index of the first row in `rows`, relative to the whole result set.
    pub offset: usize,
    /// True when at least one more row exists past this page.
    pub has_more: bool,
    pub elapsed: std::time::Duration,
    pub error: Option<String>,
}

impl QueryResult {
    fn failed(message: String, elapsed: std::time::Duration) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: None,
            offset: 0,
            has_more: false,
            elapsed,
            error: Some(message),
        }
    }
}

/// A column of a table, as reported by the engine's catalog/information
/// schema. Pure metadata — no rows.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

/// A table and its columns, as listed by `spawn_schema`.
#[derive(Clone, Debug, PartialEq)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
}

/// Runs `sql` against the sqlite database at `db_path`, blocking the calling
/// thread, and returns the page starting at `offset`. Never panics: every
/// rusqlite error is captured into `QueryResult::error` instead of
/// propagating.
///
/// Production code goes through `spawn_query` instead, which also captures
/// an `InterruptHandle` for cancellation; this one stays around for tests
/// that want the open-and-query path without a worker thread.
#[cfg(test)]
pub fn run_sqlite_query(db_path: &str, sql: &str, offset: usize) -> QueryResult {
    let started = Instant::now();
    let conn = match Connection::open(db_path) {
        Ok(conn) => conn,
        Err(err) => {
            return QueryResult::failed(
                format!("No se pudo abrir {db_path}: {err}"),
                started.elapsed(),
            )
        }
    };
    run_sqlite(&conn, sql, started, offset)
}

fn run_sqlite(conn: &Connection, sql: &str, started: Instant, offset: usize) -> QueryResult {
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
    };

    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();

    if columns.is_empty() {
        // Not a row-returning statement: drop the prepared read and run it
        // as a command instead. Pagination does not apply here.
        drop(stmt);
        return match conn.execute(sql, []) {
            Ok(affected) => QueryResult {
                rows_affected: Some(affected),
                elapsed: started.elapsed(),
                ..QueryResult::default()
            },
            Err(err) => QueryResult::failed(err.to_string(), started.elapsed()),
        };
    }

    let column_count = columns.len();
    let mut cursor = match stmt.query([]) {
        Ok(rows) => rows,
        Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
    };

    for _ in 0..offset {
        match cursor.next() {
            Ok(Some(_)) => {}
            Ok(None) => {
                return QueryResult {
                    columns,
                    offset,
                    elapsed: started.elapsed(),
                    ..QueryResult::default()
                }
            }
            Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
        }
    }

    let mut rows = Vec::with_capacity(PAGE_SIZE);
    while rows.len() < PAGE_SIZE {
        match cursor.next() {
            Ok(Some(row)) => {
                let mut values = Vec::with_capacity(column_count);
                for index in 0..column_count {
                    match row.get_ref(index) {
                        Ok(value) => values.push(sqlite_value_to_string(value)),
                        Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
                    }
                }
                rows.push(values);
            }
            Ok(None) => {
                return QueryResult {
                    columns,
                    rows,
                    offset,
                    has_more: false,
                    elapsed: started.elapsed(),
                    ..QueryResult::default()
                }
            }
            Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
        }
    }

    // One more row, peeked but not collected, just to know if a next page exists.
    let has_more = matches!(cursor.next(), Ok(Some(_)));

    QueryResult {
        columns,
        rows,
        offset,
        has_more,
        elapsed: started.elapsed(),
        error: None,
        rows_affected: None,
    }
}

fn sqlite_value_to_string(value: ValueRef) -> String {
    match value {
        ValueRef::Null => String::from("NULL"),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
        ValueRef::Blob(blob) => format!("<blob {} bytes>", blob.len()),
    }
}

fn run_postgres(
    client: &mut postgres::Client,
    sql: &str,
    started: Instant,
    offset: usize,
) -> QueryResult {
    let stmt = match client.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
    };

    if stmt.columns().is_empty() {
        return match client.execute(&stmt, &[]) {
            Ok(affected) => QueryResult {
                rows_affected: Some(affected as usize),
                elapsed: started.elapsed(),
                ..QueryResult::default()
            },
            Err(err) => QueryResult::failed(err.to_string(), started.elapsed()),
        };
    }

    let columns: Vec<String> = stmt
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect();
    let column_count = columns.len();

    let mut cursor = match client.query_raw(&stmt, Vec::<i32>::new()) {
        Ok(rows) => rows,
        Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
    };

    for _ in 0..offset {
        match cursor.next() {
            Ok(Some(_)) => {}
            Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
            Ok(None) => {
                return QueryResult {
                    columns,
                    offset,
                    elapsed: started.elapsed(),
                    ..QueryResult::default()
                }
            }
        }
    }

    let mut rows = Vec::with_capacity(PAGE_SIZE);
    while rows.len() < PAGE_SIZE {
        match cursor.next() {
            Ok(Some(row)) => {
                let values = (0..column_count)
                    .map(|index| postgres_value_to_string(&row, index))
                    .collect();
                rows.push(values);
            }
            Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
            Ok(None) => {
                return QueryResult {
                    columns,
                    rows,
                    offset,
                    has_more: false,
                    elapsed: started.elapsed(),
                    ..QueryResult::default()
                }
            }
        }
    }

    let has_more = matches!(cursor.next(), Ok(Some(_)));

    QueryResult {
        columns,
        rows,
        offset,
        has_more,
        elapsed: started.elapsed(),
        error: None,
        rows_affected: None,
    }
}

/// Renders one cell by matching the column's real postgres type, so a NULL
/// of a supported type still prints as `NULL` rather than being confused
/// with an unsupported type. Types outside this list (arrays, intervals,
/// network/geometric types, ...) render as `<TYPE no soportado>`.
fn postgres_value_to_string(row: &postgres::Row, index: usize) -> String {
    use postgres::types::Type;

    macro_rules! render {
        ($t:ty) => {
            match row.try_get::<_, Option<$t>>(index) {
                Ok(Some(value)) => value.to_string(),
                Ok(None) => String::from("NULL"),
                Err(err) => format!("<error: {err}>"),
            }
        };
    }

    let ty = row.columns()[index].type_().clone();
    match ty {
        Type::BOOL => render!(bool),
        Type::INT2 => render!(i16),
        Type::INT4 => render!(i32),
        Type::INT8 => render!(i64),
        Type::FLOAT4 => render!(f32),
        Type::FLOAT8 => render!(f64),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => render!(String),
        Type::UUID => render!(uuid::Uuid),
        Type::NUMERIC => render!(rust_decimal::Decimal),
        Type::DATE => render!(chrono::NaiveDate),
        Type::TIME => render!(chrono::NaiveTime),
        Type::TIMESTAMP => render!(chrono::NaiveDateTime),
        Type::TIMESTAMPTZ => render!(chrono::DateTime<chrono::Utc>),
        Type::JSON | Type::JSONB => render!(serde_json::Value),
        other => format!("<{} no soportado>", other.name()),
    }
}

fn run_mysql(conn: &mut mysql::Conn, sql: &str, started: Instant, offset: usize) -> QueryResult {
    let mut result = match conn.query_iter(sql) {
        Ok(result) => result,
        Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
    };

    let columns: Vec<String> = result
        .columns()
        .as_ref()
        .iter()
        .map(|column| column.name_str().into_owned())
        .collect();

    if columns.is_empty() {
        // Drain so the OK packet's affected-rows count is final.
        for row in result.by_ref() {
            if let Err(err) = row {
                return QueryResult::failed(err.to_string(), started.elapsed());
            }
        }
        return QueryResult {
            rows_affected: Some(result.affected_rows() as usize),
            elapsed: started.elapsed(),
            ..QueryResult::default()
        };
    }

    let column_count = columns.len();
    let mut rows = Vec::with_capacity(PAGE_SIZE);
    let mut has_more = false;
    let mut seen = 0usize;
    for row_result in result.by_ref() {
        let row = match row_result {
            Ok(row) => row,
            Err(err) => return QueryResult::failed(err.to_string(), started.elapsed()),
        };
        if seen < offset {
            seen += 1;
            continue;
        }
        if rows.len() < PAGE_SIZE {
            let values = (0..column_count)
                .map(|index| {
                    row.as_ref(index)
                        .map(mysql_value_to_string)
                        .unwrap_or_else(|| String::from("NULL"))
                })
                .collect();
            rows.push(values);
        } else {
            has_more = true;
            break;
        }
    }

    QueryResult {
        columns,
        rows,
        offset,
        has_more,
        elapsed: started.elapsed(),
        error: None,
        rows_affected: None,
    }
}

/// Mysql's wire value is already untyped (`mysql::Value`), so unlike
/// postgres there is no per-type allowlist: every variant the protocol can
/// send is handled here.
fn mysql_value_to_string(value: &mysql::Value) -> String {
    use mysql::Value;
    match value {
        Value::NULL => String::from("NULL"),
        Value::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(d) => d.to_string(),
        Value::Date(year, month, day, hour, minute, second, micro) => {
            if *hour == 0 && *minute == 0 && *second == 0 && *micro == 0 {
                format!("{year:04}-{month:02}-{day:02}")
            } else {
                format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
            }
        }
        Value::Time(negative, days, hours, minutes, seconds, _micro) => {
            let sign = if *negative { "-" } else { "" };
            let total_hours = *days * 24 + u32::from(*hours);
            format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}")
        }
    }
}

/// Lists every table and its columns for the active database/schema.
/// Sqlite has no information_schema, so it walks `sqlite_master` plus one
/// `PRAGMA table_info` per table instead.
fn sqlite_schema(conn: &Connection) -> Result<Vec<TableInfo>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|err| err.to_string())?;
    let table_names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|err| err.to_string())?;

    let mut tables = Vec::with_capacity(table_names.len());
    for name in table_names {
        let quoted = format!("\"{}\"", name.replace('"', "\"\""));
        let mut column_stmt = conn
            .prepare(&format!("PRAGMA table_info({quoted})"))
            .map_err(|err| err.to_string())?;
        let columns = column_stmt
            .query_map([], |row| {
                Ok(ColumnInfo {
                    name: row.get(1)?,
                    data_type: row.get(2)?,
                    nullable: row.get::<_, i64>(3)? == 0,
                    primary_key: row.get::<_, i64>(5)? > 0,
                })
            })
            .and_then(Iterator::collect::<Result<Vec<_>, _>>)
            .map_err(|err| err.to_string())?;
        tables.push(TableInfo { name, columns });
    }
    Ok(tables)
}

fn postgres_schema(client: &mut postgres::Client) -> Result<Vec<TableInfo>, String> {
    let sql = "
        SELECT c.table_name, c.column_name, c.data_type, c.is_nullable,
               COALESCE(pk.is_pk, false) AS is_primary_key
        FROM information_schema.columns c
        LEFT JOIN (
            SELECT tc.table_name, kcu.column_name, true AS is_pk
            FROM information_schema.table_constraints tc
            JOIN information_schema.key_column_usage kcu
              ON tc.constraint_name = kcu.constraint_name
             AND tc.table_schema = kcu.table_schema
            WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = 'public'
        ) pk ON pk.table_name = c.table_name AND pk.column_name = c.column_name
        WHERE c.table_schema = 'public'
        ORDER BY c.table_name, c.ordinal_position";
    let rows = client.query(sql, &[]).map_err(|err| err.to_string())?;
    Ok(group_columns_into_tables(rows.into_iter().map(|row| {
        let table_name: String = row.get(0);
        let is_nullable: String = row.get(3);
        let column = ColumnInfo {
            name: row.get(1),
            data_type: row.get(2),
            nullable: is_nullable == "YES",
            primary_key: row.get(4),
        };
        (table_name, column)
    })))
}

fn mysql_schema(conn: &mut mysql::Conn) -> Result<Vec<TableInfo>, String> {
    let sql = "
        SELECT c.TABLE_NAME, c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, c.COLUMN_KEY
        FROM information_schema.columns c
        WHERE c.TABLE_SCHEMA = DATABASE()
        ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION";
    let rows: Vec<(String, String, String, String, String)> =
        conn.query(sql).map_err(|err| err.to_string())?;
    Ok(group_columns_into_tables(rows.into_iter().map(
        |(table, column, data_type, is_nullable, key)| {
            (
                table,
                ColumnInfo {
                    name: column,
                    data_type,
                    nullable: is_nullable == "YES",
                    primary_key: key == "PRI",
                },
            )
        },
    )))
}

/// Groups a column stream already ordered by table name into `TableInfo`s,
/// shared by the postgres and mysql introspection queries.
fn group_columns_into_tables(rows: impl Iterator<Item = (String, ColumnInfo)>) -> Vec<TableInfo> {
    let mut tables: Vec<TableInfo> = Vec::new();
    for (table_name, column) in rows {
        match tables.last_mut() {
            Some(last) if last.name == table_name => last.columns.push(column),
            _ => tables.push(TableInfo {
                name: table_name,
                columns: vec![column],
            }),
        }
    }
    tables
}

/// One schema load running on a worker thread, mirroring `InFlight` but for
/// a one-shot table/column listing instead of a paged query.
pub struct SchemaInFlight {
    receiver: Receiver<Result<Vec<TableInfo>, String>>,
}

impl SchemaInFlight {
    pub fn try_finish(&self) -> Option<Result<Vec<TableInfo>, String>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(String::from(
                "El worker de esquema termino sin resultado",
            ))),
        }
    }
}

pub fn spawn_schema(engine: Engine, conn_string: String) -> SchemaInFlight {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = match engine {
            Engine::Sqlite => Connection::open(&conn_string)
                .map_err(|err| format!("No se pudo abrir {conn_string}: {err}"))
                .and_then(|conn| sqlite_schema(&conn)),
            Engine::Postgres => postgres::Client::connect(&conn_string, postgres::NoTls)
                .map_err(|err| format!("No se pudo conectar: {err}"))
                .and_then(|mut client| postgres_schema(&mut client)),
            Engine::Mysql => mysql::Conn::new(conn_string.as_str())
                .map_err(|err| format!("No se pudo conectar: {err}"))
                .and_then(|mut conn| mysql_schema(&mut conn)),
        };
        let _ = sender.send(result);
    });
    SchemaInFlight { receiver }
}

/// However a query in flight gets cancelled, per engine. Sqlite aborts the
/// statement directly; postgres opens a side connection carrying the
/// backend's cancellation secret; mysql opens a side connection and issues
/// `KILL QUERY` against the connection id captured when the query started.
/// The mysql path needs the `KILL QUERY` privilege — a best-effort rather
/// than a protocol-level guarantee like the other two.
enum Canceler {
    Sqlite(InterruptHandle),
    Postgres(postgres::CancelToken),
    Mysql { dsn: String, connection_id: u64 },
}

impl Canceler {
    fn cancel(&self) {
        match self {
            Canceler::Sqlite(handle) => handle.interrupt(),
            Canceler::Postgres(token) => {
                let _ = token.cancel_query(postgres::NoTls);
            }
            Canceler::Mysql { dsn, connection_id } => {
                if let Ok(mut killer) = mysql::Conn::new(dsn.as_str()) {
                    let _ = killer.query_drop(format!("KILL QUERY {connection_id}"));
                }
            }
        }
    }
}

/// A query running on a worker thread. Dropping it abandons the query: the
/// thread finishes on its own and its result is discarded. Mirrors
/// `http::InFlight`, plus `cancel`, which HTTP never needed since a stuck
/// request is far rarer than a slow query.
pub struct InFlight {
    pub db_path: String,
    pub started: Instant,
    receiver: Receiver<QueryResult>,
    canceler: Arc<Mutex<Option<Canceler>>>,
}

impl InFlight {
    /// Non-blocking: `Some` once the worker is done.
    pub fn try_finish(&self) -> Option<QueryResult> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(QueryResult::failed(
                String::from("El worker SQL termino sin resultado"),
                self.started.elapsed(),
            )),
        }
    }

    /// Asks the engine to abort the statement in flight. A no-op if the
    /// connection has not finished opening yet or the query already ended;
    /// `try_finish` still delivers whatever result follows.
    pub fn cancel(&self) {
        if let Some(canceler) = self.canceler.lock().unwrap().as_ref() {
            canceler.cancel();
        }
    }
}

pub fn spawn_query(engine: Engine, conn_string: String, sql: String, offset: usize) -> InFlight {
    let (sender, receiver) = mpsc::channel();
    let canceler: Arc<Mutex<Option<Canceler>>> = Arc::new(Mutex::new(None));
    let canceler_for_worker = Arc::clone(&canceler);
    let worker_conn_string = conn_string.clone();
    thread::spawn(move || {
        let started = Instant::now();
        let result = match engine {
            Engine::Sqlite => match Connection::open(&worker_conn_string) {
                Ok(conn) => {
                    *canceler_for_worker.lock().unwrap() =
                        Some(Canceler::Sqlite(conn.get_interrupt_handle()));
                    run_sqlite(&conn, &sql, started, offset)
                }
                Err(err) => QueryResult::failed(
                    format!("No se pudo abrir {worker_conn_string}: {err}"),
                    started.elapsed(),
                ),
            },
            Engine::Postgres => {
                match postgres::Client::connect(&worker_conn_string, postgres::NoTls) {
                    Ok(mut client) => {
                        *canceler_for_worker.lock().unwrap() =
                            Some(Canceler::Postgres(client.cancel_token()));
                        run_postgres(&mut client, &sql, started, offset)
                    }
                    Err(err) => QueryResult::failed(
                        format!("No se pudo conectar: {err}"),
                        started.elapsed(),
                    ),
                }
            }
            Engine::Mysql => match mysql::Conn::new(worker_conn_string.as_str()) {
                Ok(mut conn) => {
                    if let Ok(Some(id)) = conn.query_first::<u64, _>("SELECT CONNECTION_ID()") {
                        *canceler_for_worker.lock().unwrap() = Some(Canceler::Mysql {
                            dsn: worker_conn_string.clone(),
                            connection_id: id,
                        });
                    }
                    run_mysql(&mut conn, &sql, started, offset)
                }
                Err(err) => {
                    QueryResult::failed(format!("No se pudo conectar: {err}"), started.elapsed())
                }
            },
        };
        let _ = sender.send(result);
    });
    InFlight {
        db_path: conn_string,
        started: Instant::now(),
        receiver,
        canceler,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn memory_conn() -> Connection {
        Connection::open_in_memory().expect("in-memory sqlite")
    }

    #[test]
    fn select_returns_columns_and_rows() {
        let conn = memory_conn();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO users (name) VALUES ('ana'), ('beto');",
        )
        .unwrap();

        let result = run_sqlite(
            &conn,
            "SELECT id, name FROM users ORDER BY id",
            Instant::now(),
            0,
        );

        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.rows, vec![vec!["1", "ana"], vec!["2", "beto"]]);
        assert!(result.error.is_none());
        assert!(result.rows_affected.is_none());
        assert!(!result.has_more);
        assert_eq!(result.offset, 0);
    }

    #[test]
    fn insert_reports_rows_affected() {
        let conn = memory_conn();
        conn.execute_batch("CREATE TABLE t (id INTEGER)").unwrap();

        let result = run_sqlite(
            &conn,
            "INSERT INTO t (id) VALUES (1), (2), (3)",
            Instant::now(),
            0,
        );

        assert_eq!(result.rows_affected, Some(3));
        assert!(result.columns.is_empty());
        assert!(result.error.is_none());
    }

    #[test]
    fn invalid_sql_is_captured_as_error() {
        let conn = memory_conn();

        let result = run_sqlite(&conn, "SELEC * FROM nowhere", Instant::now(), 0);

        assert!(result.error.is_some());
    }

    #[test]
    fn null_and_blob_values_render_without_panicking() {
        let conn = memory_conn();
        conn.execute_batch(
            "CREATE TABLE t (a INTEGER, b BLOB);
             INSERT INTO t (a, b) VALUES (NULL, X'0102');",
        )
        .unwrap();

        let result = run_sqlite(&conn, "SELECT a, b FROM t", Instant::now(), 0);

        assert_eq!(
            result.rows,
            vec![vec![String::from("NULL"), String::from("<blob 2 bytes>")]]
        );
    }

    #[test]
    fn opening_a_directory_as_a_database_is_captured_as_error() {
        // A directory always exists, so this fails on sqlite trying to read
        // it as a database file rather than on a flaky missing-path check.
        let dir = std::env::temp_dir();
        let result = run_sqlite_query(dir.to_str().unwrap(), "SELECT 1", 0);
        assert!(result.error.is_some());
    }

    #[test]
    fn large_result_is_paged_instead_of_collected_whole() {
        let conn = memory_conn();
        conn.execute_batch(
            "CREATE TABLE t (n INTEGER);
             WITH RECURSIVE gen(n) AS (
                 SELECT 1 UNION ALL SELECT n + 1 FROM gen WHERE n < 500
             )
             INSERT INTO t SELECT n FROM gen;",
        )
        .unwrap();

        let first = run_sqlite(&conn, "SELECT n FROM t ORDER BY n", Instant::now(), 0);
        assert_eq!(first.rows.len(), PAGE_SIZE);
        assert!(first.has_more);
        assert_eq!(first.rows[0][0], "1");

        let second = run_sqlite(
            &conn,
            "SELECT n FROM t ORDER BY n",
            Instant::now(),
            PAGE_SIZE,
        );
        assert_eq!(second.rows.len(), PAGE_SIZE);
        assert!(second.has_more);
        assert_eq!(second.rows[0][0], (PAGE_SIZE + 1).to_string());

        let last = run_sqlite(&conn, "SELECT n FROM t ORDER BY n", Instant::now(), 400);
        assert_eq!(last.rows.len(), 100);
        assert!(!last.has_more);
    }

    #[test]
    fn cancel_interrupts_a_running_query() {
        // A single aggregate row, but it cannot be produced until the whole
        // 100-million-step recursion finishes — plenty of time to interrupt
        // it mid-computation, before `cursor.next()` ever returns.
        let in_flight = spawn_query(
            Engine::Sqlite,
            String::from(":memory:"),
            String::from(
                "SELECT COUNT(*) FROM (
                     WITH RECURSIVE cnt(x) AS (
                         SELECT 0 UNION ALL SELECT x + 1 FROM cnt WHERE x < 100000000
                     )
                     SELECT x FROM cnt
                 )",
            ),
            0,
        );
        thread::sleep(Duration::from_millis(50));
        in_flight.cancel();

        let mut result = None;
        for _ in 0..200 {
            if let Some(found) = in_flight.try_finish() {
                result = Some(found);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let result = result.expect("the worker should finish shortly after being interrupted");
        assert!(result.error.is_some());
    }

    #[test]
    fn engine_parse_accepts_known_aliases_and_rejects_unknown_text() {
        assert_eq!(Engine::parse("sqlite"), Some(Engine::Sqlite));
        assert_eq!(Engine::parse("PostgreSQL"), Some(Engine::Postgres));
        assert_eq!(Engine::parse(" mysql "), Some(Engine::Mysql));
        assert_eq!(Engine::parse("oracle"), None);
    }

    #[test]
    fn engine_from_stored_falls_back_to_sqlite() {
        assert_eq!(Engine::from_stored(""), Engine::Sqlite);
        assert_eq!(Engine::from_stored("unknown"), Engine::Sqlite);
        assert_eq!(Engine::from_stored("mysql"), Engine::Mysql);
    }

    #[test]
    fn sqlite_schema_lists_tables_and_columns_with_primary_keys() {
        let conn = memory_conn();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER);",
        )
        .unwrap();

        let tables = sqlite_schema(&conn).unwrap();

        assert_eq!(tables.len(), 2);
        let users = tables.iter().find(|t| t.name == "users").unwrap();
        let id_column = users.columns.iter().find(|c| c.name == "id").unwrap();
        assert!(id_column.primary_key);
        let name_column = users.columns.iter().find(|c| c.name == "name").unwrap();
        assert!(!name_column.nullable);
        assert!(!name_column.primary_key);
    }

    #[test]
    fn mysql_value_renders_every_variant() {
        use mysql::Value;
        assert_eq!(mysql_value_to_string(&Value::NULL), "NULL");
        assert_eq!(
            mysql_value_to_string(&Value::Bytes(b"hola".to_vec())),
            "hola"
        );
        assert_eq!(mysql_value_to_string(&Value::Int(-5)), "-5");
        assert_eq!(mysql_value_to_string(&Value::UInt(5)), "5");
        assert_eq!(
            mysql_value_to_string(&Value::Date(2024, 1, 2, 0, 0, 0, 0)),
            "2024-01-02"
        );
        assert_eq!(
            mysql_value_to_string(&Value::Date(2024, 1, 2, 3, 4, 5, 0)),
            "2024-01-02 03:04:05"
        );
        assert_eq!(
            mysql_value_to_string(&Value::Time(false, 0, 1, 2, 3, 0)),
            "01:02:03"
        );
    }
}
