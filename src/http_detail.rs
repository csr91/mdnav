//! The request detail of the HTTP client: the selected request as it goes
//! out, with its variables resolved and its secrets masked, plus an "Avisos"
//! block with everything that keeps it from being sent as written. The lines
//! are built here once, so the render and the scroll clamping in `App` agree.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

use crate::{
    app::HttpView,
    http::{
        self, Auth, Body, HttpRequestSpec, ParamKind, RequestSettings, ResolvedBody,
        ResolvedRequest,
    },
};

/// Body lines shown before the rest is summed up in one line.
const BODY_PREVIEW_LINES: usize = 10;
/// Leading characters of a secret left on screen.
const SECRET_VISIBLE_CHARS: usize = 4;
/// Auth schemes kept readable in front of a masked credential.
const AUTH_SCHEMES: [&str; 5] = ["bearer", "basic", "token", "digest", "apikey"];
/// Name fragments of headers, params and fields that usually carry secrets.
/// Names ending in `key` (`api_key`, `X-API-Key`, ...) count too.
const SECRET_MARKERS: [&str; 9] = [
    "auth",
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "cookie",
    "session",
    "credential",
];

pub fn method_color(method: &str) -> Color {
    match method {
        "GET" => Color::Green,
        "POST" => Color::Yellow,
        "PUT" => Color::Blue,
        "PATCH" => Color::Magenta,
        "DELETE" => Color::Red,
        _ => Color::Gray,
    }
}

/// Every line of the request detail, before scrolling. Sections without
/// content are left out.
pub fn request_detail_lines(view: Option<&HttpView>) -> Vec<Line<'static>> {
    let Some(view) = view else {
        return vec![
            Line::from("Selecciona o crea un request"),
            Line::from(""),
            Line::from("l/→ abre carpetas · N crea un request · S envía"),
        ];
    };

    let (parse_error, resolved) = match &view.request {
        Ok((_, resolved)) => (None, Some(resolved)),
        Err(error) => (Some(error.as_str()), None),
    };
    let avisos = build_avisos(
        parse_error,
        view.missing_environment.as_deref(),
        &view.environment_errors,
        resolved,
    );
    match &view.request {
        Ok((spec, resolved)) => resolved_lines(spec, resolved, &avisos),
        Err(_) => error_lines(&avisos),
    }
}

/// What the user should know before sending: why the YAML cannot be read, a
/// missing environment, unresolved variables, the resolver's warnings (auth
/// or body not supported, scripts not run, broken `folder.yml`) and broken
/// environment files.
pub fn build_avisos(
    parse_error: Option<&str>,
    missing_environment: Option<&str>,
    environment_errors: &[String],
    resolved: Option<&ResolvedRequest>,
) -> Vec<String> {
    let mut avisos: Vec<String> = parse_error.map(str::to_string).into_iter().collect();
    if let Some(name) = missing_environment {
        avisos.push(format!("El environment '{name}' ya no existe"));
    }
    if let Some(resolved) = resolved {
        avisos.extend(
            resolved
                .unresolved
                .iter()
                .map(|name| format!("Variable sin resolver: {{{{{name}}}}}")),
        );
        avisos.extend(resolved.warnings.iter().cloned());
    }
    avisos.extend(environment_errors.iter().cloned());
    avisos
}

