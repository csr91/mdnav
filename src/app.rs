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
use crossterm::{
    event::{KeyCode, KeyEvent, KeyEventKind},
};
use reqwest::blocking::Client;
use serde_json::json;

use crate::{
    config::{config_path, AppConfig},
    docs::{collect_markdown_tree, parent_dir_if_within, DocItem},
    markdown::{load_preview, mermaid_terminal_canvas, MermaidBlock, MermaidCanvas, PreviewDocument},
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
    Search,
    Toc,
    CommandPalette,
    Find,
    Create,
    Git,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateKind {
    Folder,
    File,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateStep {
    ChooseKind,
    EnterName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitState {
    CommandList,
    Output,
    CommitInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpSection {
    Shortcuts,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewCursor {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionState {
    pub anchor: PreviewCursor,
    pub cursor: PreviewCursor,
    pub preferred_column: usize,
    pub previous_fullscreen: FullscreenPanel,
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
    pub mermaid_canvas: MermaidCanvas,
    pub mermaid_canvas_x: usize,
    pub mermaid_canvas_y: usize,
    pub mermaid_selected_node: Option<usize>,
    pub config: AppConfig,
    pub help_section: HelpSection,
    pub web_link_popup: Option<String>,
    pub selector_path: Option<PathBuf>,
    pub pending_cd: Option<PathBuf>,
    pub pending_external_edit: Option<PathBuf>,
    pub selection: Option<SelectionState>,
    pub running: bool,
    pub status: String,
    pub search_query: String,
    pub search_results: Vec<usize>, // indices into items
    pub search_cursor: usize,       // index into search_results
    pub toc_entries: Vec<(usize, String)>, // (line_index, heading text)
    pub toc_cursor: usize,
    pub preview_link_cursor: Option<usize>, // index into preview.links
    // Command palette
    pub palette_query: String,
    pub palette_cursor: usize,
    // Find in file
    pub find_query: String,
    pub find_results: Vec<usize>, // line indices in preview
    pub find_cursor: usize,
    // Create
    pub create_kind: CreateKind,
    pub create_name: String,
    pub create_step: CreateStep,
    // Git
    pub git_cursor: usize,
    pub git_output: Vec<String>,
    pub git_output_scroll: usize,
    pub git_available: bool,
    pub git_state: GitState,
    pub git_commit_input: String,
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
            mermaid_canvas: MermaidCanvas::default(),
            mermaid_canvas_x: 0,
            mermaid_canvas_y: 0,
            mermaid_selected_node: None,
            config,
            help_section: HelpSection::Shortcuts,
            web_link_popup: None,
            selector_path: None,
            pending_cd: None,
            pending_external_edit: None,
            selection: None,
            running: true,
            status: String::from("Listo"),
            search_query: String::new(),
            search_results: Vec::new(),
            search_cursor: 0,
            toc_entries: Vec::new(),
            toc_cursor: 0,
            preview_link_cursor: None,
            palette_query: String::new(),
            palette_cursor: 0,
            find_query: String::new(),
            find_results: Vec::new(),
            find_cursor: 0,
            create_kind: CreateKind::File,
            create_name: String::new(),
            create_step: CreateStep::ChooseKind,
            git_cursor: 0,
            git_output: Vec::new(),
            git_output_scroll: 0,
            git_available: git_is_available(),
            git_state: GitState::CommandList,
            git_commit_input: String::new(),
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        if self.overlay != Overlay::None {
            return self.handle_overlay_key(key);
        }

        if self.selection.is_some() {
            return self.handle_selection_key(key);
        }

        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('?') => self.toggle_help(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
            KeyCode::Char(')') => self.toggle_fullscreen(),
            KeyCode::Char('M') => self.open_mermaid_flow()?,
            KeyCode::Right => self.activate_selected()?,
            KeyCode::Enter if self.focus == Focus::Preview => self.follow_active_link()?,
            KeyCode::Enter => self.activate_selected()?,
            KeyCode::Left | KeyCode::Backspace => self.collapse_or_parent()?,
            KeyCode::Char('j') if self.focus == Focus::Preview => self.scroll_preview(1),
            KeyCode::Char('k') if self.focus == Focus::Preview => self.scroll_preview(-1),
            KeyCode::Char('.') if self.focus == Focus::Preview => self.scroll_preview(1),
            KeyCode::Char(',') if self.focus == Focus::Preview => self.scroll_preview(-1),
            KeyCode::PageDown if self.focus == Focus::Preview => self.scroll_preview(20),
            KeyCode::PageUp if self.focus == Focus::Preview => self.scroll_preview(-20),
            KeyCode::Char(']') if self.focus == Focus::Preview => self.move_link_cursor(1),
            KeyCode::Char('[') if self.focus == Focus::Preview => self.move_link_cursor(-1),
            KeyCode::Char('Y') => self.toggle_selection_mode(),
            KeyCode::Char('E') => self.edit_target_in_nano()?,
            KeyCode::Char('!') => self.set_split_level(1),
            KeyCode::Char('@') => self.set_split_level(2),
            KeyCode::Char('#') => self.set_split_level(3),
            KeyCode::Char('$') => self.set_split_level(4),
            KeyCode::Char('%') => self.set_split_level(5),
            KeyCode::Char('G') => self.queue_cd_to_target_dir(),
            KeyCode::Char('/') => self.open_command_palette(),
            KeyCode::Char('T') => self.open_toc(),
            _ => {}
        }

        Ok(())
    }

    fn handle_selection_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('Y') => self.exit_selection_mode(),
            KeyCode::Left => self.move_selection_cursor(-1, 0, key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)),
            KeyCode::Right => self.move_selection_cursor(1, 0, key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)),
            KeyCode::Up => self.move_selection_cursor(0, -1, key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)),
            KeyCode::Down => self.move_selection_cursor(0, 1, key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)),
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
                KeyCode::Tab => self.cycle_mermaid_node(true),
                KeyCode::BackTab => self.cycle_mermaid_node(false),
                KeyCode::Enter => self.open_selected_node_url()?,
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
            Overlay::Toc => match key.code {
                KeyCode::Esc | KeyCode::Char('T') | KeyCode::Char('q') => {
                    self.close_overlay("TOC cerrado")
                }
                KeyCode::Up => {
                    self.toc_cursor = self.toc_cursor.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.toc_cursor =
                        (self.toc_cursor + 1).min(self.toc_entries.len().saturating_sub(1));
                }
                KeyCode::Enter => self.jump_to_toc_entry(),
                _ => {}
            },
            Overlay::Search => match key.code {
                KeyCode::Esc => self.close_search(),
                KeyCode::Enter => self.confirm_search(),
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.update_search_results();
                }
                KeyCode::Down => self.move_search_cursor(1),
                KeyCode::Up => self.move_search_cursor(-1),
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.update_search_results();
                }
                _ => {}
            },
            Overlay::CommandPalette => match key.code {
                KeyCode::Esc => self.close_overlay("Palette cerrada"),
                KeyCode::Enter => self.confirm_palette_command(),
                KeyCode::Backspace => {
                    self.palette_query.pop();
                    self.update_palette_cursor();
                }
                KeyCode::Down => self.move_palette_cursor(1),
                KeyCode::Up => self.move_palette_cursor(-1),
                KeyCode::Char(c) => {
                    self.palette_query.push(c);
                    self.update_palette_cursor();
                }
                _ => {}
            },
            Overlay::Find => match key.code {
                KeyCode::Esc => self.close_overlay("Búsqueda en archivo cerrada"),
                KeyCode::Enter => self.confirm_find(),
                KeyCode::Backspace => {
                    self.find_query.pop();
                    self.update_find_results();
                }
                KeyCode::Down => self.move_find_cursor(1),
                KeyCode::Up => self.move_find_cursor(-1),
                KeyCode::Char(c) => {
                    self.find_query.push(c);
                    self.update_find_results();
                }
                _ => {}
            },
            Overlay::Create => match key.code {
                KeyCode::Esc => self.close_overlay("Crear cancelado"),
                KeyCode::Up | KeyCode::Down if self.create_step == CreateStep::ChooseKind => {
                    self.create_kind = match self.create_kind {
                        CreateKind::Folder => CreateKind::File,
                        CreateKind::File => CreateKind::Folder,
                    };
                }
                KeyCode::Enter if self.create_step == CreateStep::ChooseKind => {
                    self.create_step = CreateStep::EnterName;
                    self.create_name.clear();
                    self.status = match self.create_kind {
                        CreateKind::Folder => String::from("Nombre de la carpeta:"),
                        CreateKind::File => String::from("Nombre del archivo:"),
                    };
                }
                KeyCode::Enter if self.create_step == CreateStep::EnterName => {
                    self.confirm_create();
                }
                KeyCode::Backspace if self.create_step == CreateStep::EnterName => {
                    self.create_name.pop();
                }
                KeyCode::Char(c) if self.create_step == CreateStep::EnterName => {
                    self.create_name.push(c);
                }
                _ => {}
            },
            Overlay::Git => match self.git_state {
                GitState::CommandList => match key.code {
                    KeyCode::Esc => self.close_overlay("Git cerrado"),
                    KeyCode::Up => self.move_git_cursor(-1),
                    KeyCode::Down => self.move_git_cursor(1),
                    KeyCode::Enter => self.run_git_command(),
                    _ => {}
                },
                GitState::Output => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.git_state = GitState::CommandList;
                        self.status = String::from("Git: elige un comando");
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.git_output_scroll = self.git_output_scroll.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.git_output_scroll =
                            (self.git_output_scroll + 1).min(self.git_output.len().saturating_sub(1));
                    }
                    _ => {}
                },
                GitState::CommitInput => match key.code {
                    KeyCode::Esc => {
                        self.git_state = GitState::CommandList;
                        self.git_commit_input.clear();
                        self.status = String::from("Commit cancelado");
                    }
                    KeyCode::Enter => self.run_git_commit(),
                    KeyCode::Backspace => {
                        self.git_commit_input.pop();
                    }
                    KeyCode::Char(c) => {
                        self.git_commit_input.push(c);
                    }
                    _ => {}
                },
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
            .mermaid_canvas
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let max_y = self.mermaid_canvas.lines.len().saturating_sub(1);

        self.mermaid_canvas_x =
            ((self.mermaid_canvas_x as isize + dx).clamp(0, max_x as isize)) as usize;
        self.mermaid_canvas_y =
            ((self.mermaid_canvas_y as isize + dy).clamp(0, max_y as isize)) as usize;
    }

    fn cycle_mermaid_node(&mut self, forward: bool) {
        let n = self.mermaid_canvas.nodes.len();
        if n == 0 {
            return;
        }
        self.mermaid_selected_node = Some(match self.mermaid_selected_node {
            None => {
                if forward {
                    0
                } else {
                    n - 1
                }
            }
            Some(i) => {
                if forward {
                    (i + 1) % n
                } else if i == 0 {
                    n - 1
                } else {
                    i - 1
                }
            }
        });
        self.scroll_to_selected_node();
        if let Some(idx) = self.mermaid_selected_node {
            if let Some(node) = self.mermaid_canvas.nodes.get(idx) {
                self.status = if node.url.is_some() {
                    format!("Nodo: {} [Enter para abrir link]", node.label)
                } else {
                    format!("Nodo: {}", node.label)
                };
            }
        }
    }

    fn scroll_to_selected_node(&mut self) {
        let Some(idx) = self.mermaid_selected_node else {
            return;
        };
        let Some(node) = self.mermaid_canvas.nodes.get(idx) else {
            return;
        };
        // Scroll so the node is visible with a small margin
        let margin_x = 4usize;
        let margin_y = 2usize;
        if node.x < self.mermaid_canvas_x + margin_x {
            self.mermaid_canvas_x = node.x.saturating_sub(margin_x);
        }
        if node.y < self.mermaid_canvas_y + margin_y {
            self.mermaid_canvas_y = node.y.saturating_sub(margin_y);
        }
        // Rough right/bottom bound (assume ~80 cols, ~22 rows viewport)
        if node.x + node.width > self.mermaid_canvas_x + 72 {
            self.mermaid_canvas_x = node.x + node.width + margin_x;
        }
        if node.y + node.height > self.mermaid_canvas_y + 18 {
            self.mermaid_canvas_y = node.y + node.height + margin_y;
        }
    }

    fn open_selected_node_url(&mut self) -> Result<()> {
        let Some(idx) = self.mermaid_selected_node else {
            self.status = String::from("Ningún nodo seleccionado");
            return Ok(());
        };
        let Some(node) = self.mermaid_canvas.nodes.get(idx).cloned() else {
            return Ok(());
        };
        let Some(url) = node.url.clone() else {
            self.status = format!("El nodo '{}' no tiene link", node.label);
            return Ok(());
        };
        let opened = open_url_in_browser(&url)?;
        let copied = copy_to_clipboard(&url).unwrap_or(false);
        self.status = if opened && copied {
            format!("Link abierto y copiado: {url}")
        } else if opened {
            format!("Link abierto: {url}")
        } else {
            format!("Link: {url}")
        };
        Ok(())
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

    fn selected_item_path(&self) -> Option<PathBuf> {
        self.items
            .get(self.selected_index)
            .map(|item| item.path.clone())
    }

    fn action_target_path(&self) -> Option<PathBuf> {
        if self.focus == Focus::Tree {
            self.selected_item_path()
        } else {
            self.current_file
                .clone()
                .or_else(|| self.selected_item_path())
        }
    }

    fn toggle_selection_mode(&mut self) {
        if self.selection.is_some() {
            self.exit_selection_mode();
            return;
        }

        if self.focus != Focus::Preview {
            self.status = String::from("Shift+Y funciona con foco en Preview");
            return;
        }

        if self.preview.lines.is_empty() {
            self.status = String::from("No hay contenido para seleccionar");
            return;
        }

        let line = self.preview_scroll.min(self.preview.lines.len().saturating_sub(1));
        let column = 0;
        let cursor = PreviewCursor { line, column };
        self.selection = Some(SelectionState {
            anchor: cursor,
            cursor,
            preferred_column: column,
            previous_fullscreen: self.fullscreen,
        });
        self.fullscreen = FullscreenPanel::Preview;
        self.status = String::from("Modo seleccion activo");
    }

    fn edit_target_in_nano(&mut self) -> Result<()> {
        let Some(target) = self.action_target_path() else {
            self.status = String::from("No hay archivo para editar");
            return Ok(());
        };

        if target.is_dir() {
            self.status = String::from("Shift+E solo abre archivos");
            return Ok(());
        }

        self.pending_external_edit = Some(target);
        self.running = false;
        self.status = String::from("Relanzando mdnav despues de nano");
        Ok(())
    }

    pub fn restore_path_focus(&mut self, path: &std::path::Path) -> Result<()> {
        let mut current = path.parent().map(|parent| parent.to_path_buf());
        while let Some(dir) = current {
            if dir.starts_with(&self.root) {
                self.expanded_dirs.insert(dir.clone());
                current = dir.parent().map(|parent| parent.to_path_buf());
            } else {
                break;
            }
        }

        self.reload_items()?;

        if let Some(index) = self.items.iter().position(|item| item.path == path) {
            self.selected_index = index;
        }

        if path.is_file() {
            self.open_file(path.to_path_buf())?;
        } else {
            self.status = format!("Reabierto en {}", self.relative_label(path));
        }

        Ok(())
    }

    fn queue_cd_to_target_dir(&mut self) {
        let Some(target) = self.action_target_path() else {
            self.status = String::from("No hay item para preparar cd");
            return;
        };

        let dir = if target.is_dir() {
            target
        } else if let Some(parent) = target.parent() {
            parent.to_path_buf()
        } else {
            self.status = String::from("No se pudo resolver el directorio");
            return;
        };

        let label = self.relative_label(&dir);
        self.pending_cd = Some(dir);
        self.status = format!("Directorio pendiente para salir: {label}");
    }

    fn relative_label(&self, path: &std::path::Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
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

    fn exit_selection_mode(&mut self) {
        if let Some(selection) = self.selection.take() {
            self.fullscreen = selection.previous_fullscreen;
            self.status = String::from("Modo seleccion cerrado");
        }
    }

    fn move_selection_cursor(&mut self, dx: isize, dy: isize, extend: bool) {
        let Some(mut selection) = self.selection else {
            return;
        };

        let mut line = selection.cursor.line as isize + dy;
        line = line.clamp(0, self.preview.lines.len().saturating_sub(1) as isize);
        let line = line as usize;

        let line_len = self.preview_line_len(line);
        let column = if dy != 0 {
            selection.preferred_column.min(line_len)
        } else {
            (selection.cursor.column as isize + dx).clamp(0, line_len as isize) as usize
        };

        selection.cursor = PreviewCursor { line, column };
        selection.preferred_column = column;

        if !extend {
            selection.anchor = selection.cursor;
        }

        self.selection = Some(selection);
        self.ensure_selection_visible();
        self.status = if self.has_selected_text() {
            String::from("Seleccion extendida")
        } else {
            String::from("Cursor de seleccion")
        };
    }

    fn ensure_selection_visible(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };

        let line = selection.cursor.line;
        if line < self.preview_scroll {
            self.preview_scroll = line;
        } else {
            let bottom_margin = 12usize;
            if line >= self.preview_scroll.saturating_add(bottom_margin) {
                self.preview_scroll = line.saturating_sub(bottom_margin.saturating_sub(1));
            }
        }
    }

    fn preview_line_len(&self, line: usize) -> usize {
        self.preview
            .lines
            .get(line)
            .map(|preview_line| preview_line.text.chars().count())
            .unwrap_or(0)
    }

    fn has_selected_text(&self) -> bool {
        self.selection
            .map(|selection| selection.anchor != selection.cursor)
            .unwrap_or(false)
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
        self.mermaid_canvas = MermaidCanvas::default();
        self.mermaid_canvas_x = 0;
        self.mermaid_canvas_y = 0;
        self.mermaid_selected_node = None;
        self.web_link_popup = None;
        self.preview_link_cursor = None;
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
                self.mermaid_canvas = mermaid_terminal_canvas(&diagram);
                self.mermaid_canvas_x = 0;
                self.mermaid_canvas_y = 0;
                self.mermaid_selected_node = None;
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

    fn move_link_cursor(&mut self, delta: isize) {
        let n = self.preview.links.len();
        if n == 0 {
            self.status = String::from("No hay links en este archivo");
            return;
        }
        self.preview_link_cursor = Some(match self.preview_link_cursor {
            None => {
                if delta > 0 { 0 } else { n - 1 }
            }
            Some(i) => ((i as isize + delta).rem_euclid(n as isize)) as usize,
        });
        if let Some(idx) = self.preview_link_cursor {
            if let Some(link) = self.preview.links.get(idx) {
                self.preview_scroll = link.line_index;
                let kind = if link.resolved.is_some() { "interno" } else { "externo" };
                self.status = format!(
                    "Link {}/{}: {} → {} ({kind})  Enter=abrir",
                    idx + 1, n, link.label, link.raw_target
                );
            }
        }
    }

    fn follow_active_link(&mut self) -> Result<()> {
        let Some(idx) = self.preview_link_cursor else {
            return self.activate_selected();
        };
        let Some(link) = self.preview.links.get(idx).cloned() else {
            return Ok(());
        };

        if let Some(resolved) = link.resolved {
            let mut current = resolved.parent().map(|p| p.to_path_buf());
            while let Some(dir) = current {
                if dir.starts_with(&self.root) {
                    self.expanded_dirs.insert(dir.clone());
                    current = dir.parent().map(|p| p.to_path_buf());
                } else {
                    break;
                }
            }
            self.reload_items()?;
            if let Some(index) = self.items.iter().position(|item| item.path == resolved) {
                self.selected_index = index;
            }
            self.open_file(resolved)?;
        } else {
            open_url_in_browser(&link.raw_target)?;
            self.status = format!("Link externo abierto: {}", link.raw_target);
        }
        Ok(())
    }

    fn open_toc(&mut self) {
        use crate::markdown::PreviewLineKind;
        self.toc_entries = self
            .preview
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, line)| {
                if let PreviewLineKind::Heading(_) = line.kind {
                    Some((i, line.text.clone()))
                } else {
                    None
                }
            })
            .collect();

        if self.toc_entries.is_empty() {
            self.status = String::from("No hay headings en este archivo");
            return;
        }
        self.toc_cursor = 0;
        self.overlay = Overlay::Toc;
        self.status = format!("{} headings encontrados", self.toc_entries.len());
    }

    fn jump_to_toc_entry(&mut self) {
        if let Some(&(line_index, _)) = self.toc_entries.get(self.toc_cursor) {
            self.preview_scroll = line_index;
            self.focus = Focus::Preview;
        }
        self.close_overlay("TOC: saltando a heading");
    }

    fn open_search(&mut self) {
        self.search_query.clear();
        self.search_results.clear();
        self.search_cursor = 0;
        self.overlay = Overlay::Search;
        self.status = String::from("Buscar: escribe para filtrar");
    }

    // ── Command Palette ───────────────────────────────────────────────────────

    fn open_command_palette(&mut self) {
        self.palette_query.clear();
        self.palette_cursor = 0;
        self.overlay = Overlay::CommandPalette;
        self.status = String::from("Palette: escribe para filtrar");
    }

    pub fn palette_commands(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("files", "buscar archivo en el árbol"),
            ("find", "buscar texto en el archivo actual"),
            ("create", "crear carpeta o archivo"),
            ("git", "ejecutar comandos git"),
        ]
    }

    pub fn palette_filtered(&self) -> Vec<(&'static str, &'static str)> {
        let q = self.palette_query.to_lowercase();
        self.palette_commands()
            .into_iter()
            .filter(|(name, desc)| {
                q.is_empty() || name.contains(q.as_str()) || desc.contains(q.as_str())
            })
            .collect()
    }

    fn update_palette_cursor(&mut self) {
        let n = self.palette_filtered().len();
        if self.palette_cursor >= n && n > 0 {
            self.palette_cursor = n - 1;
        }
    }

    fn move_palette_cursor(&mut self, delta: isize) {
        let n = self.palette_filtered().len();
        if n == 0 {
            return;
        }
        self.palette_cursor =
            ((self.palette_cursor as isize + delta).rem_euclid(n as isize)) as usize;
    }

    fn confirm_palette_command(&mut self) {
        let filtered = self.palette_filtered();
        let Some(&(name, _)) = filtered.get(self.palette_cursor) else {
            return;
        };
        match name {
            "files" => self.open_search(),
            "find" => self.open_find(),
            "create" => self.open_create(),
            "git" => self.open_git(),
            _ => {}
        }
    }

    // ── Find in file ──────────────────────────────────────────────────────────

    fn open_find(&mut self) {
        self.find_query.clear();
        self.find_results.clear();
        self.find_cursor = 0;
        self.overlay = Overlay::Find;
        self.status = String::from("Find: escribe para buscar en el archivo");
    }

    fn update_find_results(&mut self) {
        let q = self.find_query.to_lowercase();
        if q.is_empty() {
            self.find_results.clear();
        } else {
            self.find_results = self
                .preview
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.text.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        self.find_cursor = 0;
        self.status = format!(
            "Find: \"{}\" — {} resultado(s)",
            self.find_query,
            self.find_results.len()
        );
    }

    fn move_find_cursor(&mut self, delta: isize) {
        let n = self.find_results.len();
        if n == 0 {
            return;
        }
        self.find_cursor = ((self.find_cursor as isize + delta).rem_euclid(n as isize)) as usize;
    }

    fn confirm_find(&mut self) {
        if let Some(&line_index) = self.find_results.get(self.find_cursor) {
            self.preview_scroll = line_index;
            self.focus = Focus::Preview;
        }
        self.close_overlay("Find: saltando a resultado");
    }

    // ── Create folder/file ────────────────────────────────────────────────────

    fn open_create(&mut self) {
        self.create_kind = CreateKind::File;
        self.create_name.clear();
        self.create_step = CreateStep::ChooseKind;
        self.overlay = Overlay::Create;
        self.status = String::from("Crear: elige el tipo con ↑↓, Enter para confirmar");
    }

    fn current_tree_dir(&self) -> Option<PathBuf> {
        let item = self.items.get(self.selected_index)?;
        if item.is_dir {
            Some(item.path.clone())
        } else {
            item.path.parent().map(|p| p.to_path_buf())
        }
    }

    fn confirm_create(&mut self) {
        let name = self.create_name.trim().to_string();
        if name.is_empty() {
            self.status = String::from("Nombre vacío, cancelado");
            self.overlay = Overlay::None;
            return;
        }
        let Some(dir) = self.current_tree_dir() else {
            self.status = String::from("No se pudo determinar el directorio");
            self.overlay = Overlay::None;
            return;
        };
        let target = dir.join(&name);
        let result = match self.create_kind {
            CreateKind::Folder => fs::create_dir_all(&target),
            CreateKind::File => {
                if let Some(parent) = target.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&target, "").map(|_| ())
            }
        };
        match result {
            Ok(()) => {
                self.expanded_dirs.insert(dir);
                let _ = self.reload_items();
                if target.is_file() {
                    if let Some(index) = self.items.iter().position(|item| item.path == target) {
                        self.selected_index = index;
                    }
                    let _ = self.open_file(target.clone());
                } else if let Some(index) = self.items.iter().position(|item| item.path == target) {
                    self.selected_index = index;
                    self.overlay = Overlay::None;
                } else {
                    self.overlay = Overlay::None;
                }
                let kind_label = match self.create_kind {
                    CreateKind::Folder => "Carpeta",
                    CreateKind::File => "Archivo",
                };
                self.status = format!("{kind_label} creado: {name}");
            }
            Err(e) => {
                self.status = format!("Error al crear: {e}");
                self.overlay = Overlay::None;
            }
        }
    }

    // ── Git ───────────────────────────────────────────────────────────────────

    fn open_git(&mut self) {
        if !self.git_available {
            self.status = String::from("git no está disponible en PATH");
            self.overlay = Overlay::None;
            return;
        }
        self.git_cursor = 0;
        self.git_state = GitState::CommandList;
        self.git_output.clear();
        self.git_output_scroll = 0;
        self.overlay = Overlay::Git;
        self.status = String::from("Git: elige un comando");
    }

    pub fn git_commands() -> &'static [(&'static str, &'static str, &'static [&'static str])] {
        &[
            ("status",    "git status",             &["status"] as &[&str]),
            ("log",       "git log --oneline -20",  &["log", "--oneline", "-20"]),
            ("diff",      "git diff",               &["diff"]),
            ("add .",     "git add .",              &["add", "."]),
            ("commit",    "git commit (pide mensaje)", &[]),
            ("pull",      "git pull",               &["pull"]),
            ("push",      "git push",               &["push"]),
            ("branch",    "git branch",             &["branch"]),
            ("stash",     "git stash",              &["stash"]),
            ("stash pop", "git stash pop",          &["stash", "pop"]),
        ]
    }

    fn move_git_cursor(&mut self, delta: isize) {
        let n = Self::git_commands().len() as isize;
        self.git_cursor = ((self.git_cursor as isize + delta).rem_euclid(n)) as usize;
    }

    fn run_git_command(&mut self) {
        let cmds = Self::git_commands();
        let Some(&(name, _, args)) = cmds.get(self.git_cursor) else {
            return;
        };
        if name == "commit" {
            self.git_commit_input.clear();
            self.git_state = GitState::CommitInput;
            self.status = String::from("Mensaje de commit: (Enter para confirmar)");
            return;
        }
        let work_dir = self.root.clone();
        match Command::new("git")
            .args(args)
            .current_dir(&work_dir)
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let combined = if stderr.trim().is_empty() {
                    stdout
                } else if stdout.trim().is_empty() {
                    stderr
                } else {
                    format!("{stdout}\n{stderr}")
                };
                self.git_output = combined
                    .lines()
                    .map(|l| l.to_string())
                    .collect();
                if self.git_output.is_empty() {
                    self.git_output = vec![String::from("(sin salida)")];
                }
                self.git_output_scroll = 0;
                self.git_state = GitState::Output;
                self.status = format!("git {name}  (Esc para volver)");
            }
            Err(e) => {
                self.status = format!("Error ejecutando git: {e}");
            }
        }
    }

    fn run_git_commit(&mut self) {
        let msg = self.git_commit_input.trim().to_string();
        if msg.is_empty() {
            self.status = String::from("Mensaje vacío, commit cancelado");
            self.git_state = GitState::CommandList;
            return;
        }
        let work_dir = self.root.clone();
        match Command::new("git")
            .args(["commit", "-m", &msg])
            .current_dir(&work_dir)
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let combined = if stderr.trim().is_empty() { stdout } else { format!("{stdout}\n{stderr}") };
                self.git_output = combined.lines().map(|l| l.to_string()).collect();
                if self.git_output.is_empty() {
                    self.git_output = vec![String::from("(sin salida)")];
                }
                self.git_output_scroll = 0;
                self.git_commit_input.clear();
                self.git_state = GitState::Output;
                self.status = String::from("git commit  (Esc para volver)");
            }
            Err(e) => {
                self.status = format!("Error en commit: {e}");
                self.git_state = GitState::CommandList;
            }
        }
    }

    fn update_search_results(&mut self) {
        let query = self.search_query.to_lowercase();
        self.search_results = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.name.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        self.search_cursor = 0;
        self.status = format!(
            "Buscar: \"{}\" — {} resultado(s)",
            self.search_query,
            self.search_results.len()
        );
    }

    fn move_search_cursor(&mut self, delta: isize) {
        if self.search_results.is_empty() {
            return;
        }
        let n = self.search_results.len() as isize;
        self.search_cursor =
            ((self.search_cursor as isize + delta).rem_euclid(n)) as usize;
    }

    fn confirm_search(&mut self) {
        if let Some(&item_index) = self.search_results.get(self.search_cursor) {
            self.selected_index = item_index;
            self.focus = Focus::Tree;
        }
        self.close_search();
    }

    fn close_search(&mut self) {
        self.search_query.clear();
        self.search_results.clear();
        self.overlay = Overlay::None;
        self.status = String::from("Busqueda cerrada");
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

fn git_is_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
