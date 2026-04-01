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
    MermaidEdge,
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

pub fn mermaid_terminal_canvas(diagram: &MermaidBlock) -> Vec<String> {
    let raw_lines = diagram
        .source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    let aliases = collect_mermaid_aliases(&raw_lines);
    let edges = raw_lines
        .iter()
        .filter_map(|line| simplify_mermaid_edge(line, &aliases))
        .collect::<Vec<_>>();

    if edges.is_empty() {
        return vec![
            String::from("No diagram data"),
            String::new(),
            String::from("No pude detectar conexiones Mermaid en este bloque."),
        ];
    }

    render_mermaid_canvas_from_edges(&edges)
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

fn summarize_mermaid(source: &str) -> Vec<(PreviewLineKind, String)> {
    let raw_lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    let aliases = collect_mermaid_aliases(&raw_lines);
    let edges = raw_lines
        .iter()
        .filter_map(|line| simplify_mermaid_edge(line, &aliases))
        .collect::<Vec<_>>();

    if !edges.is_empty() {
        render_mermaid_ascii(source, &edges)
    } else if let Some(first) = raw_lines.first() {
        vec![(PreviewLineKind::MermaidEdge, sanitize_mermaid_line(first))]
    } else {
        Vec::new()
    }
}

fn simplify_mermaid_edge(line: &str, aliases: &HashMap<String, String>) -> Option<(String, String)> {
    if !line.contains("--") && !line.contains("->") {
        return None;
    }

    let normalized = normalize_mermaid_edge(line);

    let parts = normalized.split("->").map(str::trim).collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }

    let left = parse_mermaid_node(parts.first()?.trim(), aliases);
    let right = parse_mermaid_node(parts.last()?.trim(), aliases);
    Some((left, right))
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

fn normalize_mermaid_edge(line: &str) -> String {
    let mut normalized = sanitize_mermaid_line(line);

    for pattern in ["==>", "-.->", "-->", "---", "--"] {
        normalized = normalized.replace(pattern, "->");
    }

    normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_mermaid_ascii(source: &str, edges: &[(String, String)]) -> Vec<(PreviewLineKind, String)> {
    if is_left_to_right(source) {
        if let Some(lines) = render_split_merge_lr(edges) {
            return lines;
        }
    }

    if let Some(chain) = build_linear_chain(edges) {
        return render_linear_chain_boxes(&chain);
    }

    let mut lines = Vec::new();
    lines.push((
        PreviewLineKind::MermaidEdge,
        String::from("Complex diagram: terminal summary"),
    ));
    lines.push((PreviewLineKind::Normal, String::new()));
    for (left, right) in edges.iter().take(6) {
        lines.push((PreviewLineKind::MermaidEdge, format!("  {} -> {}", left, right)));
    }
    lines
}

fn is_left_to_right(source: &str) -> bool {
    source
        .lines()
        .next()
        .map(|line| line.contains("flowchart LR") || line.contains("graph LR"))
        .unwrap_or(false)
}

fn build_linear_chain(edges: &[(String, String)]) -> Option<Vec<String>> {
    if edges.is_empty() {
        return None;
    }

    for window in edges.windows(2) {
        if window[0].1 != window[1].0 {
            return None;
        }
    }

    let mut chain = Vec::new();
    chain.push(edges.first()?.0.clone());
    for (_, right) in edges {
        chain.push(right.clone());
    }
    Some(chain)
}

fn render_split_merge_lr(edges: &[(String, String)]) -> Option<Vec<(PreviewLineKind, String)>> {
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    let mut incoming: HashMap<String, usize> = HashMap::new();

    for (left, right) in edges {
        outgoing.entry(left.clone()).or_default().push(right.clone());
        *incoming.entry(right.clone()).or_insert(0) += 1;
        incoming.entry(left.clone()).or_insert(0);
    }

    let split = outgoing
        .iter()
        .find_map(|(node, targets)| (targets.len() == 2).then_some(node.clone()))?;
    let merge = incoming
        .iter()
        .find_map(|(node, count)| (*count == 2).then_some(node.clone()))?;

    let prefix = build_path_to(&split, edges)?;
    let suffix = build_path_from(&merge, edges);
    let branches = outgoing.get(&split)?.clone();

    if branches.len() != 2 {
        return None;
    }

    let top_branch = branches[0].clone();
    let bottom_branch = branches[1].clone();

    if !edge_exists(edges, &top_branch, &merge) || !edge_exists(edges, &bottom_branch, &merge) {
        return None;
    }

    let mut lines = Vec::new();
    let prefix_text = join_boxes(prefix.iter().cloned());
    let merge_box = boxed_label(&merge);
    let suffix_text = if suffix.is_empty() {
        String::new()
    } else {
        format!(" -> {}", join_boxes(suffix.into_iter()))
    };

    lines.push((
        PreviewLineKind::MermaidEdge,
        format!("{} -> {}", prefix_text, boxed_label(&split)),
    ));
    lines.push((
        PreviewLineKind::MermaidEdge,
        format!("{}  |-> {} --\\", " ".repeat(prefix_text.chars().count()), boxed_label(&top_branch)),
    ));
    lines.push((
        PreviewLineKind::MermaidEdge,
        format!(
            "{}  |              > {}{}",
            " ".repeat(prefix_text.chars().count()),
            merge_box,
            suffix_text
        ),
    ));
    lines.push((
        PreviewLineKind::MermaidEdge,
        format!("{}  |-> {} --/", " ".repeat(prefix_text.chars().count()), boxed_label(&bottom_branch)),
    ));

    Some(lines)
}

fn build_path_to(target: &str, edges: &[(String, String)]) -> Option<Vec<String>> {
    let mut reverse: HashMap<String, String> = HashMap::new();
    for (left, right) in edges {
        reverse.entry(right.clone()).or_insert_with(|| left.clone());
    }

    let mut path = Vec::new();
    let mut current = reverse.get(target)?.clone();
    path.push(current.clone());

    while let Some(prev) = reverse.get(&current) {
        current = prev.clone();
        path.push(current.clone());
    }

    path.reverse();
    Some(path)
}

fn build_path_from(start: &str, edges: &[(String, String)]) -> Vec<String> {
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    for (left, right) in edges {
        outgoing.entry(left.clone()).or_default().push(right.clone());
    }

    let mut path = Vec::new();
    let mut current = start.to_string();
    while let Some(next_nodes) = outgoing.get(&current) {
        if next_nodes.len() != 1 {
            break;
        }
        let next = next_nodes[0].clone();
        path.push(next.clone());
        current = next;
    }
    path
}

fn edge_exists(edges: &[(String, String)], from: &str, to: &str) -> bool {
    edges.iter().any(|(left, right)| left == from && right == to)
}

fn boxed_label(label: &str) -> String {
    format!("[ {} ]", label)
}

fn join_boxes<I>(labels: I) -> String
where
    I: IntoIterator<Item = String>,
{
    labels
        .into_iter()
        .map(|label| boxed_label(&label))
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn render_mermaid_canvas_from_edges(edges: &[(String, String)]) -> Vec<String> {
    let nodes = collect_nodes(edges);
    let levels = compute_levels(edges, &nodes);
    let grouped = group_by_level(&levels, &nodes);
    let node_layouts = nodes
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
    let x_gap = 4usize;
    let y_gap = 2usize;
    let left_margin = 2usize;
    let top_margin = 1usize;

    let mut level_widths = BTreeMap::new();
    let mut level_heights = BTreeMap::new();
    for (level, level_nodes) in &grouped {
        let content_width = level_nodes
            .iter()
            .map(|node| node_layouts.get(node).map(|(width, _, _)| *width).unwrap_or(12))
            .sum::<usize>()
            + level_nodes.len().saturating_sub(1) * x_gap;
        level_widths.insert(*level, content_width);

        let level_height = level_nodes
            .iter()
            .map(|node| node_layouts.get(node).map(|(_, height, _)| *height).unwrap_or(3))
            .max()
            .unwrap_or(3);
        level_heights.insert(*level, level_height);
    }

    let canvas_width = left_margin * 2 + level_widths.values().copied().max().unwrap_or(40).max(40);
    let max_level = levels.values().copied().max().unwrap_or(0);
    let content_height = (0..=max_level)
        .map(|level| level_heights.get(&level).copied().unwrap_or(3) + y_gap)
        .sum::<usize>();
    let canvas_height = top_margin * 2 + content_height;

    let mut canvas = vec![vec![' '; canvas_width]; canvas_height.max(12)];
    let mut positions: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();
    let mut current_y = top_margin;

    for (level, level_nodes) in grouped {
        let content_width = level_widths.get(&level).copied().unwrap_or(0);
        let start_x = left_margin + (canvas_width.saturating_sub(content_width + left_margin * 2)) / 2;
        let y = current_y;
        let mut cursor_x = start_x;

        for node in &level_nodes {
            let (width, height, lines) = node_layouts
                .get(node)
                .cloned()
                .unwrap_or_else(|| (12, 3, vec![node.clone()]));
            draw_box(&mut canvas, cursor_x, y, width, &lines);
            positions.insert(node.clone(), (cursor_x, y, width, height));
            cursor_x += width + x_gap;
        }

        current_y += level_heights.get(&level).copied().unwrap_or(3) + y_gap;
    }

    for (from, to) in edges {
        let Some(&(from_x, from_y, from_width, from_height)) = positions.get(from) else {
            continue;
        };
        let Some(&(to_x, to_y, to_width, _to_height)) = positions.get(to) else {
            continue;
        };

        let from_center = from_x + from_width / 2;
        let to_center = to_x + to_width / 2;
        let start_y = from_y + from_height.saturating_sub(1);
        let end_y = to_y;
        let mid_y = start_y + 1 + (end_y.saturating_sub(start_y + 1)) / 2;

        if start_y + 1 <= mid_y {
            draw_vertical(&mut canvas, from_center, start_y + 1, mid_y);
        }
        draw_horizontal(&mut canvas, from_center, to_center, mid_y);
        if mid_y + 1 <= end_y.saturating_sub(1) {
            draw_vertical(&mut canvas, to_center, mid_y + 1, end_y.saturating_sub(1));
        }
        put_char(&mut canvas, to_center, end_y.saturating_sub(1), 'v');
    }

    trim_canvas(canvas)
}

fn collect_nodes(edges: &[(String, String)]) -> Vec<String> {
    let mut nodes = Vec::new();
    for (from, to) in edges {
        if !nodes.contains(from) {
            nodes.push(from.clone());
        }
        if !nodes.contains(to) {
            nodes.push(to.clone());
        }
    }
    nodes
}

fn compute_levels(edges: &[(String, String)], nodes: &[String]) -> HashMap<String, usize> {
    let mut incoming: HashMap<String, usize> = HashMap::new();
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    let mut levels: HashMap<String, usize> = HashMap::new();

    for node in nodes {
        incoming.insert(node.clone(), 0);
        levels.insert(node.clone(), 0);
    }

    for (from, to) in edges {
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

fn group_by_level(levels: &HashMap<String, usize>, nodes: &[String]) -> BTreeMap<usize, Vec<String>> {
    let mut grouped: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for node in nodes {
        let level = levels.get(node).copied().unwrap_or(0);
        grouped.entry(level).or_default().push(node.clone());
    }
    grouped
}

fn draw_box_label(canvas: &mut [Vec<char>], x: usize, y: usize, width: usize, label: &str) {
    let lines = wrap_label_lines(label, width.saturating_sub(4).max(8));
    draw_box(canvas, x, y, width, &lines);
}

fn draw_box(canvas: &mut [Vec<char>], x: usize, y: usize, width: usize, lines: &[String]) {
    let top = format!("+{}+", "-".repeat(width.saturating_sub(2)));
    let bottom = top.clone();

    draw_text(canvas, x, y, &top);
    for (index, line) in lines.iter().enumerate() {
        draw_text(canvas, x, y + 1 + index, &box_line(width, line));
    }
    draw_text(canvas, x, y + 1 + lines.len(), &bottom);
}

fn box_line(width: usize, text: &str) -> String {
    let inner = width.saturating_sub(2);
    let content = fit_text(text, inner);
    let padding = inner.saturating_sub(content.chars().count());
    let left_pad = padding / 2;
    let right_pad = padding.saturating_sub(left_pad);
    format!("|{}{}{}|", " ".repeat(left_pad), content, " ".repeat(right_pad))
}

fn wrap_label_lines(value: &str, width: usize) -> Vec<String> {
    let max_width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in value.split_whitespace() {
        let word_len = word.chars().count();
        let current_len = current.chars().count();
        let needed = if current.is_empty() { word_len } else { current_len + 1 + word_len };

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

fn draw_vertical(canvas: &mut [Vec<char>], x: usize, y1: usize, y2: usize) {
    for y in y1..=y2 {
        put_char(canvas, x, y, '|');
    }
}

fn draw_horizontal(canvas: &mut [Vec<char>], x1: usize, x2: usize, y: usize) {
    let start = x1.min(x2);
    let end = x1.max(x2);
    for x in start..=end {
        if x == x1 || x == x2 {
            put_char(canvas, x, y, '+');
        } else {
            put_char(canvas, x, y, '-');
        }
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

    while lines.last().map(|line| line.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }

    lines.into_iter().map(|line| line.trim_end().to_string()).collect()
}

fn render_linear_chain_boxes(chain: &[String]) -> Vec<(PreviewLineKind, String)> {
    let max_width = chain
        .iter()
        .map(|node| node.chars().count())
        .max()
        .unwrap_or(0)
        .max(8);

    let inner_width = max_width + 2;
    let top = format!("+{}+", "-".repeat(inner_width));
    let bottom = top.clone();
    let arrow_pad = (top.len().saturating_sub(1)) / 2;

    let mut lines = Vec::new();
    for (index, node) in chain.iter().enumerate() {
        let padding = inner_width.saturating_sub(node.chars().count());
        let right_padding = padding.saturating_sub(1);
        let content = format!("| {}{}|", node, " ".repeat(right_padding));

        lines.push((PreviewLineKind::MermaidEdge, top.clone()));
        lines.push((PreviewLineKind::MermaidEdge, content));
        lines.push((PreviewLineKind::MermaidEdge, bottom.clone()));

        if index + 1 < chain.len() {
            lines.push((
                PreviewLineKind::MermaidEdge,
                format!("{}|", " ".repeat(arrow_pad)),
            ));
            lines.push((
                PreviewLineKind::MermaidEdge,
                format!("{}v", " ".repeat(arrow_pad)),
            ));
        }
    }

    lines
}