fn resolved_lines(
    spec: &HttpRequestSpec,
    resolved: &ResolvedRequest,
    avisos: &[String],
) -> Vec<Line<'static>> {
    let white = Style::default().fg(Color::White);
    let mut header = vec![Span::styled(
        format!("{} ", resolved.method),
        Style::default()
            .fg(method_color(&resolved.method))
            .add_modifier(Modifier::BOLD),
    )];
    if resolved.url.is_empty() {
        header.push(Span::styled("(sin URL)", gray()));
    } else {
        header.extend(value_spans(&mask_url_query(&resolved.url), white));
    }
    let mut lines = vec![Line::from(header)];
    // The template, aligned under the resolved URL.
    if spec.url != resolved.url {
        lines.push(Line::from(Span::styled(
            format!(
                "{}{}",
                " ".repeat(resolved.method.chars().count() + 1),
                mask_url_query(&spec.url)
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
    push_avisos(&mut lines, avisos);

    let params = resolved
        .params
        .iter()
        .map(|(kind, name, value)| match kind {
            ParamKind::Path => entry_line(name, " (path) = ", value),
            ParamKind::Query => entry_line(name, " = ", &mask_if_secret(name, value)),
        })
        .collect();
    push_section(
        &mut lines,
        format!("Params ({})", resolved.params.len()),
        params,
    );

    let headers = resolved
        .headers
        .iter()
        .map(|(name, value)| entry_line(name, ": ", &mask_if_secret(name, value)))
        .collect();
    push_section(
        &mut lines,
        format!("Headers ({})", resolved.headers.len()),
        headers,
    );

    if spec.auth != Auth::None {
        let mut auth = vec![Line::from(Span::styled(
            format!("  {}", resolved.auth_label),
            white,
        ))];
        auth.extend(resolved.auth_fields.iter().map(|field| {
            let value = if field.secret {
                mask_secret(&field.value)
            } else {
                field.value.clone()
            };
            entry_line(&field.name, " = ", &value)
        }));
        push_section(&mut lines, String::from("Auth"), auth);
    }

    if let Some((title, body)) = body_section(spec, resolved) {
        push_section(&mut lines, title, body);
    }

    push_section(
        &mut lines,
        String::from("Settings"),
        settings_lines(&resolved.settings),
    );

    if let Some(docs) = &spec.docs {
        push_section(
            &mut lines,
            String::from("Docs"),
            docs.lines()
                .map(|line| Line::from(line.to_string()))
                .collect(),
        );
    }
    lines
}

/// Shown instead of the detail when the YAML cannot be read as a request.
fn error_lines(avisos: &[String]) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "No se pudo leer el request",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    ))];
    push_avisos(&mut lines, avisos);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Shift+E abre el archivo en el editor.",
        gray(),
    )));
    lines
}

fn body_section(
    spec: &HttpRequestSpec,
    resolved: &ResolvedRequest,
) -> Option<(String, Vec<Line<'static>>)> {
    match (&spec.body, &resolved.body) {
        (Body::Raw { kind, .. }, ResolvedBody::Text(text)) => {
            let pretty = (kind == "json").then(|| pretty_json(text)).flatten();
            let rows = pretty
                .as_deref()
                .unwrap_or(text)
                .lines()
                .map(|line| {
                    let mut spans = vec![Span::raw("  ")];
                    spans.extend(value_spans(line, Style::default().fg(Color::White)));
                    Line::from(spans)
                })
                .collect();
            Some((format!("Body · {kind}"), cut_short(rows)))
        }
        (Body::Form(_), ResolvedBody::Form(entries)) => {
            let rows = entries
                .iter()
                .map(|(name, value)| entry_line(name, " = ", &mask_if_secret(name, value)))
                .collect();
            Some((String::from("Body · form-urlencoded"), cut_short(rows)))
        }
        (Body::Unsupported(kind), _) => Some((
            format!("Body · {kind}"),
            vec![Line::from(Span::styled(
                "  no se envía: tipo no soportado",
                gray(),
            ))],
        )),
        _ => None,
    }
}

fn pretty_json(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// Keeps the first `BODY_PREVIEW_LINES` rows and counts the rest.
fn cut_short(mut rows: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if rows.len() > BODY_PREVIEW_LINES {
        let hidden = rows.len() - BODY_PREVIEW_LINES;
        rows.truncate(BODY_PREVIEW_LINES);
        rows.push(Line::from(Span::styled(
            format!("  … {hidden} líneas más"),
            gray(),
        )));
    }
    rows
}

/// Only what differs from the defaults: no timeout and following up to five
/// redirects go unsaid.
fn settings_lines(settings: &RequestSettings) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(timeout) = settings.timeout {
        lines.push(entry_line("timeout", ": ", &http::format_elapsed(timeout)));
    }
    if !settings.follow_redirects {
        lines.push(entry_line("redirects", ": ", "no se siguen"));
    } else if settings.max_redirects != RequestSettings::default().max_redirects {
        lines.push(entry_line(
            "redirects",
            ": ",
            &format!("hasta {}", settings.max_redirects),
        ));
    }
    lines
}

