use std::{
    collections::{BTreeSet, HashMap},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::{
    config::{config_path, trash_dir, AppConfig, TreeInfoMode, TreeSortMode},
    docs::{
        collect_dir_tree, collect_markdown_tree, copy_path, has_subdirs, move_path,
        parent_dir_if_within, DocItem,
    },
    http::{self, HttpResponse},
    markdown::{
        load_preview, mermaid_terminal_canvas, MermaidBlock, MermaidCanvas, PreviewDocument,
    },
    postman,
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
    Tools,
    Help,
    MermaidSelect,
    MermaidOutput,
    MermaidTerminalView,
    WebLink,
    Search,
    Toc,
    Grep,
    CommandPalette,
    RecentFiles,
    Marks,
    Find,
    Create,
    Git,
    Rename,
    DestPicker,
    Alert,
    HttpMenu,
    HttpClient,
    HttpEnv,
    HttpInput,
}

/// The single reversible file operation kept around for `undo`. Nothing is ever
/// hard-deleted: a delete parks the item in the trash and an undone copy goes
/// to the trash as well, so a wrong `u` cannot destroy anything either.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LastOperation {
    Delete { trashed: PathBuf, original: PathBuf },
    Moved { from: PathBuf, to: PathBuf },
    Copied { created: PathBuf },
}

/// Trash entries older than this are pruned on startup so undo storage stays
/// bounded without ever surprising a user who deleted something last week.
const TRASH_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// One entry of the destination folder shown next to the picker tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerPreviewEntry {
    pub name: String,
    pub is_dir: bool,
    /// True when the entry has the same name as the item being moved or
    /// copied, so confirming here would hit an existing path.
    pub collides: bool,
}

