//! HTTP requests stored as OpenCollection YAML files — the open format Bruno
//! uses natively — so a collection is just a folder inside the repo: one
//! `.yml` per request, `environments/*.yml` and a gitignored `.env` for secrets.
//!
//! The YAML is read into `serde_json::Value` instead of rigid structs on
//! purpose: the spec accepts several shapes for the same field (`auth: inherit`
//! or an object, `params` as a list or split by kind, body variants, ...) and a
//! tolerant reader keeps collections written by other tools working.

use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::markdown::{highlight_code_block, PreviewLine, PreviewLineKind};

pub const COLLECTION_FILE: &str = "opencollection.yml";
const FOLDER_FILE: &str = "folder.yml";
const ENVIRONMENTS_DIR: &str = "environments";
const DOTENV_FILE: &str = ".env";

/// Response bytes kept for display. Anything bigger is truncated: a TUI is not
/// the place to browse a 200 MB download.
const RESPONSE_BODY_LIMIT: usize = 5 * 1024 * 1024;
/// Syntax highlighting is the slow part of rendering a response; past this
/// many lines the rest is shown as plain text.
const HIGHLIGHT_LINE_LIMIT: usize = 3000;
/// Guards against variables that reference each other in a cycle.
const MAX_INTERPOLATION_DEPTH: usize = 8;

const GRAY: [u8; 3] = [150, 150, 150];

// ── Collection discovery ─────────────────────────────────────────────────────

/// Returns the closest ancestor directory (or `path` itself) that holds an
/// `opencollection.yml`.
pub fn find_collection_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    start
        .ancestors()
        .find(|dir| dir.join(COLLECTION_FILE).is_file())
        .map(Path::to_path_buf)
}

pub fn is_collection_dir(dir: &Path) -> bool {
    dir.join(COLLECTION_FILE).is_file()
}

pub fn is_yaml_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml"))
        .unwrap_or(false)
}

/// True for the JSON exports the Postman importer understands.
pub fn is_postman_export(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            let name = name.to_ascii_lowercase();
            name.ends_with(".postman_collection.json")
                || name.ends_with(".postman_environment.json")
        })
        .unwrap_or(false)
}

/// When `path` can be a request file, returns its collection root. Collection
/// and folder settings and environment files are never requests.
pub fn request_collection_root(path: &Path) -> Option<PathBuf> {
    if !is_yaml_file(path) {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    if name.eq_ignore_ascii_case(COLLECTION_FILE) || name.eq_ignore_ascii_case(FOLDER_FILE) {
        return None;
    }
    let root = find_collection_root(path)?;
    if path.parent() == Some(root.join(ENVIRONMENTS_DIR).as_path()) {
        return None;
    }
    Some(root)
}

// ── Request model ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyValue {
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Query,
    Path,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub entry: KeyValue,
    pub kind: ParamKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    None,
    /// `json`, `text`, `xml` or `sparql`, sent as-is after interpolation.
    Raw {
        kind: String,
        data: String,
    },
    Form(Vec<KeyValue>),
    /// A body type mdnav cannot send yet (multipart, file, graphql).
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    None,
    Inherit,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    ApiKey {
        key: String,
        value: String,
        in_query: bool,
    },
    Unsupported(String),
}

