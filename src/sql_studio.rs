//! `:@sql` studio: a library of saved sqlite connections and queries,
//! independent of the working directory — mirrors `studio.rs` for HTTP.
//! This file owns the on-disk library (connections, their environments and
//! queries, all plain files) and the tree/prompt UI state; `sql.rs` stays
//! the pure query execution domain.
//!
//! Each connection can hold several named environments (dev/staging/prod),
//! each a single value (today a sqlite path; a future driver's connection
//! string). A secret environment's value never touches `connection.toml`:
//! it lives in a gitignored `.env` next to it, keyed by the environment
//! name — the same convention `http.rs` uses for HTTP environments.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    http,
    sql::{self, ColumnInfo, Engine, InFlight, QueryResult, SchemaInFlight},
    studio::validate_entry_name,
};

const CONNECTION_FILE: &str = "connection.toml";
const QUERY_EXT: &str = "sql";
const DEFAULT_ENVIRONMENT: &str = "default";

#[derive(Clone, Serialize, Deserialize, Default)]
struct EnvironmentFile {
    name: String,
    #[serde(default)]
    secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct ConnectionFile {
    engine: String,
    /// Legacy phase-2 format: a single flat path. Migrated into a `default`
    /// environment the first time the connection is written back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default)]
    environments: Vec<EnvironmentFile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlEnvironmentEntry {
    pub name: String,
    pub secret: bool,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlQueryEntry {
    pub name: String,
    pub path: PathBuf,
}

/// A table and its columns, cached in memory once loaded. Never written to
/// `connection.toml`: this is live database state, not configuration, and
/// would just go stale sitting on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlSchemaTable {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SqlConnection {
    pub name: String,
    pub path: PathBuf,
    pub engine: Engine,
    pub environments: Vec<SqlEnvironmentEntry>,
    pub queries: Vec<SqlQueryEntry>,
    pub expanded: bool,
    /// Whether the "Tablas" branch is expanded. Loading is lazy and keyed
    /// off this: expanding it the first time triggers `spawn_schema`.
    pub tables_expanded: bool,
    pub tables: Option<Vec<SqlSchemaTable>>,
    pub tables_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlRow {
    Connection(usize),
    Environment(usize, usize),
    Query(usize, usize),
    /// The "Tablas" branch header under a connection.
    TablesGroup(usize),
    Table(usize, usize),
    Column(usize, usize, usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlFocus {
    Tree,
    Query,
    Results,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SqlPromptKind {
    NewConnectionName,
    NewConnectionEngine {
        name: String,
    },
    NewConnectionValue {
        name: String,
        engine: Engine,
    },
    RenameConnection {
        path: PathBuf,
    },
    RenameQuery {
        path: PathBuf,
    },
    NewQueryName {
        connection_dir: PathBuf,
    },
    NewEnvironmentName {
        connection_dir: PathBuf,
    },
    NewEnvironmentValue {
        connection_dir: PathBuf,
        name: String,
    },
    NewEnvironmentSecret {
        connection_dir: PathBuf,
        name: String,
        value: String,
    },
    RenameEnvironment {
        connection_dir: PathBuf,
        old_name: String,
    },
    ConfirmDeleteConnection {
        path: PathBuf,
        label: String,
    },
    ConfirmDeleteQuery {
        path: PathBuf,
        label: String,
    },
    ConfirmDeleteEnvironment {
        connection_dir: PathBuf,
        name: String,
        label: String,
    },
}

pub struct SqlPrompt {
    pub kind: SqlPromptKind,
    pub title: String,
    pub input: String,
}

pub struct SqlStudio {
    pub active: bool,
    pub focus: SqlFocus,
    pub connections_root: Option<PathBuf>,
    pub connections: Vec<SqlConnection>,
    pub cursor: usize,
    pub active_connection: Option<PathBuf>,
    pub loaded_query: Option<PathBuf>,
    /// The query text as last loaded, saved or run. Compared against
    /// `query_input` to decide whether leaving would lose work.
    pub loaded_query_snapshot: String,
    pub query_input: String,
    pub in_flight: Option<InFlight>,
    pub result: Option<QueryResult>,
    /// The engine, query text and connection value the current/last page
    /// came from — fetching another page re-runs exactly this, regardless
    /// of what the user has since typed into `query_input`.
    pub last_run_engine: Engine,
    pub last_run_query: String,
    pub last_run_db_path: String,
    pub row_offset: usize,
    pub col_offset: usize,
    pub prompt: Option<SqlPrompt>,
    /// Showing the g/d/Esc save-or-discard prompt because the query editor
    /// has unsaved text and the user tried to leave.
    pub exit_prompt: bool,
    /// Which connection (by index) is loading its table list, if any.
    pub schema_in_flight: Option<(usize, SchemaInFlight)>,
}

impl Default for SqlStudio {
    fn default() -> Self {
        Self {
            active: false,
            focus: SqlFocus::Tree,
            connections_root: None,
            connections: Vec::new(),
            cursor: 0,
            active_connection: None,
            loaded_query: None,
            loaded_query_snapshot: String::new(),
            query_input: String::new(),
            in_flight: None,
            result: None,
            last_run_engine: Engine::default(),
            last_run_query: String::new(),
            last_run_db_path: String::new(),
            row_offset: 0,
            col_offset: 0,
            prompt: None,
            exit_prompt: false,
            schema_in_flight: None,
        }
    }
}

impl SqlStudio {
    /// Dropping `in_flight` abandons any query on the wire: the worker
    /// thread finishes on its own and its result is discarded.
    pub fn close(&mut self) {
        self.active = false;
        self.in_flight = None;
        self.schema_in_flight = None;
        self.prompt = None;
        self.exit_prompt = false;
    }

    /// True when the query editor holds text that neither matches the
    /// loaded/saved file nor has just been run — leaving now would lose it.
    pub fn is_query_dirty(&self) -> bool {
        !self.query_input.trim().is_empty() && self.query_input != self.loaded_query_snapshot
    }

    pub fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            SqlFocus::Tree => SqlFocus::Query,
            SqlFocus::Query => SqlFocus::Results,
            SqlFocus::Results => SqlFocus::Tree,
        };
    }

    pub fn cycle_focus_back(&mut self) {
        self.focus = match self.focus {
            SqlFocus::Tree => SqlFocus::Results,
            SqlFocus::Query => SqlFocus::Tree,
            SqlFocus::Results => SqlFocus::Query,
        };
    }

    pub fn visible_rows(&self) -> Vec<SqlRow> {
        let mut rows = Vec::new();
        for (ci, connection) in self.connections.iter().enumerate() {
            rows.push(SqlRow::Connection(ci));
            if connection.expanded {
                for ei in 0..connection.environments.len() {
                    rows.push(SqlRow::Environment(ci, ei));
                }
                rows.push(SqlRow::TablesGroup(ci));
                if connection.tables_expanded {
                    if let Some(tables) = &connection.tables {
                        for (ti, table) in tables.iter().enumerate() {
                            rows.push(SqlRow::Table(ci, ti));
                            if table.expanded {
                                for coli in 0..table.columns.len() {
                                    rows.push(SqlRow::Column(ci, ti, coli));
                                }
                            }
                        }
                    }
                }
                for qi in 0..connection.queries.len() {
                    rows.push(SqlRow::Query(ci, qi));
                }
            }
        }
        rows
    }

    pub fn selected_row(&self) -> Option<SqlRow> {
        self.visible_rows().get(self.cursor).copied()
    }

    /// The connection folder that owns whatever row is currently focused —
    /// `n`/`Shift+E` use this so they work from a connection, environment or
    /// query row alike.
    pub fn selected_connection_dir(&self) -> Option<PathBuf> {
        match self.selected_row()? {
            SqlRow::Connection(ci)
            | SqlRow::Environment(ci, _)
            | SqlRow::Query(ci, _)
            | SqlRow::TablesGroup(ci)
            | SqlRow::Table(ci, _)
            | SqlRow::Column(ci, _, _) => Some(self.connections[ci].path.clone()),
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.visible_rows().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i32 + delta).clamp(0, len as i32 - 1) as usize;
    }

    pub fn clamp_cursor(&mut self) {
        let len = self.visible_rows().len();
        if self.cursor >= len {
            self.cursor = len.saturating_sub(1);
        }
    }

    pub fn toggle_expanded(&mut self, index: usize) {
        if let Some(connection) = self.connections.get_mut(index) {
            connection.expanded = !connection.expanded;
        }
    }

    /// Selects the connection at `path` in the tree: expands it, makes it
    /// the active connection and moves the cursor onto its row.
    pub fn focus_connection(&mut self, path: &Path) {
        let Some(index) = self.connections.iter().position(|c| c.path == path) else {
            return;
        };
        self.connections[index].expanded = true;
        self.active_connection = Some(self.connections[index].path.clone());
        if let Some(cursor) = self.row_index(SqlRow::Connection(index)) {
            self.cursor = cursor;
        }
    }

    /// Selects the query at `path`: expands its parent connection, makes
    /// that connection active and moves the cursor onto the query's row.
    pub fn focus_query(&mut self, path: &Path) {
        let Some((ci, qi)) = self
            .connections
            .iter()
            .enumerate()
            .find_map(|(ci, connection)| {
                connection
                    .queries
                    .iter()
                    .position(|query| query.path == path)
                    .map(|qi| (ci, qi))
            })
        else {
            return;
        };
        self.connections[ci].expanded = true;
        self.active_connection = Some(self.connections[ci].path.clone());
        if let Some(cursor) = self.row_index(SqlRow::Query(ci, qi)) {
            self.cursor = cursor;
        }
    }

    fn row_index(&self, target: SqlRow) -> Option<usize> {
        self.visible_rows().iter().position(|row| *row == target)
    }

    pub fn scroll_rows(&mut self, delta: i32) {
        let Some(result) = self.result.as_ref() else {
            return;
        };
        let max = result.rows.len().saturating_sub(1);
        self.row_offset = clamp_scroll(self.row_offset, delta, max);
    }

    pub fn scroll_columns(&mut self, delta: i32) {
        let Some(result) = self.result.as_ref() else {
            return;
        };
        let max = result.columns.len().saturating_sub(1);
        self.col_offset = clamp_scroll(self.col_offset, delta, max);
    }
}

fn clamp_scroll(current: usize, delta: i32, max: usize) -> usize {
    (current as i32 + delta).clamp(0, max as i32) as usize
}

// ── Persistence: the library lives independent of the working directory ────

/// The folder that holds every saved connection, independent of the working
/// directory. Mirrors `studio::collections_dir`.
pub fn connections_dir(custom: &str) -> Result<PathBuf> {
    let path = if custom.trim().is_empty() {
        dirs::config_dir()
            .context("No se pudo resolver la carpeta de config")?
            .join("mdnav")
            .join("sql")
    } else {
        PathBuf::from(custom.trim())
    };
    fs::create_dir_all(&path).with_context(|| format!("No se pudo crear {}", path.display()))?;
    Ok(http::canonical_path(&path))
}

/// Loads the library, seeding a starter connection with an example table
/// the first time it is opened — same idea as the HTTP studio's example
/// collection.
pub fn ensure_library(dir: &Path) -> Result<Vec<SqlConnection>> {
    fs::create_dir_all(dir)?;
    let mut found = list_connections(dir);
    if found.is_empty() {
        create_example_connection(dir)?;
        found = list_connections(dir);
    }
    if let Some(first) = found.first_mut() {
        first.expanded = true;
    }
    Ok(found)
}

fn create_example_connection(dir: &Path) -> Result<()> {
    let folder = dir.join("ejemplo");
    fs::create_dir_all(&folder)?;
    let db_path = folder.join("ejemplo.sqlite");
    sql::init_example_db(&db_path)?;
    write_connection_file(
        &folder,
        "sqlite",
        vec![EnvironmentFile {
            name: DEFAULT_ENVIRONMENT.to_string(),
            secret: false,
            value: Some(db_path.to_string_lossy().into_owned()),
        }],
    )?;
    write_query(
        &folder.join(format!("listar-notas.{QUERY_EXT}")),
        "SELECT * FROM notas ORDER BY id;",
    )?;
    Ok(())
}

/// Re-scans the library from disk, carrying over which connections were
/// expanded so a refresh after a CRUD op does not collapse the tree.
pub fn refresh(dir: &Path, previous: &[SqlConnection]) -> Vec<SqlConnection> {
    let mut connections = list_connections(dir);
    for connection in connections.iter_mut() {
        if let Some(prior) = previous.iter().find(|p| p.path == connection.path) {
            connection.expanded = prior.expanded;
            connection.tables_expanded = prior.tables_expanded;
            connection.tables = prior.tables.clone();
            connection.tables_error = prior.tables_error.clone();
        }
    }
    connections
}

fn list_connections(dir: &Path) -> Vec<SqlConnection> {
    let mut connections = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return connections;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(connection) = load_connection(&path) {
            connections.push(connection);
        }
    }
    connections.sort_by_key(|connection| connection.name.to_lowercase());
    connections
}

fn load_connection(dir: &Path) -> Option<SqlConnection> {
    let file = load_connection_file(dir)?;
    let name = dir.file_name()?.to_string_lossy().into_owned();
    Some(SqlConnection {
        name,
        path: http::canonical_path(dir),
        engine: Engine::from_stored(&file.engine),
        environments: resolve_environments(dir, &file),
        queries: list_queries(dir),
        expanded: false,
        tables_expanded: false,
        tables: None,
        tables_error: None,
    })
}

fn load_connection_file(dir: &Path) -> Option<ConnectionFile> {
    let text = fs::read_to_string(dir.join(CONNECTION_FILE)).ok()?;
    toml::from_str(&text).ok()
}

/// Turns the stored entries (secret values kept out of `connection.toml`)
/// into ready-to-use environments, falling back to the legacy phase-2 flat
/// `path` field when nothing has been migrated to `environments` yet.
fn resolve_environments(dir: &Path, file: &ConnectionFile) -> Vec<SqlEnvironmentEntry> {
    if !file.environments.is_empty() {
        let dotenv = http::read_dotenv(dir);
        return file
            .environments
            .iter()
            .map(|entry| {
                let value = if entry.secret {
                    dotenv
                        .get(&secret_key(&entry.name))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    entry.value.clone().unwrap_or_default()
                };
                SqlEnvironmentEntry {
                    name: entry.name.clone(),
                    secret: entry.secret,
                    value,
                }
            })
            .collect();
    }
    match &file.path {
        Some(path) => vec![SqlEnvironmentEntry {
            name: DEFAULT_ENVIRONMENT.to_string(),
            secret: false,
            value: path.clone(),
        }],
        None => Vec::new(),
    }
}

/// The `.env` key a secret environment's value is stored under.
fn secret_key(env_name: &str) -> String {
    env_name
        .to_uppercase()
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
}

fn list_queries(dir: &Path) -> Vec<SqlQueryEntry> {
    let mut queries = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return queries;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some(QUERY_EXT) {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
        else {
            continue;
        };
        queries.push(SqlQueryEntry { name, path });
    }
    queries.sort_by_key(|query| query.name.to_lowercase());
    queries
}

fn write_connection_file(
    dir: &Path,
    engine: &str,
    environments: Vec<EnvironmentFile>,
) -> Result<()> {
    let file = ConnectionFile {
        engine: engine.to_string(),
        path: None,
        environments,
    };
    save_connection_file(dir, &file)
}

fn save_connection_file(dir: &Path, file: &ConnectionFile) -> Result<()> {
    let text = toml::to_string(file).context("No se pudo serializar la conexion")?;
    let target = dir.join(CONNECTION_FILE);
    fs::write(&target, text).with_context(|| format!("No se pudo escribir {}", target.display()))
}

/// Drops the legacy flat `path` field once a connection gains a real
/// `environments` list, converting it into a `default` entry first if the
/// list was still empty.
fn migrate_legacy_path(file: &mut ConnectionFile) {
    if file.environments.is_empty() {
        if let Some(path) = file.path.take() {
            file.environments.push(EnvironmentFile {
                name: DEFAULT_ENVIRONMENT.to_string(),
                secret: false,
                value: Some(path),
            });
        }
    } else {
        file.path = None;
    }
}

pub fn create_connection(root: &Path, name: &str, engine: Engine, value: &str) -> Result<PathBuf> {
    validate_entry_name(name)?;
    let value = value.trim();
    if value.is_empty() {
        bail!("Hace falta el valor de conexion");
    }
    let folder = root.join(http::sanitize_file_name(name));
    if folder.exists() {
        bail!("Ya existe {}", folder.display());
    }
    fs::create_dir_all(&folder)?;
    write_connection_file(
        &folder,
        engine.as_str(),
        vec![EnvironmentFile {
            name: DEFAULT_ENVIRONMENT.to_string(),
            secret: false,
            value: Some(value.to_string()),
        }],
    )?;
    Ok(http::canonical_path(&folder))
}

pub fn rename_connection(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_entry_name(new_name)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("La conexion no tiene carpeta"))?;
    let dest = parent.join(http::sanitize_file_name(new_name));
    if dest != path && dest.exists() {
        bail!("Ya existe {}", dest.display());
    }
    if dest != path {
        fs::rename(path, &dest)
            .with_context(|| format!("No se pudo renombrar {}", path.display()))?;
    }
    Ok(http::canonical_path(&dest))
}

/// Copies the whole folder, `.env` secrets included: a duplicated
/// connection is expected to work exactly like the original.
pub fn duplicate_connection(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("La conexion no tiene carpeta"))?;
    let base = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("conexion");
    let dest = unique_sibling_dir(parent, base);
    crate::docs::copy_path(path, &dest)?;
    Ok(http::canonical_path(&dest))
}

pub fn delete_connection(path: &Path) -> Result<()> {
    fs::remove_dir_all(path).with_context(|| format!("No se pudo borrar {}", path.display()))
}

fn unique_sibling_dir(parent: &Path, base: &str) -> PathBuf {
    let mut used = HashSet::new();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                used.insert(name.to_lowercase());
            }
        }
    }
    let mut candidate = format!("{base}-copia");
    let mut counter = 2;
    while used.contains(&candidate.to_lowercase()) {
        candidate = format!("{base}-copia-{counter}");
        counter += 1;
    }
    parent.join(candidate)
}