fn gray() -> Style {
    Style::default().fg(Color::Gray)
}

/// A blank line, the section title and its rows; nothing without rows.
fn push_section(lines: &mut Vec<Line<'static>>, title: String, rows: Vec<Line<'static>>) {
    if rows.is_empty() {
        return;
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        title,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.extend(rows);
}

/// The avisos stand out on a yellow title right below the URL. Multi-line
/// messages (YAML errors quote the offending line) keep their own rows.
fn push_avisos(lines: &mut Vec<Line<'static>>, avisos: &[String]) {
    if avisos.is_empty() {
        return;
    }
    let yellow = Style::default().fg(Color::Yellow);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(" Avisos ({}) ", avisos.len()),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    for aviso in avisos {
        for (index, row) in aviso.lines().enumerate() {
            let prefix = if index == 0 { "  ⚠ " } else { "    " };
            lines.push(Line::from(Span::styled(format!("{prefix}{row}"), yellow)));
        }
    }
}

/// `  name<separator>value`, with unresolved variables in the value marked.
fn entry_line(name: &str, separator: &str, value: &str) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("  {name}"), Style::default().fg(Color::Cyan)),
        Span::styled(separator.to_string(), gray()),
    ];
    spans.extend(value_spans(value, Style::default().fg(Color::White)));
    Line::from(spans)
}

/// `text` in `style`, except unresolved `{{variables}}`, which stand out.
fn value_spans(text: &str, style: Style) -> Vec<Span<'static>> {
    let unresolved = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    split_unresolved(text)
        .into_iter()
        .map(|(segment, is_variable)| {
            Span::styled(
                segment.to_string(),
                if is_variable { unresolved } else { style },
            )
        })
        .collect()
}

/// Splits resolved text into plain runs and `{{name}}` tokens (`true`). The
/// resolver leaves only unknown variables in place, so every token left is
/// one it could not resolve.
pub fn split_unresolved(text: &str) -> Vec<(&str, bool)> {
    let mut segments = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start + 2..].find("}}") else {
            break;
        };
        let close = start + 2 + end + 2;
        if start > 0 {
            segments.push((&rest[..start], false));
        }
        segments.push((&rest[start..close], true));
        rest = &rest[close..];
    }
    if !rest.is_empty() {
        segments.push((rest, false));
    }
    segments
}

pub fn is_secret_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with("key") || SECRET_MARKERS.iter().any(|marker| name.contains(marker))
}

fn mask_if_secret(name: &str, value: &str) -> String {
    if is_secret_name(name) {
        mask_secret(value)
    } else {
        value.to_string()
    }
}

/// The first few characters and `…`, keeping a leading auth scheme (`Bearer
/// eyJh…`). Short values show at most half of themselves, and a bare unresolved
/// variable is left whole: it hides nothing and says what is missing.
pub fn mask_secret(value: &str) -> String {
    let (scheme, secret) = match value.split_once(' ') {
        Some((scheme, rest))
            if AUTH_SCHEMES
                .iter()
                .any(|known| scheme.eq_ignore_ascii_case(known)) =>
        {
            (&value[..=scheme.len()], rest)
        }
        _ => ("", value),
    };
    let is_variable = matches!(split_unresolved(secret).as_slice(), [(_, true)]);
    if secret.is_empty() || is_variable {
        return value.to_string();
    }
    let visible = SECRET_VISIBLE_CHARS.min(secret.chars().count() / 2);
    let shown: String = secret.chars().take(visible).collect();
    format!("{scheme}{shown}…")
}

