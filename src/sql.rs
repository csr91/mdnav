//! SQL execution domain: open a connection, run one statement, format one
//! page of its result. Pure and synchronous — no UI state lives here.
//! Threading and cancellation are the only impure bits (`spawn_query`,
//! `InFlight::cancel`), mirroring `http::spawn_request`.
//!
//! Phase 1 supports sqlite only; `run_query` is written against a generic
//! `rusqlite::Connection` so later phases can add other engines behind the
//! same `QueryResult` shape without reshaping this module.
//!
//! A query that returns rows is never collected whole into memory: `run_query`
//! skips to `offset`, reads at most `PAGE_SIZE` rows and peeks one more to
//! set `has_more`, so a result with millions of rows costs one page, not all
//! of them. The studio re-runs the same query at a later `offset` to fetch
//! the next page — a re-scan, not a resumed cursor, which is simpler to run
//! from a one-shot worker thread and cheap enough for a TUI helper.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result as AnyResult};
use rusqlite::{types::ValueRef, Connection, InterruptHandle};

/// Rows fetched per page. Bounds memory and worker time regardless of how
/// many rows the query would actually return.
pub const PAGE_SIZE: usize = 200;

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
    run_query(&conn, sql, started, offset)
}

/// Split out from `run_sqlite_query` so tests can exercise it against an
/// in-memory connection without touching the filesystem.
fn run_query(conn: &Connection, sql: &str, started: Instant, offset: usize) -> QueryResult {
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
                        Ok(value) => values.push(value_to_string(value)),
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

fn value_to_string(value: ValueRef) -> String {
    match value {
        ValueRef::Null => String::from("NULL"),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
        ValueRef::Blob(blob) => format!("<blob {} bytes>", blob.len()),
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
    /// Filled in by the worker once its connection is open. `cancel` calls
    /// `InterruptHandle::interrupt`, which is `Send + Sync` for exactly this
    /// cross-thread use, aborting the statement sqlite is running.
    interrupt: Arc<Mutex<Option<InterruptHandle>>>,
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

    /// Asks sqlite to abort the statement in flight. A no-op if the
    /// connection has not finished opening yet or the query already ended;
    /// `try_finish` still delivers whatever result follows.
    pub fn cancel(&self) {
        if let Some(handle) = self.interrupt.lock().unwrap().as_ref() {
            handle.interrupt();
        }
    }
}

pub fn spawn_query(db_path: String, sql: String, offset: usize) -> InFlight {
    let (sender, receiver) = mpsc::channel();
    let interrupt: Arc<Mutex<Option<InterruptHandle>>> = Arc::new(Mutex::new(None));
    let interrupt_for_worker = Arc::clone(&interrupt);
    let worker_path = db_path.clone();
    thread::spawn(move || {
        let started = Instant::now();
        let conn = match Connection::open(&worker_path) {
            Ok(conn) => conn,
            Err(err) => {
                let message = format!("No se pudo abrir {worker_path}: {err}");
                let _ = sender.send(QueryResult::failed(message, started.elapsed()));
                return;
            }
        };
        *interrupt_for_worker.lock().unwrap() = Some(conn.get_interrupt_handle());
        let _ = sender.send(run_query(&conn, &sql, started, offset));
    });
    InFlight {
        db_path,
        started: Instant::now(),
        receiver,
        interrupt,
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

        let result = run_query(
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

        let result = run_query(
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

        let result = run_query(&conn, "SELEC * FROM nowhere", Instant::now(), 0);

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

        let result = run_query(&conn, "SELECT a, b FROM t", Instant::now(), 0);

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

        let first = run_query(&conn, "SELECT n FROM t ORDER BY n", Instant::now(), 0);
        assert_eq!(first.rows.len(), PAGE_SIZE);
        assert!(first.has_more);
        assert_eq!(first.rows[0][0], "1");

        let second = run_query(
            &conn,
            "SELECT n FROM t ORDER BY n",
            Instant::now(),
            PAGE_SIZE,
        );
        assert_eq!(second.rows.len(), PAGE_SIZE);
        assert!(second.has_more);
        assert_eq!(second.rows[0][0], (PAGE_SIZE + 1).to_string());

        let last = run_query(&conn, "SELECT n FROM t ORDER BY n", Instant::now(), 400);
        assert_eq!(last.rows.len(), 100);
        assert!(!last.has_more);
    }

    #[test]
    fn cancel_interrupts_a_running_query() {
        // A single aggregate row, but it cannot be produced until the whole
        // 100-million-step recursion finishes — plenty of time to interrupt
        // it mid-computation, before `cursor.next()` ever returns.
        let in_flight = spawn_query(
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
}