impl Auth {
    fn label(&self) -> String {
        match self {
            Self::None => String::from("none"),
            Self::Inherit => String::from("inherit"),
            Self::Bearer { .. } => String::from("bearer"),
            Self::Basic { .. } => String::from("basic"),
            Self::ApiKey { in_query, .. } => {
                if *in_query {
                    String::from("api key (query)")
                } else {
                    String::from("api key (header)")
                }
            }
            Self::Unsupported(kind) => format!("{kind} (no soportado)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestSettings {
    /// `None` means no timeout, which is what `timeout: 0` asks for.
    pub timeout: Option<Duration>,
    pub follow_redirects: bool,
    pub max_redirects: usize,
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self {
            timeout: None,
            follow_redirects: true,
            max_redirects: 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequestSpec {
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<KeyValue>,
    pub params: Vec<Param>,
    pub body: Body,
    pub auth: Auth,
    pub settings: RequestSettings,
    pub script_count: usize,
    pub assertion_count: usize,
    pub docs: Option<String>,
}

/// Headers, auth and variables declared under `request:` in
/// `opencollection.yml` or a `folder.yml`, inherited by the requests below.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestDefaults {
    pub headers: Vec<KeyValue>,
    pub auth: Option<Auth>,
    pub variables: Vec<KeyValue>,
}

fn parse_yaml(content: &str) -> Result<Value> {
    serde_saphyr::from_str::<Value>(content).map_err(|err| anyhow!("YAML invalido: {err}"))
}

pub fn parse_request(content: &str) -> Result<HttpRequestSpec> {
    let doc = parse_yaml(content)?;
    if !doc.is_object() {
        bail!("El archivo no describe un request HTTP");
    }

    if let Some(kind) = str_field(&doc["info"], "type") {
        if kind != "http" {
            bail!("Tipo de request no soportado todavia: {kind}");
        }
    } else if doc.get("http").is_none() {
        bail!("El archivo no describe un request HTTP (falta la seccion http)");
    }

    let http = &doc["http"];
    let method = str_field(http, "method")
        .unwrap_or_else(|| String::from("GET"))
        .to_ascii_uppercase();
    let runtime = &doc["runtime"];

    Ok(HttpRequestSpec {
        name: str_field(&doc["info"], "name").unwrap_or_default(),
        method,
        url: str_field(http, "url").unwrap_or_default(),
        headers: parse_key_values(http.get("headers")),
        params: parse_params(http.get("params")),
        body: parse_body(http.get("body")),
        auth: http.get("auth").map(parse_auth).unwrap_or(Auth::None),
        settings: parse_settings(&doc["settings"]),
        script_count: runtime["scripts"].as_array().map(Vec::len).unwrap_or(0),
        assertion_count: runtime["assertions"].as_array().map(Vec::len).unwrap_or(0),
        docs: doc.get("docs").and_then(docs_text),
    })
}

pub fn parse_request_defaults(content: &str) -> Result<RequestDefaults> {
    let doc = parse_yaml(content)?;
    let request = &doc["request"];
    Ok(RequestDefaults {
        headers: parse_key_values(request.get("headers")),
        auth: request.get("auth").map(parse_auth),
        variables: parse_variables(request.get("variables")),
    })
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).filter(|v| !v.is_null()).map(value_to_string)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

fn is_disabled(value: &Value) -> bool {
    value.get("disabled").and_then(Value::as_bool) == Some(true)
}

fn parse_key_values(value: Option<&Value>) -> Vec<KeyValue> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(KeyValue {
                        name: str_field(entry, "name")?,
                        value: str_field(entry, "value").unwrap_or_default(),
                        enabled: !is_disabled(entry),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Accepts both the spec shape (`[{name, value, type}]`) and the split shape
/// shown in Bruno's docs (`{query: [...], path: [...]}`).
fn parse_params(value: Option<&Value>) -> Vec<Param> {
    let Some(value) = value else {
        return Vec::new();
    };

    if let Some(entries) = value.as_array() {
        return entries
            .iter()
            .filter_map(|entry| {
                let kind = match str_field(entry, "type").as_deref() {
                    Some("path") => ParamKind::Path,
                    _ => ParamKind::Query,
                };
                Some(Param {
                    entry: KeyValue {
                        name: str_field(entry, "name")?,
                        value: str_field(entry, "value").unwrap_or_default(),
                        enabled: !is_disabled(entry),
                    },
                    kind,
                })
            })
            .collect();
    }

    let mut params = Vec::new();
    for (key, kind) in [("query", ParamKind::Query), ("path", ParamKind::Path)] {
        params.extend(
            parse_key_values(value.get(key))
                .into_iter()
                .map(|entry| Param { entry, kind }),
        );
    }
    params
}

fn parse_body(value: Option<&Value>) -> Body {
    let Some(value) = value else {
        return Body::None;
    };

    // Body variants: use the selected one, or the first.
    let value = match value.as_array() {
        Some(variants) => {
            let Some(variant) = variants
                .iter()
                .find(|variant| variant.get("selected").and_then(Value::as_bool) == Some(true))
                .or_else(|| variants.first())
            else {
                return Body::None;
            };
            &variant["body"]
        }
        None => value,
    };

    match str_field(value, "type").as_deref() {
        None | Some("none") => Body::None,
        Some(kind @ ("json" | "text" | "xml" | "sparql")) => Body::Raw {
            kind: kind.to_string(),
            data: str_field(value, "data").unwrap_or_default(),
        },
        Some("form-urlencoded") => Body::Form(parse_key_values(value.get("data"))),
        Some(other) => Body::Unsupported(other.to_string()),
    }
}

/// Reads credentials from the object itself (the spec) and falls back to a
/// nested block named after the type (an older shape still found in docs).
fn parse_auth(value: &Value) -> Auth {
    if let Some(mode) = value.as_str() {
        return match mode {
            "inherit" => Auth::Inherit,
            "none" | "" => Auth::None,
            other => Auth::Unsupported(other.to_string()),
        };
    }

    let Some(kind) = str_field(value, "type") else {
        return Auth::None;
    };
    let field = |name: &str| {
        str_field(value, name)
            .or_else(|| str_field(&value[kind.as_str()], name))
            .unwrap_or_default()
    };

    match kind.as_str() {
        "none" => Auth::None,
        "inherit" => Auth::Inherit,
        "bearer" => Auth::Bearer {
            token: field("token"),
        },
        "basic" => Auth::Basic {
            username: field("username"),
            password: field("password"),
        },
        "apikey" => Auth::ApiKey {
            key: field("key"),
            value: field("value"),
            in_query: field("placement") == "query",
        },
        other => Auth::Unsupported(other.to_string()),
    }
}

fn parse_settings(value: &Value) -> RequestSettings {
    let defaults = RequestSettings::default();
    RequestSettings {
        timeout: value
            .get("timeout")
            .and_then(Value::as_u64)
            .filter(|millis| *millis > 0)
            .map(Duration::from_millis),
        follow_redirects: value
            .get("followRedirects")
            .and_then(Value::as_bool)
            .unwrap_or(defaults.follow_redirects),
        max_redirects: value
            .get("maxRedirects")
            .and_then(Value::as_u64)
            .map(|max| max as usize)
            .unwrap_or(defaults.max_redirects),
    }
}

fn docs_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.clone(),
        other => str_field(other, "content")?,
    };
    (!text.trim().is_empty()).then_some(text)
}

/// Variable values can be a plain string, a typed `{type, data}` value or a
/// list of variants (the selected one wins).
fn variable_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Array(variants) => {
            let variant = variants
                .iter()
                .find(|variant| variant.get("selected").and_then(Value::as_bool) == Some(true))
                .or_else(|| variants.first())?;
            variable_value(&variant["value"])
        }
        Value::Object(_) => value.get("data").map(value_to_string),
        other => Some(value_to_string(other)),
    }
}

fn parse_variables(value: Option<&Value>) -> Vec<KeyValue> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(KeyValue {
                        name: str_field(entry, "name")?,
                        value: variable_value(&entry["value"]).unwrap_or_default(),
                        enabled: !is_disabled(entry),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── Environments and secrets ─────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvVariable {
    pub name: String,
    /// `None` for secret variables: their value never lives in the file.
    pub value: Option<String>,
    pub secret: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    pub name: String,
    pub path: PathBuf,
    pub variables: Vec<EnvVariable>,
}

pub fn parse_environment(content: &str, path: &Path) -> Result<Environment> {
    let doc = parse_yaml(content)?;
    let fallback_name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();

    let variables = doc["variables"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let secret = entry.get("secret").and_then(Value::as_bool) == Some(true);
                    Some(EnvVariable {
                        name: str_field(entry, "name")?,
                        value: if secret {
                            None
                        } else {
                            Some(variable_value(&entry["value"]).unwrap_or_default())
                        },
                        secret,
                        enabled: !is_disabled(entry),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Environment {
        name: str_field(&doc, "name").unwrap_or(fallback_name),
        path: path.to_path_buf(),
        variables,
    })
}

/// Loads `environments/*.yml`, sorted by name. Files that fail to parse are
/// reported back instead of silently disappearing from the picker.
pub fn load_environments(root: &Path) -> (Vec<Environment>, Vec<String>) {
    let mut environments = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = fs::read_dir(root.join(ENVIRONMENTS_DIR)) else {
        return (environments, errors);
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_yaml_file(&path) {
            continue;
        }
        match fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|content| parse_environment(&content, &path))
        {
            Ok(environment) => environments.push(environment),
            Err(err) => errors.push(format!("{}: {err}", display_file_name(&path))),
        }
    }

    environments.sort_by(|left, right| left.name.cmp(&right.name));
    (environments, errors)
}

/// Minimal `.env` reader: `KEY=value`, optional `export`, quotes and comments.
pub fn parse_dotenv(content: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }

        let raw_value = raw_value.trim();
        let value = if raw_value.len() >= 2
            && raw_value.starts_with('"')
            && raw_value.ends_with('"')
        {
            raw_value[1..raw_value.len() - 1]
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
        } else if raw_value.len() >= 2 && raw_value.starts_with('\'') && raw_value.ends_with('\'') {
            raw_value[1..raw_value.len() - 1].to_string()
        } else {
            raw_value
                .split_once(" #")
                .map(|(value, _)| value)
                .unwrap_or(raw_value)
                .trim_end()
                .to_string()
        };
        values.insert(key.to_string(), value);
    }
    values
}

// ── Variable resolution ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct VariableScope {
    vars: HashMap<String, String>,
    dotenv: HashMap<String, String>,
}

impl VariableScope {
    fn lookup(&self, name: &str) -> Option<String> {
        if let Some(key) = process_env_key(name) {
            return self
                .dotenv
                .get(&key)
                .cloned()
                .or_else(|| env::var(&key).ok());
        }
        self.vars.get(name).cloned()
    }

    /// Replaces every `{{name}}`. Unknown names stay in the text verbatim and
    /// are collected in `unresolved` so the UI can point them out.
    pub fn interpolate(&self, input: &str, unresolved: &mut Vec<String>) -> String {
        self.interpolate_at(input, unresolved, 0)
    }

    fn interpolate_at(&self, input: &str, unresolved: &mut Vec<String>, depth: usize) -> String {
        let mut output = String::with_capacity(input.len());
        let mut rest = input;

        while let Some(start) = rest.find("{{") {
            let Some(end) = rest[start + 2..].find("}}") else {
                break;
            };
            output.push_str(&rest[..start]);
            let token = &rest[start..start + 2 + end + 2];
            let name = rest[start + 2..start + 2 + end].trim();

            match self.lookup(name) {
                Some(value) if depth < MAX_INTERPOLATION_DEPTH => {
                    output.push_str(&self.interpolate_at(&value, unresolved, depth + 1));
                }
                Some(value) => output.push_str(&value),
                None => {
                    if !unresolved.iter().any(|known| known == name) {
                        unresolved.push(name.to_string());
                    }
                    output.push_str(token);
                }
            }
            rest = &rest[start + 2 + end + 2..];
        }

        output.push_str(rest);
        output
    }
}

/// `process.env.NAME` or `process.env['NAME']` → `NAME`.
fn process_env_key(name: &str) -> Option<String> {
    if let Some(key) = name.strip_prefix("process.env.") {
        return Some(key.to_string());
    }
    let key = name.strip_prefix("process.env[")?.strip_suffix(']')?;
    Some(key.trim_matches(|c| c == '\'' || c == '"').to_string())
}

/// Request defaults from the collection root down to the request's folder,
/// outermost first. Each entry carries the label used to explain inheritance.
fn defaults_chain(
    root: &Path,
    request_path: &Path,
) -> (Vec<(String, RequestDefaults)>, Vec<String>) {
    let mut chain = Vec::new();
    let mut warnings = Vec::new();

    let mut push = |file: PathBuf, label: String| {
        let Ok(content) = fs::read_to_string(&file) else {
            return;
        };
        match parse_request_defaults(&content) {
            Ok(defaults) => chain.push((label, defaults)),
            Err(err) => warnings.push(format!("{}: {err}", display_file_name(&file))),
        }
    };

    push(root.join(COLLECTION_FILE), String::from("collection"));

    if let Some(parent) = request_path.parent() {
        if let Ok(relative) = parent.strip_prefix(root) {
            let mut dir = root.to_path_buf();
            for component in relative.components() {
                dir.push(component);
                let label = format!("folder {}", component.as_os_str().to_string_lossy());
                push(dir.join(FOLDER_FILE), label);
            }
        }
    }

    (chain, warnings)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedBody {
    None,
    Text(String),
    Form(Vec<(String, String)>),
}

/// A credential of the applied auth, kept apart so the UI can show it masked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthField {
    pub name: String,
    pub value: String,
    pub secret: bool,
}

/// A request with every variable, default and auth applied: exactly what goes
/// over the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Enabled path params, then the query params appended to the URL, with
    /// their values resolved. An api key sent in the query is under auth.
    pub params: Vec<(ParamKind, String, String)>,
    pub body: ResolvedBody,
    pub settings: RequestSettings,
    pub auth_label: String,
    pub auth_fields: Vec<AuthField>,
    pub unresolved: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn resolve_request(
    root: &Path,
    request_path: &Path,
    spec: &HttpRequestSpec,
    environment: Option<&Environment>,
) -> ResolvedRequest {
    let (chain, mut warnings) = defaults_chain(root, request_path);
    let dotenv = fs::read_to_string(root.join(DOTENV_FILE))
        .map(|content| parse_dotenv(&content))
        .unwrap_or_default();

    // Precedence, lowest first: collection < environment < folders. Secret
    // variables are looked up by name in `.env` and then the process env.
    let mut vars = HashMap::new();
    if let Some((_, collection)) = chain.first() {
        insert_enabled(&mut vars, &collection.variables);
    }
    if let Some(environment) = environment {
        for variable in environment.variables.iter().filter(|v| v.enabled) {
            let value = match &variable.value {
                Some(value) => Some(value.clone()),
                None => dotenv
                    .get(&variable.name)
                    .cloned()
                    .or_else(|| env::var(&variable.name).ok()),
            };
            if let Some(value) = value {
                vars.insert(variable.name.clone(), value);
            }
        }
    }
    for (_, folder) in chain.iter().skip(1) {
        insert_enabled(&mut vars, &folder.variables);
    }

    let scope = VariableScope { vars, dotenv };
    let mut unresolved = Vec::new();

    // Headers: collection, then folders, then the request; later wins.
    let mut headers: Vec<(String, String)> = Vec::new();
    for entry in chain
        .iter()
        .flat_map(|(_, defaults)| defaults.headers.iter())
        .chain(spec.headers.iter())
        .filter(|entry| entry.enabled)
    {
        let name = scope.interpolate(&entry.name, &mut unresolved);
        let value = scope.interpolate(&entry.value, &mut unresolved);
        set_header(&mut headers, name, value);
    }

    // URL: path params, then query params unless the URL already has a query
    // string (Bruno keeps both in sync, so appending would duplicate them).
    let mut url = scope.interpolate(&spec.url, &mut unresolved);
    let path_params: Vec<(String, String)> = spec
        .params
        .iter()
        .filter(|param| param.kind == ParamKind::Path && param.entry.enabled)
        .map(|param| {
            (
                param.entry.name.clone(),
                scope.interpolate(&param.entry.value, &mut unresolved),
            )
        })
        .collect();
    url = apply_path_params(&url, &path_params);
    // Like Bruno, a missing scheme means http — unless the URL starts with a
    // variable that did not resolve, where guessing would only mislead.
    if !url.is_empty() && !url.contains("://") && !url.starts_with("{{") {
        url = format!("http://{url}");
    }
    let mut query: Vec<(String, String)> = if url.contains('?') {
        Vec::new()
    } else {
        spec.params
            .iter()
            .filter(|param| param.kind == ParamKind::Query && param.entry.enabled)
            .map(|param| {
                (
                    scope.interpolate(&param.entry.name, &mut unresolved),
                    scope.interpolate(&param.entry.value, &mut unresolved),
                )
            })
            .collect()
    };
    let params: Vec<(ParamKind, String, String)> = path_params
        .iter()
        .map(|(name, value)| (ParamKind::Path, name.clone(), value.clone()))
        .chain(
            query
                .iter()
                .map(|(name, value)| (ParamKind::Query, name.clone(), value.clone())),
        )
        .collect();

    // Auth: `inherit` walks up folder by folder to the collection.
    let (auth, auth_source) = match &spec.auth {
        Auth::Inherit => chain
            .iter()
            .rev()
            .find_map(|(label, defaults)| match &defaults.auth {
                Some(Auth::Inherit) | None => None,
                Some(auth) => Some((auth.clone(), Some(label.clone()))),
            })
            .unwrap_or((Auth::None, None)),
        other => (other.clone(), None),
    };
    let has_authorization = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
    let field = |name: &str, value: &str, secret: bool| AuthField {
        name: name.to_string(),
        value: value.to_string(),
        secret,
    };
    let mut auth_fields = Vec::new();
    match &auth {
        Auth::Bearer { token } if !has_authorization => {
            let token = scope.interpolate(token, &mut unresolved);
            auth_fields.push(field("token", &token, true));
            set_header(
                &mut headers,
                String::from("Authorization"),
                format!("Bearer {token}"),
            );
        }
        Auth::Basic { username, password } if !has_authorization => {
            let username = scope.interpolate(username, &mut unresolved);
            let password = scope.interpolate(password, &mut unresolved);
            auth_fields.push(field("username", &username, false));
            auth_fields.push(field("password", &password, true));
            let encoded = base64_encode(format!("{username}:{password}").as_bytes());
            set_header(
                &mut headers,
                String::from("Authorization"),
                format!("Basic {encoded}"),
            );
        }
        Auth::ApiKey {
            key,
            value,
            in_query,
        } => {
            let key = scope.interpolate(key, &mut unresolved);
            let value = scope.interpolate(value, &mut unresolved);
            auth_fields.push(field(&key, &value, true));
            if *in_query {
                query.push((key, value));
            } else {
                set_header(&mut headers, key, value);
            }
        }
        Auth::Unsupported(kind) => {
            warnings.push(format!(
                "Auth '{kind}' no esta soportada: se envia sin auth"
            ));
        }
        _ => {}
    }
    let auth_label = match auth_source {
        Some(source) => format!("{} · heredada de {source}", auth.label()),
        None if spec.auth == Auth::Inherit => String::from("inherit · sin auth en los padres"),
        None => auth.label(),
    };

    if !query.is_empty() {
        let encoded = query
            .iter()
            .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let separator = if url.contains('?') { '&' } else { '?' };
        url = format!("{url}{separator}{encoded}");
    }

    let has_content_type = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    let body = match &spec.body {
        Body::None => ResolvedBody::None,
        Body::Raw { kind, data } => {
            if !has_content_type {
                let content_type = match kind.as_str() {
                    "json" => "application/json",
                    "xml" => "application/xml",
                    "sparql" => "application/sparql-query",
                    _ => "text/plain",
                };
                set_header(
                    &mut headers,
                    String::from("Content-Type"),
                    content_type.to_string(),
                );
            }
            ResolvedBody::Text(scope.interpolate(data, &mut unresolved))
        }
        Body::Form(entries) => {
            if !has_content_type {
                set_header(
                    &mut headers,
                    String::from("Content-Type"),
                    String::from("application/x-www-form-urlencoded"),
                );
            }
            ResolvedBody::Form(
                entries
                    .iter()
                    .filter(|entry| entry.enabled)
                    .map(|entry| {
                        (
                            scope.interpolate(&entry.name, &mut unresolved),
                            scope.interpolate(&entry.value, &mut unresolved),
                        )
                    })
                    .collect(),
            )
        }
        Body::Unsupported(kind) => {
            warnings.push(format!(
                "Body '{kind}' no esta soportado: se envia sin body"
            ));
            ResolvedBody::None
        }
    };

    if spec.script_count > 0 {
        warnings.push(format!(
            "{} script(s) del request no se ejecutan en mdnav",
            spec.script_count
        ));
    }
    if spec.assertion_count > 0 {
        warnings.push(format!(
            "{} assertion(s) no se evaluan en mdnav",
            spec.assertion_count
        ));
    }

    ResolvedRequest {
        method: spec.method.clone(),
        url,
        headers,
        params,
        body,
        settings: spec.settings.clone(),
        auth_label,
        auth_fields,
        unresolved,
        warnings,
    }
}

/// True when resolving `spec` warns whatever its folders or environment say:
/// an auth or body mdnav cannot send, scripts or assertions it does not run.
/// Cheap enough for every leaf of the client's tree.
pub fn spec_has_warnings(spec: &HttpRequestSpec) -> bool {
    matches!(spec.auth, Auth::Unsupported(_))
        || matches!(spec.body, Body::Unsupported(_))
        || spec.script_count > 0
        || spec.assertion_count > 0
}

fn insert_enabled(vars: &mut HashMap<String, String>, entries: &[KeyValue]) {
    for entry in entries.iter().filter(|entry| entry.enabled) {
        vars.insert(entry.name.clone(), entry.value.clone());
    }
}

/// Sets a header replacing any previous value, ignoring name case.
fn set_header(headers: &mut Vec<(String, String)>, name: String, value: String) {
    headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(&name));
    headers.push((name, value));
}

/// Replaces `:name` path segments, e.g. `/users/:id` → `/users/42`.
fn apply_path_params(url: &str, params: &[(String, String)]) -> String {
    let mut result = url.to_string();
    for (name, value) in params {
        let token = format!(":{name}");
        let mut output = String::with_capacity(result.len());
        let mut rest = result.as_str();
        while let Some(index) = rest.find(&token) {
            let before = &rest[..index];
            let after = &rest[index + token.len()..];
            let starts_segment = before.ends_with('/');
            let ends_segment = after.is_empty() || after.starts_with(['/', '?', '#']);
            output.push_str(before);
            if starts_segment && ends_segment {
                output.push_str(value);
            } else {
                output.push_str(&token);
            }
            rest = after;
        }
        output.push_str(rest);
        result = output;
    }
    result
}

/// RFC 3986 percent-encoding for query and form components.
pub fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        output.push(TABLE[(triple >> 18) as usize & 63] as char);
        output.push(TABLE[(triple >> 12) as usize & 63] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[triple as usize & 63] as char
        } else {
            '='
        });
    }
    output
}