/// Masks the values of query params with secret-looking names, such as an api
/// key sent in the query.
pub fn mask_url_query(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let query = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) if is_secret_name(name) => {
                format!("{name}={}", mask_secret(value))
            }
            _ => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::http::{AuthField, KeyValue};

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    fn spec() -> HttpRequestSpec {
        HttpRequestSpec {
            name: String::from("Get user"),
            method: String::from("POST"),
            url: String::from("{{base}}/users/:id"),
            headers: vec![KeyValue {
                name: String::from("Accept"),
                value: String::from("application/json"),
                enabled: true,
            }],
            params: Vec::new(),
            body: Body::Raw {
                kind: String::from("json"),
                data: String::new(),
            },
            auth: Auth::Inherit,
            settings: RequestSettings::default(),
            script_count: 0,
            assertion_count: 0,
            docs: Some(String::from("Trae un usuario")),
        }
    }

    fn resolved() -> ResolvedRequest {
        let items = (1..=12)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        ResolvedRequest {
            method: String::from("POST"),
            url: String::from("https://api.test/users/42?page=1&api_key=abcdef123"),
            headers: vec![
                (String::from("Accept"), String::from("application/json")),
                (
                    String::from("Authorization"),
                    String::from("Bearer eyJhbGciOi"),
                ),
                (String::from("X-Org"), String::from("{{org}}")),
            ],
            params: vec![
                (ParamKind::Path, String::from("id"), String::from("42")),
                (ParamKind::Query, String::from("page"), String::from("1")),
            ],
            body: ResolvedBody::Text(format!("{{\"items\": [{items}]}}")),
            settings: RequestSettings::default(),
            auth_label: String::from("bearer · heredada de collection"),
            auth_fields: vec![AuthField {
                name: String::from("token"),
                value: String::from("eyJhbGciOi"),
                secret: true,
            }],
            unresolved: vec![String::from("org")],
            warnings: vec![String::from(
                "1 script(s) del request no se ejecutan en mdnav",
            )],
        }
    }

    fn view(request: Result<(HttpRequestSpec, ResolvedRequest), String>) -> HttpView {
        HttpView {
            collection_root: PathBuf::from("api"),
            request_path: PathBuf::from("api").join("get-user.yml"),
            request,
            environment: None,
            missing_environment: Some(String::from("dev")),
            environment_errors: Vec::new(),
        }
    }

    #[test]
    fn masks_secrets_keeping_the_scheme_and_unresolved_variables() {
        assert_eq!(mask_secret("Bearer eyJhbGciOi"), "Bearer eyJh…");
        assert_eq!(mask_secret("basic dXNlcjpwYXNz"), "basic dXNl…");
        assert_eq!(mask_secret("s3cr3t-value"), "s3cr…");
        assert_eq!(mask_secret("abc"), "a…", "short values show at most half");
        assert_eq!(mask_secret("x"), "…");
        assert_eq!(mask_secret("ñandú-secreto"), "ñand…");
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("{{token}}"), "{{token}}");
        assert_eq!(mask_secret("Bearer {{token}}"), "Bearer {{token}}");
        assert_eq!(mask_secret("{{a}}-{{b}}"), "{{a}…");
    }

    #[test]
    fn recognizes_secret_names() {
        for name in [
            "Authorization",
            "X-API-Key",
            "api_key",
            "key",
            "Cookie",
            "access_token",
            "client_secret",
            "password",
        ] {
            assert!(is_secret_name(name), "{name}");
        }
        for name in ["Accept", "Content-Type", "X-Trace", "page", "Keep-Alive"] {
            assert!(!is_secret_name(name), "{name}");
        }
    }

    #[test]
    fn masks_secret_query_params_in_urls() {
        assert_eq!(
            mask_url_query("https://h/s?q=a&api_key=abcdef123&token={{t}}"),
            "https://h/s?q=a&api_key=abcd…&token={{t}}"
        );
        assert_eq!(mask_url_query("https://h/s"), "https://h/s");
    }

    #[test]
    fn splits_out_unresolved_variables() {
        assert_eq!(
            split_unresolved("{{base}}/users/{{id}}?x=1"),
            [
                ("{{base}}", true),
                ("/users/", false),
                ("{{id}}", true),
                ("?x=1", false)
            ]
        );
        assert_eq!(split_unresolved("plain"), [("plain", false)]);
        assert_eq!(split_unresolved("{{open"), [("{{open", false)]);
        assert!(split_unresolved("").is_empty());

        let spans = value_spans("a{{b}}", Style::default());
        assert_eq!(spans[1].content, "{{b}}");
        assert_eq!(spans[1].style.fg, Some(Color::Red));
        assert_eq!(spans[0].style.fg, None);
    }

    #[test]
    fn builds_avisos_in_order() {
        let resolved = resolved();
        assert_eq!(
            build_avisos(
                None,
                Some("dev"),
                &[String::from("qa.yml: YAML invalido")],
                Some(&resolved)
            ),
            [
                "El environment 'dev' ya no existe",
                "Variable sin resolver: {{org}}",
                "1 script(s) del request no se ejecutan en mdnav",
                "qa.yml: YAML invalido",
            ]
        );
        assert_eq!(
            build_avisos(Some("YAML invalido: linea 3"), None, &[], None),
            ["YAML invalido: linea 3"]
        );
        assert!(build_avisos(None, None, &[], None).is_empty());
    }

    #[test]
    fn detail_lists_sections_in_order_with_secrets_masked() {
        let lines = texts(&request_detail_lines(Some(&view(Ok((spec(), resolved()))))));
        let position = |text: &str| {
            lines
                .iter()
                .position(|line| line == text)
                .unwrap_or_else(|| panic!("{text:?} in {lines:#?}"))
        };

        assert_eq!(
            lines[0],
            "POST https://api.test/users/42?page=1&api_key=abcd…"
        );
        assert_eq!(lines[1], "     {{base}}/users/:id", "the template, dimmed");
        assert_eq!(lines[3], " Avisos (3) ", "right after the URL");
        assert_eq!(lines[4], "  ⚠ El environment 'dev' ya no existe");
        assert!(position("Params (2)") < position("Headers (3)"));
        assert!(position("Headers (3)") < position("Auth"));
        assert!(position("Auth") < position("Body · json"));
        assert!(position("Body · json") < position("Docs"));
        position("  id (path) = 42");
        position("  Authorization: Bearer eyJh…");
        position("  X-Org: {{org}}");
        position("  bearer · heredada de collection");
        position("  token = eyJh…");
        position("  … 6 líneas más");
        assert!(
            !lines.iter().any(|line| line.contains("Settings")),
            "default settings are omitted"
        );
        assert_eq!(lines.last().map(String::as_str), Some("Trae un usuario"));
    }

    #[test]
    fn detail_omits_empty_sections_and_shows_settings() {
        let spec = HttpRequestSpec {
            url: String::from("https://api.test"),
            headers: Vec::new(),
            body: Body::None,
            auth: Auth::None,
            docs: None,
            ..spec()
        };
        let resolved = ResolvedRequest {
            url: String::from("https://api.test"),
            headers: Vec::new(),
            params: Vec::new(),
            body: ResolvedBody::None,
            settings: RequestSettings {
                timeout: Some(Duration::from_millis(1500)),
                follow_redirects: false,
                max_redirects: 5,
            },
            auth_fields: Vec::new(),
            unresolved: Vec::new(),
            warnings: Vec::new(),
            ..resolved()
        };
        let view = HttpView {
            missing_environment: None,
            ..view(Ok((spec, resolved)))
        };
        assert_eq!(
            texts(&request_detail_lines(Some(&view))),
            [
                "POST https://api.test",
                "",
                "Settings",
                "  timeout: 1.50 s",
                "  redirects: no se siguen",
            ]
        );
    }

    #[test]
    fn unreadable_yaml_shows_the_error_instead_of_the_detail() {
        let lines = texts(&request_detail_lines(Some(&view(Err(String::from(
            "YAML invalido: linea 3\n  http: [",
        ))))));
        assert_eq!(
            lines,
            [
                "No se pudo leer el request",
                "",
                " Avisos (2) ",
                "  ⚠ YAML invalido: linea 3",
                "      http: [",
                "  ⚠ El environment 'dev' ya no existe",
                "",
                "Shift+E abre el archivo en el editor.",
            ]
        );
    }
}
