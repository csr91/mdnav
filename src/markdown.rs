use std::collections::{BTreeMap, HashMap};

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::docs::LinkTarget;

#[derive(Clone, Debug)]
pub enum PreviewLineKind {
    Normal,
    Heading(u8),
    CodeFence,
    MermaidTitle,
}

#[derive(Clone, Debug)]
pub struct PreviewLine {
    pub text: String,
    pub kind: PreviewLineKind,
}

#[derive(Clone, Debug)]
pub struct MermaidBlock {
    pub title: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct CanvasNode {
    pub label: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MermaidCanvas {
    pub lines: Vec<String>,
    pub nodes: Vec<CanvasNode>,
}

#[derive(Clone, Debug, Default)]
pub struct PreviewDocument {
    pub lines: Vec<PreviewLine>,
    pub links: Vec<LinkTarget>,
    pub mermaid_blocks: usize,
    pub mermaid_diagrams: Vec<MermaidBlock>,
}

pub fn load_preview(path: &Path) -> Result<PreviewDocument> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("No se pudo leer {}", path.display()))?;
    Ok(render_preview(path, &content))
}

pub fn mermaid_terminal_canvas(diagram: &MermaidBlock) -> MermaidCanvas {
    // Diagrams that are not flowcharts can't be rendered as boxes — show source
    if let Some(kind) = non_flowchart_type(&diagram.source) {
        return render_source_canvas(diagram, kind);
    }

    let raw_lines = diagram
        .source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        // Strip subgraph declarations and their closing `end` — edges inside are
        // kept, the grouping itself isn't meaningful for box rendering
        .filter(|line| !is_subgraph_keyword(line))
        .collect::<Vec<_>>();

    let aliases = collect_mermaid_aliases(&raw_lines);
    let urls = parse_click_urls(&diagram.source, &aliases);
    let edges = raw_lines
        .iter()
        .filter_map(|line| simplify_mermaid_edge(line, &aliases))
        .collect::<Vec<_>>();

    if edges.is_empty() {
        return MermaidCanvas {
            lines: vec![
                String::from("Sin conexiones detectadas"),
                String::new(),
                String::from("El bloque no contiene aristas reconocibles (-->  ---  ==>)."),
            ],
            nodes: vec![],
        };
    }

    let is_lr = is_left_to_right(&diagram.source);
    render_mermaid_canvas_from_edges(&edges, &urls, is_lr)
}

fn is_left_to_right(source: &str) -> bool {
    source
        .lines()
        .next()
        .map(|line| {
            line.contains("flowchart LR")
                || line.contains("graph LR")
                || line.contains("flowchart RL")
                || line.contains("graph RL")
        })
        .unwrap_or(false)
}

/// Returns the diagram kind name if the source is NOT a flowchart/graph.
fn non_flowchart_type(source: &str) -> Option<&'static str> {
    let first = source
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.starts_with("sequenceDiagram") {
        Some("sequenceDiagram")
    } else if first.starts_with("pie") {
        Some("pie")
    } else if first.starts_with("gitGraph") {
        Some("gitGraph")
    } else if first.starts_with("classDiagram") {
        Some("classDiagram")
    } else if first.starts_with("stateDiagram") {
        Some("stateDiagram")
    } else if first.starts_with("erDiagram") {
        Some("erDiagram")
    } else if first.starts_with("gantt") {
        Some("gantt")
    } else if first.starts_with("journey") {
        Some("journey")
    } else {
        None
    }
}

/// Render the raw source lines as a plain canvas with a type header.
fn render_source_canvas(diagram: &MermaidBlock, kind: &str) -> MermaidCanvas {
    let header = format!("[ {} ]  —  abrí en HTML o Web para render visual", kind);
    let separator = "─".repeat(header.chars().count().min(60));
    let mut lines = vec![header, separator, String::new()];
    for line in diagram.source.lines() {
        lines.push(line.to_string());
    }
    MermaidCanvas { lines, nodes: vec![] }
}

fn is_subgraph_keyword(line: &str) -> bool {
    let t = line.trim();
    t == "end" || t.starts_with("subgraph") || t.starts_with("classDef") || t.starts_with("class ")
}

