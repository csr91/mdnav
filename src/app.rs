use std::{
    collections::BTreeSet,
    env,
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use arboard::Clipboard;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use reqwest::blocking::Client;
use serde_json::json;

use crate::{
    config::{config_path, AppConfig},
    docs::{collect_markdown_tree, parent_dir_if_within, DocItem},
    markdown::{load_preview, mermaid_terminal_canvas, MermaidBlock, PreviewDocument},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Focus {
    Tree,
    Preview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullscreenPanel {
    None,
    Tree,
    Preview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MermaidOutputMode {
    Terminal,
    Html,
    Web,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Overlay {
    None,
    Help,
    MermaidSelect,
    MermaidOutput,
    MermaidTerminalView,
    WebLink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpSection {
    Shortcuts,
    Settings,
}

pub struct App {
    pub root: PathBuf,
    pub items: Vec<DocItem>,
    pub selected_index: usize,
    pub current_file: Option<PathBuf>,
    pub preview: PreviewDocument,
    pub preview_scroll: usize,
    pub expanded_dirs: BTreeSet<PathBuf>,
    pub focus: Focus,
    pub fullscreen: FullscreenPanel,
    pub split_level: u8,
    pub overlay: Overlay,
    pub mermaid_selected_index: usize,
    pub mermaid_output_selected_index: usize,
    pub mermaid_active_index: usize,
    pub mermaid_terminal_canvas: Vec<String>,
    pub mermaid_canvas_x: usize,
    pub mermaid_canvas_y: usize,
    pub config: AppConfig,
    pub help_section: HelpSection,
    pub web_link_popup: Option<String>,
    pub running: bool,
    pub status: String,
}

impl App {
    pub fn new(root: PathBuf, config: AppConfig) -> Result<Self> {
        let mut expanded_dirs = BTreeSet::new();
        expanded_dirs.insert(root.clone());

        let items = collect_markdown_tree(&root, &expanded_dirs, config.only_mds)?;
        let selected_index = items.iter().position(|item| !item.is_dir).unwrap_or(0);
        let current_file = items
            .get(selected_index)
            .filter(|item| !item.is_dir)
            .map(|item| item.path.clone());
        let preview = if let Some(path) = &current_file {
            load_preview(path)?
        } else {
            PreviewDocument::default()
        };

        Ok(Self {
            root,
            items,
            selected_index,
            current_file,
            preview,
            preview_scroll: 0,
            expanded_dirs,
            focus: Focus::Tree,
            fullscreen: FullscreenPanel::None,
            split_level: 3,
            overlay: Overlay::None,
            mermaid_selected_index: 0,
            mermaid_output_selected_index: 0,
            mermaid_active_index: 0,
            mermaid_terminal_canvas: Vec::new(),
            mermaid_canvas_x: 0,
            mermaid_canvas_y: 0,
            config,
            help_section: HelpSection::Shortcuts,
            web_link_popup: None,
            running: true,
            status: String::from("Listo"),
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        if self.overlay != Overlay::None {
            return self.handle_overlay_key(key);
        }

        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('?') => self.toggle_help(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
            KeyCode::Char(')') => self.toggle_fullscreen(),
            KeyCode::Char('M') => self.open_mermaid_flow()?,
            KeyCode::Right | KeyCode::Enter => self.activate_selected()?,
            KeyCode::Left | KeyCode::Backspace => self.collapse_or_parent()?,
            KeyCode::Char('j') if self.focus == Focus::Preview => self.scroll_preview(1),
            KeyCode::Char('k') if self.focus == Focus::Preview => self.scroll_preview(-1),
            KeyCode::Char('.') if self.focus == Focus::Preview => self.scroll_preview(1),
            KeyCode::Char(',') if self.focus == Focus::Preview => self.scroll_preview(-1),
            KeyCode::Char('!') => self.set_split_level(1),
            KeyCode::Char('@') => self.set_split_level(2),
            KeyCode::Char('#') => self.set_split_level(3),
            KeyCode::Char('$') => self.set_split_level(4),
            KeyCode::Char('%') => self.set_split_level(5),
            _ => {}
        }

        Ok(())
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> Result<()> {
        match self.overlay {
            Overlay::Help => match key.code {
                KeyCode::Char('?') | KeyCode::Esc => self.close_overlay("Ayuda cerrada"),
                KeyCode::Left | KeyCode::Char('h') => self.help_section = HelpSection::Shortcuts,
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                    self.help_section = HelpSection::Settings
                }
                KeyCode::BackTab => self.help_section = HelpSection::Shortcuts,
                KeyCode::Enter | KeyCode::Char(' ') if self.help_section == HelpSection::Settings => {
                    self.toggle_only_mds()?;
                }
                _ => {}
            },
            Overlay::MermaidSelect => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_overlay("Seleccion Mermaid cancelada"),
                KeyCode::Up => {
                    self.mermaid_selected_index = self.mermaid_selected_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    let max_index = self.preview.mermaid_diagrams.len().saturating_sub(1);
                    self.mermaid_selected_index = (self.mermaid_selected_index + 1).min(max_index);
                }
                KeyCode::Enter => {
                    self.mermaid_active_index = self.mermaid_selected_index;
                    self.overlay = Overlay::MermaidOutput;
                    self.mermaid_output_selected_index = 0;
                    self.status = String::from("Elegi salida Mermaid");
                }
                _ => {}
            },
            Overlay::MermaidOutput => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_overlay("Salida Mermaid cancelada"),
                KeyCode::Up => {
                    self.mermaid_output_selected_index =
                        self.mermaid_output_selected_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.mermaid_output_selected_index =
                        (self.mermaid_output_selected_index + 1).min(2);
                }
                KeyCode::Enter => {
                    let mode = match self.mermaid_output_selected_index {
                        0 => MermaidOutputMode::Terminal,
                        1 => MermaidOutputMode::Html,
                        _ => MermaidOutputMode::Web,
                    };
                    self.open_mermaid_output(self.mermaid_active_index, mode)?;
                }
                _ => {}
            },
            Overlay::MermaidTerminalView => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('M') => {
                    self.close_overlay("Vista Mermaid cerrada")
                }
                KeyCode::Up | KeyCode::Char('k') => self.pan_mermaid(0, -1),
                KeyCode::Down | KeyCode::Char('j') => self.pan_mermaid(0, 1),
                KeyCode::Left | KeyCode::Char('h') => self.pan_mermaid(-4, 0),
                KeyCode::Right | KeyCode::Char('l') => self.pan_mermaid(4, 0),
                _ => {}
            },
            Overlay::WebLink => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.web_link_popup = None;
                    self.close_overlay("Popup de link cerrado");
                }
                _ => {}
            },
            Overlay::None => {}
        }

        Ok(())
    }

    fn toggle_help(&mut self) {
        if self.overlay == Overlay::Help {
            self.close_overlay("Ayuda cerrada");
        } else {
            self.overlay = Overlay::Help;
            self.help_section = HelpSection::Shortcuts;
            self.status = String::from("Ayuda abierta");
        }
    }

    fn close_overlay(&mut self, status: &str) {
        self.overlay = Overlay::None;
        self.status = String::from(status);
    }

    fn pan_mermaid(&mut self, dx: isize, dy: isize) {
        let max_x = self
            .mermaid_terminal_canvas
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let max_y = self.mermaid_terminal_canvas.len().saturating_sub(1);

        self.mermaid_canvas_x =
            ((self.mermaid_canvas_x as isize + dx).clamp(0, max_x as isize)) as usize;
        self.mermaid_canvas_y =
            ((self.mermaid_canvas_y as isize + dy).clamp(0, max_y as isize)) as usize;
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Tree => Focus::Preview,
            Focus::Preview => Focus::Tree,
        };

        if self.fullscreen != FullscreenPanel::None {
            self.fullscreen = match self.focus {
                Focus::Tree => FullscreenPanel::Tree,
                Focus::Preview => FullscreenPanel::Preview,
            };
        }

        self.status = format!("Foco: {:?}", self.focus);
    }

    fn toggle_fullscreen(&mut self) {
        let target = match self.focus {
            Focus::Tree => FullscreenPanel::Tree,
            Focus::Preview => FullscreenPanel::Preview,
        };

        self.fullscreen = if self.fullscreen == target {
            FullscreenPanel::None
        } else {
            target
        };

        self.status = match self.fullscreen {
            FullscreenPanel::None => String::from("Pantalla completa desactivada"),
            FullscreenPanel::Tree => String::from("Pantalla completa: navegacion"),
            FullscreenPanel::Preview => String::from("Pantalla completa: preview"),
        };
    }

    fn set_split_level(&mut self, level: u8) {
        self.split_level = level.clamp(1, 5);
        self.status = format!("Separacion ajustada: {}", self.split_level);
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() || self.focus != Focus::Tree {
            return;
        }

        let max_index = self.items.len().saturating_sub(1) as isize;
        let next = (self.selected_index as isize + delta).clamp(0, max_index) as usize;
        self.selected_index = next;

        if let Some(item) = self.items.get(self.selected_index) {
            self.status = item.relative.display().to_string();
        }
    }

    fn activate_selected(&mut self) -> Result<()> {
        let Some(item) = self.items.get(self.selected_index).cloned() else {
            return Ok(());
        };

        if item.is_dir {
            if self.expanded_dirs.contains(&item.path) {
                self.expanded_dirs.remove(&item.path);
                self.status = format!("Colapsada {}", item.relative.display());
            } else {
                self.expanded_dirs.insert(item.path.clone());
                self.status = format!("Expandida {}", item.relative.display());
            }
            self.reload_items()?;
        } else {
            self.open_file(item.path)?;
        }

        Ok(())
    }

    fn collapse_or_parent(&mut self) -> Result<()> {
        let Some(item) = self.items.get(self.selected_index).cloned() else {
            return Ok(());
        };

        if item.is_dir && self.expanded_dirs.contains(&item.path) {
            self.expanded_dirs.remove(&item.path);
            self.reload_items()?;
            self.status = format!("Colapsada {}", item.relative.display());
            return Ok(());
        }

        if let Some(parent) = parent_dir_if_within(&self.root, &item.path) {
            if let Some(index) = self.items.iter().position(|candidate| candidate.path == parent) {
                self.selected_index = index;
                self.status = format!("Padre {}", self.items[index].relative.display());
            }
        }

        Ok(())
    }

    fn scroll_preview(&mut self, delta: isize) {
        let max_scroll = self.preview.lines.len().saturating_sub(1) as isize;
        let next = (self.preview_scroll as isize + delta).clamp(0, max_scroll) as usize;
        self.preview_scroll = next;
    }

    fn reload_items(&mut self) -> Result<()> {
        let selected_path = self.items.get(self.selected_index).map(|item| item.path.clone());
        self.items = collect_markdown_tree(&self.root, &self.expanded_dirs, self.config.only_mds)?;

        if let Some(path) = selected_path {
            if let Some(index) = self.items.iter().position(|item| item.path == path) {
                self.selected_index = index;
            } else {
                self.selected_index = self.items.len().saturating_sub(1);
            }
        }

        Ok(())
    }

    fn toggle_only_mds(&mut self) -> Result<()> {
        self.config.only_mds = !self.config.only_mds;
        let path = self.config.save()?;
        self.reload_items()?;

        if let Some(current) = &self.current_file {
            if !self.items.iter().any(|item| &item.path == current) {
                self.current_file = None;
                self.preview = PreviewDocument::default();
                self.preview_scroll = 0;
            }
        }

        let display_path = config_path().unwrap_or(path);
        self.status = format!(
            "Only Mds: {} | {}",
            if self.config.only_mds { "on" } else { "off" },
            display_path.display()
        );
        Ok(())
    }

    fn open_file(&mut self, path: PathBuf) -> Result<()> {
        self.preview = load_preview(&path)?;
        self.preview_scroll = 0;
        self.current_file = Some(path.clone());
        self.overlay = Overlay::None;
        self.mermaid_selected_index = 0;
        self.mermaid_output_selected_index = 0;
        self.mermaid_active_index = 0;
        self.mermaid_terminal_canvas.clear();
        self.mermaid_canvas_x = 0;
        self.mermaid_canvas_y = 0;
        self.web_link_popup = None;
        let link_hint = self
            .preview
            .links
            .first()
            .map(|link| {
                let resolution = if link.resolved.is_some() {
                    "ok"
                } else {
                    "externo"
                };
                format!(" | primer link: {} -> {} ({resolution})", link.label, link.raw_target)
            })
            .unwrap_or_default();
        self.status = format!(
            "{} | links: {} | mermaid: {}{}",
            path.strip_prefix(&self.root).unwrap_or(&path).display(),
            self.preview.links.len(),
            self.preview.mermaid_blocks,
            link_hint
        );
        Ok(())
    }

    fn open_mermaid_flow(&mut self) -> Result<()> {
        match self.preview.mermaid_diagrams.len() {
            0 => {
                self.status = String::from("No hay Mermaid en el documento actual");
            }
            1 => {
                self.mermaid_active_index = 0;
                self.mermaid_output_selected_index = 0;
                self.overlay = Overlay::MermaidOutput;
                self.status = String::from("Elegi salida Mermaid");
            }
            _ => {
                self.overlay = Overlay::MermaidSelect;
                self.mermaid_selected_index = 0;
                self.status = String::from("Selecciona un Mermaid para abrir");
            }
        }

        Ok(())
    }

    fn open_mermaid_output(&mut self, index: usize, mode: MermaidOutputMode) -> Result<()> {
        let Some(diagram) = self.preview.mermaid_diagrams.get(index).cloned() else {
            return Ok(());
        };

        match mode {
            MermaidOutputMode::Terminal => {
                self.mermaid_terminal_canvas = mermaid_terminal_canvas(&diagram);
                self.mermaid_canvas_x = 0;
                self.mermaid_canvas_y = 0;
                self.overlay = Overlay::MermaidTerminalView;
                self.status = format!("Vista terminal Mermaid: {}", diagram.title);
            }
            MermaidOutputMode::Html => {
                let html_path = write_mermaid_temp_file(&diagram)?;
                let opened = open_in_browser(&html_path)?;
                self.overlay = Overlay::None;
                self.status = if opened {
                    format!("Mermaid abierto en navegador: {}", diagram.title)
                } else {
                    format!("Mermaid generado en: {}", html_path.display())
                };
            }
            MermaidOutputMode::Web => {
                let share_url = share_mermaid_via_web(&diagram)?;
                let opened = open_url_in_browser(&share_url)?;
                let copied = copy_to_clipboard(&share_url).unwrap_or(false);
                self.web_link_popup = Some(share_url.clone());
                self.overlay = Overlay::WebLink;
                self.status = if copied && opened {
                    String::from("Link web abierto y copiado")
                } else if copied {
                    String::from("Link web copiado")
                } else if opened {
                    format!("Link web abierto: {share_url}")
                } else {
                    format!("Link web Mermaid: {share_url}")
                };
            }
        }
        Ok(())
    }
}

fn share_mermaid_via_web(diagram: &MermaidBlock) -> Result<String> {
    let base_url = env::var("MDNAV_WEB_BASE_URL")
        .unwrap_or_else(|_| String::from("https://mdnav-web.vercel.app"));
    let trimmed_base = base_url.trim_end_matches('/');
    let hash = generate_share_hash();
    let api_url = format!("{trimmed_base}/api/diagrams/{hash}");

    let client = Client::new();
    let payload = json!({
        "mermaid": diagram.source,
        "title": diagram.title,
        "ttlSeconds": 3600
    });

    let mut request = client.post(&api_url).json(&payload);
    if let Ok(token) = env::var("MDNAV_WEB_WRITE_TOKEN") {
        if !token.trim().is_empty() {
            request = request.header("x-mdnav-token", token);
        }
    }

    let response = request.send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_else(|_| String::from("sin detalle"));
        return Err(anyhow::anyhow!("Error web Mermaid {status}: {body}"));
    }

    let body: serde_json::Value = response.json()?;
    let url = body
        .get("url")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{trimmed_base}/{hash}"));

    Ok(url)
}

fn generate_share_hash() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("mdnav-{timestamp:x}")
}