// ── Environments ─────────────────────────────────────────────────────────

pub fn create_environment(
    connection_dir: &Path,
    name: &str,
    value: &str,
    secret: bool,
) -> Result<()> {
    validate_entry_name(name)?;
    let value = value.trim();
    if value.is_empty() {
        bail!("Hace falta un valor");
    }
    let mut file = load_connection_file(connection_dir)
        .ok_or_else(|| anyhow!("No se pudo leer la conexion"))?;
    migrate_legacy_path(&mut file);
    if file.environments.iter().any(|entry| entry.name == name) {
        bail!("Ya existe un entorno \"{name}\"");
    }
    if secret {
        http::upsert_dotenv(connection_dir, &secret_key(name), value)?;
        file.environments.push(EnvironmentFile {
            name: name.to_string(),
            secret: true,
            value: None,
        });
    } else {
        file.environments.push(EnvironmentFile {
            name: name.to_string(),
            secret: false,
            value: Some(value.to_string()),
        });
    }
    save_connection_file(connection_dir, &file)
}

pub fn rename_environment(connection_dir: &Path, old_name: &str, new_name: &str) -> Result<()> {
    validate_entry_name(new_name)?;
    let mut file = load_connection_file(connection_dir)
        .ok_or_else(|| anyhow!("No se pudo leer la conexion"))?;
    migrate_legacy_path(&mut file);
    if old_name != new_name && file.environments.iter().any(|entry| entry.name == new_name) {
        bail!("Ya existe un entorno \"{new_name}\"");
    }
    let secret = {
        let Some(entry) = file
            .environments
            .iter_mut()
            .find(|entry| entry.name == old_name)
        else {
            bail!("El entorno \"{old_name}\" ya no existe");
        };
        entry.name = new_name.to_string();
        entry.secret
    };
    if secret {
        let dotenv = http::read_dotenv(connection_dir);
        if let Some(value) = dotenv.get(&secret_key(old_name)) {
            http::upsert_dotenv(connection_dir, &secret_key(new_name), value)?;
        }
        http::remove_dotenv_key(connection_dir, &secret_key(old_name))?;
    }
    save_connection_file(connection_dir, &file)
}