pub fn render_preview(current_path: &Path, markdown: &str) -> PreviewDocument {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(markdown, options);

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_kind = PreviewLineKind::Normal;
    let mut links = Vec::new();
    let mut list_depth = 0usize;
    let mut code_block = false;
    let mut mermaid_block = false;
    let mut mermaid_blocks = 0usize;
    let mut mermaid_diagrams = Vec::new();
    let mut current_mermaid = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_line(&mut current, &mut current_kind, &mut lines);
                current_kind = PreviewLineKind::Heading(heading_level(level));
                current.push_str(&"#".repeat(heading_level(level) as usize));
                current.push(' ');
            }
            Event::Start(Tag::Paragraph) => {
                current_kind = PreviewLineKind::Normal;
            }
            Event::Start(Tag::List(_)) => {
                flush_line(&mut current, &mut current_kind, &mut lines);
                list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                flush_line(&mut current, &mut current_kind, &mut lines);
            }
            Event::Start(Tag::Item) => {
                flush_line(&mut current, &mut current_kind, &mut lines);
                current_kind = PreviewLineKind::Normal;
                current.push_str(&"  ".repeat(list_depth.saturating_sub(1)));
                current.push_str("- ");
            }
            Event::End(TagEnd::Item) => {
                flush_line(&mut current, &mut current_kind, &mut lines);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_line(&mut current, &mut current_kind, &mut lines);
                code_block = true;

                let language = match kind {
                    pulldown_cmark::CodeBlockKind::Indented => String::from("text"),
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                };

                if language.eq_ignore_ascii_case("mermaid") {
                    mermaid_block = true;
                    current_mermaid.clear();
                    mermaid_blocks += 1;
                } else {
                    lines.push(PreviewLine {
                        text: format!("```{language}"),
                        kind: PreviewLineKind::CodeFence,
                    });
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                code_block = false;
                if !mermaid_block {
                    lines.push(PreviewLine {
                        text: String::from("```"),
                        kind: PreviewLineKind::CodeFence,
                    });
                } else {
                    let source = current_mermaid.join("\n");
                    let title = format!("Mermaid {}", mermaid_blocks);
                    lines.push(PreviewLine {
                        text: format!("{title}    Shift+M to open"),
                        kind: PreviewLineKind::MermaidTitle,
                    });
                    mermaid_diagrams.push(MermaidBlock { title, source });
                    lines.push(PreviewLine {
                        text: String::new(),
                        kind: PreviewLineKind::Normal,
                    });
                }
                mermaid_block = false;
                current_mermaid.clear();
                lines.push(PreviewLine {
                    text: String::new(),
                    kind: PreviewLineKind::Normal,
                });
            }
            Event::Start(Tag::Link { dest_url, title, .. }) => {
                let label = if title.is_empty() {
                    dest_url.to_string()
                } else {
                    title.to_string()
                };
                let resolved = resolve_link(current_path, &dest_url);
                links.push(LinkTarget {
                    label,
                    raw_target: dest_url.to_string(),
                    resolved,
                    line_index: lines.len(),
                });
            }
            Event::Text(text) => {
                if code_block && mermaid_block {
                    for raw_line in text.lines() {
                        if raw_line.trim().is_empty() {
                            continue;
                        }
                        current_mermaid.push(raw_line.to_string());
                    }
                } else if code_block {
                    lines.push(PreviewLine {
                        text: text.to_string(),
                        kind: PreviewLineKind::CodeFence,
                    });
                } else {
                    current.push_str(&text);
                }
            }
            Event::Code(code) => {
                current.push('`');
                current.push_str(&code);
                current.push('`');
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_line(&mut current, &mut current_kind, &mut lines);
            }
            Event::Rule => {
                flush_line(&mut current, &mut current_kind, &mut lines);
                lines.push(PreviewLine {
                    text: String::from("----------------"),
                    kind: PreviewLineKind::Normal,
                });
            }
            Event::End(TagEnd::Paragraph) => {
                flush_line(&mut current, &mut current_kind, &mut lines);
                lines.push(PreviewLine {
                    text: String::new(),
                    kind: PreviewLineKind::Normal,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                flush_line(&mut current, &mut current_kind, &mut lines);
                current_kind = PreviewLineKind::Normal;
                lines.push(PreviewLine {
                    text: String::new(),
                    kind: PreviewLineKind::Normal,
                });
            }
            _ => {}
        }
    }

    flush_line(&mut current, &mut current_kind, &mut lines);

    PreviewDocument {
        lines,
        links,
        mermaid_blocks,
        mermaid_diagrams,
    }
}

