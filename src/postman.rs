//! One-shot conversion of Postman v2.1 exports into an OpenCollection folder.
//! After the import everything is plain files; the JSON is not used again.
//!
//! Postman scripts (`pm.*`) are kept as fenced code in each request's `docs`
//! instead of `runtime.scripts`: Bruno would try to run them with its own API
//! and fail, so preserving them as documentation is the honest option.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::http::{
    append_dotenv, ensure_dotenv_ignored, find_collection_root, is_collection_dir,
    sanitize_file_name, to_yaml, unique_name, write_new, OcAuthOut, OcBodyOut, OcCollectionFileOut,
    OcDefaultsOut, OcEnvVariableOut, OcEnvironmentFileOut, OcFolderFileOut, OcHttpOut, OcInfoOut,
    OcKeyValueOut, OcMultipartOut, OcParamOut, OcRequestFileOut, OcSettingsOut, OcVariableOut,
    COLLECTION_FILE,
};

#[derive(Debug, Default)]
pub struct ImportSummary {
    /// The new collection folder, or the new environment file.
    pub dest: PathBuf,
    pub requests: usize,
    pub folders: usize,
    pub environments: usize,
    pub scripts: usize,
    pub warnings: Vec<String>,
}

pub fn import(source: &Path) -> Result<ImportSummary> {
    let content = fs::read_to_string(source)
        .with_context(|| format!("No se pudo leer {}", source.display()))?;
    let doc: Value = serde_json::from_str(&content).context("JSON invalido")?;

    let schema = doc["info"]["schema"].as_str().unwrap_or_default();
    if doc.get("item").is_some() || schema.contains("collection") {
        if schema.contains("v1.") || doc.get("requests").is_some() {
            bail!("Collection Postman v1: exportala de nuevo como Collection v2.1");
        }
        import_collection(source, &doc)
    } else if doc.get("values").is_some() {
        import_environment(source, &doc)
    } else {
        bail!("No parece un export de Postman (Collection v2.1 o Environment)");
    }
}

fn import_collection(source: &Path, doc: &Value) -> Result<ImportSummary> {
    let name = doc["info"]["name"]
        .as_str()
        .unwrap_or("postman-collection")
        .to_string();
    let parent = source.parent().context("El JSON no tiene carpeta padre")?;
    let dest = parent.join(sanitize_file_name(&name));
    if dest.exists() {
        bail!("Ya existe {}: no se sobrescribe", dest.display());
    }
    fs::create_dir(&dest).with_context(|| format!("No se pudo crear {}", dest.display()))?;

    let mut summary = ImportSummary {
        dest: dest.clone(),
        ..ImportSummary::default()
    };
    let result = write_collection(&dest, &name, doc, &mut summary);
    if result.is_err() {
        // Never leave half a collection behind.
        let _ = fs::remove_dir_all(&dest);
    }
    result.map(|()| summary)
}