fn write_mermaid_temp_file(diagram: &MermaidBlock) -> Result<PathBuf> {
    let mut path = env::temp_dir();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    path.push(format!("mdnav-mermaid-{timestamp}.html"));

    let escaped_title = html_escape(&diagram.title);
    let html = format!(
        "<!doctype html>\
<html>\
<head>\
<meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{escaped_title}</title>\
<script type=\"module\">\
import mermaid from 'https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.esm.min.mjs';\
mermaid.initialize({{ startOnLoad: true, theme: 'dark' }});\
</script>\
<style>\
body {{ margin: 0; padding: 24px; background: #101418; color: #e6edf3; font-family: ui-monospace, SFMono-Regular, monospace; }}\
.frame {{ max-width: 1200px; margin: 0 auto; background: #161b22; border: 1px solid #30363d; border-radius: 14px; padding: 20px; }}\
h1 {{ font-size: 18px; margin-top: 0; color: #7cc7ff; }}\
.mermaid {{ background: #0d1117; border-radius: 12px; padding: 18px; overflow: auto; }}\
</style>\
</head>\
<body>\
<div class=\"frame\">\
<h1>{escaped_title}</h1>\
<pre class=\"mermaid\">{}</pre>\
</div>\
</body>\
</html>",
        html_escape(&diagram.source)
    );

    fs::write(&path, html)?;
    Ok(path)
}