// ── Execution ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub method: String,
    pub url: String,
    /// `None` when the request never got a response (DNS, refused, timeout).
    pub status: Option<u16>,
    pub reason: String,
    pub elapsed: Duration,
    pub headers: Vec<(String, String)>,
    pub size: usize,
    pub truncated: bool,
    pub content_type: Option<String>,
    /// Pretty-printed when the body is JSON; copied by `y`.
    pub body_text: String,
    /// `body_text` already highlighted, computed off the UI thread.
    pub body_lines: Vec<PreviewLine>,
    pub error: Option<String>,
}

impl HttpResponse {
    fn failed(request: &ResolvedRequest, elapsed: Duration, error: String) -> Self {
        Self {
            method: request.method.clone(),
            url: request.url.clone(),
            status: None,
            reason: String::new(),
            elapsed,
            headers: Vec::new(),
            size: 0,
            truncated: false,
            content_type: None,
            body_text: String::new(),
            body_lines: Vec::new(),
            error: Some(error),
        }
    }
}

/// Sends the request and reads the response. Blocking: always call it from a
/// worker thread (see `spawn_request`), never from the UI loop.
pub fn execute(request: &ResolvedRequest) -> HttpResponse {
    let started = Instant::now();
    let fail = |error: String| HttpResponse::failed(request, started.elapsed(), error);

    let policy = if request.settings.follow_redirects {
        reqwest::redirect::Policy::limited(request.settings.max_redirects)
    } else {
        reqwest::redirect::Policy::none()
    };
    let client = match reqwest::blocking::Client::builder()
        .redirect(policy)
        .timeout(request.settings.timeout)
        .user_agent(concat!("mdnav/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(err) => return fail(format!("No se pudo crear el cliente HTTP: {err}")),
    };
    let method = match reqwest::Method::from_bytes(request.method.as_bytes()) {
        Ok(method) => method,
        Err(_) => return fail(format!("Metodo HTTP invalido: {}", request.method)),
    };

    let mut builder = client.request(method, &request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder = match &request.body {
        ResolvedBody::None => builder,
        ResolvedBody::Text(text) => builder.body(text.clone()),
        ResolvedBody::Form(entries) => builder.body(
            entries
                .iter()
                .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
                .collect::<Vec<_>>()
                .join("&"),
        ),
    };

    let response = match builder.send() {
        Ok(response) => response,
        Err(err) => return fail(describe_reqwest_error(&err)),
    };

    let status = response.status();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let content_length = response.content_length();
    let final_url = response.url().to_string();

    let mut bytes = Vec::new();
    let read_result = response
        .take(RESPONSE_BODY_LIMIT as u64 + 1)
        .read_to_end(&mut bytes);
    let elapsed = started.elapsed();
    if let Err(err) = read_result {
        let mut failed = fail(format!("Error leyendo la response: {err}"));
        failed.status = Some(status.as_u16());
        return failed;
    }

    let truncated = bytes.len() > RESPONSE_BODY_LIMIT;
    bytes.truncate(RESPONSE_BODY_LIMIT);
    let size = if truncated {
        content_length
            .map(|len| len as usize)
            .unwrap_or(bytes.len())
    } else {
        bytes.len()
    };
    let (body_text, body_lines) = render_body(&bytes, content_type.as_deref());

    HttpResponse {
        method: request.method.clone(),
        url: final_url,
        status: Some(status.as_u16()),
        reason: status.canonical_reason().unwrap_or("").to_string(),
        elapsed,
        headers,
        size,
        truncated,
        content_type,
        body_text,
        body_lines,
        error: None,
    }
}

fn describe_reqwest_error(err: &reqwest::Error) -> String {
    let kind = if err.is_timeout() {
        "Timeout"
    } else if err.is_connect() {
        "No se pudo conectar"
    } else if err.is_builder() {
        "Request invalido"
    } else if err.is_redirect() {
        "Demasiados redirects"
    } else {
        "Error HTTP"
    };
    // reqwest nests the useful detail (DNS, refused, TLS, ...) in sources.
    let mut detail = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(inner) = source {
        detail = format!("{detail}: {inner}");
        source = inner.source();
    }
    format!("{kind}: {detail}")
}

fn plain_line(text: &str, color: [u8; 3]) -> PreviewLine {
    PreviewLine {
        text: text.to_string(),
        kind: PreviewLineKind::Normal,
        highlights: vec![(text.to_string(), color)],
    }
}

fn render_body(bytes: &[u8], content_type: Option<&str>) -> (String, Vec<PreviewLine>) {
    if bytes.is_empty() {
        return (String::new(), Vec::new());
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        let notice = format!("<contenido binario, {}>", format_size(bytes.len()));
        return (String::new(), vec![plain_line(&notice, GRAY)]);
    };

    let content_type = content_type.unwrap_or("").to_ascii_lowercase();
    let trimmed = text.trim_start();
    let looks_like_json = trimmed.starts_with('{') || trimmed.starts_with('[');
    let (body_text, language) = if content_type.contains("json") || looks_like_json {
        match serde_json::from_str::<Value>(text) {
            Ok(value) => (
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string()),
                "json",
            ),
            Err(_) => (text.to_string(), "txt"),
        }
    } else if content_type.contains("html") {
        (text.to_string(), "html")
    } else if content_type.contains("xml") {
        (text.to_string(), "xml")
    } else {
        (text.to_string(), "txt")
    };

    let raw_lines: Vec<String> = body_text.lines().map(str::to_string).collect();
    let split = raw_lines.len().min(HIGHLIGHT_LINE_LIMIT);
    let mut lines: Vec<PreviewLine> = highlight_code_block(&raw_lines[..split], language)
        .into_iter()
        .map(|(text, highlights)| PreviewLine {
            text,
            kind: PreviewLineKind::Normal,
            highlights,
        })
        .collect();
    lines.extend(raw_lines[split..].iter().map(|line| PreviewLine {
        text: line.clone(),
        kind: PreviewLineKind::Normal,
        highlights: Vec::new(),
    }));
    (body_text, lines)
}

/// A request running on a worker thread. Dropping it abandons the request:
/// the thread finishes on its own and its result is discarded.
pub struct InFlight {
    pub request_path: PathBuf,
    pub label: String,
    pub started: Instant,
    receiver: Receiver<HttpResponse>,
}

impl InFlight {
    /// Non-blocking: `Some` once the worker is done.
    pub fn try_finish(&self) -> Option<HttpResponse> {
        match self.receiver.try_recv() {
            Ok(response) => Some(response),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(HttpResponse {
                method: String::new(),
                url: String::new(),
                status: None,
                reason: String::new(),
                elapsed: self.started.elapsed(),
                headers: Vec::new(),
                size: 0,
                truncated: false,
                content_type: None,
                body_text: String::new(),
                body_lines: Vec::new(),
                error: Some(String::from("El worker HTTP termino sin respuesta")),
            }),
        }
    }
}

pub fn spawn_request(request_path: PathBuf, request: ResolvedRequest) -> InFlight {
    let (sender, receiver) = mpsc::channel();
    let label = format!("{} {}", request.method, request.url);
    thread::spawn(move || {
        let _ = sender.send(execute(&request));
    });
    InFlight {
        request_path,
        label,
        started: Instant::now(),
        receiver,
    }
}

pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn format_elapsed(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 1000 {
        format!("{millis} ms")
    } else {
        format!("{:.2} s", elapsed.as_secs_f64())
    }
}

fn display_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

// ── Writing collections ──────────────────────────────────────────────────────
//
// Output mirrors the field order Bruno writes, so files created by mdnav look
// native when the same folder is opened in Bruno.

#[derive(Serialize)]
pub(crate) struct OcInfoOut {
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<usize>,
}

#[derive(Serialize)]
pub(crate) struct OcKeyValueOut {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Serialize)]
pub(crate) struct OcParamOut {
    pub name: String,
    pub value: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Serialize)]