fn write_collection(
    dest: &Path,
    name: &str,
    doc: &Value,
    summary: &mut ImportSummary,
) -> Result<()> {
    let defaults = OcDefaultsOut {
        headers: Vec::new(),
        auth: doc
            .get("auth")
            .and_then(|auth| map_auth(auth, &mut summary.warnings)),
        variables: doc["variable"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(OcVariableOut {
                            name: entry["key"].as_str()?.to_string(),
                            value: text(&entry["value"]),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    };
    let collection = OcCollectionFileOut {
        opencollection: "1.0.0",
        info: OcInfoOut {
            name: name.to_string(),
            kind: None,
            seq: None,
        },
        request: defaults.non_empty(),
    };
    write_new(&dest.join(COLLECTION_FILE), &to_yaml(&collection)?)?;
    ensure_dotenv_ignored(dest)?;

    let items = doc["item"].as_array().cloned().unwrap_or_default();
    write_items(dest, &items, summary)?;

    if summary.scripts > 0 {
        summary.warnings.push(format!(
            "{} script(s) de Postman quedaron en docs: no se ejecutan",
            summary.scripts
        ));
    }
    Ok(())
}

fn write_items(dir: &Path, items: &[Value], summary: &mut ImportSummary) -> Result<()> {
    let mut used: HashSet<String> = ["folder.yml", COLLECTION_FILE, "environments"]
        .iter()
        .map(|name| name.to_string())
        .collect();

    for (index, item) in items.iter().enumerate() {
        let name = item["name"].as_str().unwrap_or("Untitled").to_string();
        let seq = index + 1;

        if let Some(children) = item.get("item").and_then(Value::as_array) {
            let folder_name = unique_name(&mut used, &sanitize_file_name(&name), "");
            let folder_dir = dir.join(folder_name);
            fs::create_dir(&folder_dir)
                .with_context(|| format!("No se pudo crear {}", folder_dir.display()))?;
            let defaults = OcDefaultsOut {
                auth: item
                    .get("auth")
                    .and_then(|auth| map_auth(auth, &mut summary.warnings)),
                ..OcDefaultsOut::default()
            };
            let folder = OcFolderFileOut {
                info: OcInfoOut {
                    name,
                    kind: Some("folder"),
                    seq: Some(seq),
                },
                request: defaults.non_empty(),
            };
            write_new(&folder_dir.join("folder.yml"), &to_yaml(&folder)?)?;
            summary.folders += 1;
            write_items(&folder_dir, children, summary)?;
        } else if item.get("request").is_some() {
            let file_name = unique_name(&mut used, &sanitize_file_name(&name), ".yml");
            let request = map_request(item, &name, seq, summary);
            write_new(&dir.join(file_name), &to_yaml(&request)?)?;
            summary.requests += 1;
        }
    }
    Ok(())
}

fn map_request(
    item: &Value,
    name: &str,
    seq: usize,
    summary: &mut ImportSummary,
) -> OcRequestFileOut {
    let request = &item["request"];
    // v2.1 allows a bare URL string as the whole request.
    if let Some(url) = request.as_str() {
        return bare_url_request(name, seq, url);
    }

    let method = request["method"]
        .as_str()
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let (url, params) = map_url(&request["url"]);
    let headers: Vec<OcKeyValueOut> = key_values(&request["header"]);
    let body = map_body(&request["body"], &headers, &mut summary.warnings);
    let auth = match request.get("auth") {
        None | Some(Value::Null) => Some(OcAuthOut::Mode("inherit")),
        Some(auth) => map_auth(auth, &mut summary.warnings),
    };

    let mut docs = Vec::new();
    if let Some(description) = description(&request["description"]) {
        docs.push(description);
    }
    docs.extend(script_sections(item, &mut summary.scripts));

    OcRequestFileOut {
        info: OcInfoOut {
            name: name.to_string(),
            kind: Some("http"),
            seq: Some(seq),
        },
        http: OcHttpOut {
            method,
            url,
            params,
            headers,
            body,
            auth,
        },
        settings: OcSettingsOut { encode_url: true },
        docs: (!docs.is_empty()).then(|| docs.join("\n\n")),
    }
}

fn bare_url_request(name: &str, seq: usize, url: &str) -> OcRequestFileOut {
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

/// Postman URLs are a string or an object with `raw` plus parsed parts.
fn map_url(url: &Value) -> (String, Vec<OcParamOut>) {
    if let Some(raw) = url.as_str() {
        return (raw.to_string(), Vec::new());
    }

    let raw = url["raw"].as_str().map(str::to_string).unwrap_or_else(|| {
        let protocol = url["protocol"].as_str().unwrap_or("https");
        let host = join_parts(&url["host"], ".");
        let path = join_parts(&url["path"], "/");
        format!("{protocol}://{host}/{path}")
    });

    let mut params: Vec<OcParamOut> = url["query"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(OcParamOut {
                        name: entry["key"].as_str()?.to_string(),
                        value: text(&entry["value"]),
                        kind: "query",
                        disabled: entry["disabled"].as_bool() == Some(true),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    params.extend(
        url["variable"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(OcParamOut {
                            name: entry["key"].as_str()?.to_string(),
                            value: text(&entry["value"]),
                            kind: "path",
                            disabled: false,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    (raw, params)
}

fn join_parts(value: &Value, separator: &str) -> String {
    match value {
        Value::Array(parts) => parts.iter().map(text).collect::<Vec<_>>().join(separator),
        other => text(other),
    }
}

fn map_body(
    body: &Value,
    headers: &[OcKeyValueOut],
    warnings: &mut Vec<String>,
) -> Option<OcBodyOut> {
    match body["mode"].as_str()? {
        "raw" => {
            let data = body["raw"].as_str().unwrap_or_default().to_string();
            if data.is_empty() {
                return None;
            }
            let declared_json = headers.iter().any(|header| {
                header.name.eq_ignore_ascii_case("content-type") && header.value.contains("json")
            });
            let kind = match body["options"]["raw"]["language"].as_str() {
                Some("json") => "json",
                Some("xml") => "xml",
                Some(_) => "text",
                None if declared_json || serde_json::from_str::<Value>(&data).is_ok() => "json",
                None => "text",
            };
            Some(OcBodyOut::Raw { kind, data })
        }
        "urlencoded" => Some(OcBodyOut::Form {
            kind: "form-urlencoded",
            data: key_values(&body["urlencoded"]),
        }),
        "formdata" => Some(OcBodyOut::Multipart {
            kind: "multipart-form",
            data: body["formdata"]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| {
                            let is_file = entry["type"].as_str() == Some("file");
                            Some(OcMultipartOut {
                                name: entry["key"].as_str()?.to_string(),
                                kind: if is_file { "file" } else { "text" },
                                value: if is_file {
                                    join_parts(&entry["src"], ",")
                                } else {
                                    text(&entry["value"])
                                },
                                disabled: entry["disabled"].as_bool() == Some(true),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }),
        // GraphQL over HTTP is a JSON POST: keep it sendable.
        "graphql" => {
            let graphql = &body["graphql"];
            let variables = graphql["variables"]
                .as_str()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .unwrap_or_else(|| Value::Object(Default::default()));
            let payload = serde_json::json!({
                "query": text(&graphql["query"]),
                "variables": variables,
            });
            Some(OcBodyOut::Raw {
                kind: "json",
                data: serde_json::to_string_pretty(&payload).unwrap_or_default(),
            })
        }
        other => {
            warnings.push(format!("Body '{other}' de Postman no se importo"));
            None
        }
    }
}

/// `noauth` maps to no auth (omitted), unknown types are reported.
fn map_auth(auth: &Value, warnings: &mut Vec<String>) -> Option<OcAuthOut> {
    let kind = auth["type"].as_str()?;
    let param = |key: &str| auth_param(auth, kind, key);
    match kind {
        "noauth" => None,
        "bearer" => Some(OcAuthOut::Bearer {
            kind: "bearer",
            token: param("token"),
        }),
        "basic" => Some(OcAuthOut::Basic {
            kind: "basic",
            username: param("username"),
            password: param("password"),
        }),
        "apikey" => Some(OcAuthOut::ApiKey {
            kind: "apikey",
            key: param("key"),
            value: param("value"),
            placement: if param("in") == "query" {
                "query"
            } else {
                "header"
            },
        }),
        other => {
            warnings.push(format!("Auth '{other}' de Postman no se importo"));
            None
        }
    }
}

/// v2.1 stores auth params as `[{key, value}]`; v2.0 used a plain object.
fn auth_param(auth: &Value, kind: &str, key: &str) -> String {
    let block = &auth[kind];
    if let Some(entries) = block.as_array() {
        return entries
            .iter()
            .find(|entry| entry["key"].as_str() == Some(key))
            .map(|entry| text(&entry["value"]))
            .unwrap_or_default();
    }
    text(&block[key])
}

fn key_values(value: &Value) -> Vec<OcKeyValueOut> {
    value
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(OcKeyValueOut {
                        name: entry["key"].as_str()?.to_string(),
                        value: text(&entry["value"]),
                        disabled: entry["disabled"].as_bool() == Some(true),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn description(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Object(_) => value["content"].as_str()?.to_string(),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

fn script_sections(item: &Value, counter: &mut usize) -> Vec<String> {
    let Some(events) = item["event"].as_array() else {
        return Vec::new();
    };
    events
        .iter()
        .filter_map(|event| {
            let code = match &event["script"]["exec"] {
                Value::Array(lines) => lines.iter().map(text).collect::<Vec<_>>().join("\n"),
                other => text(other),
            };
            if code.trim().is_empty() {
                return None;
            }
            let title = match event["listen"].as_str() {
                Some("prerequest") => "Postman pre-request script",
                Some("test") => "Postman tests",
                _ => "Postman script",
            };
            *counter += 1;
            Some(format!("## {title}\n\n```js\n{code}\n```"))
        })
        .collect()
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Environments go into an existing collection: the one containing the JSON,
/// or the only collection next to it (where a collection import lands).
fn import_environment(source: &Path, doc: &Value) -> Result<ImportSummary> {
    let root = match find_collection_root(source) {
        Some(root) => root,
        None => sibling_collection(source)?,
    };

    let name = doc["name"].as_str().map(str::to_string).unwrap_or_else(|| {
        source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
            .trim_end_matches(".json")
            .trim_end_matches(".postman_environment")
            .to_string()
    });

    let mut secrets = Vec::new();
    let variables = doc["values"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let key = entry["key"].as_str()?.to_string();
                    let disabled = entry["enabled"].as_bool() == Some(false);
                    let value = text(&entry["value"]);
                    if entry["type"].as_str() == Some("secret") {
                        // The secret moves to the gitignored .env; the YAML
                        // only keeps a reference to it.
                        let env_key = env_key(&key);
                        secrets.push((env_key.clone(), value));
                        Some(OcEnvVariableOut {
                            name: key,
                            value: format!("{{{{process.env.{env_key}}}}}"),
                            disabled,
                        })
                    } else {
                        Some(OcEnvVariableOut {
                            name: key,
                            value,
                            disabled,
                        })
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let environments = root.join("environments");
    fs::create_dir_all(&environments)
        .with_context(|| format!("No se pudo crear {}", environments.display()))?;
    let dest = environments.join(format!("{}.yml", sanitize_file_name(&name)));
    if dest.exists() {
        bail!("Ya existe {}: no se sobrescribe", dest.display());
    }
    write_new(&dest, &to_yaml(&OcEnvironmentFileOut { name, variables })?)?;
    append_dotenv(&root, &secrets)?;

    let mut warnings = Vec::new();
    if !secrets.is_empty() {
        warnings.push(format!(
            "{} secreto(s) se guardaron en .env (gitignored)",
            secrets.len()
        ));
    }
    Ok(ImportSummary {
        dest,
        environments: 1,
        warnings,
        ..ImportSummary::default()
    })
}

fn sibling_collection(source: &Path) -> Result<PathBuf> {
    let parent = source.parent().context("El JSON no tiene carpeta padre")?;
    let collections: Vec<PathBuf> = fs::read_dir(parent)
        .with_context(|| format!("No se pudo leer {}", parent.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && is_collection_dir(path))
        .collect();
    match collections.as_slice() {
        [single] => Ok(single.clone()),
        [] => bail!(
            "No hay una collection donde guardar el environment: importa primero la collection o move el JSON adentro de una"
        ),
        _ => bail!("Hay varias collections al lado del JSON: movelo adentro de la que corresponda"),
    }
}

/// `api-key.prod` → `API_KEY_PROD`, a valid environment variable name.
fn env_key(name: &str) -> String {
    let key: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if key.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{key}")
    } else {
        key
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::http::{
        load_environments, parse_dotenv, parse_request, parse_request_defaults, resolve_request,
        Auth, Body, ParamKind,
    };

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("mdnav-postman-{label}-{}-{id}", std::process::id()));
            fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const COLLECTION: &str = r#"{
      "info": {
        "name": "Shop API",
        "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
      },
      "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "{{token}}", "type": "string" }] },
      "variable": [{ "key": "base_url", "value": "https://shop.test" }],
      "item": [
        {
          "name": "Users",
          "item": [
            {
              "name": "Get user",
              "event": [
                { "listen": "test", "script": { "exec": ["pm.test('ok', () => {", "  pm.response.to.have.status(200);", "});"] } }
              ],
              "request": {
                "method": "GET",
                "header": [{ "key": "Accept", "value": "application/json" }, { "key": "X-Debug", "value": "1", "disabled": true }],
                "url": {
                  "raw": "{{base_url}}/users/:id?expand=orders",
                  "host": ["{{base_url}}"],
                  "path": ["users", ":id"],
                  "query": [{ "key": "expand", "value": "orders" }],
                  "variable": [{ "key": "id", "value": "7" }]
                },
                "description": "Fetches one user."
              }
            },
            {
              "name": "Get user",
              "request": { "method": "DELETE", "url": "{{base_url}}/users/7", "auth": { "type": "noauth" } }
            }
          ]
        },
        {
          "name": "Login",
          "request": {
            "method": "POST",
            "body": { "mode": "urlencoded", "urlencoded": [{ "key": "user", "value": "ana" }, { "key": "pass", "value": "x", "disabled": true }] },
            "url": "{{base_url}}/login"
          }
        },
        {
          "name": "Create order",
          "request": {
            "method": "POST",
            "header": [{ "key": "Content-Type", "value": "application/json" }],
            "body": { "mode": "raw", "raw": "{\"sku\": \"A1\"}" },
            "url": "{{base_url}}/orders",
            "auth": { "type": "basic", "basic": [{ "key": "username", "value": "u" }, { "key": "password", "value": "p" }] }
          }
        },
        {
          "name": "Search",
          "request": {
            "method": "POST",
            "body": { "mode": "graphql", "graphql": { "query": "{ products { id } }", "variables": "{\"first\": 2}" } },
            "url": "{{base_url}}/graphql"
          }
        },
        { "name": "Health", "request": "https://shop.test/health" }
      ]
    }"#;

    fn read(path: PathBuf) -> String {
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }

    #[test]
    fn imports_a_collection_into_opencollection_files() {
        let temp = TestDir::new("collection");
        let source = temp.path().join("shop.postman_collection.json");
        fs::write(&source, COLLECTION).unwrap();

        let summary = import(&source).unwrap();
        let root = temp.path().join("Shop API");
        assert_eq!(summary.dest, root);
        assert_eq!(summary.requests, 6);
        assert_eq!(summary.folders, 1);
        assert_eq!(summary.scripts, 1);
        assert!(read(root.join(".gitignore")).contains(".env"));

        let defaults = parse_request_defaults(&read(root.join(COLLECTION_FILE))).unwrap();
        assert_eq!(
            defaults.auth,
            Some(Auth::Bearer {
                token: String::from("{{token}}")
            })
        );
        assert_eq!(defaults.variables[0].name, "base_url");
        assert!(read(root.join("Users/folder.yml")).contains("type: folder"));

        let get_user = parse_request(&read(root.join("Users/Get user.yml"))).unwrap();
        assert_eq!(get_user.url, "{{base_url}}/users/:id?expand=orders");
        assert_eq!(
            get_user.auth,
            Auth::Inherit,
            "Postman requests inherit by default"
        );
        assert_eq!(get_user.params.len(), 2);
        assert_eq!(get_user.params[1].kind, ParamKind::Path);
        assert!(!get_user.headers[1].enabled);
        assert_eq!(
            get_user.script_count, 0,
            "pm scripts never become runtime scripts"
        );
        let docs = get_user.docs.clone().unwrap();
        assert!(docs.starts_with("Fetches one user."));
        assert!(docs.contains("```js\npm.test('ok'"));

        // Same name in the same folder gets a suffix instead of overwriting.
        let delete = parse_request(&read(root.join("Users/Get user-2.yml"))).unwrap();
        assert_eq!(delete.method, "DELETE");
        assert_eq!(delete.auth, Auth::None, "noauth means no auth");

        let login = parse_request(&read(root.join("Login.yml"))).unwrap();
        match login.body {
            Body::Form(entries) => {
                assert_eq!(entries.len(), 2);
                assert!(!entries[1].enabled);
            }
            other => panic!("unexpected body {other:?}"),
        }

        let order = parse_request(&read(root.join("Create order.yml"))).unwrap();
        assert_eq!(
            order.body,
            Body::Raw {
                kind: String::from("json"),
                data: String::from(r#"{"sku": "A1"}"#),
            }
        );
        assert_eq!(
            order.auth,
            Auth::Basic {
                username: String::from("u"),
                password: String::from("p"),
            }
        );

        let search = parse_request(&read(root.join("Search.yml"))).unwrap();
        match search.body {
            Body::Raw { kind, data } => {
                assert_eq!(kind, "json");
                let payload: Value = serde_json::from_str(&data).unwrap();
                assert_eq!(payload["query"], "{ products { id } }");
                assert_eq!(payload["variables"]["first"], 2);
            }
            other => panic!("unexpected body {other:?}"),
        }

        let health = parse_request(&read(root.join("Health.yml"))).unwrap();
        assert_eq!(health.url, "https://shop.test/health");

        // The imported request resolves end to end with collection variables.
        let resolved = resolve_request(
            &root,
            &root.join("Users/Get user.yml"),
            &get_user,
            None,
            None,
        );
        assert_eq!(resolved.url, "https://shop.test/users/7?expand=orders");
        assert_eq!(resolved.unresolved, vec![String::from("token")]);
    }

    #[test]
    fn never_overwrites_an_existing_collection() {
        let temp = TestDir::new("existing");
        let source = temp.path().join("shop.postman_collection.json");
        fs::write(&source, COLLECTION).unwrap();
        fs::create_dir(temp.path().join("Shop API")).unwrap();
        fs::write(temp.path().join("Shop API").join("keep.md"), "mine").unwrap();

        assert!(import(&source).is_err());
        assert_eq!(read(temp.path().join("Shop API").join("keep.md")), "mine");
    }

    #[test]
    fn imports_environments_moving_secrets_to_dotenv() {
        let temp = TestDir::new("environment");
        let collection = temp.path().join("shop.postman_collection.json");
        fs::write(&collection, COLLECTION).unwrap();
        import(&collection).unwrap();

        let source = temp.path().join("prod.postman_environment.json");
        fs::write(
            &source,
            r#"{
              "name": "Production",
              "values": [
                { "key": "base_url", "value": "https://shop.prod", "enabled": true },
                { "key": "api-key", "value": "k\"1", "type": "secret", "enabled": true },
                { "key": "legacy", "value": "x", "enabled": false }
              ],
              "_postman_variable_scope": "environment"
            }"#,
        )
        .unwrap();

        let summary = import(&source).unwrap();
        let root = temp.path().join("Shop API");
        assert_eq!(summary.dest, root.join("environments/Production.yml"));
        assert_eq!(summary.environments, 1);

        let (environments, errors) = load_environments(&root);
        assert!(errors.is_empty(), "{errors:?}");
        let production = &environments[0];
        assert_eq!(production.name, "Production");
        assert_eq!(
            production.variables[1].value.as_deref(),
            Some("{{process.env.API_KEY}}")
        );
        assert!(!production.variables[2].enabled);

        let dotenv = parse_dotenv(&read(root.join(".env")));
        assert_eq!(dotenv["API_KEY"], "k\"1");
        assert!(
            !read(summary.dest.clone()).contains("k\\\"1"),
            "the secret never lands in the YAML"
        );

        assert!(import(&source).is_err(), "environments are not overwritten");
    }

    #[test]
    fn rejects_environments_without_a_target_collection() {
        let temp = TestDir::new("orphan");
        let source = temp.path().join("dev.postman_environment.json");
        fs::write(&source, r#"{"name": "dev", "values": []}"#).unwrap();

        let error = import(&source).unwrap_err().to_string();
        assert!(error.contains("No hay una collection"), "{error}");
    }

    #[test]
    fn rejects_unknown_json() {
        let temp = TestDir::new("unknown");
        let source = temp.path().join("data.json");
        fs::write(&source, r#"{"hello": "world"}"#).unwrap();
        assert!(import(&source).is_err());

        fs::write(&source, r#"{"info": {"schema": "https://schema.getpostman.com/json/collection/v1.0.0/collection.json"}, "requests": []}"#).unwrap();
        assert!(import(&source).unwrap_err().to_string().contains("v2.1"));
    }

    #[test]
    fn builds_valid_environment_variable_names() {
        assert_eq!(env_key("api-key.prod"), "API_KEY_PROD");
        assert_eq!(env_key("2fa"), "_2FA");
    }
}