fn open_in_browser(path: &PathBuf) -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.display().to_string()])
            .spawn()?;
        return Ok(true);
    }

    #[cfg(target_os = "linux")]
    {
        if env::var_os("DISPLAY").is_none() && env::var_os("WAYLAND_DISPLAY").is_none() {
            return Ok(false);
        }
        Command::new("xdg-open").arg(path).spawn()?;
        return Ok(true);
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
        return Ok(true);
    }

    #[allow(unreachable_code)]
    Ok(false)
}

fn open_url_in_browser(url: &str) -> Result<bool> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd").args(["/C", "start", "", url]).spawn()?;
        return Ok(true);
    }

    #[cfg(target_os = "linux")]
    {
        if env::var_os("DISPLAY").is_none() && env::var_os("WAYLAND_DISPLAY").is_none() {
            return Ok(false);
        }
        Command::new("xdg-open").arg(url).spawn()?;
        return Ok(true);
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?;
        return Ok(true);
    }

    #[allow(unreachable_code)]
    Ok(false)
}

fn copy_to_clipboard(value: &str) -> Result<bool> {
    let mut clipboard = match Clipboard::new() {
        Ok(clipboard) => clipboard,
        Err(_) => return Ok(false),
    };

    clipboard.set_text(value.to_string())?;
    Ok(true)
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