fn flush_line(current: &mut String, current_kind: &mut PreviewLineKind, lines: &mut Vec<PreviewLine>) {
    if !current.trim().is_empty() {
        lines.push(PreviewLine {
            text: current.trim_end().to_string(),
            kind: current_kind.clone(),
        });
        current.clear();
    } else if !current.is_empty() {
        lines.push(PreviewLine {
            text: String::new(),
            kind: PreviewLineKind::Normal,
        });
        current.clear();
    }

    *current_kind = PreviewLineKind::Normal;
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn resolve_link(current_path: &Path, raw_target: &str) -> Option<PathBuf> {
    if raw_target.starts_with("http://") || raw_target.starts_with("https://") {
        return None;
    }

    let clean_target = raw_target.split('#').next().unwrap_or_default().trim();
    if clean_target.is_empty() {
        return None;
    }

    let parent = current_path.parent()?;
    Some(parent.join(clean_target))
}

fn sanitize_mermaid_line(line: &str) -> String {
    line.replace("<br/>", " / ")
        .replace("<br>", " / ")
        .replace("-->", "->")
        .replace('\t', "    ")
}

// ── Edge parsing ─────────────────────────────────────────────────────────────

/// Extract a pipe-enclosed edge label: `-->|label|` or `--|label|`
fn extract_pipe_label(line: &str) -> Option<String> {
    let arrow_pos = line.find("--")?;
    let after_arrow = &line[arrow_pos..];
    let pipe_open = after_arrow.find('|')?;
    let rest = &after_arrow[pipe_open + 1..];
    let pipe_close = rest.find('|')?;
    let label = rest[..pipe_close].trim();
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}

/// Extract a dash-enclosed edge label: `A -- label text --> B`
/// Only matches when there is text between `--` and `-->` / `---`.
fn extract_dash_label(line: &str) -> Option<String> {
    // Find `--` not immediately followed by `>` or another `-`
    let pos = line.find("--")?;
    let after = &line[pos + 2..];
    if after.starts_with('>') || after.starts_with('-') {
        return None;
    }
    // The label ends at the next `--`
    let end = after.find("--")?;
    let label = after[..end].trim().trim_matches('"');
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}

fn extract_edge_label(line: &str) -> Option<String> {
    extract_pipe_label(line).or_else(|| extract_dash_label(line))
}

fn simplify_mermaid_edge(
    line: &str,
    aliases: &HashMap<String, String>,
) -> Option<(String, String, Option<String>)> {
    if !line.contains("--") && !line.contains("->") {
        return None;
    }

    let label = extract_edge_label(line);
    let normalized = normalize_mermaid_edge(line);

    let parts = normalized.split("->").map(str::trim).collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }

    let left = parse_mermaid_node(parts.first()?.trim(), aliases);
    let right = parse_mermaid_node(parts.last()?.trim(), aliases);
    Some((left, right, label))
}

fn parse_mermaid_node(node: &str, aliases: &HashMap<String, String>) -> String {
    let trimmed = node.trim();

    if let Some(label) = extract_between(trimmed, '[', ']')
        .or_else(|| extract_between(trimmed, '{', '}'))
        .or_else(|| extract_between(trimmed, '(', ')'))
        .or_else(|| extract_odd_shape_label(trimmed))
    {
        return cleanup_mermaid_label(&label);
    }

    if let Some(alias) = aliases.get(trimmed) {
        return alias.clone();
    }

    cleanup_mermaid_label(trimmed)
}

fn extract_between(value: &str, open: char, close: char) -> Option<String> {
    let start = value.find(open)?;
    let end = value.rfind(close)?;
    if end <= start {
        return None;
    }
    Some(value[start + 1..end].to_string())
}