/// Upper bound on preview rows: big folders must not slow the picker down.
const PICKER_PREVIEW_LIMIT: usize = 200;
const GREP_RESULT_LIMIT: usize = 300;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrepResult {
    pub path: PathBuf,
    pub line_index: usize,
    pub line_text: String,
    pub match_ordinal: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BookmarkResult {
    pub path: PathBuf,
    pub label: String,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseTab {
    Body,
    Headers,
    Info,
}

pub const RESPONSE_TABS: [ResponseTab; 3] =
    [ResponseTab::Body, ResponseTab::Headers, ResponseTab::Info];

/// Actions of the `:http` menu.
pub const HTTP_MENU_ITEMS: &[(&str, &str)] = &[
    ("send", "enviar el request abierto  (Shift+S)"),
    ("env", "elegir environment  (:httpenv)"),
    ("new", "crear un request en la carpeta actual"),
    ("init", "convertir la carpeta actual en una collection"),
    ("import", "importar el JSON de Postman seleccionado"),
    ("copy", "copiar el body de la response  (y)"),
];

/// HTTP state that outlives a single `App`: Shift+E relaunches the app after
/// the editor closes, and responses or a request on the wire must survive it.
#[derive(Default)]
pub struct HttpSession {
    pub responses: HashMap<PathBuf, HttpResponse>,
    pub in_flight: Option<http::InFlight>,
}

/// The open file when it is a request of an OpenCollection.
pub struct HttpView {
    pub collection_root: PathBuf,
    pub request_path: PathBuf,
    /// `None` when the YAML could not be read as a request.
    pub spec: Option<http::HttpRequestSpec>,
    pub environment: Option<String>,
    pub environments: Vec<http::Environment>,
}

pub struct HttpState {
    pub session: HttpSession,
    pub view: Option<HttpView>,
    /// Inside the preview focus, true when the response pane has it.
    pub response_focus: bool,
    pub response_tab: ResponseTab,
    pub response_scroll: usize,
    pub menu_cursor: usize,
    pub env_root: Option<PathBuf>,
    pub env_options: Vec<String>,
    /// 0 is "no environment", then `env_options` in order.
    pub env_cursor: usize,
    pub input: String,
    pub client_root: Option<PathBuf>,
    pub client_collections: Vec<PathBuf>,
    pub client_collection_cursor: usize,
    pub client_requests: Vec<PathBuf>,
    pub client_cursor: usize,
    pub client_focus: u8,
    pub client_creating_collection: bool,
}

impl Default for HttpState {
    fn default() -> Self {
        Self {
            session: HttpSession::default(),
            view: None,
            response_focus: false,
            response_tab: ResponseTab::Body,
            response_scroll: 0,
            menu_cursor: 0,
            env_root: None,
            env_options: Vec::new(),
            env_cursor: 0,
            input: String::new(),
            client_root: None,
            client_collections: Vec::new(),
            client_collection_cursor: 0,
            client_requests: Vec::new(),
            client_cursor: 0,
            client_focus: 0,
            client_creating_collection: false,
        }
    }
}

pub struct App {
    pub root: PathBuf,
    pub items: Vec<DocItem>,
    pub selected_index: usize,
    pub current_file: Option<PathBuf>,
    pub preview: PreviewDocument,
    pub preview_scroll: usize,
    pub json_viewer_active: bool,
    pub json_viewer_raw: String,
    pub json_viewer_pretty: String,
    pub json_viewer_error: Option<String>,
    pub json_viewer_processed: bool,
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
    pub picker_preview: Vec<PickerPreviewEntry>, // contents of the highlighted directory
    pub picker_preview_total: usize, // entries before truncation
    pub picker_preview_error: Option<String>, // set when the directory is unreadable
    /// Last reversible file operation, consumed by `undo`.
    pub last_operation: Option<LastOperation>,
    pub toc_entries: Vec<(usize, String)>, // (line_index, heading text)
    pub toc_cursor: usize,
    pub preview_link_cursor: Option<usize>, // index into preview.links
    // Command palette
    pub palette_query: String,
    pub palette_cursor: usize,
    /// When true the palette lists every command instead of the recent ones.
    pub palette_show_all: bool,
    // Alert popup
    pub alert_message: String,
    // Find in file
    pub find_query: String,
    pub find_results: Vec<usize>, // line indices in preview
    pub find_cursor: usize,
    pub active_find_query: String,
    pub grep_query: String,
    pub grep_results: Vec<GrepResult>,
    pub grep_cursor: usize,
    pub recent_file_cursor: usize,
    pub marks_query: String,
    pub marks_results: Vec<BookmarkResult>,
    pub marks_cursor: usize,
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
    pub http: HttpState,
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
        // Start on the welcome screen. Opening the first document automatically
        // makes the initial view feel arbitrary, especially in large folders.
        let current_file: Option<PathBuf> = None;
        let preview = PreviewDocument::default();

        let file_mtime = current_file.as_ref().and_then(|p| get_file_mtime(p));
        let tree_sig = compute_tree_sig(&root, &expanded_dirs);

        let mut app = Self {
            root,
            items,
            selected_index,
            current_file,
            preview,
            preview_scroll: 0,
            json_viewer_active: false,
            json_viewer_raw: String::new(),
            json_viewer_pretty: String::new(),
            json_viewer_error: None,
            json_viewer_processed: false,
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
            picker_preview: Vec::new(),
            picker_preview_total: 0,
            picker_preview_error: None,
            last_operation: None,
            toc_entries: Vec::new(),
            toc_cursor: 0,
            preview_link_cursor: None,
            palette_query: String::new(),
            palette_cursor: 0,
            palette_show_all: false,
            alert_message: String::new(),
            find_query: String::new(),
            find_results: Vec::new(),
            find_cursor: 0,
            active_find_query: String::new(),
            grep_query: String::new(),
            grep_results: Vec::new(),
            grep_cursor: 0,
            recent_file_cursor: 0,
            marks_query: String::new(),
            marks_results: Vec::new(),
            marks_cursor: 0,
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
            http: HttpState::default(),
        };
        app.rebuild_tree_info_cache();
        app.refresh_git_status_cache();
        app.refresh_http_view();
        prune_trash();
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

        if self.json_viewer_active {
            match key.code {
                KeyCode::Esc => {
                    self.json_viewer_active = false;
                    self.status = String::from("JSON Viewer cerrado");
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::BackTab => {
                    self.toggle_focus();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.preview_scroll = self.preview_scroll.saturating_add(1);
                    return Ok(());
                }
                KeyCode::PageUp => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(20);
                    return Ok(());
                }
                KeyCode::PageDown => {
                    self.preview_scroll = self.preview_scroll.saturating_add(20);
                    return Ok(());
                }
                KeyCode::Char(')') => {
                    if self.fullscreen == FullscreenPanel::Preview {
                        self.fullscreen = FullscreenPanel::None;
                    } else {
                        self.fullscreen = FullscreenPanel::Preview;
                        self.focus = Focus::Preview;
                    }
                    self.preview_scroll = 0;
                    return Ok(());
                }
                KeyCode::F(5) => {
                    self.process_json_viewer();
                    return Ok(());
                }
                KeyCode::Enter if self.focus == Focus::Preview => {
                    self.process_json_viewer();
                    return Ok(());
                }
                KeyCode::Char('y' | 'Y') if self.focus == Focus::Preview => {
                    let copied = copy_to_clipboard(&self.json_viewer_pretty).unwrap_or(false);
                    self.status =
                        if self.json_viewer_processed && self.json_viewer_error.is_none() && copied
                        {
                            format!(
                                "JSON formateado copiado ({} caracteres)",
                                self.json_viewer_pretty.chars().count()
                            )
                        } else if !self.json_viewer_processed || self.json_viewer_error.is_some() {
                            String::from("Procesa un JSON valido antes de copiarlo")
                        } else {
                            String::from("No se pudo copiar el JSON al portapapeles")
                        };
                    return Ok(());
                }
                KeyCode::Backspace if self.focus == Focus::Tree => {
                    self.json_viewer_raw.pop();
                    self.json_viewer_processed = false;
                    self.json_viewer_error = None;
                    return Ok(());
                }
                KeyCode::Enter if self.focus == Focus::Tree => {
                    self.json_viewer_raw.push('\n');
                    self.json_viewer_processed = false;
                    self.json_viewer_error = None;
                    return Ok(());
                }
                KeyCode::Char(ch)
                    if self.focus == Focus::Tree
                        && !key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                        && !key.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
                {
                    self.json_viewer_raw.push(ch);
                    self.json_viewer_processed = false;
                    self.json_viewer_error = None;
                    return Ok(());
                }
                _ => return Ok(()),
            }
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
                    self.status = String::from("Delete cancelled");
                    return Ok(());
                }
                _ => {
                    self.pending_delete = None;
                }
            }
        }

        if self.focus == Focus::Preview
            && self.http_response_focused()
            && self.handle_response_key(key)?
        {
            return Ok(());
        }

        match key.code {
            KeyCode::Char('?') => self.toggle_help(),
            KeyCode::Char('S') => self.send_http_request()?,
            KeyCode::Esc if self.http_busy() => self.abandon_http_request(),
            KeyCode::Up => match self.focus {
                Focus::Tree => self.move_selection(-1),
                Focus::Preview => self.scroll_preview(-1),
            },
            KeyCode::Down => match self.focus {
                Focus::Tree => self.move_selection(1),
                Focus::Preview => self.scroll_preview(1),
            },
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
            KeyCode::Char('@') => self.toggle_json_viewer()?,
            KeyCode::Char('#') => self.set_split_level(3),
            KeyCode::Char('$') => self.set_split_level(4),
            KeyCode::Char('%') => self.set_split_level(5),
            KeyCode::Char('G') => self.queue_cd_to_target_dir(),
            KeyCode::Char(':') => self.open_command_palette(),
            KeyCode::Char('T') => self.open_toc(),
            KeyCode::Char('u') => self.undo_last_operation()?,
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
            Overlay::Alert => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.alert_message.clear();
                    self.close_overlay("Aviso cerrado");
                }
                _ => {}
            },
            Overlay::CommandPalette => match key.code {
                KeyCode::Esc => self.close_overlay("Palette cerrada"),
                KeyCode::Enter => {
                    let keep_open = self.confirm_palette_command()?;
                    // Close the palette unless the command opened another overlay
                    // (create, git, dest picker, ...). Otherwise action commands
                    // like delete or copypath would stay hidden behind it.
                    if !keep_open && self.overlay == Overlay::CommandPalette {
                        self.overlay = Overlay::None;
                    }
                }
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
            Overlay::Grep => match key.code {
                KeyCode::Esc => self.close_grep(),
                KeyCode::Enter => self.confirm_grep()?,
                KeyCode::Backspace => {
                    self.grep_query.pop();
                    self.update_grep_results();
                }
                KeyCode::Down | KeyCode::Char('j') => self.move_grep_cursor(1),
                KeyCode::Up | KeyCode::Char('k') => self.move_grep_cursor(-1),
                KeyCode::Char(c) => {
                    self.grep_query.push(c);
                    self.update_grep_results();
                }
                _ => {}
            },
            Overlay::RecentFiles => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_overlay("Recent files cerrado"),
                KeyCode::Up | KeyCode::Char('k') => self.move_recent_file_cursor(-1),
                KeyCode::Down | KeyCode::Char('j') => self.move_recent_file_cursor(1),
                KeyCode::Enter => self.open_recent_file()?,
                _ => {}
            },
            Overlay::Marks => match key.code {
                KeyCode::Esc => self.close_marks(),
                KeyCode::Enter => self.open_mark()?,
                KeyCode::Backspace => {
                    self.marks_query.pop();
                    self.update_marks_results();
                }
                KeyCode::Up | KeyCode::Char('k') => self.move_marks_cursor(-1),
                KeyCode::Down | KeyCode::Char('j') => self.move_marks_cursor(1),
                KeyCode::Char(c) => {
                    self.marks_query.push(c);
                    self.update_marks_results();
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
            Overlay::Tools => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_overlay("Herramientas cerradas"),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.http.menu_cursor = self.http.menu_cursor.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.http.menu_cursor = (self.http.menu_cursor + 1).min(1)
                }
                KeyCode::Enter => {
                    self.overlay = Overlay::None;
                    if self.http.menu_cursor == 0 {
                        self.toggle_json_viewer()?;
                    } else {
                        self.open_http_client();
                    }
                }
                _ => {}
            },
            Overlay::HttpMenu => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_overlay("HTTP cerrado"),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.http.menu_cursor = self.http.menu_cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.http.menu_cursor =
                        (self.http.menu_cursor + 1).min(HTTP_MENU_ITEMS.len() - 1);
                }
                KeyCode::Enter => self.run_http_menu_item()?,
                _ => {}
            },
            Overlay::HttpClient => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close_overlay("HTTP Client cerrado"),
                KeyCode::Tab => self.http.client_focus = (self.http.client_focus + 1) % 3,
                KeyCode::BackTab => self.http.client_focus = (self.http.client_focus + 2) % 3,
                KeyCode::Left | KeyCode::Char('[') if self.http.client_focus == 0 => {
                    self.shift_http_collection(-1)
                }
                KeyCode::Right | KeyCode::Char(']') if self.http.client_focus == 0 => {
                    self.shift_http_collection(1)
                }
                KeyCode::Up | KeyCode::Char('k') if self.http.client_focus == 0 => {
                    self.shift_http_collection(-1)
                }
                KeyCode::Down | KeyCode::Char('j') if self.http.client_focus == 0 => {
                    self.shift_http_collection(1)
                }
                KeyCode::Up | KeyCode::Char('k') if self.http.client_focus == 1 => {
                    self.http.client_cursor = self.http.client_cursor.saturating_sub(1);
                    self.open_client_cursor_request()?;
                }
                KeyCode::Down | KeyCode::Char('j') if self.http.client_focus == 1 => {
                    self.http.client_cursor = (self.http.client_cursor + 1)
                        .min(self.http.client_requests.len().saturating_sub(1));
                    self.open_client_cursor_request()?;
                }
                KeyCode::Up | KeyCode::Char('k') if self.http.client_focus == 2 => {
                    self.http.response_scroll = self.http.response_scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') if self.http.client_focus == 2 => {
                    self.http.response_scroll = self.http.response_scroll.saturating_add(1)
                }
                KeyCode::Char('[') | KeyCode::Char('h') if self.http.client_focus == 2 => {
                    self.move_response_tab(-1)
                }
                KeyCode::Char(']') | KeyCode::Char('l') if self.http.client_focus == 2 => {
                    self.move_response_tab(1)
                }
                KeyCode::Enter => {
                    self.open_client_cursor_request()?;
                    self.http.client_focus = 2;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    if let Some(path) = self
                        .http
                        .client_requests
                        .get(self.http.client_cursor)
                        .cloned()
                    {
                        self.open_file(path)?;
                        self.send_http_request()?;
                    } else {
                        self.status = String::from("Selecciona o abre un request para enviarlo");
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') => self.start_client_request_input(),
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    self.http.input.clear();
                    self.http.client_creating_collection = true;
                    self.overlay = Overlay::HttpInput;
                    self.status = String::from("Ruta de carpeta nueva para la collection:");
                }
                KeyCode::Char('e') => self.open_http_env_picker(),
                KeyCode::Char('i') | KeyCode::Char('I') => {
                    self.import_postman_selected()?;
                    self.http.client_collections = collect_http_collections(&self.root);
                    if let Some(root) = self.current_collection_root() {
                        self.http.client_collection_cursor = self
                            .http
                            .client_collections
                            .iter()
                            .position(|p| p == &root)
                            .unwrap_or(0);
                        self.http.client_root = self
                            .http
                            .client_collections
                            .get(self.http.client_collection_cursor)
                            .cloned();
                        self.refresh_http_client_requests();
                    }
                }
                KeyCode::Char('m') | KeyCode::Char('M') => self.open_http_menu(),
                _ => {}
            },
            Overlay::HttpEnv => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.http.env_root = None;
                    self.close_overlay("Environment sin cambios");
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.http.env_cursor = self.http.env_cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.http.env_cursor =
                        (self.http.env_cursor + 1).min(self.http.env_options.len());
                }
                KeyCode::Enter => self.confirm_http_env()?,
                _ => {}
            },
            Overlay::HttpInput => match key.code {
                KeyCode::Esc => {
                    self.http.client_creating_collection = false;
                    self.close_overlay("Accion HTTP cancelada");
                }
                KeyCode::Enter if self.http.client_creating_collection => {
                    self.create_http_collection_at_input_path()?
                }
                KeyCode::Enter => self.confirm_new_request()?,
                KeyCode::Backspace => {
                    self.http.input.pop();
                }
                KeyCode::Char(c) => self.http.input.push(c),
                _ => {}
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

    /// Shows a blocking alert popup. Used for recoverable failures (permission
    /// denied, unreadable file, ...) that must never take the whole app down.
    pub fn show_alert(&mut self, message: impl Into<String>) {
        self.alert_message = message.into();
        self.status = self.alert_message.clone();
        self.overlay = Overlay::Alert;
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
        if self.focus == Focus::Preview && self.http.view.is_some() && !self.http.response_focus {
            self.http.response_focus = true;
            self.status = String::from("Focus: response");
            return;
        }
        self.http.response_focus = false;

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

        self.status = match self.focus {
            Focus::Tree => String::from("Focus: tree"),
            Focus::Preview => String::from("Focus: preview"),
        };
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
        self.status = String::from("Selection mode active");
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

        if self.pending_cd.as_ref() == Some(&dir) {
            self.running = false;
            return;
        }

        let label = self.relative_label(&dir);
        self.pending_cd = Some(dir);
        self.status = format!("Shift+G de nuevo para salir en {label}");
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
                if let Err(err) = std::fs::read_dir(&item.path) {
                    self.show_alert(format!(
                        "No se puede abrir {}

{}",
                        item.path.display(),
                        err
                    ));
                    return Ok(());
                }
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
            self.status = String::from("No text selected");
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
            self.status = String::from("No text selected");
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
                "Text copied! ({} chars)  y=new selection  Esc=exit",
                text.chars().count()
            )
        } else {
            String::from("Clipboard copy failed")
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
            format!("Path copied: {path_str}")
        } else {
            String::from("Path copy failed")
        };
        Ok(())
    }

    fn exit_selection_mode(&mut self) {
        if let Some(selection) = self.selection.take() {
            self.fullscreen = selection.previous_fullscreen;
            self.status = String::from("Selection closed");
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
            String::from("Selection extended")
        } else {
            String::from("Selection cursor")
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

    fn open_marks(&mut self) {
        self.marks_query.clear();
        self.marks_cursor = 0;
        self.overlay = Overlay::Marks;
        self.update_marks_results();
        self.status = if self.marks_results.is_empty() {
            String::from("Marks: no hay bookmarks guardados")
        } else {
            String::from("Marks: escribe para filtrar bookmarks")
        };
    }

    fn update_marks_results(&mut self) {
        let query = self.marks_query.to_lowercase();
        self.marks_results = self
            .config
            .bookmarks
            .iter()
            .filter_map(|bookmark| {
                let path = PathBuf::from(bookmark);
                let label = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| bookmark.clone());
                let haystack = format!("{label} {bookmark}").to_lowercase();
                (query.is_empty() || haystack.contains(&query))
                    .then_some(BookmarkResult { path, label })
            })
            .collect();
        if self.marks_cursor >= self.marks_results.len() {
            self.marks_cursor = self.marks_results.len().saturating_sub(1);
        }
        self.status = format!(
            "Marks: \"{}\" - {} bookmark(s)",
            self.marks_query,
            self.marks_results.len()
        );
    }

    fn move_marks_cursor(&mut self, delta: isize) {
        let n = self.marks_results.len();
        if n == 0 {
            return;
        }
        self.marks_cursor = ((self.marks_cursor as isize + delta).rem_euclid(n as isize)) as usize;
    }

    fn open_mark(&mut self) -> Result<()> {
        let Some(mark) = self.marks_results.get(self.marks_cursor).cloned() else {
            return Ok(());
        };

        if !mark.path.is_dir() {
            self.status = format!("Mark no existe: {}", mark.path.display());
            return Ok(());
        }

        self.root = mark.path.clone();
        self.expanded_dirs.clear();
        self.expanded_dirs.insert(self.root.clone());
        self.selected_index = 0;
        self.current_file = None;
        self.preview = PreviewDocument::default();
        self.http.view = None;
        self.preview_scroll = 0;
        self.reload_items()?;
        if let Some(index) = self.items.iter().position(|item| !item.is_dir) {
            self.selected_index = index;
            let path = self.items[index].path.clone();
            self.open_file(path)?;
        }
        self.overlay = Overlay::None;
        self.status = format!("Mark abierto: {}", mark.label);
        Ok(())
    }

    fn close_marks(&mut self) {
        self.marks_query.clear();
        self.marks_results.clear();
        self.marks_cursor = 0;
        self.overlay = Overlay::None;
        self.status = String::from("Marks cerrado");
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

    pub fn current_file_breadcrumb(&self) -> Option<String> {
        let path = self.current_file.as_ref()?;
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        Some(relative.display().to_string().replace('\\', " / "))
    }

    pub fn current_file_metadata(&self) -> Option<String> {
        let path = self.current_file.as_ref()?;
        let metadata = std::fs::metadata(path).ok();
        let size = metadata
            .as_ref()
            .map(|metadata| format_file_size(metadata.len()));
        let lines = self.preview.lines.len();
        let git = self
            .items
            .iter()
            .find(|item| &item.path == path)
            .and_then(|item| self.git_status_for_item(item))
            .map(git_status_label);

        let mut parts = Vec::new();
        parts.push(format!("{lines} lines"));
        if lines > 0 {
            let current_line = self.preview_scroll.min(lines.saturating_sub(1)) + 1;
            let percent = ((current_line * 100) / lines).clamp(1, 100);
            parts.push(format!("line {current_line}/{lines}"));
            parts.push(format!("{percent}%"));
        }
        if let Some(size) = size {
            parts.push(size);
        }
        if let Some(git) = git {
            parts.push(git.to_string());
        }
        Some(parts.join(" · "))
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
                self.refresh_http_view();
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
                self.http.view = None;
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
        self.json_viewer_active = false;
        self.status = format!("Opening {}...", self.relative_label(&path));
        self.preview = load_preview(&path)?;
        self.preview_scroll = 0;
        self.file_mtime = get_file_mtime(&path);
        self.current_file = Some(path.clone());
        self.active_find_query.clear();
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
        self.http.response_focus = false;
        self.http.response_scroll = 0;
        self.refresh_http_view();
        self.config.record_recent_file(&path.display().to_string());
        let _ = self.config.save();
        if let Some(status) = self.http_open_status(&path) {
            self.status = status;
            return Ok(());
        }
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

    fn toggle_json_viewer(&mut self) -> Result<()> {
        if self.json_viewer_active {
            self.json_viewer_active = false;
            self.status = String::from("JSON Viewer cerrado");
            return Ok(());
        }

        let is_json = |path: &Path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        };
        let path = self
            .current_file
            .as_ref()
            .filter(|path| is_json(path))
            .cloned()
            .or_else(|| {
                self.items
                    .get(self.selected_index)
                    .filter(|item| !item.is_dir && is_json(&item.path))
                    .map(|item| item.path.clone())
            });
        if let Some(path) = path {
            if self.current_file.as_ref() != Some(&path) {
                self.open_file(path.clone())?;
            }
            self.json_viewer_raw = fs::read_to_string(&path)
                .with_context(|| format!("No se pudo leer {}", path.display()))?;
        } else {
            self.json_viewer_raw.clear();
        }
        self.json_viewer_pretty.clear();
        self.json_viewer_error = None;
        self.json_viewer_processed = false;
        self.preview_scroll = 0;
        self.focus = Focus::Tree;
        self.json_viewer_active = true;
        self.status =
            String::from("Pega JSON y pulsa Tab para procesar | Shift+0 vista limpia | Esc cierra");
        Ok(())
    }

    fn process_json_viewer(&mut self) {
        match serde_json::from_str::<serde_json::Value>(&self.json_viewer_raw) {
            Ok(value) => match serde_json::to_string_pretty(&value) {
                Ok(pretty) => {
                    self.json_viewer_pretty = pretty;
                    self.json_viewer_error = None;
                    self.json_viewer_processed = true;
                    self.status = String::from("JSON procesado");
                }
                Err(error) => {
                    self.json_viewer_error = Some(error.to_string());
                    self.json_viewer_processed = true;
                }
            },
            Err(error) => {
                self.json_viewer_error = Some(error.to_string());
                self.json_viewer_processed = true;
                self.status = String::from("JSON invalido; revisa el texto de entrada");
            }
        }
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
                let share_url = crate::share::publish_share(
                    "mermaid",
                    &diagram.source,
                    Some(&diagram.title),
                    None,
                    None,
                )?;
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
        self.palette_show_all = false;
        self.overlay = Overlay::CommandPalette;
        self.status = String::from("Palette: escribe para filtrar");
    }

    /// Every palette command, sorted alphabetically by name.
    pub fn palette_commands(&self) -> Vec<(&'static str, &'static str)> {
        let mut commands: Vec<(&'static str, &'static str)> = vec![
            ("bookmark", "marcar/desmarcar bookmark  (Shift+B)"),
            ("bookmarks", "mostrar/ocultar bookmarks"),
            ("@commands", "ver la lista completa de comandos"),
            ("@jsonviewer", "abrir JSON Viewer"),
            ("@http", "abrir HTTP Client"),
            ("@tools", "abrir herramientas integradas"),
            ("copy", "copiar archivo a otro directorio"),
            (
                "copypath",
                "copiar ruta del item seleccionado  (Ctrl+Shift+C)",
            ),
            ("create", "crear carpeta o archivo"),
            ("delete", "eliminar archivo o carpeta  (Shift+X)"),
            ("edit", "abrir editor sobre el archivo  (Shift+E)"),
            ("files", "buscar archivo en el arbol"),
            ("find", "buscar texto en el archivo actual"),
            ("fullscreen", "pantalla completa del panel  (Shift+0)"),
            ("git", "ejecutar comandos git"),
            ("gitinfo", "mostrar/ocultar info Git en el arbol"),
            ("goto", "cd pendiente al directorio  (Shift+G)"),
            ("grep", "buscar texto en archivos visibles"),
            ("@http", "abrir HTTP Client"),
            ("httpenv", "elegir environment de la collection"),
            ("json", "abrir JSON Viewer  (@)"),
            ("marks", "abrir bookmarks guardados"),
            ("mermaid", "acciones Mermaid  (Shift+M)"),
            ("move", "mover archivo o carpeta a otro directorio"),
            ("q", "salir de mdnav"),
            ("recent", "abrir archivos recientes"),
            ("rename", "renombrar archivo o carpeta  (Shift+R)"),
            ("select", "activar cursor de seleccion  (Shift+Y)"),
            ("sort", "ordenar arbol: nombre / fecha / tamaño"),
            ("split1", "proporcion paneles 1  (Shift+1)"),
            ("split2", "proporcion paneles 2  (Shift+2)"),
            ("split3", "proporcion paneles 3  (Shift+3)"),
            ("split4", "proporcion paneles 4  (Shift+4)"),
            ("split5", "proporcion paneles 5  (Shift+5)"),
            ("toc", "tabla de contenidos  (Shift+T)"),
            ("treeinfo", "info en arbol: tamaño / lineas / off"),
            ("undo", "deshacer la ultima operacion de archivo  (u)"),
        ];
        commands.sort_by_key(|(name, _)| *name);
        commands
    }

    pub fn palette_command_groups(&self) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
        vec![
            (
                "Navegacion",
                vec![
                    ("files", "buscar archivo en el arbol"),
                    ("goto", "cd pendiente al directorio  (Shift+G)"),
                    ("recent", "abrir archivos recientes"),
                    ("marks", "abrir bookmarks guardados"),
                    ("toc", "tabla de contenidos  (Shift+T)"),
                    ("q", "salir de mdnav"),
                ],
            ),
            (
                "Busqueda",
                vec![
                    ("find", "buscar texto en el archivo actual"),
                    ("grep", "buscar texto en archivos visibles"),
                    ("@commands", "ver la lista completa de comandos"),
                ],
            ),
            (
                "Archivos",
                vec![
                    ("create", "crear carpeta o archivo"),
                    ("edit", "abrir editor sobre el archivo  (Shift+E)"),
                    ("rename", "renombrar archivo o carpeta  (Shift+R)"),
                    ("move", "mover archivo o carpeta a otro directorio"),
                    ("copy", "copiar archivo a otro directorio"),
                    (
                        "copypath",
                        "copiar ruta del item seleccionado  (Ctrl+Shift+C)",
                    ),
                    ("delete", "eliminar archivo o carpeta  (Shift+X)"),
                    ("undo", "deshacer la ultima operacion de archivo  (u)"),
                ],
            ),
            (
                "Preview",
                vec![
                    ("select", "activar cursor de seleccion  (Shift+Y)"),
                    ("mermaid", "acciones Mermaid  (Shift+M)"),
                    ("@jsonviewer", "abrir JSON Viewer"),
                    ("@tools", "abrir herramientas integradas"),
                    ("json", "abrir JSON Viewer  (@)"),
                    ("fullscreen", "pantalla completa del panel  (Shift+0)"),
                    ("split1", "proporcion paneles 1  (Shift+1)"),
                    ("split2", "proporcion paneles 2  (Shift+2)"),
                    ("split3", "proporcion paneles 3  (Shift+3)"),
                    ("split4", "proporcion paneles 4  (Shift+4)"),
                    ("split5", "proporcion paneles 5  (Shift+5)"),
                ],
            ),
            (
                "Arbol",
                vec![
                    ("bookmark", "marcar/desmarcar bookmark  (Shift+B)"),
                    ("bookmarks", "mostrar/ocultar bookmarks"),
                    ("sort", "ordenar arbol: nombre / fecha / tamano"),
                    ("treeinfo", "info en arbol: tamano / lineas / off"),
                    ("gitinfo", "mostrar/ocultar info Git en el arbol"),
                ],
            ),
            ("Git", vec![("git", "ejecutar comandos git")]),
            (
                "HTTP",
                vec![
                    ("@http", "abrir HTTP Client"),
                    ("httpenv", "elegir environment de la collection"),
                ],
            ),
        ]
    }

    fn palette_catalog_commands(&self) -> Vec<(&'static str, &'static str)> {
        self.palette_command_groups()
            .into_iter()
            .flat_map(|(_, commands)| commands)
            .collect()
    }

    fn palette_recent_commands(&self) -> Vec<(&'static str, &'static str)> {
        let commands = self.palette_commands();
        self.config
            .recent_commands
            .iter()
            .filter_map(|recent| {
                commands
                    .iter()
                    .copied()
                    .find(|(name, _)| *name == recent.as_str())
            })
            .collect()
    }

    pub fn palette_filtered(&self) -> Vec<(&'static str, &'static str)> {
        if self.palette_show_all {
            return self.palette_catalog_commands();
        }

        let q = self.palette_query.to_lowercase();
        if q.is_empty() {
            return self.palette_recent_commands();
        }

        if "@commands".starts_with(q.as_str()) {
            return vec![("@commands", "ver la lista completa de comandos")];
        }

        let commands = self.palette_commands();
        let recent_matches = self
            .palette_recent_commands()
            .into_iter()
            .filter(|(name, desc)| {
                name.starts_with(q.as_str())
                    || desc.contains(q.as_str())
                    || palette_aliases(name)
                        .iter()
                        .any(|alias| alias.contains(q.as_str()))
            });
        let other_matches = commands.into_iter().filter(|(name, desc)| {
            name.starts_with(q.as_str())
                || desc.contains(q.as_str())
                || palette_aliases(name)
                    .iter()
                    .any(|alias| alias.contains(q.as_str()))
        });
        let mut matches = Vec::new();
        for command in recent_matches.chain(other_matches) {
            if !matches.iter().any(|(name, _)| name == &command.0) {
                matches.push(command);
            }
        }

        let prefix_matches = matches
            .iter()
            .copied()
            .filter(|(name, _)| name.starts_with(q.as_str()))
            .take(7)
            .collect::<Vec<_>>();

        if !prefix_matches.is_empty() {
            return prefix_matches;
        }

        matches.into_iter().take(1).collect()
    }

    fn update_palette_cursor(&mut self) {
        self.palette_show_all = self.palette_query == "@commands";
        let n = self.palette_filtered().len();
        if self.palette_cursor >= n && n > 0 {
            self.palette_cursor = n - 1;
        } else if n <= 1 {
            self.palette_cursor = 0;
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

    /// Runs the selected command. Returns true when the palette should stay
    /// open (only `@commands`, which switches to the full catalogue).
    fn confirm_palette_command(&mut self) -> Result<bool> {
        let filtered = self.palette_filtered();
        let Some(&(name, _)) = filtered.get(self.palette_cursor) else {
            return Ok(false);
        };

        if name == "@commands" {
            self.palette_show_all = true;
            self.palette_query = String::from("@commands");
            self.palette_cursor = 0;
            self.status = String::from("Palette: lista completa");
            return Ok(true);
        }

        self.config.record_recent_command(name);
        self.config.save()?;

        match name {
            "@tools" => self.overlay = Overlay::Tools,
            "@http" => self.open_http_client(),
            "q" => {
                self.overlay = Overlay::None;
                self.running = false;
            }
            "recent" => self.open_recent_files(),
            "files" => self.open_search(),
            "find" => self.open_find(),
            "grep" => self.open_grep(),
            "marks" => self.open_marks(),
            "create" => self.open_create(),
            "git" => self.open_git(),
            "httpenv" => self.open_http_env_picker(),
            "json" => self.toggle_json_viewer()?,
            "@jsonviewer" => self.toggle_json_viewer()?,
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
            "undo" => self.undo_last_operation()?,
            _ => {}
        }
        Ok(false)
    }

    // ── Find in file ──────────────────────────────────────────────────────────

    fn open_find(&mut self) {
        self.find_query = self.active_find_query.clone();
        self.find_results.clear();
        self.find_cursor = 0;
        self.overlay = Overlay::Find;
        self.status = String::from("Find: escribe para buscar en el archivo");
        self.update_find_results();
    }

    fn update_find_results(&mut self) {
        let q = self.find_query.to_lowercase();
        if q.is_empty() {
            self.find_results.clear();
            self.active_find_query.clear();
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
            self.active_find_query = self.find_query.clone();
        }
        self.close_overlay("Find: saltando a resultado");
    }

    fn open_grep(&mut self) {
        self.grep_query.clear();
        self.grep_results.clear();
        self.grep_cursor = 0;
        self.overlay = Overlay::Grep;
        self.status = String::from("Grep: escribe para buscar en archivos visibles");
    }

    fn update_grep_results(&mut self) {
        self.grep_results.clear();
        self.grep_cursor = 0;

        let query = self.grep_query.to_lowercase();
        if query.trim().is_empty() {
            self.status = String::from("Grep: escribe para buscar en archivos visibles");
            return;
        }

        for item in self
            .items
            .iter()
            .filter(|item| !item.is_dir && !item.is_bookmark)
        {
            if self.grep_results.len() >= GREP_RESULT_LIMIT {
                break;
            }

            let Ok(content) = fs::read_to_string(&item.path) else {
                continue;
            };

            let mut match_ordinal = 0;
            for (line_index, line) in content.lines().enumerate() {
                if !line.to_lowercase().contains(&query) {
                    continue;
                }

                self.grep_results.push(GrepResult {
                    path: item.path.clone(),
                    line_index,
                    line_text: line.trim().to_string(),
                    match_ordinal,
                });
                match_ordinal += 1;

                if self.grep_results.len() >= GREP_RESULT_LIMIT {
                    break;
                }
            }
        }

        let suffix = if self.grep_results.len() >= GREP_RESULT_LIMIT {
            "+"
        } else {
            ""
        };
        self.status = format!(
            "Grep: \"{}\" - {}{} resultado(s)",
            self.grep_query,
            self.grep_results.len(),
            suffix
        );
    }

    fn move_grep_cursor(&mut self, delta: isize) {
        let n = self.grep_results.len();
        if n == 0 {
            return;
        }
        self.grep_cursor = ((self.grep_cursor as isize + delta).rem_euclid(n as isize)) as usize;
    }

    fn confirm_grep(&mut self) -> Result<()> {
        let Some(result) = self.grep_results.get(self.grep_cursor).cloned() else {
            return Ok(());
        };

        if let Some(index) = self.items.iter().position(|item| item.path == result.path) {
            self.selected_index = index;
        }

        self.open_file(result.path)?;
        let query = self.grep_query.to_lowercase();
        let matching_preview_line = self
            .preview
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.text.to_lowercase().contains(&query))
            .nth(result.match_ordinal)
            .map(|(index, _)| index)
            .unwrap_or(result.line_index);
        self.preview_scroll = matching_preview_line.min(self.preview.lines.len().saturating_sub(1));
        self.focus = Focus::Preview;
        self.active_find_query = self.grep_query.clone();
        self.overlay = Overlay::None;
        self.status = format!("Grep: jumped to line {}", result.line_index + 1);
        Ok(())
    }

    fn close_grep(&mut self) {
        self.grep_query.clear();
        self.grep_results.clear();
        self.grep_cursor = 0;
        self.overlay = Overlay::None;
        self.status = String::from("Grep closed");
    }

    fn open_recent_files(&mut self) {
        self.config
            .recent_files
            .retain(|path| PathBuf::from(path).exists());
        self.recent_file_cursor = 0;
        if self.config.recent_files.is_empty() {
            self.status = String::from("No recent files yet");
            return;
        }
        self.overlay = Overlay::RecentFiles;
        self.status = String::from("Recent files: Enter=open  Esc=close");
    }

    fn move_recent_file_cursor(&mut self, delta: isize) {
        let n = self.config.recent_files.len();
        if n == 0 {
            return;
        }
        self.recent_file_cursor =
            ((self.recent_file_cursor as isize + delta).rem_euclid(n as isize)) as usize;
    }

    fn open_recent_file(&mut self) -> Result<()> {
        let Some(path) = self
            .config
            .recent_files
            .get(self.recent_file_cursor)
            .map(PathBuf::from)
        else {
            self.close_overlay("No recent file selected");
            return Ok(());
        };

        if !path.exists() {
            self.config.recent_files.remove(self.recent_file_cursor);
            let _ = self.config.save();
            self.status = String::from("Recent file no longer exists");
            return Ok(());
        }

        if let Some(index) = self.items.iter().position(|item| item.path == path) {
            self.selected_index = index;
        }
        self.open_file(path)
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
        self.status = format!("Delete '{name}'?  Enter=confirm  Esc=cancel");
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
        match move_to_trash(&target) {
            Ok(trashed) => {
                if self.current_file.as_ref() == Some(&target) {
                    self.current_file = None;
                    self.preview = crate::markdown::PreviewDocument::default();
                    self.http.view = None;
                }
                self.last_operation = Some(LastOperation::Delete {
                    trashed,
                    original: target,
                });
                self.reload_items()?;
                self.status = format!("Moved to trash: {name}  (u=undo)");
            }
            Err(e) => {
                self.status = format!("Error al eliminar: {e:#}");
            }
        }
        Ok(())
    }

    /// Reverts the last file operation. Restores a deleted item from the trash,
    /// moves a renamed or moved item back, or sends an unwanted copy to the
    /// trash. Only one step is kept: this is a safety net, not a history.
    fn undo_last_operation(&mut self) -> Result<()> {
        let Some(operation) = self.last_operation.take() else {
            self.status = String::from("No hay nada para deshacer");
            return Ok(());
        };

        let outcome: Result<String> = match &operation {
            LastOperation::Delete { trashed, original } => move_path(trashed, original)
                .map(|()| format!("Restaurado: {}", display_name(original))),
            LastOperation::Moved { from, to } => {
                move_path(to, from).map(|()| format!("Revertido: {}", display_name(from)))
            }
            LastOperation::Copied { created } => move_to_trash(created)
                .map(|_| format!("Copia descartada: {}", display_name(created))),
        };

        match outcome {
            Ok(status) => {
                self.reload_items()?;
                self.clear_preview_if_missing();
                self.status = status;
            }
            Err(e) => {
                // Keep the slot so the user can retry once the cause is fixed.
                self.last_operation = Some(operation);
                self.show_alert(format!(
                    "No se pudo deshacer:

{e:#}"
                ));
            }
        }
        Ok(())
    }

    /// Drops the preview when the open file no longer exists on disk.
    fn clear_preview_if_missing(&mut self) {
        let missing = self
            .current_file
            .as_ref()
            .map(|path| !path.exists())
            .unwrap_or(false);
        if missing {
            self.current_file = None;
            self.preview = crate::markdown::PreviewDocument::default();
            self.http.view = None;
            self.file_mtime = None;
        }
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
                self.last_operation = Some(LastOperation::Moved {
                    from: target.clone(),
                    to: dest.clone(),
                });
                let _ = self.reload_items();
                if let Some(index) = self.items.iter().position(|item| item.path == dest) {
                    self.selected_index = index;
                }
                if dest.is_file() {
                    let _ = self.open_file(dest);
                }
                self.overlay = Overlay::None;
                self.status = format!("Renombrado a: {new_name}  (u para deshacer)");
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

    // ── HTTP ──────────────────────────────────────────────────────────────────

    pub fn http_response_focused(&self) -> bool {
        self.http.view.is_some() && self.http.response_focus
    }

    pub fn http_busy(&self) -> bool {
        self.http.session.in_flight.is_some()
    }

    /// The last response of the open request, if it was ever sent.
    pub fn current_http_response(&self) -> Option<&HttpResponse> {
        let view = self.http.view.as_ref()?;
        self.http.session.responses.get(&view.request_path)
    }

    /// The request on the wire, when it is the open one.
    pub fn current_request_in_flight(&self) -> Option<&http::InFlight> {
        let view = self.http.view.as_ref()?;
        self.http
            .session
            .in_flight
            .as_ref()
            .filter(|in_flight| in_flight.request_path == view.request_path)
    }

    fn active_environment_name(&self, root: &Path) -> Option<String> {
        self.config
            .http_environments
            .get(&root.display().to_string())
            .cloned()
    }

    /// Rebuilds the request pane when the open file is a request of an
    /// OpenCollection, and clears the HTTP view for any other file.
    fn refresh_http_view(&mut self) {
        let Some(path) = self.current_file.clone() else {
            self.http.view = None;
            return;
        };
        let Some(root) = http::request_collection_root(&path) else {
            self.http.view = None;
            return;
        };

        let (environments, mut warnings) = http::load_environments(&root);
        let active = self.active_environment_name(&root);
        let environment = active
            .as_deref()
            .and_then(|name| environments.iter().find(|env| env.name == name));
        if let (Some(name), None) = (active.as_deref(), environment) {
            warnings.push(format!("El environment '{name}' ya no existe"));
        }
        let environment_name = environment.map(|env| env.name.clone());

        let spec = fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|content| http::parse_request(&content));
        let spec = match spec {
            Ok(spec) => {
                let resolved = http::resolve_request(&root, &path, &spec, environment);
                self.preview =
                    http::request_preview(&spec, &resolved, environment_name.as_deref(), &warnings);
                Some(spec)
            }
            Err(err) => {
                self.preview = http::error_preview(&format!("{err:#}"));
                None
            }
        };
        self.preview_scroll = self
            .preview_scroll
            .min(self.preview.lines.len().saturating_sub(1));
        self.preview_link_cursor = None;

        self.http.view = Some(HttpView {
            collection_root: root,
            request_path: path,
            spec,
            environment: environment_name,
            environments,
        });
    }

    /// Status shown when opening a request or a Postman export.
    fn http_open_status(&self, path: &Path) -> Option<String> {
        if let Some(view) = &self.http.view {
            let environment = view.environment.as_deref().unwrap_or("sin environment");
            return Some(format!(
                "Request HTTP · env: {environment} · Shift+S envia · Tab va a la response"
            ));
        }
        http::is_postman_export(path)
            .then(|| String::from("Export de Postman · :@http → import lo convierte en collection"))
    }

    fn send_http_request(&mut self) -> Result<()> {
        if self.http.view.is_none() {
            self.show_alert(
                "Shift+S envia el request abierto.

Abri un .yml de una collection (una carpeta con opencollection.yml) o crea una con :@http → init.",
            );
            return Ok(());
        }
        if let Some(in_flight) = &self.http.session.in_flight {
            self.status = format!(
                "Ya hay un request en curso: {} (Esc lo abandona)",
                in_flight.label
            );
            return Ok(());
        }

        // Re-read the request, its folders and environments: any of them may
        // have changed since the file was opened.
        self.refresh_http_view();
        let Some(view) = self.http.view.as_ref() else {
            return Ok(());
        };
        let Some(spec) = view.spec.as_ref() else {
            self.status = String::from("El request tiene errores: revisalo con Shift+E");
            return Ok(());
        };
        let environment = view
            .environment
            .as_deref()
            .and_then(|name| view.environments.iter().find(|env| env.name == name));
        let resolved =
            http::resolve_request(&view.collection_root, &view.request_path, spec, environment);
        // A URL with unresolved variables cannot reach any server.
        if resolved.url.contains("{{") {
            let in_url: Vec<&str> = resolved
                .unresolved
                .iter()
                .map(String::as_str)
                .filter(|name| resolved.url.contains(name))
                .collect();
            self.show_alert(format!(
                "La URL tiene variables sin resolver: {}

Elegi un environment con :httpenv o definilas en la collection.",
                in_url.join(", ")
            ));
            return Ok(());
        }
        let unresolved = resolved.unresolved.len();
        let in_flight = http::spawn_request(view.request_path.clone(), resolved);

        self.status = if unresolved > 0 {
            format!(
                "Enviando {} · {unresolved} variable(s) sin resolver",
                in_flight.label
            )
        } else {
            format!("Enviando {}", in_flight.label)
        };
        self.http.session.in_flight = Some(in_flight);
        self.http.response_scroll = 0;
        Ok(())
    }

    /// Stops waiting for the request on the wire. The worker thread cannot be
    /// interrupted, so its eventual result is simply discarded.
    fn abandon_http_request(&mut self) {
        if let Some(in_flight) = self.http.session.in_flight.take() {
            self.status = format!("Request abandonado: {}", in_flight.label);
        }
    }

    /// Collects the response of the request on the wire once it is done.
    /// Called on every tick of the UI loop.
    pub fn poll_http(&mut self) {
        let Some(in_flight) = self.http.session.in_flight.as_ref() else {
            return;
        };
        let Some(response) = in_flight.try_finish() else {
            return;
        };
        let path = in_flight.request_path.clone();
        self.http.session.in_flight = None;
        self.status = response_summary(&response);

        let is_open = self
            .http
            .view
            .as_ref()
            .map(|view| view.request_path == path)
            .unwrap_or(false);
        if is_open {
            self.http.response_scroll = 0;
            // Errors are explained in the body tab: never leave them hidden.
            if response.error.is_some() {
                self.http.response_tab = ResponseTab::Body;
            }
            if self.focus == Focus::Preview {
                self.http.response_focus = true;
            }
        }
        self.http.session.responses.insert(path, response);
    }

    /// Keys of the focused response pane. Returns false so everything else
    /// (Tab, Shift+S, `:`, Esc, ...) falls through to the regular bindings.
    fn handle_response_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char(',') => self.scroll_response(-1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('.') => self.scroll_response(1),
            KeyCode::PageUp => self.scroll_response(-20),
            KeyCode::PageDown => self.scroll_response(20),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('[') => self.move_response_tab(-1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(']') => self.move_response_tab(1),
            KeyCode::Char('y') => self.copy_response_body()?,
            KeyCode::Char('Y') => {
                self.status = String::from("En la response, y copia el body completo");
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Lines of the active response tab, used to clamp scrolling.
    pub fn response_line_count(&self) -> usize {
        let Some(response) = self.current_http_response() else {
            return 0;
        };
        match self.http.response_tab {
            ResponseTab::Body => response.body_lines.len().max(1),
            ResponseTab::Headers => response.headers.len().max(1),
            ResponseTab::Info => 8,
        }
    }

    fn scroll_response(&mut self, delta: isize) {
        let max_scroll = self.response_line_count().saturating_sub(1) as isize;
        self.http.response_scroll =
            (self.http.response_scroll as isize + delta).clamp(0, max_scroll) as usize;
    }

    fn move_response_tab(&mut self, delta: isize) {
        let index = RESPONSE_TABS
            .iter()
            .position(|tab| *tab == self.http.response_tab)
            .unwrap_or(0) as isize;
        let next = (index + delta).rem_euclid(RESPONSE_TABS.len() as isize) as usize;
        self.http.response_tab = RESPONSE_TABS[next];
        self.http.response_scroll = 0;
    }

    fn copy_response_body(&mut self) -> Result<()> {
        let Some(body) = self
            .current_http_response()
            .map(|response| response.body_text.clone())
        else {
            self.status = String::from("Todavia no hay response: Shift+S para enviar");
            return Ok(());
        };
        if body.is_empty() {
            self.status = String::from("La response no tiene body de texto");
            return Ok(());
        }
        self.status = if copy_to_clipboard(&body)? {
            format!("Body copiado ({})", http::format_size(body.len()))
        } else {
            String::from("No se pudo acceder al portapapeles")
        };
        Ok(())
    }

    fn open_http_menu(&mut self) {
        self.http.menu_cursor = 0;
        self.overlay = Overlay::HttpMenu;
        self.status = String::from("HTTP: elegi una accion");
    }

    fn open_http_client(&mut self) {
        let current = self.current_collection_root().or_else(|| {
            self.selected_item_path()
                .and_then(|path| http::find_collection_root(&path))
        });
        self.http.client_collections = collect_http_collections(&self.root);
        if let Some(root) = current {
            if let Some(index) = self.http.client_collections.iter().position(|p| p == &root) {
                self.http.client_collection_cursor = index;
            } else {
                self.http.client_collections.push(root.clone());
                self.http.client_collection_cursor = self.http.client_collections.len() - 1;
            }
        }
        self.http.client_root = self
            .http
            .client_collections
            .get(self.http.client_collection_cursor)
            .cloned();
        self.refresh_http_client_requests();
        self.http.client_focus = if self.http.client_requests.is_empty() {
            0
        } else {
            1
        };
        let _ = self.open_client_cursor_request();
        self.overlay = Overlay::HttpClient;
        self.status = String::from("HTTP Client");
    }

    fn shift_http_collection(&mut self, delta: isize) {
        if self.http.client_collections.is_empty() {
            return;
        }
        let len = self.http.client_collections.len() as isize;
        self.http.client_collection_cursor =
            (self.http.client_collection_cursor as isize + delta).rem_euclid(len) as usize;
        self.http.client_root = self
            .http
            .client_collections
            .get(self.http.client_collection_cursor)
            .cloned();
        self.http.client_cursor = 0;
        self.refresh_http_client_requests();
        let _ = self.open_client_cursor_request();
    }

    fn open_client_cursor_request(&mut self) -> Result<()> {
        let Some(path) = self
            .http
            .client_requests
            .get(self.http.client_cursor)
            .cloned()
        else {
            self.http.view = None;
            return Ok(());
        };
        self.open_file(path)?;
        self.http.response_scroll = 0;
        Ok(())
    }

    fn refresh_http_client_requests(&mut self) {
        let Some(root) = self.http.client_root.clone() else {
            self.http.client_requests.clear();
            self.http.client_cursor = 0;
            return;
        };
        let mut requests = collect_http_requests(&root);
        requests.sort();
        self.http.client_requests = requests;
        self.http.client_cursor = self
            .http
            .client_cursor
            .min(self.http.client_requests.len().saturating_sub(1));
    }

    fn start_client_request_input(&mut self) {
        if self.http.client_root.is_none() {
            self.status = String::from("Primero crea una collection con C");
            return;
        }
        self.http.input.clear();
        self.http.client_creating_collection = false;
        self.overlay = Overlay::HttpInput;
        self.status = String::from("Nombre del request nuevo:");
    }

    fn run_http_menu_item(&mut self) -> Result<()> {
        let Some(&(name, _)) = HTTP_MENU_ITEMS.get(self.http.menu_cursor) else {
            return Ok(());
        };
        self.overlay = Overlay::None;
        match name {
            "send" => self.send_http_request()?,
            "env" => self.open_http_env_picker(),
            "new" => self.start_new_request_input(),
            "init" => self.init_http_collection()?,
            "import" => self.import_postman_selected()?,
            "copy" => self.copy_response_body()?,
            _ => {}
        }
        Ok(())
    }

    /// The collection the HTTP actions apply to: the open request's, or the
    /// one containing the selected tree item.
    fn current_collection_root(&self) -> Option<PathBuf> {
        if self.overlay == Overlay::HttpClient {
            if let Some(root) = &self.http.client_root {
                return Some(root.clone());
            }
        }
        if let Some(view) = &self.http.view {
            return Some(view.collection_root.clone());
        }
        self.selected_item_path()
            .and_then(|path| http::find_collection_root(&path))
    }

    fn open_http_env_picker(&mut self) {
        let Some(root) = self.current_collection_root() else {
            self.show_alert("No hay collection: abri un request o usa :@http → init");
            return;
        };
        let (environments, errors) = http::load_environments(&root);
        let active = self.active_environment_name(&root);
        self.http.env_options = environments.into_iter().map(|env| env.name).collect();
        self.http.env_cursor = active
            .and_then(|name| {
                self.http
                    .env_options
                    .iter()
                    .position(|option| *option == name)
            })
            .map(|index| index + 1)
            .unwrap_or(0);
        self.http.env_root = Some(root);
        self.overlay = Overlay::HttpEnv;
        self.status = if errors.is_empty() {
            String::from("Environment: Enter para activarlo")
        } else {
            format!("Environment: {}", errors.join(" | "))
        };
    }

    fn confirm_http_env(&mut self) -> Result<()> {
        self.overlay = Overlay::None;
        let Some(root) = self.http.env_root.take() else {
            return Ok(());
        };
        let key = root.display().to_string();
        let choice = self
            .http
            .env_cursor
            .checked_sub(1)
            .and_then(|index| self.http.env_options.get(index))
            .cloned();
        match &choice {
            Some(name) => {
                self.config.http_environments.insert(key, name.clone());
            }
            None => {
                self.config.http_environments.remove(&key);
            }
        }
        self.config.save()?;
        self.refresh_http_view();
        self.status = match choice {
            Some(name) => format!("Environment activo: {name}"),
            None => String::from("Sin environment"),
        };
        Ok(())
    }

    fn start_new_request_input(&mut self) {
        let Some(dir) = self.current_tree_dir() else {
            self.status = String::from("No se pudo determinar la carpeta");
            return;
        };
        if http::find_collection_root(&dir).is_none() {
            self.show_alert("La carpeta no esta dentro de una collection: usa :@http → init");
            return;
        }
        self.http.input.clear();
        self.overlay = Overlay::HttpInput;
        self.status = String::from("Nombre del request nuevo:");
    }

    fn confirm_new_request(&mut self) -> Result<()> {
        self.overlay = Overlay::None;
        let Some(dir) = self.http.client_root.clone() else {
            return Ok(());
        };
        let path = http::create_request(&dir, &self.http.input)?;
        self.http.client_root = Some(dir);
        self.refresh_http_client_requests();
        self.restore_path_focus(&path)?;
        self.status = format!(
            "Request creado: {} · Shift+E para editarlo",
            display_name(&path)
        );
        Ok(())
    }

    fn init_http_collection(&mut self) -> Result<()> {
        let Some(dir) = self.current_tree_dir() else {
            self.status = String::from("No se pudo determinar la carpeta");
            return Ok(());
        };
        let created = http::init_collection(&dir)?;
        if let Some(example) = created
            .iter()
            .find(|path| path.file_name().is_some_and(|name| name == "health.yml"))
        {
            self.restore_path_focus(example)?;
        }
        self.status = format!(
            "Collection creada en {} ({} archivos) · copia .env.sample a .env",
            display_name(&dir),
            created.len()
        );
        Ok(())
    }

    fn create_http_collection_at_input_path(&mut self) -> Result<()> {
        let raw = self.http.input.trim();
        if raw.is_empty() {
            self.show_alert("Indica la carpeta donde guardar la collection");
            return Ok(());
        }
        let path = PathBuf::from(raw);
        let path = if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        };
        fs::create_dir_all(&path)?;
        let created = http::init_collection(&path)?;
        self.http.client_root = Some(path.clone());
        self.http.client_collections.push(path.clone());
        self.http.client_collection_cursor = self.http.client_collections.len() - 1;
        self.reload_items()?;
        self.refresh_http_client_requests();
        self.http.client_focus = 1;
        self.http.client_creating_collection = false;
        self.overlay = Overlay::HttpClient;
        self.status = format!(
            "Collection creada en {} ({} archivos)",
            path.display(),
            created.len()
        );
        Ok(())
    }

    fn import_postman_selected(&mut self) -> Result<()> {
        let Some(source) = self.action_target_path().filter(|path| path.is_file()) else {
            self.show_alert("Selecciona en el arbol el JSON exportado de Postman");
            return Ok(());
        };
        let is_json = source
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
        if !is_json {
            self.show_alert("import espera el .json exportado de Postman");
            return Ok(());
        }

        let summary = postman::import(&source)?;
        if summary.dest.is_dir() {
            self.expanded_dirs.insert(summary.dest.clone());
        }
        self.restore_path_focus(&summary.dest)?;

        let mut message = if summary.environments > 0 {
            format!(
                "Environment importado: {}",
                self.relative_label(&summary.dest)
            )
        } else {
            format!(
                "Collection importada en {}\n\n{} request(s), {} folder(s)",
                self.relative_label(&summary.dest),
                summary.requests,
                summary.folders
            )
        };
        if !summary.warnings.is_empty() {
            message.push_str("\n\n");
            message.push_str(
                &summary
                    .warnings
                    .iter()
                    .map(|warning| format!("⚠ {warning}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        self.show_alert(message);
        Ok(())
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
                self.git_output = vec![format!("Completed: git {name} ({})", output.status)];
                self.git_output
                    .extend(combined.lines().map(|l| l.to_string()));
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
                self.git_output = vec![format!("Completed: git commit ({})", output.status)];
                self.git_output
                    .extend(combined.lines().map(|l| l.to_string()));
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
        self.refresh_picker_preview();

        let verb = match self.file_op_kind {
            Some(FileOpKind::Copy) => "Copiar",
            _ => "Mover",
        };
        let dest = self
            .picker_dirs
            .get(self.picker_cursor)
            .map(|item| item.name.clone())
            .unwrap_or_default();

        if let Some(error) = &self.picker_preview_error {
            self.status = format!("{verb} a: {dest}  — no se puede leer: {error}");
            return;
        }

        if let Some(entry) = self.picker_preview.iter().find(|entry| entry.collides) {
            self.status = format!("{verb} a: {dest}  — ya existe \"{}\" ahi", entry.name);
            return;
        }

        self.status = format!("{verb} a: {dest}  (l/→ entrar | h/← salir | Enter confirmar)");
    }

    /// Reloads the listing of the highlighted destination so the user sees what
    /// is already inside the folder — and whether the operation would collide —
    /// before confirming.
    fn refresh_picker_preview(&mut self) {
        self.picker_preview.clear();
        self.picker_preview_total = 0;
        self.picker_preview_error = None;

        let Some(dir) = self
            .picker_dirs
            .get(self.picker_cursor)
            .map(|item| item.path.clone())
        else {
            return;
        };

        let source_name = self
            .file_op_source
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string());

        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                self.picker_preview_error = Some(err.to_string());
                return;
            }
        };

        let mut rows: Vec<PickerPreviewEntry> = entries
            .flatten()
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                let collides = source_name.as_deref() == Some(name.as_str());
                PickerPreviewEntry {
                    name,
                    is_dir,
                    collides,
                }
            })
            .collect();

        rows.sort_by(|left, right| {
            (!left.is_dir)
                .cmp(&!right.is_dir)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });

        self.picker_preview_total = rows.len();
        rows.truncate(PICKER_PREVIEW_LIMIT);
        self.picker_preview = rows;
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
                self.last_operation = Some(match kind {
                    FileOpKind::Move => LastOperation::Moved {
                        from: source.clone(),
                        to: dest.clone(),
                    },
                    FileOpKind::Copy => LastOperation::Copied {
                        created: dest.clone(),
                    },
                });
                let _ = self.reload_items();
                if let Some(index) = self.items.iter().position(|item| item.path == dest) {
                    self.selected_index = index;
                }
                let verb = if kind == FileOpKind::Move {
                    "Movido"
                } else {
                    "Copiado"
                };
                self.close_dest_picker(&format!(
                    "{verb} a: {}  (u para deshacer)",
                    dest_dir.display()
                ));
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

fn collect_http_requests(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut requests = Vec::new();
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "environments") {
                    pending.push(path);
                }
            } else if http::request_collection_root(&path).as_deref() == Some(root) {
                requests.push(path);
            }
        }
    }
    requests
}

fn collect_http_collections(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut collections = Vec::new();
    while let Some(dir) = pending.pop() {
        if http::is_collection_dir(&dir) {
            collections.push(dir);
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && !path.file_name().is_some_and(|name| {
                    matches!(
                        name.to_string_lossy().as_ref(),
                        ".git" | "target" | "node_modules" | ".venv"
                    )
                })
            {
                pending.push(path);
            }
        }
    }
    collections.sort();
    collections
}

fn git_status_label(status: GitStatusKind) -> &'static str {
    match status {
        GitStatusKind::Ignored => "ignored",
        GitStatusKind::Untracked => "untracked",
        GitStatusKind::Modified => "modified",
        GitStatusKind::Staged => "staged",
        GitStatusKind::Renamed => "renamed",
        GitStatusKind::Deleted => "deleted",
        GitStatusKind::Conflicted => "conflict",
    }
}