pub fn duplicate_environment(connection_dir: &Path, name: &str) -> Result<String> {
    let mut file = load_connection_file(connection_dir)
        .ok_or_else(|| anyhow!("No se pudo leer la conexion"))?;
    migrate_legacy_path(&mut file);
    let Some(source) = file
        .environments
        .iter()
        .find(|entry| entry.name == name)
        .cloned()
    else {
        bail!("El entorno \"{name}\" ya no existe");
    };
    let existing: HashSet<String> = file
        .environments
        .iter()
        .map(|entry| entry.name.to_lowercase())
        .collect();
    let mut candidate = format!("{name}-copia");
    let mut counter = 2;
    while existing.contains(&candidate.to_lowercase()) {
        candidate = format!("{name}-copia-{counter}");
        counter += 1;
    }
    if source.secret {
        let dotenv = http::read_dotenv(connection_dir);
        if let Some(value) = dotenv.get(&secret_key(name)) {
            http::upsert_dotenv(connection_dir, &secret_key(&candidate), value)?;
        }
    }
    file.environments.push(EnvironmentFile {
        name: candidate.clone(),
        secret: source.secret,
        value: source.value,
    });
    save_connection_file(connection_dir, &file)?;
    Ok(candidate)
}

pub fn delete_environment(connection_dir: &Path, name: &str) -> Result<()> {
    let mut file = load_connection_file(connection_dir)
        .ok_or_else(|| anyhow!("No se pudo leer la conexion"))?;
    migrate_legacy_path(&mut file);
    let before = file.environments.len();
    file.environments.retain(|entry| entry.name != name);
    if file.environments.len() == before {
        bail!("El entorno \"{name}\" ya no existe");
    }
    http::remove_dotenv_key(connection_dir, &secret_key(name))?;
    save_connection_file(connection_dir, &file)
}