pub(crate) struct OcMultipartOut {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub value: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum OcBodyOut {
    Raw {
        #[serde(rename = "type")]
        kind: &'static str,
        data: String,
    },
    Form {
        #[serde(rename = "type")]
        kind: &'static str,
        data: Vec<OcKeyValueOut>,
    },
    Multipart {
        #[serde(rename = "type")]
        kind: &'static str,
        data: Vec<OcMultipartOut>,
    },
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum OcAuthOut {
    /// `inherit`.
    Mode(&'static str),
    Bearer {
        #[serde(rename = "type")]
        kind: &'static str,
        token: String,
    },
    Basic {
        #[serde(rename = "type")]
        kind: &'static str,
        username: String,
        password: String,
    },
    ApiKey {
        #[serde(rename = "type")]
        kind: &'static str,
        key: String,
        value: String,
        placement: &'static str,
    },
}

#[derive(Serialize)]
pub(crate) struct OcHttpOut {
    pub method: String,
    pub url: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<OcParamOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<OcKeyValueOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<OcBodyOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<OcAuthOut>,
}

#[derive(Serialize)]
pub(crate) struct OcSettingsOut {
    #[serde(rename = "encodeUrl")]
    pub encode_url: bool,
}

#[derive(Serialize)]
pub(crate) struct OcRequestFileOut {
    pub info: OcInfoOut,
    pub http: OcHttpOut,
    pub settings: OcSettingsOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct OcVariableOut {
    pub name: String,
    pub value: String,
}

#[derive(Default, Serialize)]
pub(crate) struct OcDefaultsOut {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<OcKeyValueOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<OcAuthOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<OcVariableOut>,
}

impl OcDefaultsOut {
    /// `None` when there is nothing to write, so the `request:` key is omitted.
    pub fn non_empty(self) -> Option<Self> {
        (!self.headers.is_empty() || self.auth.is_some() || !self.variables.is_empty())
            .then_some(self)
    }
}

#[derive(Serialize)]
pub(crate) struct OcCollectionFileOut {
    pub opencollection: &'static str,
    pub info: OcInfoOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<OcDefaultsOut>,
}

#[derive(Serialize)]
pub(crate) struct OcFolderFileOut {
    pub info: OcInfoOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<OcDefaultsOut>,
}

#[derive(Serialize)]
pub(crate) struct OcEnvVariableOut {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Serialize)]
pub(crate) struct OcEnvironmentFileOut {
    pub name: String,
    pub variables: Vec<OcEnvVariableOut>,
}

pub(crate) fn to_yaml<T: Serialize>(value: &T) -> Result<String> {
    serde_saphyr::to_string(value).map_err(|err| anyhow!("No se pudo generar el YAML: {err}"))
}

/// Writes a brand-new file. Never overwrites: an existing path is an error.
pub(crate) fn write_new(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("No se pudo crear {}", path.display()))?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("No se pudo escribir {}", path.display()))
}

/// Makes sure `dir/.gitignore` ignores `.env`, creating the file if needed.
pub(crate) fn ensure_dotenv_ignored(dir: &Path) -> Result<()> {
    let path = dir.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|line| line.trim() == DOTENV_FILE) {
        return Ok(());
    }
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    fs::write(&path, format!("{existing}{separator}{DOTENV_FILE}\n"))
        .with_context(|| format!("No se pudo actualizar {}", path.display()))
}

