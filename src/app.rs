use std::{
    collections::{BTreeSet, HashMap},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use reqwest::blocking::Client;
use serde_json::json;

use crate::{
    config::{config_path, AppConfig, TreeInfoMode, TreeSortMode},
    docs::{
        collect_dir_tree, collect_markdown_tree, copy_path, has_subdirs, move_path,
        parent_dir_if_within, DocItem,
    },
    markdown::{
        load_preview, mermaid_terminal_canvas, MermaidBlock, MermaidCanvas, PreviewDocument,
    },
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
    Rename,
    DestPicker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileOpKind {
    Move,
    Copy,
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
pub enum GitStatusKind {
    Ignored,
    Untracked,
    Modified,
    Staged,
    Renamed,
    Deleted,
    Conflicted,
}

impl GitStatusKind {
    fn priority(self) -> u8 {
        match self {
            Self::Ignored => 1,
            Self::Untracked => 2,
            Self::Modified => 3,
            Self::Staged => 4,
            Self::Renamed => 5,
            Self::Deleted => 6,
            Self::Conflicted => 7,
        }
    }
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
    pub anchored: bool,
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
    // Move / copy destination picker
    pub file_op_kind: Option<FileOpKind>,
    pub file_op_source: Option<PathBuf>,
    pub picker_dirs: Vec<DocItem>, // currently visible directory nodes
    pub picker_expanded: BTreeSet<PathBuf>, // expanded directories in the picker
    pub picker_cursor: usize,      // index into picker_dirs
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
    // Rename
    pub rename_input: String,
    // Git
    pub git_cursor: usize,
    pub git_output: Vec<String>,
    pub git_output_scroll: usize,
    pub git_available: bool,
    pub git_state: GitState,
    pub git_commit_input: String,
    pub settings_cursor: usize,
    pub file_mtime: Option<SystemTime>,
    pub tree_sig: u64,
    pub pending_go_up: bool,
    pub pending_delete: Option<PathBuf>,
    pub tree_info_cache: HashMap<PathBuf, String>,
    pub line_count_cache: HashMap<PathBuf, (SystemTime, usize)>,
    pub git_status_cache: HashMap<PathBuf, GitStatusKind>,
}

impl App {
    pub fn new(root: PathBuf, config: AppConfig) -> Result<Self> {
        let mut expanded_dirs = BTreeSet::new();
        expanded_dirs.insert(root.clone());

        let mut items =
            collect_markdown_tree(&root, &expanded_dirs, config.only_mds, &config.tree_sort)?;
        inject_bookmarks(&mut items, &config);
        let selected_index = items
            .iter()
            .position(|item| !item.is_dir && !item.is_bookmark)
            .unwrap_or(0);
        let current_file = items
            .get(selected_index)
            .filter(|item| !item.is_dir)
            .map(|item| item.path.clone());
        let preview = if let Some(path) = &current_file {
            load_preview(path)?
        } else {
            PreviewDocument::default()
        };

        let file_mtime = current_file.as_ref().and_then(|p| get_file_mtime(p));
        let tree_sig = compute_tree_sig(&root, &expanded_dirs);

        let mut app = Self {
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
            file_op_kind: None,
            file_op_source: None,
            picker_dirs: Vec::new(),
            picker_expanded: BTreeSet::new(),
            picker_cursor: 0,
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
            rename_input: String::new(),
            git_cursor: 0,
            git_output: Vec::new(),
            git_output_scroll: 0,
            git_available: git_is_available(),
            git_state: GitState::CommandList,
            git_commit_input: String::new(),
            settings_cursor: 0,
            file_mtime,
            tree_sig,
            pending_go_up: false,
            pending_delete: None,
            tree_info_cache: HashMap::new(),
            line_count_cache: HashMap::new(),
            git_status_cache: HashMap::new(),
        };
        app.rebuild_tree_info_cache();
        app.refresh_git_status_cache();
        Ok(app)
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

        let was_pending_go_up = self.pending_go_up;
        self.pending_go_up = false;

        if self.pending_delete.is_some() {
            match key.code {
                KeyCode::Enter => {
                    self.confirm_delete()?;
                    return Ok(());
                }
                KeyCode::Esc | KeyCode::Char('X') => {
                    self.pending_delete = None;
                    self.status = String::from("Eliminacion cancelada");
                    return Ok(());
                }
                _ => {
                    self.pending_delete = None;
                }
            }
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
            KeyCode::Left | KeyCode::Backspace => {
                if was_pending_go_up && self.focus == Focus::Tree {
                    self.go_up_root()?;
                } else {
                    self.collapse_or_parent()?;
                }
            }
            KeyCode::Char('j') => match self.focus {
                Focus::Tree => self.move_selection(1),
                Focus::Preview => self.scroll_preview(1),
            },
            KeyCode::Char('k') => match self.focus {
                Focus::Tree => self.move_selection(-1),
                Focus::Preview => self.scroll_preview(-1),
            },
            KeyCode::Char('h') => match self.focus {
                Focus::Tree => {
                    if was_pending_go_up {
                        self.go_up_root()?;
                    } else {
                        self.collapse_or_parent()?;
                    }
                }
                Focus::Preview => self.move_link_cursor(-1),
            },
            KeyCode::Char('l') => match self.focus {
                Focus::Tree => self.activate_selected()?,
                Focus::Preview => self.move_link_cursor(1),
            },
            KeyCode::Char('.') if self.focus == Focus::Preview => self.scroll_preview(1),
            KeyCode::Char(',') if self.focus == Focus::Preview => self.scroll_preview(-1),
            KeyCode::PageDown if self.focus == Focus::Preview => self.scroll_preview(20),
            KeyCode::PageUp if self.focus == Focus::Preview => self.scroll_preview(-20),
            KeyCode::Char(']') if self.focus == Focus::Preview => self.move_link_cursor(1),
            KeyCode::Char('[') if self.focus == Focus::Preview => self.move_link_cursor(-1),
            KeyCode::Char('Y') => self.toggle_selection_mode(),
            KeyCode::Char('E') => self.edit_target_in_nano()?,
            KeyCode::Char('R') => self.open_rename()?,
            KeyCode::Char('B') => self.toggle_bookmark()?,
            KeyCode::Char('X') => self.request_delete()?,
            KeyCode::Char('C')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                self.copy_path_to_clipboard()?;
            }
            KeyCode::Char('!') => self.set_split_level(1),
            KeyCode::Char('@') => self.set_split_level(2),
            KeyCode::Char('#') => self.set_split_level(3),
            KeyCode::Char('$') => self.set_split_level(4),
            KeyCode::Char('%') => self.set_split_level(5),
            KeyCode::Char('G') => self.queue_cd_to_target_dir(),
            KeyCode::Char(':') => self.open_command_palette(),
            KeyCode::Char('T') => self.open_toc(),
            _ => {}
        }

        Ok(())
    }

    fn handle_selection_key(&mut self, key: KeyEvent) -> Result<()> {
        let shift = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::SHIFT);
        let anchored = self.selection.map(|s| s.anchored).unwrap_or(false);
        let extend = shift || anchored;
        match key.code {
            KeyCode::Esc | KeyCode::Char('Y') => self.exit_selection_mode(),
            KeyCode::Char('y') => {
                if anchored {
                    self.copy_selected_text()?;
                } else {
                    if let Some(s) = self.selection.as_mut() {
                        s.anchor = s.cursor;
                        s.anchored = true;
                    }
                    self.status = String::from("Select: ON — mover extiende, y para copiar");
                }
            }
            KeyCode::Left | KeyCode::Char('h') => self.move_selection_cursor(-1, 0, extend),
            KeyCode::Right | KeyCode::Char('l') => self.move_selection_cursor(1, 0, extend),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection_cursor(0, -1, extend),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection_cursor(0, 1, extend),
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
                KeyCode::Up | KeyCode::Char('k') if self.help_section == HelpSection::Settings => {
                    self.settings_cursor = self.settings_cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j')
                    if self.help_section == HelpSection::Settings =>
                {
                    self.settings_cursor = (self.settings_cursor + 1).min(3);
                }
                KeyCode::Enter | KeyCode::Char(' ')
                    if self.help_section == HelpSection::Settings =>
                {
                    match self.settings_cursor {
                        0 => self.toggle_only_mds()?,
                        1 => self.toggle_editor()?,
                        2 => self.toggle_language()?,
                        3 => self.toggle_show_bookmarks()?,
                        _ => {}
                    }
                }
                _ => {}
            },
            Overlay::MermaidSelect => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.close_overlay("Seleccion Mermaid cancelada")
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.mermaid_selected_index = self.mermaid_selected_index.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
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
                KeyCode::Up | KeyCode::Char('k') => {
                    self.mermaid_output_selected_index =
                        self.mermaid_output_selected_index.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
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
                KeyCode::Up | KeyCode::Char('k') => {
                    self.toc_cursor = self.toc_cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
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
            Overlay::DestPicker => match key.code {
                KeyCode::Esc => self.close_dest_picker("Operacion cancelada"),
                KeyCode::Enter => self.confirm_dest_picker(),
                KeyCode::Down | KeyCode::Char('j') => self.move_picker_cursor(1),
                KeyCode::Up | KeyCode::Char('k') => self.move_picker_cursor(-1),
                KeyCode::Right | KeyCode::Char('l') => self.expand_picker_dir(),
                KeyCode::Left | KeyCode::Char('h') => self.collapse_picker_dir(),
                _ => {}
            },
            Overlay::CommandPalette => match key.code {
                KeyCode::Esc => self.close_overlay("Palette cerrada"),
                KeyCode::Enter => {
                    self.confirm_palette_command()?;
                    // Close the palette unless the command opened another overlay
                    // (create, git, dest picker, ...). Otherwise action commands
                    // like delete or copypath would stay hidden behind it.
                    if self.overlay == Overlay::CommandPalette {
                        self.overlay = Overlay::None;
                    }
                }
                KeyCode::Backspace => {
                    self.palette_query.pop();
                    self.update_palette_cursor();
                }
                KeyCode::Down | KeyCode::Char('j') => self.move_palette_cursor(1),
                KeyCode::Up | KeyCode::Char('k') => self.move_palette_cursor(-1),
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
                    KeyCode::Up | KeyCode::Char('k') => self.move_git_cursor(-1),
                    KeyCode::Down | KeyCode::Char('j') => self.move_git_cursor(1),
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
                        self.git_output_scroll = (self.git_output_scroll + 1)
                            .min(self.git_output.len().saturating_sub(1));
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
            Overlay::Rename => match key.code {
                KeyCode::Esc => self.close_overlay("Renombrar cancelado"),
                KeyCode::Enter => self.confirm_rename(),
                KeyCode::Backspace => {
                    self.rename_input.pop();
                }
                KeyCode::Char(c) => self.rename_input.push(c),
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

        let line = self
            .preview_scroll
            .min(self.preview.lines.len().saturating_sub(1));
        let column = 0;
        let cursor = PreviewCursor { line, column };
        self.selection = Some(SelectionState {
            anchor: cursor,
            cursor,
            preferred_column: column,
            previous_fullscreen: self.fullscreen,
            anchored: false,
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
        self.status = format!("Relanzando mdnav despues de {}", self.config.editor);
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

        if item.is_bookmark && item.is_dir {
            self.root = item.path.clone();
            self.expanded_dirs.clear();
            self.expanded_dirs.insert(item.path.clone());
            self.reload_items()?;
            self.selected_index = 0;
            self.status = format!("Bookmark: {}", item.name);
            return Ok(());
        }

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
            if let Some(index) = self
                .items
                .iter()
                .position(|candidate| candidate.path == parent)
            {
                self.selected_index = index;
                self.status = format!("Padre {}", self.items[index].relative.display());
                return Ok(());
            }
        }

        if self.focus == Focus::Tree {
            self.pending_go_up = true;
            self.status = String::from("Go up? ← de nuevo para subir un nivel");
        }

        Ok(())
    }

    fn go_up_root(&mut self) -> Result<()> {
        let Some(parent) = self.root.parent().map(|p| p.to_path_buf()) else {
            self.status = String::from("Ya estas en el directorio raiz");
            return Ok(());
        };
        self.root = parent.clone();
        self.expanded_dirs.clear();
        self.expanded_dirs.insert(parent);
        self.reload_items()?;
        self.selected_index = 0;
        self.status = format!("Subido a {}", self.root.display());
        Ok(())
    }

    fn scroll_preview(&mut self, delta: isize) {
        let max_scroll = self.preview.lines.len().saturating_sub(1) as isize;
        let next = (self.preview_scroll as isize + delta).clamp(0, max_scroll) as usize;
        self.preview_scroll = next;
    }

    fn copy_selected_text(&mut self) -> Result<()> {
        let Some(selection) = self.selection else {
            self.status = String::from("Nada seleccionado");
            return Ok(());
        };

        let (start, end) = if (selection.anchor.line, selection.anchor.column)
            <= (selection.cursor.line, selection.cursor.column)
        {
            (selection.anchor, selection.cursor)
        } else {
            (selection.cursor, selection.anchor)
        };

        if start == end {
            self.status = String::from("Nada seleccionado");
            return Ok(());
        }

        let mut text = String::new();
        for line_idx in start.line..=end.line {
            let Some(line) = self.preview.lines.get(line_idx) else {
                continue;
            };
            let chars: Vec<char> = line.text.chars().collect();
            let col_start = if line_idx == start.line {
                start.column.min(chars.len())
            } else {
                0
            };
            let col_end = if line_idx == end.line {
                end.column.min(chars.len())
            } else {
                chars.len()
            };
            if line_idx > start.line {
                text.push('\n');
            }
            text.extend(&chars[col_start..col_end]);
        }

        let copied = copy_to_clipboard(&text).unwrap_or(false);
        if let Some(s) = self.selection.as_mut() {
            s.anchored = false;
            s.anchor = s.cursor;
        }
        self.status = if copied {
            format!(
                "Copiado! ({} caracteres)  y=nueva seleccion  Esc=salir",
                text.chars().count()
            )
        } else {
            String::from("Error al copiar al portapapeles")
        };
        Ok(())
    }

    fn copy_path_to_clipboard(&mut self) -> Result<()> {
        let Some(target) = self.action_target_path() else {
            self.status = String::from("No hay archivo seleccionado");
            return Ok(());
        };
        let path_str = target.display().to_string();
        let copied = copy_to_clipboard(&path_str).unwrap_or(false);
        self.status = if copied {
            format!("Ruta copiada: {path_str}")
        } else {
            String::from("Error al copiar la ruta")
        };
        Ok(())
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
        let selected_path = self
            .items
            .get(self.selected_index)
            .map(|item| item.path.clone());
        let mut items = collect_markdown_tree(
            &self.root,
            &self.expanded_dirs,
            self.config.only_mds,
            &self.config.tree_sort,
        )?;
        inject_bookmarks(&mut items, &self.config);
        self.items = items;

        if let Some(path) = selected_path {
            if let Some(index) = self.items.iter().position(|item| item.path == path) {
                self.selected_index = index;
            } else {
                self.selected_index = 0;
            }
        }

        self.tree_sig = compute_tree_sig(&self.root, &self.expanded_dirs);
        self.rebuild_tree_info_cache();
        self.refresh_git_status_cache();
        Ok(())
    }

    fn toggle_bookmark(&mut self) -> Result<()> {
        let Some(target) = self.action_target_path() else {
            self.status = String::from("No hay item para marcar como bookmark");
            return Ok(());
        };

        if !target.is_dir() {
            self.status = String::from("Solo se pueden marcar carpetas como bookmark");
            return Ok(());
        }
        let path_str = target.display().to_string();
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone());

        if let Some(pos) = self.config.bookmarks.iter().position(|b| b == &path_str) {
            self.config.bookmarks.remove(pos);
            self.status = format!("Bookmark eliminado: {name}");
        } else {
            self.config.bookmarks.push(path_str);
            self.status = format!("Bookmark agregado: {name}");
        }

        self.config.save()?;
        self.reload_items()?;
        Ok(())
    }

    fn toggle_show_bookmarks(&mut self) -> Result<()> {
        self.config.show_bookmarks = !self.config.show_bookmarks;
        self.config.save()?;
        self.reload_items()?;
        self.status = format!(
            "Bookmarks: {}",
            if self.config.show_bookmarks {
                "visible"
            } else {
                "ocultos"
            }
        );
        Ok(())
    }

    fn toggle_tree_info(&mut self) -> Result<()> {
        self.config.tree_info = self.config.tree_info.next();
        self.config.save()?;
        self.rebuild_tree_info_cache();
        self.status = format!(
            "Tree info: {}",
            match self.config.tree_info {
                TreeInfoMode::Off => "off",
                TreeInfoMode::Size => "tamaño",
                TreeInfoMode::Lines => "líneas",
            }
        );
        Ok(())
    }

    fn toggle_tree_sort(&mut self) -> Result<()> {
        self.config.tree_sort = self.config.tree_sort.next();
        self.config.save()?;
        self.reload_items()?;
        self.status = format!(
            "Tree sort: {}",
            match self.config.tree_sort {
                TreeSortMode::Name => "nombre",
                TreeSortMode::Modified => "fecha",
                TreeSortMode::Size => "tamaño",
            }
        );
        Ok(())
    }

    fn rebuild_tree_info_cache(&mut self) {
        self.tree_info_cache.clear();
        if self.config.tree_info == TreeInfoMode::Off {
            return;
        }
        let paths: Vec<PathBuf> = self
            .items
            .iter()
            .filter(|item| !item.is_dir)
            .map(|item| item.path.clone())
            .collect();
        for path in paths {
            let info = match self.config.tree_info {
                TreeInfoMode::Off => continue,
                TreeInfoMode::Size => fs::metadata(&path).ok().map(|m| format_file_size(m.len())),
                TreeInfoMode::Lines => {
                    if is_image_path(&path) {
                        None
                    } else {
                        count_lines_cached(&path, &mut self.line_count_cache)
                            .map(|n| format!("{}L", n))
                    }
                }
            };
            if let Some(s) = info {
                self.tree_info_cache.insert(path, s);
            }
        }
    }

    pub fn git_status_for_item(&self, item: &DocItem) -> Option<GitStatusKind> {
        if !self.config.show_git_status {
            return None;
        }

        if item.is_bookmark {
            return None;
        }

        if !item.is_dir {
            return self.git_status_cache.get(&item.path).copied();
        }

        self.git_status_cache
            .iter()
            .filter(|(path, _)| path.starts_with(&item.path))
            .map(|(_, status)| *status)
            .max_by_key(|status| status.priority())
    }

    fn refresh_git_status_cache(&mut self) {
        self.git_status_cache.clear();
        if !self.git_available || !self.config.show_git_status {
            return;
        }

        let Ok(output) = Command::new("git")
            .args([
                "-c",
                "core.quotePath=false",
                "status",
                "--porcelain=v1",
                "--ignored",
            ])
            .current_dir(&self.root)
            .output()
        else {
            return;
        };

        if !output.status.success() {
            return;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let Some((path, status)) = parse_git_status_line(line) else {
                continue;
            };
            let full_path = self.root.join(path);
            self.git_status_cache
                .entry(full_path)
                .and_modify(|current| {
                    if status.priority() > current.priority() {
                        *current = status;
                    }
                })
                .or_insert(status);
        }
    }

    fn toggle_git_status_visual(&mut self) -> Result<()> {
        self.config.show_git_status = !self.config.show_git_status;
        self.config.save()?;
        self.refresh_git_status_cache();
        self.status = format!(
            "Git status visual: {}",
            if self.config.show_git_status {
                "on"
            } else {
                "off"
            }
        );
        Ok(())
    }

    pub fn check_external_changes(&mut self) -> Result<()> {
        if let Some(path) = self.current_file.clone() {
            let new_mtime = get_file_mtime(&path);
            if new_mtime != self.file_mtime {
                self.file_mtime = new_mtime;
                self.preview = load_preview(&path)?;
            }
        }

        let new_sig = compute_tree_sig(&self.root, &self.expanded_dirs);
        if new_sig != self.tree_sig {
            self.tree_sig = new_sig;
            self.reload_items()?;
        }

        Ok(())
    }

    fn toggle_language(&mut self) -> Result<()> {
        self.config.language = if self.config.language == "en" {
            String::from("es")
        } else {
            String::from("en")
        };
        let path = self.config.save()?;
        let display_path = config_path().unwrap_or(path);
        self.status = format!(
            "Language: {} | {}",
            self.config.language,
            display_path.display()
        );
        Ok(())
    }

    fn toggle_editor(&mut self) -> Result<()> {
        self.config.editor = if self.config.editor == "vim" {
            String::from("nano")
        } else {
            String::from("vim")
        };
        let path = self.config.save()?;
        let display_path = config_path().unwrap_or(path);
        self.status = format!(
            "Editor: {} | {}",
            self.config.editor,
            display_path.display()
        );
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
        self.file_mtime = get_file_mtime(&path);
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
                format!(
                    " | primer link: {} -> {} ({resolution})",
                    link.label, link.raw_target
                )
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
                if delta > 0 {
                    0
                } else {
                    n - 1
                }
            }
            Some(i) => ((i as isize + delta).rem_euclid(n as isize)) as usize,
        });
        if let Some(idx) = self.preview_link_cursor {
            if let Some(link) = self.preview.links.get(idx) {
                self.preview_scroll = link.line_index;
                let kind = if link.resolved.is_some() {
                    "interno"
                } else {
                    "externo"
                };
                self.status = format!(
                    "Link {}/{}: {} → {} ({kind})  Enter=abrir",
                    idx + 1,
                    n,
                    link.label,
                    link.raw_target
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
            ("q", "salir de mdnav"),
            ("files", "buscar archivo en el arbol"),
            ("find", "buscar texto en el archivo actual"),
            ("create", "crear carpeta o archivo"),
            ("git", "ejecutar comandos git"),
            ("select", "activar cursor de seleccion  (Shift+Y)"),
            ("edit", "abrir editor sobre el archivo  (Shift+E)"),
            ("rename", "renombrar archivo o carpeta  (Shift+R)"),
            ("move", "mover archivo o carpeta a otro directorio"),
            ("copy", "copiar archivo a otro directorio"),
            (
                "copypath",
                "copiar ruta del item seleccionado  (Ctrl+Shift+C)",
            ),
            ("goto", "cd pendiente al directorio  (Shift+G)"),
            ("toc", "tabla de contenidos  (Shift+T)"),
            ("mermaid", "acciones Mermaid  (Shift+M)"),
            ("fullscreen", "pantalla completa del panel  (Shift+0)"),
            ("delete", "eliminar archivo o carpeta  (Shift+X)"),
            ("bookmark", "marcar/desmarcar bookmark  (Shift+B)"),
            ("bookmarks", "mostrar/ocultar bookmarks"),
            ("gitinfo", "mostrar/ocultar info Git en el arbol"),
            ("split1", "proporcion paneles 1  (Shift+1)"),
            ("split2", "proporcion paneles 2  (Shift+2)"),
            ("split3", "proporcion paneles 3  (Shift+3)"),
            ("split4", "proporcion paneles 4  (Shift+4)"),
            ("split5", "proporcion paneles 5  (Shift+5)"),
            ("sort", "ordenar arbol: nombre / fecha / tamaño"),
            ("treeinfo", "info en arbol: tamaño / lineas / off"),
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

    fn confirm_palette_command(&mut self) -> Result<()> {
        let filtered = self.palette_filtered();
        let Some(&(name, _)) = filtered.get(self.palette_cursor) else {
            return Ok(());
        };
        match name {
            "q" => {
                self.overlay = Overlay::None;
                self.running = false;
            }
            "files" => self.open_search(),
            "find" => self.open_find(),
            "create" => self.open_create(),
            "git" => self.open_git(),
            "select" => self.toggle_selection_mode(),
            "edit" => self.edit_target_in_nano()?,
            "rename" => self.open_rename()?,
            "move" => self.open_dest_picker(FileOpKind::Move),
            "copy" => self.open_dest_picker(FileOpKind::Copy),
            "copypath" => self.copy_path_to_clipboard()?,
            "goto" => self.queue_cd_to_target_dir(),
            "toc" => self.open_toc(),
            "mermaid" => self.open_mermaid_flow()?,
            "fullscreen" => self.toggle_fullscreen(),
            "delete" => self.request_delete()?,
            "bookmark" => self.toggle_bookmark()?,
            "bookmarks" => self.toggle_show_bookmarks()?,
            "gitinfo" => self.toggle_git_status_visual()?,
            "split1" => self.set_split_level(1),
            "split2" => self.set_split_level(2),
            "split3" => self.set_split_level(3),
            "split4" => self.set_split_level(4),
            "split5" => self.set_split_level(5),
            "sort" => self.toggle_tree_sort()?,
            "treeinfo" => self.toggle_tree_info()?,
            _ => {}
        }
        Ok(())
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

    fn request_delete(&mut self) -> Result<()> {
        let Some(target) = self.action_target_path() else {
            self.status = String::from("No hay item para eliminar");
            return Ok(());
        };
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| target.display().to_string());
        self.status = format!("Eliminar '{name}'?  Enter=confirmar  Esc=cancelar");
        self.pending_delete = Some(target);
        Ok(())
    }

    fn confirm_delete(&mut self) -> Result<()> {
        let Some(target) = self.pending_delete.take() else {
            return Ok(());
        };
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| target.display().to_string());
        let result = if target.is_dir() {
            fs::remove_dir_all(&target)
        } else {
            fs::remove_file(&target)
        };
        match result {
            Ok(()) => {
                if self.current_file.as_ref() == Some(&target) {
                    self.current_file = None;
                    self.preview = crate::markdown::PreviewDocument::default();
                }
                self.reload_items()?;
                self.status = format!("Eliminado: {name}");
            }
            Err(e) => {
                self.status = format!("Error al eliminar: {e}");
            }
        }
        Ok(())
    }

    fn open_rename(&mut self) -> Result<()> {
        let Some(target) = self.action_target_path() else {
            self.status = String::from("No hay item para renombrar");
            return Ok(());
        };
        let name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        self.rename_input = name;
        self.overlay = Overlay::Rename;
        Ok(())
    }

    fn confirm_rename(&mut self) {
        let new_name = self.rename_input.trim().to_string();
        if new_name.is_empty() {
            self.close_overlay("Nombre vacio, cancelado");
            return;
        }
        let Some(target) = self.action_target_path() else {
            self.close_overlay("No hay item para renombrar");
            return;
        };
        let Some(parent) = target.parent() else {
            self.close_overlay("No se pudo resolver el directorio");
            return;
        };
        let dest = parent.join(&new_name);
        match fs::rename(&target, &dest) {
            Ok(()) => {
                let _ = self.reload_items();
                if let Some(index) = self.items.iter().position(|item| item.path == dest) {
                    self.selected_index = index;
                }
                if dest.is_file() {
                    let _ = self.open_file(dest);
                }
                self.overlay = Overlay::None;
                self.status = format!("Renombrado a: {new_name}");
            }
            Err(e) => {
                self.overlay = Overlay::None;
                self.status = format!("Error al renombrar: {e}");
            }
        }
    }

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
            ("status", "git status", &["status"] as &[&str]),
            ("log", "git log --oneline -20", &["log", "--oneline", "-20"]),
            ("diff", "git diff", &["diff"]),
            ("add .", "git add .", &["add", "."]),
            ("commit", "git commit (pide mensaje)", &[]),
            ("pull", "git pull", &["pull"]),
            ("push", "git push", &["push"]),
            ("branch", "git branch", &["branch"]),
            ("stash", "git stash", &["stash"]),
            ("stash pop", "git stash pop", &["stash", "pop"]),
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
                self.git_output = combined.lines().map(|l| l.to_string()).collect();
                if self.git_output.is_empty() {
                    self.git_output = vec![String::from("(sin salida)")];
                }
                self.git_output_scroll = 0;
                self.git_state = GitState::Output;
                self.status = format!("git {name}  (Esc para volver)");
                self.refresh_git_status_cache();
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
                let combined = if stderr.trim().is_empty() {
                    stdout
                } else {
                    format!("{stdout}\n{stderr}")
                };
                self.git_output = combined.lines().map(|l| l.to_string()).collect();
                if self.git_output.is_empty() {
                    self.git_output = vec![String::from("(sin salida)")];
                }
                self.git_output_scroll = 0;
                self.git_commit_input.clear();
                self.git_state = GitState::Output;
                self.status = String::from("git commit  (Esc para volver)");
                self.refresh_git_status_cache();
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
        self.search_cursor = ((self.search_cursor as isize + delta).rem_euclid(n)) as usize;
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

    // ── Move / copy destination picker ──────────────────────────────────────────

    fn open_dest_picker(&mut self, kind: FileOpKind) {
        let Some(source) = self.action_target_path() else {
            self.status = String::from("No hay item para mover o copiar");
            return;
        };
        self.file_op_kind = Some(kind);
        self.file_op_source = Some(source);
        self.picker_expanded.clear();
        self.picker_cursor = 0;
        self.overlay = Overlay::DestPicker;
        if let Err(e) = self.rebuild_picker_dirs() {
            self.close_dest_picker(&format!("No se pudo listar carpetas: {e}"));
            return;
        }
        self.update_picker_status();
    }

    /// Rebuilds the visible directory list from the current expansion state,
    /// keeping the cursor on the same path when possible.
    fn rebuild_picker_dirs(&mut self) -> Result<()> {
        let current = self
            .picker_dirs
            .get(self.picker_cursor)
            .map(|item| item.path.clone());
        self.picker_dirs = collect_dir_tree(&self.root, &self.picker_expanded)?;
        self.picker_cursor = current
            .and_then(|path| self.picker_dirs.iter().position(|item| item.path == path))
            .unwrap_or(0)
            .min(self.picker_dirs.len().saturating_sub(1));
        Ok(())
    }

    fn update_picker_status(&mut self) {
        let verb = match self.file_op_kind {
            Some(FileOpKind::Copy) => "Copiar",
            _ => "Mover",
        };
        let dest = self
            .picker_dirs
            .get(self.picker_cursor)
            .map(|item| item.name.as_str())
            .unwrap_or("");
        self.status = format!("{verb} a: {dest}  (l/→ entrar | h/← salir | Enter confirmar)");
    }

    fn move_picker_cursor(&mut self, delta: isize) {
        if self.picker_dirs.is_empty() {
            return;
        }
        let n = self.picker_dirs.len() as isize;
        self.picker_cursor = ((self.picker_cursor as isize + delta).rem_euclid(n)) as usize;
        self.update_picker_status();
    }

    fn expand_picker_dir(&mut self) {
        let Some(item) = self.picker_dirs.get(self.picker_cursor) else {
            return;
        };
        let path = item.path.clone();
        if self.picker_expanded.contains(&path) {
            // Already open: step into its first child if any.
            self.move_picker_cursor(1);
            return;
        }
        if has_subdirs(&path) {
            self.picker_expanded.insert(path);
            let _ = self.rebuild_picker_dirs();
            self.move_picker_cursor(1);
        }
    }

    fn collapse_picker_dir(&mut self) {
        let Some(item) = self.picker_dirs.get(self.picker_cursor) else {
            return;
        };
        let path = item.path.clone();
        if self.picker_expanded.contains(&path) {
            self.picker_expanded.remove(&path);
            let _ = self.rebuild_picker_dirs();
            self.update_picker_status();
        } else if let Some(parent) = path.parent().map(|p| p.to_path_buf()) {
            // Jump to the parent node when the current one is already collapsed.
            if let Some(index) = self.picker_dirs.iter().position(|it| it.path == parent) {
                self.picker_cursor = index;
                self.update_picker_status();
            }
        }
    }

    fn confirm_dest_picker(&mut self) {
        let (Some(kind), Some(source)) = (self.file_op_kind, self.file_op_source.clone()) else {
            self.close_dest_picker("Operacion cancelada");
            return;
        };
        let Some(dest_dir) = self
            .picker_dirs
            .get(self.picker_cursor)
            .map(|item| item.path.clone())
        else {
            return;
        };
        let Some(file_name) = source.file_name() else {
            self.close_dest_picker("Origen invalido");
            return;
        };
        let dest = dest_dir.join(file_name);

        if dest == source {
            self.close_dest_picker("El destino es la carpeta actual del item");
            return;
        }
        if dest.exists() {
            self.close_dest_picker("Ya existe un item con ese nombre en el destino");
            return;
        }
        // Evitar mover una carpeta dentro de si misma o de un descendiente.
        if source.is_dir() && dest_dir.starts_with(&source) {
            self.close_dest_picker("No se puede mover una carpeta dentro de si misma");
            return;
        }

        let result: Result<()> = match kind {
            FileOpKind::Move => move_path(&source, &dest),
            FileOpKind::Copy => copy_path(&source, &dest),
        };

        match result {
            Ok(()) => {
                let _ = self.reload_items();
                if let Some(index) = self.items.iter().position(|item| item.path == dest) {
                    self.selected_index = index;
                }
                let verb = if kind == FileOpKind::Move {
                    "Movido"
                } else {
                    "Copiado"
                };
                self.close_dest_picker(&format!("{verb} a: {}", dest_dir.display()));
            }
            Err(e) => {
                let verb = if kind == FileOpKind::Move {
                    "mover"
                } else {
                    "copiar"
                };
                self.close_dest_picker(&format!("Error al {verb}: {e}"));
            }
        }
    }

    fn close_dest_picker(&mut self, status: &str) {
        self.picker_dirs.clear();
        self.picker_expanded.clear();
        self.file_op_kind = None;
        self.file_op_source = None;
        self.overlay = Overlay::None;
        self.status = String::from(status);
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
        let body = response
            .text()
            .unwrap_or_else(|_| String::from("sin detalle"));
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

fn open_in_browser(path: &Path) -> Result<bool> {
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

fn parse_git_status_line(line: &str) -> Option<(&str, GitStatusKind)> {
    if line.len() < 4 {
        return None;
    }

    let status = &line[..2];
    let raw_path = line[3..]
        .split_once(" -> ")
        .map(|(_, new_path)| new_path)
        .unwrap_or(&line[3..])
        .trim_end_matches('/');

    if raw_path.is_empty() {
        return None;
    }

    let kind = if status == "!!" {
        GitStatusKind::Ignored
    } else if status == "??" {
        GitStatusKind::Untracked
    } else if status.contains('U') || matches!(status, "AA" | "DD") {
        GitStatusKind::Conflicted
    } else if status.contains('D') {
        GitStatusKind::Deleted
    } else if status.contains('R') {
        GitStatusKind::Renamed
    } else if status.as_bytes().first().copied() != Some(b' ') {
        GitStatusKind::Staged
    } else if status.as_bytes().get(1).copied() != Some(b' ') {
        GitStatusKind::Modified
    } else {
        return None;
    };

    Some((raw_path, kind))
}

fn inject_bookmarks(items: &mut Vec<DocItem>, config: &AppConfig) {
    if !config.show_bookmarks || config.bookmarks.is_empty() {
        return;
    }
    let bookmark_items: Vec<DocItem> = config
        .bookmarks
        .iter()
        .filter_map(|bm| {
            let path = PathBuf::from(bm);
            if !path.exists() {
                return None;
            }
            let is_dir = path.is_dir();
            let name = path
                .file_name()
                .map(|n| format!("★ {}", n.to_string_lossy()))
                .unwrap_or_else(|| format!("★ {}", bm));
            Some(DocItem {
                path: path.clone(),
                name,
                relative: path,
                depth: 0,
                is_dir,
                is_bookmark: true,
            })
        })
        .collect();
    if !bookmark_items.is_empty() {
        let mut new_items = bookmark_items;
        new_items.append(items);
        *items = new_items;
    }
}

fn get_file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn is_image_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "bmp" | "tiff" | "tif"
    )
}

fn count_lines_cached(
    path: &Path,
    cache: &mut HashMap<PathBuf, (SystemTime, usize)>,
) -> Option<usize> {
    let mtime = fs::metadata(path).ok()?.modified().ok()?;
    if let Some(&(cached_mtime, count)) = cache.get(path) {
        if cached_mtime == mtime {
            return Some(count);
        }
    }
    let content = fs::read_to_string(path).ok()?;
    let count = content.lines().count();
    cache.insert(path.to_path_buf(), (mtime, count));
    Some(count)
}

fn compute_tree_sig(root: &PathBuf, expanded_dirs: &BTreeSet<PathBuf>) -> u64 {
    std::iter::once(root)
        .chain(expanded_dirs.iter())
        .filter_map(|dir| fs::metadata(dir).ok()?.modified().ok())
        .filter_map(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .fold(0u64, |acc, d| {
            acc.wrapping_add(d.as_secs())
                .wrapping_add(d.subsec_nanos() as u64)
        })
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_porcelain_statuses() {
        let cases = [
            (
                " M docs/changed.md",
                "docs/changed.md",
                GitStatusKind::Modified,
            ),
            ("A  docs/new.md", "docs/new.md", GitStatusKind::Staged),
            (
                "?? docs/untracked.md",
                "docs/untracked.md",
                GitStatusKind::Untracked,
            ),
            ("!! target/", "target", GitStatusKind::Ignored),
            (
                " D docs/deleted.md",
                "docs/deleted.md",
                GitStatusKind::Deleted,
            ),
            (
                "R  docs/old.md -> docs/new.md",
                "docs/new.md",
                GitStatusKind::Renamed,
            ),
            (
                "UU docs/conflict.md",
                "docs/conflict.md",
                GitStatusKind::Conflicted,
            ),
        ];

        for (line, expected_path, expected_kind) in cases {
            assert_eq!(
                parse_git_status_line(line),
                Some((expected_path, expected_kind)),
                "line: {line}"
            );
        }
    }

    #[test]
    fn rejects_invalid_git_porcelain_lines() {
        assert_eq!(parse_git_status_line(""), None);
        assert_eq!(parse_git_status_line("M"), None);
        assert_eq!(parse_git_status_line("   "), None);
    }

    #[test]
    fn escapes_html_for_generated_mermaid_files() {
        assert_eq!(
            html_escape("<tag title=\"a&b\">'text'</tag>"),
            "&lt;tag title=&quot;a&amp;b&quot;&gt;&#39;text&#39;&lt;/tag&gt;"
        );
    }
}