// ── Queries ──────────────────────────────────────────────────────────────

pub fn create_query(connection_dir: &Path, name: &str, contents: &str) -> Result<PathBuf> {
    validate_entry_name(name)?;
    let path = connection_dir.join(format!("{}.{QUERY_EXT}", http::sanitize_file_name(name)));
    if path.exists() {
        bail!("Ya existe {}", path.display());
    }
    write_query(&path, contents)?;
    Ok(path)
}

pub fn rename_query(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_entry_name(new_name)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("La query no tiene carpeta"))?;
    let dest = parent.join(format!(
        "{}.{QUERY_EXT}",
        http::sanitize_file_name(new_name)
    ));
    if dest != path && dest.exists() {
        bail!("Ya existe {}", dest.display());
    }
    if dest != path {
        fs::rename(path, &dest)
            .with_context(|| format!("No se pudo renombrar {}", path.display()))?;
    }
    Ok(dest)
}

pub fn duplicate_query(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("La query no tiene carpeta"))?;
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("query");
    let mut used = HashSet::new();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                used.insert(name.to_lowercase());
            }
        }
    }
    let filename = http::unique_name(
        &mut used,
        &format!("{stem}-copia"),
        &format!(".{QUERY_EXT}"),
    );
    let dest = parent.join(filename);
    fs::copy(path, &dest).with_context(|| format!("No se pudo duplicar {}", path.display()))?;
    Ok(dest)
}