fn cleanup_mermaid_label(value: &str) -> String {
    let cleaned = value
        .replace("<br/>", " / ")
        .replace("<br>", " / ")
        .replace(['[', ']', '{', '}', '(', ')'], " ")
        .replace("%%", " ")
        .replace('"', "")
        .replace('\'', "")
        .replace("==", " ")
        .replace("-.", " ")
        .replace("..", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.is_empty() {
        value.trim().to_string()
    } else {
        cleaned
    }
}

fn collect_mermaid_aliases(lines: &[&str]) -> HashMap<String, String> {
    let mut aliases = HashMap::new();

    for line in lines {
        for segment in line.split("->") {
            let trimmed = segment.trim();
            if let Some((id, label)) = extract_mermaid_alias(trimmed) {
                aliases.insert(id, label);
            }
        }
    }

    aliases
}

fn extract_mermaid_alias(value: &str) -> Option<(String, String)> {
    let trimmed = value.trim();
    let start = trimmed.find(['[', '{', '('])?;
    if start == 0 {
        if let Some(label) = extract_odd_shape_label(trimmed) {
            let marker = trimmed.find('>')?;
            let id = trimmed[..marker].trim();
            if !id.is_empty() {
                return Some((id.to_string(), cleanup_mermaid_label(&label)));
            }
        }
        return None;
    }

    let id = trimmed[..start].trim();
    if id.is_empty() {
        return None;
    }

    let label = extract_between(trimmed, '[', ']')
        .or_else(|| extract_between(trimmed, '{', '}'))
        .or_else(|| extract_between(trimmed, '(', ')'))?;

    Some((id.to_string(), cleanup_mermaid_label(&label)))
}

fn extract_odd_shape_label(value: &str) -> Option<String> {
    let marker = value.find('>')?;
    let end = value.rfind(']')?;
    if end <= marker {
        return None;
    }
    Some(value[marker + 1..end].to_string())
}

fn strip_pipe_label(line: &str) -> String {
    let Some(arrow_pos) = line.find("--") else {
        return line.to_string();
    };
    let after = &line[arrow_pos..];
    let Some(pipe_open) = after.find('|') else {
        return line.to_string();
    };
    let Some(pipe_close) = after[pipe_open + 1..].find('|') else {
        return line.to_string();
    };
    let label_end = arrow_pos + pipe_open + 1 + pipe_close + 1;
    format!("{}{}", &line[..arrow_pos + pipe_open], &line[label_end..])
}

fn strip_dash_label(line: &str) -> String {
    // `A -- label text --> B`  →  `A --> B`
    // Only strip when there's text between `--` and `--`
    let Some(pos) = line.find("--") else {
        return line.to_string();
    };
    let after = &line[pos + 2..];
    // If immediately followed by > or -, it's already a plain arrow
    if after.starts_with('>') || after.starts_with('-') {
        return line.to_string();
    }
    let Some(end) = after.find("--") else {
        return line.to_string();
    };
    // Replace `-- <label> --` with `--`
    format!("{}{}{}", &line[..pos], "--", &after[end + 2..])
}

fn normalize_mermaid_edge(line: &str) -> String {
    // Strip pipe labels: `-->|text|` → `-->`
    let s = strip_pipe_label(line);
    // Strip dash labels: `-- text -->` → `-->`  (collapse `-- ... --` to `--`)
    let s = strip_dash_label(&s);

    let mut normalized = sanitize_mermaid_line(&s);

    for pattern in ["==>", "-.->", "-->", "---", "--"] {
        normalized = normalized.replace(pattern, "->");
    }

    normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse `click NodeId href "url"` or `click NodeId "url"` lines.
/// Returns a map of display-label → url (resolves aliases).
fn parse_click_urls(source: &str, aliases: &HashMap<String, String>) -> HashMap<String, String> {
    let mut urls = HashMap::new();

    for line in source.lines() {
        let trimmed = line.trim();
        let rest = if let Some(r) = trimmed.strip_prefix("click ") {
            r
        } else {
            continue;
        };

        let mut parts = rest.splitn(3, ' ');
        let node_id = match parts.next() {
            Some(id) => id.trim(),
            None => continue,
        };

        let remainder = parts.collect::<Vec<_>>().join(" ");
        let url_raw = remainder
            .trim()
            .trim_start_matches("href")
            .trim()
            .trim_matches('"');

        if url_raw.starts_with("http://") || url_raw.starts_with("https://") {
            // Use display label as key if alias exists, else use node_id
            let key = aliases
                .get(node_id)
                .cloned()
                .unwrap_or_else(|| node_id.to_string());
            urls.insert(key, url_raw.to_string());
        }
    }

    urls
}

// ── Canvas renderer ───────────────────────────────────────────────────────────

fn render_mermaid_canvas_from_edges(
    edges: &[(String, String, Option<String>)],
    urls: &HashMap<String, String>,
    is_lr: bool,
) -> MermaidCanvas {
    let nodes_list = collect_nodes(edges);
    let levels = compute_levels(edges, &nodes_list);
    let grouped = group_by_level(&levels, &nodes_list);

    let node_layouts = nodes_list
        .iter()
        .map(|node| {
            let lines = wrap_label_lines(node, 24);
            let inner_width = lines
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(8)
                .clamp(8, 24);
            let width = inner_width + 4;
            let height = lines.len() + 2;
            (node.clone(), (width, height, lines))
        })
        .collect::<HashMap<_, _>>();

    // For LR: levels become columns (x-axis), nodes stack vertically per column.
    // For TD: levels become rows (y-axis), nodes spread horizontally per row.
    let x_gap = 4usize;
    let y_gap = 2usize;
    let left_margin = 2usize;
    let top_margin = 1usize;

    // Compute per-level dimensions
    let mut level_widths: BTreeMap<usize, usize> = BTreeMap::new();
    let mut level_heights: BTreeMap<usize, usize> = BTreeMap::new();
    for (level, level_nodes) in &grouped {
        let max_w = level_nodes
            .iter()
            .map(|n| node_layouts.get(n).map(|(w, _, _)| *w).unwrap_or(12))
            .max()
            .unwrap_or(12);
        let max_h = level_nodes
            .iter()
            .map(|n| node_layouts.get(n).map(|(_, h, _)| *h).unwrap_or(3))
            .max()
            .unwrap_or(3);
        let total_h = level_nodes
            .iter()
            .map(|n| node_layouts.get(n).map(|(_, h, _)| *h).unwrap_or(3))
            .sum::<usize>()
            + level_nodes.len().saturating_sub(1) * y_gap;
        let total_w = level_nodes
            .iter()
            .map(|n| node_layouts.get(n).map(|(w, _, _)| *w).unwrap_or(12))
            .sum::<usize>()
            + level_nodes.len().saturating_sub(1) * x_gap;
        level_widths.insert(*level, if is_lr { max_w } else { total_w });
        level_heights.insert(*level, if is_lr { total_h } else { max_h });
    }

    let max_level = levels.values().copied().max().unwrap_or(0);

    let (canvas_width, canvas_height) = if is_lr {
        let w = left_margin * 2
            + (0..=max_level)
                .map(|l| level_widths.get(&l).copied().unwrap_or(12) + x_gap)
                .sum::<usize>();
        let h = top_margin * 2
            + level_heights.values().copied().max().unwrap_or(12).max(12);
        (w.max(40), h.max(12))
    } else {
        let w = left_margin * 2
            + level_widths.values().copied().max().unwrap_or(40).max(40);
        let h = top_margin * 2
            + (0..=max_level)
                .map(|l| level_heights.get(&l).copied().unwrap_or(3) + y_gap)
                .sum::<usize>();
        (w, h.max(12))
    };

    let mut canvas = vec![vec![' '; canvas_width]; canvas_height];
    let mut positions: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();

    if is_lr {
        // Place nodes: each level is a column, nodes stack top-to-bottom within the column
        let total_height = canvas_height.saturating_sub(top_margin * 2);
        let mut current_x = left_margin;
        for level in 0..=max_level {
            let level_nodes = match grouped.get(&level) {
                Some(n) => n,
                None => continue,
            };
            let col_h = level_heights.get(&level).copied().unwrap_or(12);
            let col_w = level_widths.get(&level).copied().unwrap_or(12);
            // Center the stack vertically
            let stack_start_y =
                top_margin + total_height.saturating_sub(col_h) / 2;
            let mut cursor_y = stack_start_y;
            for node in level_nodes {
                let (width, height, lines) = node_layouts
                    .get(node)
                    .cloned()
                    .unwrap_or_else(|| (12, 3, vec![node.clone()]));
                draw_box(&mut canvas, current_x, cursor_y, width, &lines);
                positions.insert(node.clone(), (current_x, cursor_y, width, height));
                cursor_y += height + y_gap;
            }
            current_x += col_w + x_gap;
        }
    } else {
        // TD layout: each level is a row
        let mut current_y = top_margin;
        for (level, level_nodes) in &grouped {
            let content_width = level_widths.get(level).copied().unwrap_or(0);
            let start_x =
                left_margin + (canvas_width.saturating_sub(content_width + left_margin * 2)) / 2;
            let mut cursor_x = start_x;
            for node in level_nodes {
                let (width, height, lines) = node_layouts
                    .get(node)
                    .cloned()
                    .unwrap_or_else(|| (12, 3, vec![node.clone()]));
                draw_box(&mut canvas, cursor_x, current_y, width, &lines);
                positions.insert(node.clone(), (cursor_x, current_y, width, height));
                cursor_x += width + x_gap;
            }
            current_y += level_heights.get(level).copied().unwrap_or(3) + y_gap;
        }
    }

    // Draw connectors
    for (from, to, label) in edges {
        let Some(&(from_x, from_y, from_width, from_height)) = positions.get(from) else {
            continue;
        };
        let Some(&(to_x, to_y, to_width, _)) = positions.get(to) else {
            continue;
        };

        if is_lr {
            // Horizontal connector: exit right side of from, enter left side of to
            let start_x = from_x + from_width;
            let end_x = to_x;
            let from_mid_y = from_y + from_height / 2;
            let to_mid_y = to_y + (to_width.min(from_width)) / 2; // reuse to_y center
            let to_center_y = to_y + {
                let (_, h, _) = node_layouts.get(to).cloned().unwrap_or((12, 3, vec![]));
                h / 2
            };
            let mid_x = start_x + (end_x.saturating_sub(start_x)) / 2;

            if start_x < end_x {
                // Horizontal from from_mid_y to mid_x, then vertical to to_center_y, then horizontal to end_x
                draw_horizontal(&mut canvas, start_x, mid_x, from_mid_y);
                if from_mid_y != to_center_y {
                    let y_start = from_mid_y.min(to_center_y);
                    let y_end = from_mid_y.max(to_center_y);
                    draw_vertical(&mut canvas, mid_x, y_start, y_end);
                }
                draw_horizontal(&mut canvas, mid_x, end_x, to_center_y);
                if end_x > 0 {
                    put_char(&mut canvas, end_x.saturating_sub(1), to_center_y, '►');
                }
                if let Some(lbl) = label {
                    draw_edge_label(&mut canvas, start_x, mid_x, from_mid_y, lbl);
                }
            }
            let _ = (to_mid_y, mid_x); // suppress unused warnings
        } else {
            // TD connector: exit bottom of from, enter top of to
            let from_center = from_x + from_width / 2;
            let to_center = to_x + to_width / 2;
            let start_y = from_y + from_height.saturating_sub(1);
            let end_y = to_y;
            let mid_y = start_y + 1 + (end_y.saturating_sub(start_y + 1)) / 2;

            if start_y + 1 <= mid_y {
                draw_vertical(&mut canvas, from_center, start_y + 1, mid_y);
            }
            if from_center != to_center {
                draw_horizontal(&mut canvas, from_center, to_center, mid_y);
                if let Some(lbl) = label {
                    draw_edge_label(&mut canvas, from_center, to_center, mid_y, lbl);
                }
            }
            if mid_y + 1 <= end_y.saturating_sub(1) {
                draw_vertical(&mut canvas, to_center, mid_y + 1, end_y.saturating_sub(1));
            }
            if end_y > 0 {
                put_char(&mut canvas, to_center, end_y.saturating_sub(1), '▼');
            }
        }
    }

    fix_junctions(&mut canvas);

    // Build CanvasNode list in topological order (same order as nodes_list)
    let canvas_nodes = nodes_list
        .iter()
        .filter_map(|label| {
            let &(x, y, width, height) = positions.get(label)?;
            let url = urls.get(label).cloned();
            Some(CanvasNode {
                label: label.clone(),
                x,
                y,
                width,
                height,
                url,
            })
        })
        .collect::<Vec<_>>();

    MermaidCanvas {
        lines: trim_canvas(canvas),
        nodes: canvas_nodes,
    }
}

// ── Canvas drawing primitives ─────────────────────────────────────────────────

fn draw_box(canvas: &mut [Vec<char>], x: usize, y: usize, width: usize, lines: &[String]) {
    let inner = width.saturating_sub(2);
    let top = format!("┌{}┐", "─".repeat(inner));
    let bottom = format!("└{}┘", "─".repeat(inner));

    draw_text(canvas, x, y, &top);
    for (i, line) in lines.iter().enumerate() {
        draw_text(canvas, x, y + 1 + i, &box_line(width, line));
    }
    draw_text(canvas, x, y + 1 + lines.len(), &bottom);
}

fn box_line(width: usize, text: &str) -> String {
    let inner = width.saturating_sub(2);
    let content = fit_text(text, inner);
    let padding = inner.saturating_sub(content.chars().count());
    let left_pad = padding / 2;
    let right_pad = padding.saturating_sub(left_pad);
    format!("│{}{}{}│", " ".repeat(left_pad), content, " ".repeat(right_pad))
}

fn draw_vertical(canvas: &mut [Vec<char>], x: usize, y1: usize, y2: usize) {
    for y in y1..=y2 {
        put_char(canvas, x, y, '│');
    }
}

fn draw_horizontal(canvas: &mut [Vec<char>], x1: usize, x2: usize, y: usize) {
    let start = x1.min(x2);
    let end = x1.max(x2);
    for x in start..=end {
        put_char(canvas, x, y, '─');
    }
}

fn draw_edge_label(canvas: &mut [Vec<char>], x1: usize, x2: usize, y: usize, label: &str) {
    let start = x1.min(x2).saturating_add(1);
    let end = x1.max(x2);
    let available = end.saturating_sub(start);
    let chars: Vec<char> = label.chars().collect();
    let needed = chars.len() + 2; // space on each side
    if needed > available {
        return;
    }
    let mid_start = start + (available.saturating_sub(needed)) / 2;
    put_char(canvas, mid_start, y, ' ');
    for (i, ch) in chars.iter().enumerate() {
        put_char(canvas, mid_start + 1 + i, y, *ch);
    }
    put_char(canvas, mid_start + 1 + chars.len(), y, ' ');
}

/// Post-process the canvas: replace plain `│` and `─` connector chars with
/// appropriate Unicode junction chars (┬ ┴ ├ ┤ ┼ └ ┘ ┌ ┐) based on neighbors.
/// Skips chars that are adjacent to box corners to preserve box borders.
fn fix_junctions(canvas: &mut [Vec<char>]) {
    let height = canvas.len();
    if height == 0 {
        return;
    }
    let width = canvas[0].len();

    // We need a snapshot to avoid cascading edits
    let snapshot: Vec<Vec<char>> = canvas.to_vec();

    for y in 0..height {
        for x in 0..width {
            let ch = snapshot[y][x];
            if ch != '│' && ch != '─' {
                continue;
            }

            // Skip chars adjacent to box corners (part of box border)
            let neighbors = [
                if y > 0 { snapshot[y - 1][x] } else { ' ' },
                if y + 1 < height { snapshot[y + 1][x] } else { ' ' },
                if x > 0 { snapshot[y][x - 1] } else { ' ' },
                if x + 1 < width { snapshot[y][x + 1] } else { ' ' },
            ];
            if neighbors.iter().any(|&c| is_box_corner(c)) {
                continue;
            }

            let above = y > 0 && is_vert_connector(snapshot[y - 1][x]);
            let below = y + 1 < height && is_vert_connector(snapshot[y + 1][x]);
            let left = x > 0 && is_horiz_connector(snapshot[y][x - 1]);
            let right = x + 1 < width && is_horiz_connector(snapshot[y][x + 1]);

            canvas[y][x] = junction_char(above, below, left, right, ch);
        }
    }
}

fn is_box_corner(ch: char) -> bool {
    matches!(ch, '┌' | '┐' | '└' | '┘')
}

fn is_vert_connector(ch: char) -> bool {
    matches!(ch, '│' | '┬' | '┴' | '├' | '┤' | '┼')
}

fn is_horiz_connector(ch: char) -> bool {
    matches!(ch, '─' | '┬' | '┴' | '├' | '┤' | '┼')
}

fn junction_char(above: bool, below: bool, left: bool, right: bool, current: char) -> char {
    match (above, below, left, right) {
        (true, true, true, true) => '┼',
        (true, true, true, false) => '┤',
        (true, true, false, true) => '├',
        (false, true, true, true) => '┬',
        (true, false, true, true) => '┴',
        (true, false, true, false) => '┘',
        (true, false, false, true) => '└',
        (false, true, true, false) => '┐',
        (false, true, false, true) => '┌',
        (true, true, false, false) => '│',
        (false, false, true, true) => '─',
        _ => current,
    }
}

// ── Graph topology helpers ────────────────────────────────────────────────────

fn collect_nodes(edges: &[(String, String, Option<String>)]) -> Vec<String> {
    let mut nodes = Vec::new();
    for (from, to, _) in edges {
        if !nodes.contains(from) {
            nodes.push(from.clone());
        }
        if !nodes.contains(to) {
            nodes.push(to.clone());
        }
    }
    nodes
}

fn compute_levels(
    edges: &[(String, String, Option<String>)],
    nodes: &[String],
) -> HashMap<String, usize> {
    let mut incoming: HashMap<String, usize> = HashMap::new();
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    let mut levels: HashMap<String, usize> = HashMap::new();

    for node in nodes {
        incoming.insert(node.clone(), 0);
        levels.insert(node.clone(), 0);
    }

    for (from, to, _) in edges {
        outgoing.entry(from.clone()).or_default().push(to.clone());
        *incoming.entry(to.clone()).or_insert(0) += 1;
    }

    let mut queue = nodes
        .iter()
        .filter(|node| incoming.get(*node).copied().unwrap_or(0) == 0)
        .cloned()
        .collect::<Vec<_>>();

    while let Some(node) = queue.pop() {
        let current_level = levels.get(&node).copied().unwrap_or(0);
        if let Some(targets) = outgoing.get(&node) {
            for target in targets {
                let next_level = current_level + 1;
                let entry = levels.entry(target.clone()).or_insert(0);
                if next_level > *entry {
                    *entry = next_level;
                }
                if let Some(in_count) = incoming.get_mut(target) {
                    *in_count = in_count.saturating_sub(1);
                    if *in_count == 0 {
                        queue.push(target.clone());
                    }
                }
            }
        }
    }

    levels
}

fn group_by_level(
    levels: &HashMap<String, usize>,
    nodes: &[String],
) -> BTreeMap<usize, Vec<String>> {
    let mut grouped: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for node in nodes {
        let level = levels.get(node).copied().unwrap_or(0);
        grouped.entry(level).or_default().push(node.clone());
    }
    grouped
}

// ── Text helpers ──────────────────────────────────────────────────────────────

fn wrap_label_lines(value: &str, width: usize) -> Vec<String> {
    let max_width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in value.split_whitespace() {
        let word_len = word.chars().count();
        let current_len = current.chars().count();
        let needed = if current.is_empty() {
            word_len
        } else {
            current_len + 1 + word_len
        };

        if needed <= max_width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }

        if !current.is_empty() {
            lines.push(current.clone());
            current.clear();
        }

        if word_len <= max_width {
            current.push_str(word);
        } else {
            let mut chunk = String::new();
            for ch in word.chars() {
                chunk.push(ch);
                if chunk.chars().count() == max_width.saturating_sub(1) {
                    chunk.push('~');
                    lines.push(chunk.clone());
                    chunk.clear();
                }
            }
            current = chunk;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn fit_text(value: &str, width: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= width {
        return value.to_string();
    }

    if width <= 3 {
        return chars.into_iter().take(width).collect();
    }

    let mut text = chars.into_iter().take(width - 3).collect::<String>();
    text.push_str("...");
    text
}

fn draw_text(canvas: &mut [Vec<char>], x: usize, y: usize, text: &str) {
    for (index, ch) in text.chars().enumerate() {
        put_char(canvas, x + index, y, ch);
    }
}

fn put_char(canvas: &mut [Vec<char>], x: usize, y: usize, ch: char) {
    if let Some(row) = canvas.get_mut(y) {
        if let Some(cell) = row.get_mut(x) {
            *cell = ch;
        }
    }
}

fn trim_canvas(canvas: Vec<Vec<char>>) -> Vec<String> {
    let mut lines = canvas
        .into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect::<Vec<_>>();

    while lines
        .last()
        .map(|line| line.trim().is_empty())
        .unwrap_or(false)
    {
        lines.pop();
    }

    lines
        .into_iter()
        .map(|line| line.trim_end().to_string())
        .collect()
}