/// Appends `KEY="value"` lines to the collection `.env`, creating it if needed.
pub(crate) fn append_dotenv(root: &Path, entries: &[(String, String)]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let path = root.join(DOTENV_FILE);
    let mut content = fs::read_to_string(&path).unwrap_or_default();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    for (key, value) in entries {
        let escaped = value.replace('"', "\\\"").replace('\n', "\\n");
        content.push_str(&format!("{key}=\"{escaped}\"\n"));
    }
    fs::write(&path, content).with_context(|| format!("No se pudo escribir {}", path.display()))?;
    ensure_dotenv_ignored(root)
}

/// Turns a request or folder name into a file name that is valid everywhere
/// (Windows being the strictest).
pub(crate) fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.chars().count() > 80 {
        cleaned = cleaned
            .chars()
            .take(80)
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    if cleaned.is_empty() {
        return String::from("untitled");
    }
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        cleaned.push_str("-item");
    }
    cleaned
}

/// Picks `base{ext}`, or `base-2{ext}`, `base-3{ext}`... when taken. `used`
/// compares case-insensitively because Windows and macOS file systems do.
pub(crate) fn unique_name(used: &mut HashSet<String>, base: &str, ext: &str) -> String {
    let mut candidate = format!("{base}{ext}");
    let mut counter = 2;
    while used.contains(&candidate.to_lowercase()) {
        candidate = format!("{base}-{counter}{ext}");
        counter += 1;
    }
    used.insert(candidate.to_lowercase());
    candidate
}