pub fn delete_query(path: &Path) -> Result<()> {
    fs::remove_file(path).with_context(|| format!("No se pudo borrar {}", path.display()))
}

pub fn read_query(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("No se pudo leer {}", path.display()))
}

/// Atomic write: never leaves a half-written `.sql` file behind.
pub fn write_query(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension(format!("{QUERY_EXT}.saving"));
    fs::write(&tmp, contents).with_context(|| format!("No se pudo escribir {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("No se pudo guardar {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mdnav-sql-studio-{label}-{}-{}",
            std::process::id(),
            label.len()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn env_value<'a>(connection: &'a SqlConnection, name: &str) -> &'a str {
        &connection
            .environments
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .value
    }

    #[test]
    fn empty_library_seeds_an_example_connection_with_a_query() {
        let dir = temp_dir("empty-library");

        let connections = ensure_library(&dir).unwrap();

        assert_eq!(connections.len(), 1);
        let example = &connections[0];
        assert_eq!(example.name, "ejemplo");
        assert!(example.expanded);
        assert_eq!(example.queries.len(), 1);
        assert_eq!(example.environments.len(), 1);
        let db_path = env_value(example, DEFAULT_ENVIRONMENT);
        assert!(Path::new(db_path).exists());

        let rows = sql::run_sqlite_query(db_path, "SELECT COUNT(*) FROM notas", 0);
        assert!(rows.error.is_none());
    }

    #[test]
    fn create_rename_duplicate_and_delete_a_connection() {
        let dir = temp_dir("connection-crud");
        let db_path = dir.join("datos.sqlite");
        sql::init_example_db(&db_path).unwrap();

        let created =
            create_connection(&dir, "local", Engine::Sqlite, &db_path.to_string_lossy()).unwrap();
        assert!(created.join(CONNECTION_FILE).exists());

        let renamed = rename_connection(&created, "principal").unwrap();
        assert!(renamed.ends_with("principal"));
        assert!(!created.exists());

        let duplicated = duplicate_connection(&renamed).unwrap();
        assert!(duplicated.join(CONNECTION_FILE).exists());
        assert_ne!(duplicated, renamed);

        let listed = list_connections(&dir);
        assert_eq!(listed.len(), 2);

        delete_connection(&duplicated).unwrap();
        assert!(!duplicated.exists());
        assert_eq!(list_connections(&dir).len(), 1);
    }

    #[test]
    fn create_rename_duplicate_and_delete_a_query() {
        let dir = temp_dir("query-crud");
        let db_path = dir.join("datos.sqlite");
        sql::init_example_db(&db_path).unwrap();
        let connection_dir =
            create_connection(&dir, "local", Engine::Sqlite, &db_path.to_string_lossy()).unwrap();

        let created = create_query(&connection_dir, "listar", "SELECT 1;").unwrap();
        assert_eq!(read_query(&created).unwrap(), "SELECT 1;");

        let renamed = rename_query(&created, "contar").unwrap();
        assert!(renamed.ends_with("contar.sql"));
        assert!(!created.exists());

        let duplicated = duplicate_query(&renamed).unwrap();
        assert_eq!(read_query(&duplicated).unwrap(), "SELECT 1;");

        let queries = list_queries(&connection_dir);
        assert_eq!(queries.len(), 2);

        delete_query(&duplicated).unwrap();
        assert_eq!(list_queries(&connection_dir).len(), 1);
    }

    #[test]
    fn write_query_is_atomic_and_leaves_no_temp_file_behind() {
        let dir = temp_dir("atomic-write");
        let path = dir.join("consulta.sql");

        write_query(&path, "SELECT 1;").unwrap();
        write_query(&path, "SELECT 2;").unwrap();

        assert_eq!(read_query(&path).unwrap(), "SELECT 2;");
        assert!(!path.with_extension("sql.saving").exists());
    }

    #[test]
    fn refresh_preserves_expanded_state_by_path() {
        let dir = temp_dir("refresh");
        let db_path = dir.join("datos.sqlite");
        sql::init_example_db(&db_path).unwrap();
        create_connection(&dir, "local", Engine::Sqlite, &db_path.to_string_lossy()).unwrap();

        let mut connections = list_connections(&dir);
        connections[0].expanded = true;

        let refreshed = refresh(&dir, &connections);

        assert!(refreshed[0].expanded);
    }

    #[test]
    fn legacy_flat_path_is_read_as_a_default_environment() {
        let dir = temp_dir("legacy-path");
        let connection_dir = dir.join("vieja");
        fs::create_dir_all(&connection_dir).unwrap();
        fs::write(
            connection_dir.join(CONNECTION_FILE),
            "engine = \"sqlite\"\npath = \"/tmp/vieja.sqlite\"\n",
        )
        .unwrap();

        let connection = load_connection(&connection_dir).unwrap();

        assert_eq!(connection.environments.len(), 1);
        assert_eq!(
            env_value(&connection, DEFAULT_ENVIRONMENT),
            "/tmp/vieja.sqlite"
        );
    }

    #[test]
    fn secret_environment_value_is_stored_in_dotenv_not_toml() {
        let dir = temp_dir("secret-env");
        let db_path = dir.join("datos.sqlite");
        sql::init_example_db(&db_path).unwrap();
        let connection_dir =
            create_connection(&dir, "local", Engine::Sqlite, &db_path.to_string_lossy()).unwrap();

        create_environment(
            &connection_dir,
            "prod",
            "postgres://user:s3cret@host/db",
            true,
        )
        .unwrap();

        let toml_text = fs::read_to_string(connection_dir.join(CONNECTION_FILE)).unwrap();
        assert!(!toml_text.contains("s3cret"));

        let connection = load_connection(&connection_dir).unwrap();
        assert_eq!(
            env_value(&connection, "prod"),
            "postgres://user:s3cret@host/db"
        );
    }

    #[test]
    fn rename_duplicate_and_delete_an_environment() {
        let dir = temp_dir("env-crud");
        let db_path = dir.join("datos.sqlite");
        sql::init_example_db(&db_path).unwrap();
        let connection_dir =
            create_connection(&dir, "local", Engine::Sqlite, &db_path.to_string_lossy()).unwrap();
        create_environment(&connection_dir, "staging", "secreta", true).unwrap();

        rename_environment(&connection_dir, "staging", "qa").unwrap();
        let connection = load_connection(&connection_dir).unwrap();
        assert!(connection
            .environments
            .iter()
            .any(|entry| entry.name == "qa"));
        assert_eq!(env_value(&connection, "qa"), "secreta");

        let duplicated_name = duplicate_environment(&connection_dir, "qa").unwrap();
        let connection = load_connection(&connection_dir).unwrap();
        assert_eq!(env_value(&connection, &duplicated_name), "secreta");

        delete_environment(&connection_dir, &duplicated_name).unwrap();
        let connection = load_connection(&connection_dir).unwrap();
        assert!(!connection
            .environments
            .iter()
            .any(|entry| entry.name == duplicated_name));
    }
}
