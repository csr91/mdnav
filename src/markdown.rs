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
    MermaidPlaceholder,
}

#[derive(Clone, Debug)]
pub struct PreviewLine {
    pub text: String,
    pub kind: PreviewLineKind,
}

#[derive(Clone, Debug, Default)]
pub struct PreviewDocument {
    pub lines: Vec<PreviewLine>,
    pub links: Vec<LinkTarget>,
    pub mermaid_blocks: usize,
}

pub fn load_preview(path: &Path) -> Result<PreviewDocument> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("No se pudo leer {}", path.display()))?;
    Ok(render_preview(path, &content))
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
                    mermaid_blocks += 1;
                    lines.push(PreviewLine {
                        text: String::from("[ Mermaid diagram available ]"),
                        kind: PreviewLineKind::MermaidPlaceholder,
                    });
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
                }
                mermaid_block = false;
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
                        lines.push(PreviewLine {
                            text: format!("  {}", sanitize_mermaid_line(raw_line)),
                            kind: PreviewLineKind::MermaidPlaceholder,
                        });
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
        .replace('\t', "    ")
}