/// Turns the selected folder into a collection: `opencollection.yml`, a
/// `local` environment that reads its token from `.env`, a `.env.sample`, a
/// `.gitignore` entry for `.env` and one example request.
pub fn init_collection(dir: &Path) -> Result<Vec<PathBuf>> {
    if let Some(existing) = find_collection_root(dir) {
        bail!(
            "Esta carpeta ya esta dentro de la collection {}",
            existing.display()
        );
    }

    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| String::from("collection"));
    let mut created = Vec::new();

    let collection = OcCollectionFileOut {
        opencollection: "1.0.0",
        info: OcInfoOut {
            name,
            kind: None,
            seq: None,
        },
        request: None,
    };
    let collection_path = dir.join(COLLECTION_FILE);
    write_new(&collection_path, &to_yaml(&collection)?)?;
    created.push(collection_path);

    let environments = dir.join(ENVIRONMENTS_DIR);
    fs::create_dir_all(&environments)
        .with_context(|| format!("No se pudo crear {}", environments.display()))?;
    let local = OcEnvironmentFileOut {
        name: String::from("local"),
        variables: vec![
            OcEnvVariableOut {
                name: String::from("base_url"),
                value: String::from("http://localhost:3000"),
                disabled: false,
            },
            OcEnvVariableOut {
                name: String::from("token"),
                value: String::from("{{process.env.API_TOKEN}}"),
                disabled: false,
            },
        ],
    };
    let local_path = environments.join("local.yml");
    write_new(&local_path, &to_yaml(&local)?)?;
    created.push(local_path);

    let sample_path = dir.join(".env.sample");
    if !sample_path.exists() {
        write_new(
            &sample_path,
            "# Copy to .env (gitignored) and fill in the secrets.\nAPI_TOKEN=\n",
        )?;
        created.push(sample_path);
    }
    ensure_dotenv_ignored(dir)?;

    let example = request_template("Health check", 1, "{{base_url}}/health");
    let example_path = dir.join("health.yml");
    write_new(&example_path, &to_yaml(&example)?)?;
    created.push(example_path);

    Ok(created)
}

/// Creates `<dir>/<name>.yml` with a GET template and returns its path.
pub fn create_request(dir: &Path, name: &str) -> Result<PathBuf> {
    let name = name.trim().trim_end_matches(".yml").trim();
    if name.is_empty() {
        bail!("El request necesita un nombre");
    }
    let path = dir.join(format!("{}.yml", sanitize_file_name(name)));
    if path.exists() {
        bail!("Ya existe {}", path.display());
    }

    let seq = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| request_collection_root(&entry.path()).is_some())
                .count()
        })
        .unwrap_or(0)
        + 1;
    let request = request_template(name, seq, "{{base_url}}/");
    write_new(&path, &to_yaml(&request)?)?;
    Ok(path)
}