fn palette_aliases(name: &str) -> &'static [&'static str] {
    match name {
        "files" => &["buscar", "archivo", "search", "file"],
        "find" => &["buscar texto", "texto", "grep", "search text"],
        "grep" => &["buscar global", "buscar todos", "search all", "full text"],
        "marks" => &["bookmarks", "favoritos", "marcadores", "marks"],
        "copy" => &["copiar", "duplicar"],
        "copypath" => &["copiar ruta", "path", "ruta"],
        "delete" => &["borrar", "eliminar", "remove"],
        "edit" => &["editar", "abrir editor"],
        "git" => &["repo", "version"],
        "http" => &["postman", "bruno", "api", "request", "rest", "curl"],
        "httpenv" => &["environment", "entorno", "env"],
        "json" => &["@j", "@json", "json viewer", "formatear json"],
        "toc" => &["indice", "índice", "headings", "tabla"],
        "recent" => &["recientes", "historial", "ultimos", "últimos"],
        "@commands" => &["ayuda", "comandos", "help"],
        _ => &[],
    }
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

pub(crate) fn open_url_in_browser(url: &str) -> Result<bool> {
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

pub(crate) fn copy_to_clipboard(value: &str) -> Result<bool> {
    let mut clipboard = match Clipboard::new() {
        Ok(clipboard) => clipboard,
        Err(_) => return Ok(false),
    };

    clipboard.set_text(value.to_string())?;
    Ok(true)
}

fn response_summary(response: &HttpResponse) -> String {
    match (&response.error, response.status) {
        (Some(error), _) => format!("Request fallido: {error}"),
        (None, Some(status)) => format!(
            "{status} {} · {} · {}",
            response.reason,
            http::format_elapsed(response.elapsed),
            http::format_size(response.size)
        ),
        (None, None) => String::from("Request sin respuesta"),
    }
}

/// Short, human readable name of a path, for status messages.
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// Moves `target` into the trash under a unique, timestamped name and returns
/// where it landed. Used by delete and by undoing a copy, so no user action in
/// mdnav ever removes bytes outright.
fn move_to_trash(target: &Path) -> Result<PathBuf> {
    let trash = trash_dir()?;
    fs::create_dir_all(&trash).with_context(|| format!("No se pudo crear {}", trash.display()))?;

    let name = display_name(target);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);

    let mut dest = trash.join(format!("{stamp}-{name}"));
    let mut attempt = 1u32;
    while dest.exists() {
        dest = trash.join(format!("{stamp}-{attempt}-{name}"));
        attempt += 1;
    }

    move_path(target, &dest)?;
    Ok(dest)
}

/// Removes trash entries older than `TRASH_MAX_AGE`. Best effort: a failure
/// here must never keep mdnav from starting.
fn prune_trash() {
    let Ok(trash) = trash_dir() else {
        return;
    };
    let Ok(entries) = fs::read_dir(&trash) else {
        return;
    };
    let now = SystemTime::now();

    for entry in entries.flatten() {
        let expired = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|age| age > TRASH_MAX_AGE)
            .unwrap_or(false);

        if !expired {
            continue;
        }

        let path = entry.path();
        let _ = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
    }
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