fn request_template(name: &str, seq: usize, url: &str) -> OcRequestFileOut {
    OcRequestFileOut {
        info: OcInfoOut {
            name: name.to_string(),
            kind: Some("http"),
            seq: Some(seq),
        },
        http: OcHttpOut {
            method: String::from("GET"),
            url: url.to_string(),
            params: Vec::new(),
            headers: Vec::new(),
            body: None,
            auth: Some(OcAuthOut::Mode("inherit")),
        },
        settings: OcSettingsOut { encode_url: true },
        docs: None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        net::TcpListener,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("mdnav-http-{label}-{}-{id}", std::process::id()));
            fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, relative: &str, content: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const CREATE_USER: &str = r#"
info:
  name: Create User
  type: http
  seq: 5
http:
  method: post
  url: "{{base_url}}/users/:org"
  params:
    - name: org
      value: "{{org}}"
      type: path
    - name: dry
      value: "true"
      type: query
    - name: skipped
      value: "1"
      type: query
      disabled: true
  headers:
    - name: X-Trace
      value: abc
    - name: X-Off
      value: nope
      disabled: true
  body:
    type: json
    data: |-
      {"name": "{{user}}"}
  auth: inherit
runtime:
  scripts:
    - type: tests
      code: test("ok", function() {});
settings:
  timeout: 1500
  followRedirects: false
docs: Creates a user.
"#;

    #[test]
    fn parses_an_opencollection_request() {
        let spec = parse_request(CREATE_USER).unwrap();

        assert_eq!(spec.name, "Create User");
        assert_eq!(spec.method, "POST", "methods are normalized to uppercase");
        assert_eq!(spec.url, "{{base_url}}/users/:org");
        assert_eq!(spec.params.len(), 3);
        assert_eq!(spec.params[0].kind, ParamKind::Path);
        assert!(!spec.params[2].entry.enabled);
        assert!(!spec.headers[1].enabled);
        assert_eq!(
            spec.body,
            Body::Raw {
                kind: String::from("json"),
                data: String::from(r#"{"name": "{{user}}"}"#),
            }
        );
        assert_eq!(spec.auth, Auth::Inherit);
        assert_eq!(spec.settings.timeout, Some(Duration::from_millis(1500)));
        assert!(!spec.settings.follow_redirects);
        assert_eq!(spec.script_count, 1);
        assert_eq!(spec.docs.as_deref(), Some("Creates a user."));
    }

    #[test]
    fn rejects_files_that_are_not_http_requests() {
        assert!(parse_request("info:\n  name: Q\n  type: graphql\n").is_err());
        assert!(parse_request("name: just some yaml\n").is_err());
        assert!(parse_request("::: not yaml").is_err());
    }

    #[test]
    fn reads_every_auth_shape() {
        let auth = |yaml: &str| parse_auth(&parse_yaml(yaml).unwrap());

        assert_eq!(auth("inherit"), Auth::Inherit);
        assert_eq!(auth("type: none"), Auth::None);
        assert_eq!(
            auth("type: bearer\ntoken: t"),
            Auth::Bearer {
                token: String::from("t")
            }
        );
        // Older nested shape still found in Bruno's docs.
        assert_eq!(
            auth("type: basic\nbasic:\n  username: u\n  password: p"),
            Auth::Basic {
                username: String::from("u"),
                password: String::from("p"),
            }
        );
        assert_eq!(
            auth("type: apikey\nkey: k\nvalue: v\nplacement: query"),
            Auth::ApiKey {
                key: String::from("k"),
                value: String::from("v"),
                in_query: true,
            }
        );
        assert_eq!(
            auth("type: oauth2"),
            Auth::Unsupported(String::from("oauth2"))
        );
    }

    #[test]
    fn reads_params_split_by_kind() {
        let doc =
            parse_yaml("query:\n  - name: q\n    value: x\npath:\n  - name: id\n    value: 1\n")
                .unwrap();
        let params = parse_params(Some(&doc));

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].kind, ParamKind::Query);
        assert_eq!(params[1].kind, ParamKind::Path);
        assert_eq!(params[1].entry.value, "1", "numbers are read as text");
    }

    #[test]
    fn interpolates_nested_env_and_unknown_variables() {
        let scope = VariableScope {
            vars: HashMap::from([
                (String::from("host"), String::from("api.{{domain}}")),
                (String::from("domain"), String::from("example.com")),
                (String::from("loop"), String::from("{{loop}}")),
            ]),
            dotenv: HashMap::from([(String::from("TOKEN"), String::from("s3cret"))]),
        };
        let mut unresolved = Vec::new();

        assert_eq!(
            scope.interpolate("https://{{ host }}/x", &mut unresolved),
            "https://api.example.com/x"
        );
        assert_eq!(
            scope.interpolate(
                "{{process.env.TOKEN}}|{{process.env['TOKEN']}}",
                &mut unresolved
            ),
            "s3cret|s3cret"
        );
        assert_eq!(
            scope.interpolate("{{missing}} and {{missing}}", &mut unresolved),
            "{{missing}} and {{missing}}"
        );
        assert_eq!(unresolved, vec![String::from("missing")]);

        // A self-referencing variable stops instead of recursing forever.
        assert!(scope
            .interpolate("{{loop}}", &mut unresolved)
            .contains("{{loop}}"));
        assert_eq!(
            scope.interpolate("{{unclosed", &mut unresolved),
            "{{unclosed"
        );
    }

    #[test]
    fn parses_dotenv_files() {
        let values = parse_dotenv(
            "# comment\nexport A=1\nB=\"two words\"\nC='single'\nD=plain # trailing\nE=\"line\\nbreak\"\n=skip\nnoequals\n",
        );

        assert_eq!(values["A"], "1");
        assert_eq!(values["B"], "two words");
        assert_eq!(values["C"], "single");
        assert_eq!(values["D"], "plain");
        assert_eq!(values["E"], "line\nbreak");
        assert_eq!(values.len(), 5);
    }

    /// A collection with defaults at every level, a secret in `.env` and the
    /// `CREATE_USER` request two folders down.
    fn sample_collection() -> (TestDir, PathBuf) {
        let temp = TestDir::new("collection");
        temp.write(
            COLLECTION_FILE,
            "opencollection: 1.0.0\ninfo:\n  name: Sample\nrequest:\n  headers:\n    - name: X-Trace\n      value: from-collection\n    - name: Accept\n      value: application/json\n  auth:\n    type: bearer\n    token: \"{{token}}\"\n  variables:\n    - name: org\n      value: collection-org\n    - name: user\n      value: ana\n",
        );
        temp.write(
            "users/folder.yml",
            "info:\n  name: Users\n  type: folder\nrequest:\n  variables:\n    - name: org\n      value: folder-org\n",
        );
        temp.write(
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: base_url\n    value: http://localhost:9\n  - name: org\n    value: env-org\n  - secret: true\n    name: token\n",
        );
        temp.write(".env", "token=from-dotenv\n");
        let request = temp.write("users/admins/create-user.yml", CREATE_USER);
        (temp, request)
    }

    #[test]
    fn resolves_variables_defaults_and_inherited_auth() {
        let (temp, request) = sample_collection();
        let (environments, errors) = load_environments(temp.path());
        assert!(errors.is_empty());
        assert_eq!(environments[0].name, "dev");
        assert_eq!(
            environments[0].variables[2].value, None,
            "secrets have no value"
        );

        let spec = parse_request(CREATE_USER).unwrap();
        let resolved = resolve_request(temp.path(), &request, &spec, environments.first());

        // Folder variables beat the environment, which beats the collection.
        assert_eq!(resolved.url, "http://localhost:9/users/folder-org?dry=true");
        assert_eq!(resolved.method, "POST");
        let header = |name: &str| {
            resolved
                .headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(
            header("X-Trace"),
            Some("abc"),
            "the request overrides defaults"
        );
        assert_eq!(header("Accept"), Some("application/json"));
        assert_eq!(header("X-Off"), None, "disabled headers are not sent");
        assert_eq!(header("Authorization"), Some("Bearer from-dotenv"));
        assert_eq!(header("Content-Type"), Some("application/json"));
        assert_eq!(
            resolved.body,
            ResolvedBody::Text(String::from(r#"{"name": "ana"}"#))
        );
        assert_eq!(resolved.auth_label, "bearer · heredada de collection");
        assert_eq!(
            resolved.auth_fields,
            [AuthField {
                name: String::from("token"),
                value: String::from("from-dotenv"),
                secret: true,
            }]
        );
        assert_eq!(
            resolved.params,
            [
                (
                    ParamKind::Path,
                    String::from("org"),
                    String::from("folder-org")
                ),
                (ParamKind::Query, String::from("dry"), String::from("true")),
            ],
            "only enabled params, resolved"
        );
        assert!(resolved.unresolved.is_empty(), "{:?}", resolved.unresolved);
        assert!(resolved
            .warnings
            .iter()
            .any(|warning| warning.contains("script")));
        assert!(spec_has_warnings(&spec), "the scripts show up in the tree");
    }

    #[test]
    fn spec_warnings_flag_what_mdnav_cannot_send_or_run() {
        let plain =
            parse_request("info:\n  type: http\nhttp:\n  method: GET\n  url: https://api.test\n")
                .unwrap();
        assert!(!spec_has_warnings(&plain));
        assert!(spec_has_warnings(&HttpRequestSpec {
            body: Body::Unsupported(String::from("multipart")),
            ..plain.clone()
        }));
        assert!(spec_has_warnings(&HttpRequestSpec {
            auth: Auth::Unsupported(String::from("oauth2")),
            ..plain.clone()
        }));
        assert!(spec_has_warnings(&HttpRequestSpec {
            assertion_count: 1,
            ..plain
        }));
    }

    #[test]
    fn reports_unresolved_variables_without_an_environment() {
        let (temp, request) = sample_collection();
        let spec = parse_request(CREATE_USER).unwrap();
        let resolved = resolve_request(temp.path(), &request, &spec, None);

        assert!(resolved.unresolved.contains(&String::from("base_url")));
        assert!(resolved.unresolved.contains(&String::from("token")));
        // No scheme is guessed in front of an unresolved variable.
        assert_eq!(resolved.url, "{{base_url}}/users/folder-org?dry=true");
    }

    #[test]
    fn keeps_the_url_query_instead_of_duplicating_params() {
        let temp = TestDir::new("query");
        temp.write(COLLECTION_FILE, "opencollection: 1.0.0\n");
        let request = temp.write(
            "search.yml",
            "info:\n  type: http\nhttp:\n  method: GET\n  url: https://api.test/s?q=a b\n  params:\n    - name: q\n      value: a b\n      type: query\n  auth:\n    type: apikey\n    key: api_key\n    value: k&1\n    placement: query\n",
        );
        let spec = parse_request(&fs::read_to_string(&request).unwrap()).unwrap();
        let resolved = resolve_request(temp.path(), &request, &spec, None);

        assert_eq!(resolved.url, "https://api.test/s?q=a b&api_key=k%261");
    }

    #[test]
    fn replaces_only_whole_path_segments() {
        let params = [(String::from("id"), String::from("42"))];
        assert_eq!(
            apply_path_params("http://h/users/:id/posts/:idx?x=:id", &params),
            "http://h/users/42/posts/:idx?x=:id"
        );
        assert_eq!(apply_path_params("http://h/:id", &params), "http://h/42");
    }

    #[test]
    fn encodes_query_components_and_basic_credentials() {
        assert_eq!(percent_encode("a b&c=d/é~"), "a%20b%26c%3Dd%2F%C3%A9~");
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
    }

    #[test]
    fn detects_request_files_inside_collections_only() {
        let (temp, request) = sample_collection();
        let outside = TestDir::new("outside");
        let stray = outside.write("stray.yml", "http:\n  url: x\n");

        assert_eq!(
            request_collection_root(&request),
            Some(temp.path().to_path_buf())
        );
        assert_eq!(
            request_collection_root(&temp.path().join(COLLECTION_FILE)),
            None
        );
        assert_eq!(
            request_collection_root(&temp.path().join("users/folder.yml")),
            None
        );
        assert_eq!(
            request_collection_root(&temp.path().join("environments/dev.yml")),
            None
        );
        assert_eq!(request_collection_root(&stray), None);
    }

    #[test]
    fn sanitizes_and_deduplicates_file_names() {
        assert_eq!(sanitize_file_name("Get user: by id?"), "Get user- by id-");
        assert_eq!(sanitize_file_name("  ..  "), "untitled");
        assert_eq!(sanitize_file_name("con"), "con-item");
        assert_eq!(sanitize_file_name("COM1.yml"), "COM1.yml-item");
        assert_eq!(sanitize_file_name("console"), "console");

        let mut used = HashSet::new();
        assert_eq!(unique_name(&mut used, "list", ".yml"), "list.yml");
        assert_eq!(unique_name(&mut used, "List", ".yml"), "List-2.yml");
        assert_eq!(unique_name(&mut used, "list", ".yml"), "list-3.yml");
    }

    #[test]
    fn init_writes_a_collection_that_reads_back() {
        let temp = TestDir::new("init");
        let dir = temp.path().join("api");
        fs::create_dir(&dir).unwrap();

        let created = init_collection(&dir).unwrap();
        assert_eq!(created.len(), 4);
        assert!(fs::read_to_string(dir.join(".gitignore"))
            .unwrap()
            .lines()
            .any(|line| line == ".env"));

        let (environments, errors) = load_environments(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(environments[0].name, "local");
        assert_eq!(
            environments[0].variables[1].value.as_deref(),
            Some("{{process.env.API_TOKEN}}")
        );

        let health = dir.join("health.yml");
        let spec = parse_request(&fs::read_to_string(&health).unwrap()).unwrap();
        assert_eq!(spec.url, "{{base_url}}/health");
        assert_eq!(spec.auth, Auth::Inherit);
        let resolved = resolve_request(&dir, &health, &spec, environments.first());
        assert_eq!(resolved.url, "http://localhost:3000/health");

        assert!(init_collection(&dir).is_err(), "init never runs twice");
        assert!(
            init_collection(&dir.join("environments")).is_err(),
            "nor inside an existing collection"
        );
    }

    #[test]
    fn create_request_writes_a_numbered_template() {
        let temp = TestDir::new("create");
        temp.write(COLLECTION_FILE, "opencollection: 1.0.0\n");
        temp.write("first.yml", "info:\n  type: http\nhttp:\n  url: x\n");

        let path = create_request(temp.path(), "List users").unwrap();
        let spec = parse_request(&fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(path.file_name().unwrap(), "List users.yml");
        assert_eq!(spec.name, "List users");
        assert_eq!(spec.method, "GET");
        assert!(fs::read_to_string(&path).unwrap().contains("seq: 2"));
        assert!(create_request(temp.path(), "List users").is_err());
        assert!(create_request(temp.path(), "   ").is_err());
    }

    /// Serves exactly one request on a random local port and hands back the
    /// raw request text, so tests see exactly what went over the wire.
    fn one_shot_server(response: &'static str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                raw.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if raw.len() >= end + 4 + length || read == 0 {
                        break;
                    }
                }
                if read == 0 {
                    break;
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            let _ = sender.send(String::from_utf8_lossy(&raw).to_string());
        });
        (address, receiver)
    }

    fn resolved(method: &str, url: String, body: ResolvedBody) -> ResolvedRequest {
        ResolvedRequest {
            method: method.to_string(),
            url,
            headers: vec![(String::from("Authorization"), String::from("Bearer abc"))],
            params: Vec::new(),
            body,
            settings: RequestSettings {
                timeout: Some(Duration::from_secs(10)),
                ..RequestSettings::default()
            },
            auth_label: String::from("bearer"),
            auth_fields: Vec::new(),
            unresolved: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn executes_requests_and_pretty_prints_json() {
        let (address, request_text) = one_shot_server(
            "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{\"ok\":true,\"id\":42}\n",
        );

        let response = execute(&resolved(
            "POST",
            format!("{address}/users?x=1"),
            ResolvedBody::Text(String::from("{\"name\":\"ana\"}")),
        ));
        let sent = request_text.recv_timeout(Duration::from_secs(10)).unwrap();

        assert_eq!(response.error, None);
        assert_eq!(response.status, Some(201));
        assert_eq!(response.reason, "Created");
        assert_eq!(response.size, 20);
        assert_eq!(response.body_text, "{\n  \"ok\": true,\n  \"id\": 42\n}");
        assert_eq!(response.body_lines.len(), 4);
        assert!(response
            .headers
            .iter()
            .any(|(name, value)| name == "content-type" && value == "application/json"));

        assert!(sent.starts_with("POST /users?x=1 HTTP/1.1\r\n"), "{sent}");
        assert!(sent
            .to_ascii_lowercase()
            .contains("authorization: bearer abc"));
        assert!(sent.ends_with("{\"name\":\"ana\"}"), "{sent}");
    }

    #[test]
    fn sends_form_bodies_url_encoded() {
        let (address, request_text) = one_shot_server(
            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );

        let response = execute(&resolved(
            "PUT",
            address,
            ResolvedBody::Form(vec![(String::from("a b"), String::from("1&2"))]),
        ));
        let sent = request_text.recv_timeout(Duration::from_secs(10)).unwrap();

        assert_eq!(response.status, Some(204));
        assert!(response.body_lines.is_empty());
        assert!(sent.ends_with("a%20b=1%262"), "{sent}");
    }

    #[test]
    fn reports_connection_failures_instead_of_panicking() {
        // Bind and drop: the port is very likely closed afterwards.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let response = execute(&resolved(
            "GET",
            format!("http://127.0.0.1:{port}/"),
            ResolvedBody::None,
        ));

        assert_eq!(response.status, None);
        assert!(response.error.unwrap().starts_with("No se pudo conectar"));

        let invalid = execute(&resolved(
            "NOT A METHOD",
            String::from("http://x"),
            ResolvedBody::None,
        ));
        assert!(invalid.error.unwrap().contains("Metodo HTTP invalido"));
    }

    #[test]
    fn worker_thread_delivers_the_response() {
        let (address, _request_text) = one_shot_server(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
        );
        let in_flight = spawn_request(
            PathBuf::from("ping.yml"),
            resolved("GET", address, ResolvedBody::None),
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        let response = loop {
            if let Some(response) = in_flight.try_finish() {
                break response;
            }
            assert!(Instant::now() < deadline, "worker never answered");
            thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(response.status, Some(200));
        assert_eq!(response.body_text, "hello");
        assert!(in_flight.label.starts_with("GET http://127.0.0.1:"));
    }

    #[test]
    fn renders_binary_bodies_as_a_notice() {
        let (text, lines) = render_body(&[0xff, 0xfe, 0x00], Some("application/octet-stream"));
        assert!(text.is_empty());
        assert!(lines[0].text.contains("binario"));

        let (text, _) = render_body(b"{broken", Some("application/json"));
        assert_eq!(text, "{broken", "invalid JSON is shown as-is");
    }

    /// The demo collection documented in `DocExample/docs/GUIDE` must keep
    /// working: every request parses and its URL resolves with `public`.
    #[test]
    fn doc_example_collection_resolves() {
        fn request_files(dir: &Path, found: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    request_files(&path, found);
                } else if request_collection_root(&path).is_some() {
                    found.push(path);
                }
            }
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("DocExample/api");
        assert!(is_collection_dir(&root));
        let (environments, errors) = load_environments(&root);
        assert!(errors.is_empty(), "{errors:?}");
        let public = environments
            .iter()
            .find(|environment| environment.name == "public")
            .expect("the guide tells users to pick `public`");

        let mut requests = Vec::new();
        request_files(&root, &mut requests);
        requests.sort();
        assert_eq!(requests.len(), 12);

        for path in requests {
            let spec = parse_request(&fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|err| panic!("{}: {err:#}", path.display()));
            let resolved = resolve_request(&root, &path, &spec, Some(public));
            assert!(
                resolved.url.starts_with("https://") && !resolved.url.contains("{{"),
                "{}: {}",
                path.display(),
                resolved.url
            );
            assert_eq!(
                resolved.headers.iter().find(|(name, _)| name == "Accept"),
                Some(&(String::from("Accept"), String::from("application/json"))),
                "{}: the collection header is inherited",
                path.display()
            );
        }

        let echo = root.join("echo/post-json.yml");
        let spec = parse_request(&fs::read_to_string(&echo).unwrap()).unwrap();
        let resolved = resolve_request(&root, &echo, &spec, Some(public));
        assert_eq!(resolved.auth_label, "bearer · heredada de folder echo");
    }
}
