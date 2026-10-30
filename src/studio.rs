//! Postman-like HTTP studio: an in-memory request draft that autosaves as
//! OpenCollection YAML. Named auth profiles can land later as another
//! [`AuthKind`] without changing the table or the save path.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

use crate::http::{self, Auth, Body, HttpRequestSpec, KeyValue, Param, ParamKind, RequestSettings};

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const AUTOSAVE_DELAY: Duration = Duration::from_millis(400);
const SAVED_FLASH: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StudioPanel {
    Collection,
    Editor,
    Response,
}

const PANELS: [StudioPanel; 3] = [
    StudioPanel::Collection,
    StudioPanel::Editor,
    StudioPanel::Response,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorTab {
    Params,
    Headers,
    Body,
    Auth,
}

const TABS: [EditorTab; 4] = [
    EditorTab::Params,
    EditorTab::Headers,
    EditorTab::Body,
    EditorTab::Auth,
];

impl EditorTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Params => "Params",
            Self::Headers => "Headers",
            Self::Body => "Body",
            Self::Auth => "Auth",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellCol {
    Name,
    Value,
    Kind,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorCursor {
    Url,
    BodyKind,
    Row(usize),
    AuthKind,
    AuthField(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StudioPrompt {
    NewRequest,
    NewCollection,
    NewCollectionExample,
    Rename,
    RenameCollection,
}

/// A navigation that must wait until the user saves or discards the editor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StudioLeave {
    CyclePanel(bool),
    MoveTree(isize),
    Close,
    TreeOpen,
    TreeParent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyKind {
    None,
    Json,
    Text,
    Xml,
    Sparql,
    Form,
    Unsupported,
}

const BODY_CYCLE: [BodyKind; 6] = [
    BodyKind::None,
    BodyKind::Json,
    BodyKind::Text,
    BodyKind::Xml,
    BodyKind::Sparql,
    BodyKind::Form,
];

impl BodyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json => "json",
            Self::Text => "text",
            Self::Xml => "xml",
            Self::Sparql => "sparql",
            Self::Form => "form",
            Self::Unsupported => "otro",
        }
    }

    fn from_raw(kind: &str) -> Self {
        match kind {
            "json" => Self::Json,
            "text" => Self::Text,
            "xml" => Self::Xml,
            "sparql" => Self::Sparql,
            _ => Self::Unsupported,
        }
    }

    pub fn is_text(self) -> bool {
        matches!(self, Self::Json | Self::Text | Self::Xml | Self::Sparql)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodyForm {
    pub kind: BodyKind,
    pub text: String,
    pub form: Vec<KeyValue>,
    pub unsupported_label: Option<String>,
}

impl BodyForm {
    fn from_body(body: Body) -> Self {
        match body {
            Body::None => Self {
                kind: BodyKind::None,
                text: String::new(),
                form: Vec::new(),
                unsupported_label: None,
            },
            Body::Raw { kind, data } => {
                let parsed = BodyKind::from_raw(&kind);
                Self {
                    kind: parsed,
                    text: data,
                    form: Vec::new(),
                    unsupported_label: (parsed == BodyKind::Unsupported).then_some(kind),
                }
            }
            Body::Form(entries) => Self {
                kind: BodyKind::Form,
                text: String::new(),
                form: entries,
                unsupported_label: None,
            },
            Body::Unsupported(kind) => Self {
                kind: BodyKind::Unsupported,
                text: String::new(),
                form: Vec::new(),
                unsupported_label: Some(kind),
            },
        }
    }

    pub fn to_body(&self) -> Body {
        match self.kind {
            BodyKind::None => Body::None,
            BodyKind::Form => Body::Form(self.form.clone()),
            BodyKind::Unsupported => {
                Body::Unsupported(self.unsupported_label.clone().unwrap_or_default())
            }
            kind => Body::Raw {
                kind: kind.label().to_string(),
                data: self.text.clone(),
            },
        }
    }

    fn cycle(&mut self, delta: isize) {
        let current = BODY_CYCLE.iter().position(|kind| *kind == self.kind);
        let next = match current {
            Some(index) => {
                let len = BODY_CYCLE.len() as isize;
                BODY_CYCLE[(index as isize + delta).rem_euclid(len) as usize]
            }
            None if delta < 0 => BodyKind::Form,
            None => BodyKind::None,
        };
        self.kind = next;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthKind {
    None,
    Inherit,
    Bearer,
    Basic,
    ApiKey,
    Unsupported,
}

const AUTH_CYCLE: [AuthKind; 5] = [
    AuthKind::None,
    AuthKind::Inherit,
    AuthKind::Bearer,
    AuthKind::Basic,
    AuthKind::ApiKey,
];

impl AuthKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Inherit => "inherit",
            Self::Bearer => "bearer",
            Self::Basic => "basic",
            Self::ApiKey => "apikey",
            Self::Unsupported => "otro",
        }
    }
}

/// Fields of the per-request auth block. A future profile reference is another
/// kind plus a name; the YAML keeps using `{{process.env...}}` either way.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthForm {
    pub kind: AuthKind,
    pub token: String,
    pub username: String,
    pub password: String,
    pub key: String,
    pub value: String,
    pub in_query: bool,
    pub unsupported_label: Option<String>,
}

impl AuthForm {
    fn from_auth(auth: Auth) -> Self {
        let mut form = Self {
            kind: AuthKind::None,
            token: String::new(),
            username: String::new(),
            password: String::new(),
            key: String::new(),
            value: String::new(),
            in_query: false,
            unsupported_label: None,
        };
        match auth {
            Auth::None => form.kind = AuthKind::None,
            Auth::Inherit => form.kind = AuthKind::Inherit,
            Auth::Bearer { token } => {
                form.kind = AuthKind::Bearer;
                form.token = token;
            }
            Auth::Basic { username, password } => {
                form.kind = AuthKind::Basic;
                form.username = username;
                form.password = password;
            }
            Auth::ApiKey {
                key,
                value,
                in_query,
            } => {
                form.kind = AuthKind::ApiKey;
                form.key = key;
                form.value = value;
                form.in_query = in_query;
            }
            Auth::Unsupported(kind) => {
                form.kind = AuthKind::Unsupported;
                form.unsupported_label = Some(kind);
            }
        }
        form
    }

    pub fn to_auth(&self) -> Auth {
        match self.kind {
            AuthKind::None => Auth::None,
            AuthKind::Inherit => Auth::Inherit,
            AuthKind::Bearer => Auth::Bearer {
                token: self.token.clone(),
            },
            AuthKind::Basic => Auth::Basic {
                username: self.username.clone(),
                password: self.password.clone(),
            },
            AuthKind::ApiKey => Auth::ApiKey {
                key: self.key.clone(),
                value: self.value.clone(),
                in_query: self.in_query,
            },
            AuthKind::Unsupported => {
                Auth::Unsupported(self.unsupported_label.clone().unwrap_or_default())
            }
        }
    }

    pub fn field_count(&self) -> u8 {
        match self.kind {
            AuthKind::Bearer => 1,
            AuthKind::Basic => 2,
            AuthKind::ApiKey => 3,
            _ => 0,
        }
    }

    pub fn field_label(&self, index: u8, copy: &StudioCopy) -> &'static str {
        match (self.kind, index) {
            (AuthKind::Bearer, _) => copy.auth_token,
            (AuthKind::Basic, 0) => copy.auth_user,
            (AuthKind::Basic, _) => copy.auth_password,
            (AuthKind::ApiKey, 0) => copy.auth_key,
            (AuthKind::ApiKey, 1) => copy.auth_value,
            (AuthKind::ApiKey, _) => copy.auth_placement,
            _ => "",
        }
    }

    pub fn field_value(&self, index: u8) -> String {
        match (self.kind, index) {
            (AuthKind::Bearer, _) => self.token.clone(),
            (AuthKind::Basic, 0) => self.username.clone(),
            (AuthKind::Basic, _) => self.password.clone(),
            (AuthKind::ApiKey, 0) => self.key.clone(),
            (AuthKind::ApiKey, 1) => self.value.clone(),
            (AuthKind::ApiKey, _) => {
                if self.in_query {
                    String::from("query")
                } else {
                    String::from("header")
                }
            }
            _ => String::new(),
        }
    }

    fn set_field(&mut self, index: u8, value: String) -> bool {
        let slot: &mut String = match (self.kind, index) {
            (AuthKind::Bearer, _) => &mut self.token,
            (AuthKind::Basic, 0) => &mut self.username,
            (AuthKind::Basic, _) => &mut self.password,
            (AuthKind::ApiKey, 0) => &mut self.key,
            (AuthKind::ApiKey, 1) => &mut self.value,
            _ => return false,
        };
        if *slot == value {
            return false;
        }
        *slot = value;
        true
    }

    fn cycle(&mut self, delta: isize) {
        let current = AUTH_CYCLE.iter().position(|kind| *kind == self.kind);
        let next = match current {
            Some(index) => {
                let len = AUTH_CYCLE.len() as isize;
                AUTH_CYCLE[(index as isize + delta).rem_euclid(len) as usize]
            }
            None if delta < 0 => AuthKind::ApiKey,
            None => AuthKind::None,
        };
        self.kind = next;
    }

    fn toggle_placement(&mut self) -> bool {
        if self.kind != AuthKind::ApiKey {
            return false;
        }
        self.in_query = !self.in_query;
        true
    }
}

#[derive(Clone, Debug)]
pub struct RequestDraft {
    pub path: PathBuf,
    pub name: String,
    pub seq: Option<u64>,
    pub method: String,
    pub url: String,
    pub params: Vec<Param>,
    pub headers: Vec<KeyValue>,
    pub body: BodyForm,
    pub auth: AuthForm,
    pub settings: RequestSettings,
    pub docs: Option<String>,
    pub script_count: usize,
    pub assertion_count: usize,
    source: Value,
    rewrite_body: bool,
    rewrite_auth: bool,
}

impl RequestDraft {
    pub fn to_spec(&self) -> HttpRequestSpec {
        HttpRequestSpec {
            name: self.name.clone(),
            method: self.method.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            params: self.params.clone(),
            body: self.body.to_body(),
            auth: self.auth.to_auth(),
            settings: self.settings.clone(),
            docs: self.docs.clone(),
            script_count: self.script_count,
            assertion_count: self.assertion_count,
        }
    }

    pub fn table_len(&self, tab: EditorTab) -> usize {
        match tab {
            EditorTab::Params => self.params.len(),
            EditorTab::Headers => self.headers.len(),
            EditorTab::Body if self.body.kind == BodyKind::Form => self.body.form.len(),
            _ => 0,
        }
    }

    pub fn row_name(&self, tab: EditorTab, row: usize) -> String {
        self.row_entry(tab, row)
            .map(|entry| entry.name.clone())
            .unwrap_or_default()
    }

    pub fn row_value(&self, tab: EditorTab, row: usize) -> String {
        self.row_entry(tab, row)
            .map(|entry| entry.value.clone())
            .unwrap_or_default()
    }

    pub fn row_enabled(&self, tab: EditorTab, row: usize) -> bool {
        self.row_entry(tab, row)
            .map(|entry| entry.enabled)
            .unwrap_or(true)
    }

    pub fn row_kind_label(&self, row: usize) -> &'static str {
        match self.params.get(row).map(|param| param.kind) {
            Some(ParamKind::Path) => "path",
            _ => "query",
        }
    }

    fn row_entry(&self, tab: EditorTab, row: usize) -> Option<&KeyValue> {
        match tab {
            EditorTab::Params => self.params.get(row).map(|param| &param.entry),
            EditorTab::Headers => self.headers.get(row),
            EditorTab::Body => self.body.form.get(row),
            EditorTab::Auth => None,
        }
    }

    fn row_entry_mut(&mut self, tab: EditorTab, row: usize) -> Option<&mut KeyValue> {
        match tab {
            EditorTab::Params => self.params.get_mut(row).map(|param| &mut param.entry),
            EditorTab::Headers => self.headers.get_mut(row),
            EditorTab::Body => self.body.form.get_mut(row),
            EditorTab::Auth => None,
        }
    }

    fn set_cell(&mut self, tab: EditorTab, row: usize, col: CellCol, value: &str) -> bool {
        let Some(entry) = self.row_entry_mut(tab, row) else {
            return false;
        };
        let slot = match col {
            CellCol::Name => &mut entry.name,
            CellCol::Value => &mut entry.value,
            _ => return false,
        };
        if *slot == value {
            return false;
        }
        *slot = value.to_string();
        if tab == EditorTab::Body {
            self.rewrite_body = true;
        }
        true
    }

    fn toggle_enabled(&mut self, tab: EditorTab, row: usize) -> bool {
        let Some(entry) = self.row_entry_mut(tab, row) else {
            return false;
        };
        entry.enabled = !entry.enabled;
        if tab == EditorTab::Body {
            self.rewrite_body = true;
        }
        true
    }

    fn cycle_param_kind(&mut self, row: usize) -> bool {
        let Some(param) = self.params.get_mut(row) else {
            return false;
        };
        param.kind = match param.kind {
            ParamKind::Query => ParamKind::Path,
            ParamKind::Path => ParamKind::Query,
        };
        true
    }

    fn cycle_method(&mut self) -> bool {
        let index = METHODS
            .iter()
            .position(|method| *method == self.method)
            .unwrap_or(METHODS.len() - 1);
        let next = METHODS[(index + 1) % METHODS.len()];
        if self.method == next {
            return false;
        }
        self.method = next.to_string();
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListedRequest {
    pub path: PathBuf,
    pub name: String,
    pub method: String,
    pub seq: u64,
    pub rel: String,
}

#[derive(Clone, Debug)]
pub struct CollectionEntry {
    pub path: PathBuf,
    pub name: String,
    pub requests: Vec<ListedRequest>,
    pub expanded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreeNode {
    Collection(usize),
    Request { collection: usize, request: usize },
}

#[derive(Clone, Debug)]
pub struct EditorNav {
    pub tab: EditorTab,
    pub cursor: EditorCursor,
    pub col: CellCol,
    pub editing: bool,
    pub buffer: String,
}

impl Default for EditorNav {
    fn default() -> Self {
        Self {
            tab: EditorTab::Params,
            cursor: EditorCursor::Url,
            col: CellCol::Name,
            editing: false,
            buffer: String::new(),
        }
    }
}

impl EditorNav {
    pub fn columns(tab: EditorTab) -> &'static [CellCol] {
        match tab {
            EditorTab::Params => &[
                CellCol::Name,
                CellCol::Value,
                CellCol::Kind,
                CellCol::Enabled,
            ],
            EditorTab::Headers | EditorTab::Body => {
                &[CellCol::Name, CellCol::Value, CellCol::Enabled]
            }
            EditorTab::Auth => &[],
        }
    }
}

pub struct HttpStudio {
    pub active: bool,
    pub panel: StudioPanel,
    pub nav: EditorNav,
    pub root: Option<PathBuf>,
    pub items: Vec<ListedRequest>,
    pub selected: usize,
    pub draft: Option<RequestDraft>,
    pub prompt: Option<StudioPrompt>,
    pub prompt_text: String,
    pub confirm_delete: bool,
    /// True when `x` targets the collection under the tree cursor.
    pub delete_collection: bool,
    pub save_prompt: bool,
    pub pending_leave: Option<StudioLeave>,
    pub starter: bool,
    pub starter_dir: Option<PathBuf>,
    pub load_error: Option<String>,
    pub collections_root: Option<PathBuf>,
    pub collections: Vec<CollectionEntry>,
    pub tree_cursor: usize,
    pub filter: String,
    pub filtering: bool,
    pub exit_armed: bool,
    pub dirty: bool,
    pub dirty_at: Option<Instant>,
    pub saved_at: Option<Instant>,
    pub save_error: Option<String>,
}

impl Default for HttpStudio {
    fn default() -> Self {
        Self {
            active: false,
            panel: StudioPanel::Collection,
            nav: EditorNav::default(),
            root: None,
            items: Vec::new(),
            selected: 0,
            draft: None,
            prompt: None,
            prompt_text: String::new(),
            confirm_delete: false,
            delete_collection: false,
            save_prompt: false,
            pending_leave: None,
            starter: false,
            starter_dir: None,
            load_error: None,
            collections_root: None,
            collections: Vec::new(),
            tree_cursor: 0,
            filter: String::new(),
            filtering: false,
            exit_armed: false,
            dirty: false,
            dirty_at: None,
            saved_at: None,
            save_error: None,
        }
    }
}

impl HttpStudio {
    pub fn show_collection(&mut self, root: PathBuf, items: Vec<ListedRequest>, selected: usize) {
        let selected = if items.is_empty() {
            0
        } else {
            selected.min(items.len() - 1)
        };
        let root = http::canonical_path(&root);
        self.active = true;
        self.starter = false;
        self.root = Some(root.clone());
        self.items = items.clone();
        self.selected = selected;
        if let Some(collection) = self.collections.iter_mut().find(|entry| entry.path == root) {
            collection.requests = items;
            collection.expanded = true;
        }
    }

    pub fn cursor_is_text(&self) -> bool {
        match self.nav.cursor {
            EditorCursor::Url => true,
            EditorCursor::BodyKind => self
                .draft
                .as_ref()
                .is_some_and(|draft| draft.body.kind.is_text()),
            EditorCursor::AuthField(index) => self
                .draft
                .as_ref()
                .is_some_and(|draft| !(draft.auth.kind == AuthKind::ApiKey && index == 2)),
            EditorCursor::Row(_) => matches!(self.nav.col, CellCol::Name | CellCol::Value),
            EditorCursor::AuthKind => false,
        }
    }

    /// Starts editing the focused text field and appends `ch`, so typing a URL
    /// never falls through to shortcuts such as `n`.
    pub fn begin_typing(&mut self, ch: char) {
        let _ = self.activate();
        if self.nav.editing {
            self.nav.buffer.push(ch);
        }
    }

    pub fn visible_nodes(&self) -> Vec<TreeNode> {
        let query = self.filter.to_lowercase();
        let mut nodes = Vec::new();
        for (index, collection) in self.collections.iter().enumerate() {
            let name_matches = query.is_empty() || collection.name.to_lowercase().contains(&query);
            let requests: Vec<usize> = collection
                .requests
                .iter()
                .enumerate()
                .filter(|(_, request)| {
                    query.is_empty()
                        || request.name.to_lowercase().contains(&query)
                        || request.method.to_lowercase().contains(&query)
                })
                .map(|(request_index, _)| request_index)
                .collect();
            if !name_matches && requests.is_empty() {
                continue;
            }
            nodes.push(TreeNode::Collection(index));
            if collection.expanded || !query.is_empty() {
                let shown = if query.is_empty() {
                    (0..collection.requests.len()).collect::<Vec<_>>()
                } else {
                    requests
                };
                for request in shown {
                    nodes.push(TreeNode::Request {
                        collection: index,
                        request,
                    });
                }
            }
        }
        nodes
    }

    pub fn current_node(&self) -> Option<TreeNode> {
        self.visible_nodes().get(self.tree_cursor).copied()
    }

    pub fn move_tree(&mut self, delta: isize) -> Option<TreeNode> {
        let nodes = self.visible_nodes();
        if nodes.is_empty() || delta == 0 {
            return None;
        }
        let last = nodes.len() as isize - 1;
        let next = (self.tree_cursor as isize + delta).clamp(0, last) as usize;
        if next == self.tree_cursor {
            return None;
        }
        self.tree_cursor = next;
        Some(nodes[next])
    }

    pub fn focus_tree_on(&mut self, collection: usize, request: Option<usize>) {
        let nodes = self.visible_nodes();
        let found = nodes.iter().position(|node| match (node, request) {
            (
                TreeNode::Request {
                    collection: c,
                    request: r,
                },
                Some(want),
            ) => *c == collection && *r == want,
            (TreeNode::Collection(c), None) => *c == collection,
            _ => false,
        });
        if let Some(index) = found {
            self.tree_cursor = index;
        }
    }

    pub fn toggle_collection(&mut self, index: usize) {
        if let Some(collection) = self.collections.get_mut(index) {
            collection.expanded = !collection.expanded;
        }
        let count = self.visible_nodes().len();
        if count == 0 {
            self.tree_cursor = 0;
        } else if self.tree_cursor >= count {
            self.tree_cursor = count - 1;
        }
    }

    pub fn saved_recent(&self) -> bool {
        self.saved_at
            .map(|saved| saved.elapsed() < SAVED_FLASH)
            .unwrap_or(false)
    }

    /// Unsaved field text or a dirty draft. Navigation must ask before writing.
    pub fn needs_save_prompt(&self) -> bool {
        self.nav.editing || self.dirty
    }

    /// The timer must not write the file: that would skip the save/discard prompt.
    pub fn autosave_due(&self) -> bool {
        let _ = (
            self.active,
            self.dirty,
            self.nav.editing,
            self.dirty_at,
            AUTOSAVE_DELAY,
        );
        false
    }

    pub fn cancel_field_edit(&mut self) {
        self.nav.editing = false;
        self.nav.buffer.clear();
    }

    pub fn tree_on_collection(&self) -> bool {
        matches!(self.current_node(), Some(TreeNode::Collection(_)))
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.dirty_at = Some(Instant::now());
        self.save_error = None;
    }

    pub fn note_saved(&mut self) {
        self.dirty = false;
        self.dirty_at = None;
        self.saved_at = Some(Instant::now());
        self.save_error = None;
    }

    pub fn selected_item(&self) -> Option<&ListedRequest> {
        self.items.get(self.selected)
    }

    pub fn cycle_panel(&mut self, backward: bool) {
        let index = PANELS
            .iter()
            .position(|panel| *panel == self.panel)
            .unwrap_or(0) as isize;
        let delta = if backward { -1 } else { 1 };
        let next = (index + delta).rem_euclid(PANELS.len() as isize) as usize;
        self.panel = PANELS[next];
    }

    pub fn switch_tab(&mut self, delta: isize) {
        self.nav.editing = false;
        self.nav.buffer.clear();
        let index = TABS
            .iter()
            .position(|tab| *tab == self.nav.tab)
            .unwrap_or(0) as isize;
        let next = (index + delta).rem_euclid(TABS.len() as isize) as usize;
        self.nav.tab = TABS[next];
        self.nav.col = CellCol::Name;
        if self.nav.cursor != EditorCursor::Url {
            self.nav.cursor = self.first_content_cursor();
        }
    }

    fn first_content_cursor(&self) -> EditorCursor {
        match self.nav.tab {
            EditorTab::Body => EditorCursor::BodyKind,
            EditorTab::Auth => EditorCursor::AuthKind,
            _ => EditorCursor::Row(0),
        }
    }

    pub fn move_vertical(&mut self, delta: isize) {
        if delta == 0 || self.draft.is_none() {
            return;
        }
        let tab = self.nav.tab;
        let len = self
            .draft
            .as_ref()
            .map(|draft| draft.table_len(tab))
            .unwrap_or(0);
        let field_count = self
            .draft
            .as_ref()
            .map(|draft| draft.auth.field_count())
            .unwrap_or(0);
        let cursor = self.nav.cursor;
        let next = match (cursor, delta > 0) {
            (EditorCursor::Url, true) => Some(self.first_content_cursor()),
            (EditorCursor::Url, false) => None,
            (EditorCursor::BodyKind, true)
                if self
                    .draft
                    .as_ref()
                    .is_some_and(|d| d.body.kind == BodyKind::Form) =>
            {
                Some(EditorCursor::Row(0))
            }
            (EditorCursor::BodyKind, false) => Some(EditorCursor::Url),
            (EditorCursor::AuthKind, true) if field_count > 0 => Some(EditorCursor::AuthField(0)),
            (EditorCursor::AuthKind, false) => Some(EditorCursor::Url),
            (EditorCursor::AuthField(index), true) => {
                if index + 1 < field_count {
                    Some(EditorCursor::AuthField(index + 1))
                } else {
                    None
                }
            }
            (EditorCursor::AuthField(0), false) => Some(EditorCursor::AuthKind),
            (EditorCursor::AuthField(index), false) => Some(EditorCursor::AuthField(index - 1)),
            (EditorCursor::Row(row), true) => {
                if len > 0 && row + 1 < len {
                    Some(EditorCursor::Row(row + 1))
                } else {
                    None
                }
            }
            (EditorCursor::Row(0), false) => Some(match tab {
                EditorTab::Body => EditorCursor::BodyKind,
                _ => EditorCursor::Url,
            }),
            (EditorCursor::Row(row), false) => Some(EditorCursor::Row(row - 1)),
            _ => None,
        };
        if let Some(next) = next {
            self.nav.cursor = next;
            if !matches!(next, EditorCursor::Row(_)) {
                self.nav.col = CellCol::Name;
            }
        }
    }

    /// Left/Right. `true` when the draft changed (body or auth kind).
    pub fn move_horizontal(&mut self, delta: isize) -> bool {
        if delta == 0 || self.draft.is_none() {
            return false;
        }
        match self.nav.cursor {
            EditorCursor::BodyKind => {
                if let Some(draft) = self.draft.as_mut() {
                    draft.body.cycle(delta);
                    draft.rewrite_body = true;
                }
                self.nav.cursor = EditorCursor::BodyKind;
                true
            }
            EditorCursor::AuthKind => {
                if let Some(draft) = self.draft.as_mut() {
                    draft.auth.cycle(delta);
                    draft.rewrite_auth = true;
                }
                self.nav.cursor = EditorCursor::AuthKind;
                true
            }
            EditorCursor::AuthField(index)
                if self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.auth.kind == AuthKind::ApiKey && index == 2) =>
            {
                self.draft
                    .as_mut()
                    .map(|draft| {
                        draft.rewrite_auth = true;
                        draft.auth.toggle_placement()
                    })
                    .unwrap_or(false)
            }
            EditorCursor::Row(_) => {
                let cols = EditorNav::columns(self.nav.tab);
                if cols.is_empty() {
                    return false;
                }
                let index = cols
                    .iter()
                    .position(|col| *col == self.nav.col)
                    .unwrap_or(0) as isize;
                let next = (index + delta).rem_euclid(cols.len() as isize) as usize;
                self.nav.col = cols[next];
                false
            }
            _ => false,
        }
    }

    /// Enter on the current field. `Ok(true)` when the draft changed without
    /// entering text mode (toggles). `Err(())` is unused; editing starts with
    /// `Ok(false)` and [`Self::is_editing`].
    pub fn activate(&mut self) -> bool {
        let Some(draft_kind) = self.draft.as_ref().map(|draft| {
            (
                draft.body.kind,
                draft.auth.kind,
                draft.table_len(self.nav.tab),
            )
        }) else {
            return false;
        };
        match self.nav.cursor {
            EditorCursor::Url => {
                let url = self
                    .draft
                    .as_ref()
                    .map(|draft| draft.url.clone())
                    .unwrap_or_default();
                self.begin_edit(url);
                false
            }
            EditorCursor::BodyKind if draft_kind.0.is_text() => {
                let text = self
                    .draft
                    .as_ref()
                    .map(|draft| draft.body.text.clone())
                    .unwrap_or_default();
                self.begin_edit(text);
                false
            }
            EditorCursor::BodyKind if draft_kind.0 == BodyKind::Form => {
                self.nav.cursor = EditorCursor::Row(0);
                false
            }
            EditorCursor::BodyKind => self.move_horizontal(1),
            EditorCursor::AuthKind => self.move_horizontal(1),
            EditorCursor::AuthField(index) if draft_kind.1 == AuthKind::ApiKey && index == 2 => {
                self.move_horizontal(1)
            }
            EditorCursor::AuthField(index) => {
                let value = self
                    .draft
                    .as_ref()
                    .map(|draft| draft.auth.field_value(index))
                    .unwrap_or_default();
                self.begin_edit(value);
                false
            }
            EditorCursor::Row(row) => {
                if draft_kind.2 == 0 || row >= draft_kind.2 {
                    return false;
                }
                match self.nav.col {
                    CellCol::Enabled => self
                        .draft
                        .as_mut()
                        .map(|draft| draft.toggle_enabled(self.nav.tab, row))
                        .unwrap_or(false),
                    CellCol::Kind => self
                        .draft
                        .as_mut()
                        .map(|draft| draft.cycle_param_kind(row))
                        .unwrap_or(false),
                    col => {
                        let text = self
                            .draft
                            .as_ref()
                            .and_then(|draft| match col {
                                CellCol::Name => Some(draft.row_name(self.nav.tab, row)),
                                CellCol::Value => Some(draft.row_value(self.nav.tab, row)),
                                _ => None,
                            })
                            .unwrap_or_default();
                        self.begin_edit(text);
                        false
                    }
                }
            }
        }
    }

    fn begin_edit(&mut self, text: String) {
        self.nav.buffer = text;
        self.nav.editing = true;
    }

    pub fn is_multiline(&self) -> bool {
        self.nav.editing && matches!(self.nav.cursor, EditorCursor::BodyKind)
    }

    pub fn push_char(&mut self, ch: char) {
        if self.nav.editing {
            self.nav.buffer.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.nav.editing {
            self.nav.buffer.pop();
        }
    }

    pub fn push_newline(&mut self) {
        if self.is_multiline() {
            self.nav.buffer.push('\n');
        }
    }

    /// Applies the buffer. `true` when the draft changed.
    pub fn commit_edit(&mut self) -> bool {
        if !self.nav.editing {
            return false;
        }
        let changed = self.apply_buffer();
        self.nav.editing = false;
        self.nav.buffer.clear();
        if changed {
            self.mark_dirty();
        }
        changed
    }

    fn apply_buffer(&mut self) -> bool {
        let buffer = self.nav.buffer.clone();
        let cursor = self.nav.cursor;
        let col = self.nav.col;
        let tab = self.nav.tab;
        let Some(draft) = self.draft.as_mut() else {
            return false;
        };
        match cursor {
            EditorCursor::Url => {
                if draft.url == buffer {
                    return false;
                }
                draft.url = buffer;
                true
            }
            EditorCursor::BodyKind => {
                if draft.body.text == buffer {
                    return false;
                }
                draft.body.text = buffer;
                draft.rewrite_body = true;
                true
            }
            EditorCursor::Row(row) => draft.set_cell(tab, row, col, &buffer),
            EditorCursor::AuthField(index) => {
                let changed = draft.auth.set_field(index, buffer);
                if changed {
                    draft.rewrite_auth = true;
                }
                changed
            }
            _ => false,
        }
    }

    pub fn cycle_method(&mut self) -> bool {
        let changed = self
            .draft
            .as_mut()
            .map(|draft| draft.cycle_method())
            .unwrap_or(false);
        if changed {
            self.mark_dirty();
            if let Some(draft) = &self.draft {
                let method = draft.method.clone();
                let path = draft.path.clone();
                if let Some(item) = self.items.iter_mut().find(|item| item.path == path) {
                    item.method = method;
                }
            }
        }
        changed
    }

    pub fn add_row(&mut self) -> bool {
        let _ = self.commit_edit();
        let tab = self.nav.tab;
        let Some(index) = self.push_row(tab) else {
            return false;
        };
        self.nav.tab = tab;
        self.nav.cursor = EditorCursor::Row(index);
        self.nav.col = CellCol::Name;
        self.nav.editing = true;
        self.nav.buffer.clear();
        self.mark_dirty();
        true
    }

    fn push_row(&mut self, tab: EditorTab) -> Option<usize> {
        let draft = self.draft.as_mut()?;
        let blank = KeyValue {
            name: String::new(),
            value: String::new(),
            enabled: true,
        };
        match tab {
            EditorTab::Params => {
                draft.params.push(Param {
                    entry: blank,
                    kind: ParamKind::Query,
                });
                Some(draft.params.len() - 1)
            }
            EditorTab::Headers => {
                draft.headers.push(blank);
                Some(draft.headers.len() - 1)
            }
            EditorTab::Body => {
                if draft.body.kind == BodyKind::None {
                    draft.body.kind = BodyKind::Form;
                    draft.rewrite_body = true;
                }
                if draft.body.kind != BodyKind::Form {
                    return None;
                }
                draft.rewrite_body = true;
                draft.body.form.push(blank);
                Some(draft.body.form.len() - 1)
            }
            EditorTab::Auth => None,
        }
    }

    pub fn remove_row(&mut self) -> bool {
        let _ = self.commit_edit();
        let EditorCursor::Row(row) = self.nav.cursor else {
            return false;
        };
        let tab = self.nav.tab;
        let Some(draft) = self.draft.as_mut() else {
            return false;
        };
        let removed = match tab {
            EditorTab::Params if row < draft.params.len() => {
                draft.params.remove(row);
                true
            }
            EditorTab::Headers if row < draft.headers.len() => {
                draft.headers.remove(row);
                true
            }
            EditorTab::Body if draft.body.kind == BodyKind::Form && row < draft.body.form.len() => {
                draft.body.form.remove(row);
                draft.rewrite_body = true;
                true
            }
            _ => false,
        };
        if removed {
            let len = self
                .draft
                .as_ref()
                .map(|draft| draft.table_len(tab))
                .unwrap_or(0);
            if len == 0 {
                self.nav.cursor = if tab == EditorTab::Body {
                    EditorCursor::BodyKind
                } else {
                    EditorCursor::Url
                };
            } else if row >= len {
                self.nav.cursor = EditorCursor::Row(len - 1);
            }
            self.mark_dirty();
        }
        removed
    }

    pub fn editing_text(&self, cursor: EditorCursor, col: CellCol) -> Option<&str> {
        if !self.nav.editing || self.nav.cursor != cursor {
            return None;
        }
        let column_ok = !matches!(cursor, EditorCursor::Row(_)) || self.nav.col == col;
        column_ok.then_some(self.nav.buffer.as_str())
    }
}

pub struct StudioCopy {
    pub saved: &'static str,
    pub collection: &'static str,
    pub request: &'static str,
    pub no_requests: &'static str,
    pub starter_title: &'static str,
    pub starter_body: &'static str,
    pub starter_enter: &'static str,
    pub starter_esc: &'static str,
    pub folder: &'static str,
    pub delete_title: &'static str,
    pub delete_hint: &'static str,
    pub new_title: &'static str,
    pub rename_title: &'static str,
    pub prompt_hint: &'static str,
    pub col_key: &'static str,
    pub col_value: &'static str,
    pub col_on: &'static str,
    pub col_kind: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
    pub body_kind: &'static str,
    pub auth_kind: &'static str,
    pub auth_token: &'static str,
    pub auth_user: &'static str,
    pub auth_password: &'static str,
    pub auth_key: &'static str,
    pub auth_value: &'static str,
    pub auth_placement: &'static str,
    pub empty_table: &'static str,
    pub empty_body: &'static str,
    pub load_fail: &'static str,
    pub welcome_split: &'static str,
    pub welcome_json: &'static str,
    pub welcome_http: &'static str,
    pub help_open: &'static str,
    pub help_panels: &'static str,
    pub help_tabs: &'static str,
    pub help_edit: &'static str,
    pub help_method: &'static str,
    pub help_send: &'static str,
    pub help_env: &'static str,
    pub help_new: &'static str,
    pub help_yaml: &'static str,
    pub help_close: &'static str,
    pub env_none: &'static str,
    pub unresolved: &'static str,
}

pub fn copy(lang: &str) -> StudioCopy {
    if lang == "en" {
        StudioCopy {
            saved: "saved",
            collection: "Collection",
            request: "Request",
            no_requests: "No requests. n creates one.",
            starter_title: "HTTP",
            starter_body: "There is no collection in this folder.\n\nEnter creates an example, with a request to start from.",
            starter_enter: "Enter  create example",
            starter_esc: "Esc    back to the tree",
            folder: "Folder",
            delete_title: "Delete request?",
            delete_hint: "Enter confirms    Esc cancels",
            new_title: "New request",
            rename_title: "Rename request",
            prompt_hint: "Enter confirms    Esc cancels",
            col_key: "key",
            col_value: "value",
            col_on: "on",
            col_kind: "kind",
            yes: "yes",
            no: "no",
            body_kind: "type",
            auth_kind: "type",
            auth_token: "token",
            auth_user: "username",
            auth_password: "password",
            auth_key: "key",
            auth_value: "value",
            auth_placement: "in",
            empty_table: "Empty. + adds a row.",
            empty_body: "No body. Left/Right changes the type.",
            load_fail: "Could not read the request",
            welcome_split: "panel proportion 1 to 5",
            welcome_json: "open the JSON viewer",
            welcome_http: "open the HTTP studio",
            help_open: "open the HTTP studio",
            help_panels: "cycle collection, editor, response",
            help_tabs: "h/l columns, [ ] tabs",
            help_edit: "edit the field    Esc leaves it",
            help_method: "cycle the method",
            help_send: "send with Shift+S",
            help_env: "pick the environment",
            help_new: "new    r rename    y duplicate    x delete",
            help_yaml: "edit the raw YAML",
            help_close: "leave the field, then the studio",
            env_none: "no environment",
            unresolved: "unresolved",
        }
    } else {
        StudioCopy {
            saved: "guardado",
            collection: "Collection",
            request: "Request",
            no_requests: "No hay requests. n crea uno.",
            starter_title: "HTTP",
            starter_body: "No hay una collection en esta carpeta.\n\nEnter crea una de ejemplo, con un request para empezar.",
            starter_enter: "Enter  crear ejemplo",
            starter_esc: "Esc    volver al arbol",
            folder: "Carpeta",
            delete_title: "Eliminar request?",
            delete_hint: "Enter confirma    Esc cancela",
            new_title: "Request nuevo",
            rename_title: "Renombrar request",
            prompt_hint: "Enter confirma    Esc cancela",
            col_key: "clave",
            col_value: "valor",
            col_on: "on",
            col_kind: "tipo",
            yes: "si",
            no: "no",
            body_kind: "tipo",
            auth_kind: "tipo",
            auth_token: "token",
            auth_user: "usuario",
            auth_password: "contraseña",
            auth_key: "clave",
            auth_value: "valor",
            auth_placement: "ubicacion",
            empty_table: "Vacio · + agrega una fila",
            empty_body: "Sin body. Izq/Der cambia el tipo.",
            load_fail: "No se pudo leer el request",
            welcome_split: "proporcion de paneles 1 a 5",
            welcome_json: "abrir el visor JSON",
            welcome_http: "abrir el estudio HTTP",
            help_open: "abrir el estudio HTTP",
            help_panels: "ciclar collection, editor y response",
            help_tabs: "h/l columnas, [ ] pestañas",
            help_edit: "editar el campo    Esc lo deja",
            help_method: "cambiar el método",
            help_send: "enviar con Shift+S",
            help_env: "elegir environment",
            help_new: "nuevo    r renombrar    y duplicar    x borrar",
            help_yaml: "editar el YAML crudo",
            help_close: "salir del campo y despues del estudio",
            env_none: "sin environment",
            unresolved: "sin resolver",
        }
    }
}

pub fn panel_footer(
    panel: StudioPanel,
    editing: bool,
    exit_armed: bool,
    on_collection: bool,
    save_prompt: bool,
    lang: &str,
) -> String {
    let es = lang != "en";
    if save_prompt {
        return if es {
            String::from("GUARDAR  g guardar · d descartar · Esc cancelar")
        } else {
            String::from("SAVE  g save · d discard · Esc cancel")
        };
    }
    let base = if editing {
        if es {
            "EDITANDO  Esc guarda el campo · Shift+S enviar · Tab cambia de panel"
        } else {
            "EDITING  Esc saves the field · Shift+S send · Tab switches panel"
        }
    } else {
        match panel {
            StudioPanel::Collection if on_collection => {
                if es {
                    "COLLECTION  j/k · h/l árbol · n request · c vacía · C ejemplo · r renombrar · y duplicar · x borrar · / buscar · Shift+S enviar · Esc"
                } else {
                    "COLLECTION  j/k · h/l tree · n request · c empty · C example · r rename · y duplicate · x delete · / search · Shift+S send · Esc"
                }
            }
            StudioPanel::Collection => {
                if es {
                    "REQUEST  j/k · h volver · n nuevo · r renombrar · y duplicar · x borrar · Shift+S enviar · Esc"
                } else {
                    "REQUEST  j/k · h back · n new · r rename · y duplicate · x delete · Shift+S send · Esc"
                }
            }
            StudioPanel::Editor => {
                if es {
                    "EDITOR  h/l columnas · [ ] pestañas · letra edita · m método · Shift+S enviar · e env · Esc"
                } else {
                    "EDITOR  h/l columns · [ ] tabs · letter edits · m method · Shift+S send · e env · Esc"
                }
            }
            StudioPanel::Response => {
                if es {
                    "RESPONSE  j/k · [ ] tabs · y copiar · Shift+S enviar · Esc"
                } else {
                    "RESPONSE  j/k · [ ] tabs · y copy · Shift+S send · Esc"
                }
            }
        }
    };
    if exit_armed {
        if es {
            format!("{base} · Esc de nuevo para salir")
        } else {
            format!("{base} · Esc again to leave")
        }
    } else {
        base.to_string()
    }
}

/// The folder that holds every collection, independent of the working directory.
pub fn collections_dir(custom: &str) -> Result<PathBuf> {
    let path = if custom.trim().is_empty() {
        dirs::config_dir()
            .context("No se pudo resolver la carpeta de config")?
            .join("mdnav")
            .join("collections")
    } else {
        PathBuf::from(custom.trim())
    };
    fs::create_dir_all(&path).with_context(|| format!("No se pudo crear {}", path.display()))?;
    Ok(http::canonical_path(&path))
}

pub fn collection_env_key(path: &Path) -> String {
    http::canonical_path(path).to_string_lossy().into_owned()
}

/// Looks up the active environment, treating `/path/.` and `/path` as the same
/// collection even if an older config saved the dotted form.
pub fn env_name_in<'a>(map: &'a BTreeMap<String, String>, root: &Path) -> Option<&'a str> {
    let wanted = collection_env_key(root);
    if let Some(name) = map.get(&wanted) {
        return Some(name.as_str());
    }
    map.iter().find_map(|(key, name)| {
        (collection_env_key(Path::new(key)) == wanted).then_some(name.as_str())
    })
}

pub fn list_collections(root: &Path) -> Vec<CollectionEntry> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = http::canonical_path(&entry.path());
        if !path.join(http::COLLECTION_FILE).is_file() {
            continue;
        }
        found.push(CollectionEntry {
            name: collection_title(&path),
            requests: list_requests(&path),
            expanded: false,
            path,
        });
    }
    found.sort_by(|left, right| left.name.cmp(&right.name).then(left.path.cmp(&right.path)));
    found
}

/// Creates the httpbin example the first time the library is empty, so `:@http`
/// is usable without visiting a project folder.
pub fn ensure_library(dir: &Path) -> Result<Vec<CollectionEntry>> {
    fs::create_dir_all(dir)?;
    let mut found = list_collections(dir);
    if found.is_empty() {
        let example = dir.join("ejemplo");
        fs::create_dir_all(&example)?;
        http::init_collection(&example)?;
        found = list_collections(dir);
    }
    if let Some(first) = found.first_mut() {
        first.expanded = true;
    }
    Ok(found)
}

pub fn create_named_collection(root: &Path, name: &str, with_example: bool) -> Result<PathBuf> {
    validate_entry_name(name)?;
    let folder = name.split_whitespace().collect::<Vec<_>>().join("-");
    let dir = root.join(http::sanitize_file_name(&folder));
    if dir.exists() {
        bail!("Ya existe {}", dir.display());
    }
    fs::create_dir_all(&dir)?;
    http::init_collection_in(&dir, with_example)?;
    Ok(http::canonical_path(&dir))
}

pub fn rename_collection(path: &Path, new_name: &str) -> Result<PathBuf> {
    let new_name = new_name.trim();
    validate_entry_name(new_name)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("La collection no tiene carpeta"))?;
    let folder = new_name.split_whitespace().collect::<Vec<_>>().join("-");
    let dest = parent.join(http::sanitize_file_name(&folder));
    if dest != path && dest.exists() {
        bail!("Ya existe {}", dest.display());
    }
    if dest != path {
        fs::rename(path, &dest)
            .with_context(|| format!("No se pudo renombrar {}", path.display()))?;
    }
    set_collection_display_name(&dest, new_name)?;
    Ok(http::canonical_path(&dest))
}

pub fn duplicate_collection(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("La collection no tiene carpeta"))?;
    let title = collection_title(path);
    let copy_label = if title.trim().is_empty() {
        String::from("copia")
    } else {
        format!("{title} copia")
    };
    let folder = copy_label.split_whitespace().collect::<Vec<_>>().join("-");
    let dest = unique_collection_dir(parent, &folder);
    copy_dir_all(path, &dest)?;
    set_collection_display_name(&dest, &copy_label)?;
    Ok(http::canonical_path(&dest))
}

pub fn delete_collection(path: &Path) -> Result<()> {
    fs::remove_dir_all(path).with_context(|| format!("No se pudo borrar {}", path.display()))
}

/// Moves the active-environment entry when a collection directory changes name.
pub fn retarget_env_key(map: &mut BTreeMap<String, String>, old: &Path, new: &Path) -> bool {
    let old_key = collection_env_key(old);
    let new_key = collection_env_key(new);
    let value = map.iter().find_map(|(key, value)| {
        (collection_env_key(Path::new(key)) == old_key).then(|| value.clone())
    });
    let Some(value) = value else {
        return false;
    };
    map.retain(|key, _| collection_env_key(Path::new(key)) != old_key);
    map.insert(new_key, value);
    true
}

fn unique_collection_dir(parent: &Path, base: &str) -> PathBuf {
    let mut used = HashSet::new();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                used.insert(name.to_lowercase());
            }
        }
    }
    let folder = http::unique_name(&mut used, &http::sanitize_file_name(base), "");
    parent.join(folder)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("No se pudo crear {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("No se pudo leer {}", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)
                .with_context(|| format!("No se pudo copiar {}", from.display()))?;
        }
    }
    Ok(())
}

fn set_collection_display_name(dir: &Path, name: &str) -> Result<()> {
    let path = dir.join(http::COLLECTION_FILE);
    let text =
        fs::read_to_string(&path).with_context(|| format!("No se pudo leer {}", path.display()))?;
    let mut value: Value =
        serde_saphyr::from_str(&text).map_err(|err| anyhow!("YAML invalido: {err}"))?;
    if let Some(info) = value.get_mut("info").and_then(Value::as_object_mut) {
        info.insert("name".into(), Value::String(name.to_string()));
    }
    let mut yaml = http::to_yaml(&value)?;
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }
    fs::write(&path, yaml).with_context(|| format!("No se pudo escribir {}", path.display()))
}

pub fn validate_entry_name(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("Hace falta un nombre");
    }
    if name.contains('.') || name.contains('/') || name.contains('\\') {
        bail!("El nombre no puede incluir '.' ni '/'");
    }
    Ok(())
}

fn collection_title(dir: &Path) -> String {
    let fallback = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("collection"));
    let Ok(text) = fs::read_to_string(dir.join(http::COLLECTION_FILE)) else {
        return fallback;
    };
    let Ok(value) = serde_saphyr::from_str::<Value>(&text) else {
        return fallback;
    };
    value
        .get("info")
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or(fallback)
}

/// `: @http` and `: @http env`. `@httpenv` is not this command.
pub fn http_palette_action(query: &str) -> Option<bool> {
    let mut parts = query.split_whitespace();
    if parts.next() != Some("@http") {
        return None;
    }
    match parts.next() {
        None => Some(false),
        Some("env") if parts.next().is_none() => Some(true),
        _ => None,
    }
}

pub fn list_requests(root: &Path) -> Vec<ListedRequest> {
    // `request_collection_root` returns a canonical path. On Windows that is
    // the long path (`runneradmin`), while `temp_dir()` is often the 8.3 form
    // (`RUNNER~1`). Compare and strip against the canonical root.
    let root = http::canonical_path(root);
    let mut items = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children = Vec::new();
        for entry in entries.flatten() {
            let path = http::canonical_path(&entry.path());
            if path.is_dir() {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name.starts_with('.') || name == "environments" {
                    continue;
                }
                children.push(path);
                continue;
            }
            if http::request_collection_root(&path).as_deref() != Some(root.as_path()) {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(draft) = draft_from_yaml(path.clone(), &content) else {
                continue;
            };
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            items.push(ListedRequest {
                path,
                name: display_name(&draft),
                method: draft.method,
                seq: draft.seq.unwrap_or(0),
                rel,
            });
        }
        children.sort();
        stack.extend(children);
    }
    items.sort_by(|left, right| {
        request_parent(&left.rel)
            .cmp(request_parent(&right.rel))
            .then(left.seq.cmp(&right.seq))
            .then(left.name.cmp(&right.name))
            .then(left.rel.cmp(&right.rel))
    });
    items
}

fn request_parent(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

fn display_name(draft: &RequestDraft) -> String {
    if draft.name.trim().is_empty() {
        draft
            .path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| String::from("request"))
    } else {
        draft.name.clone()
    }
}

pub fn load_draft(path: &Path) -> Result<RequestDraft> {
    let content =
        fs::read_to_string(path).with_context(|| format!("No se pudo leer {}", path.display()))?;
    draft_from_yaml(path.to_path_buf(), &content)
}

pub fn draft_from_yaml(path: PathBuf, content: &str) -> Result<RequestDraft> {
    let spec = http::parse_request(content)?;
    let source =
        serde_saphyr::from_str::<Value>(content).map_err(|err| anyhow!("YAML invalido: {err}"))?;
    let seq = source
        .get("info")
        .and_then(|info| info.get("seq"))
        .and_then(Value::as_u64);
    let body = BodyForm::from_body(spec.body);
    let auth = AuthForm::from_auth(spec.auth);
    let rewrite_body = body.kind != BodyKind::Unsupported;
    let rewrite_auth = auth.kind != AuthKind::Unsupported;
    Ok(RequestDraft {
        path,
        name: spec.name,
        seq,
        method: spec.method,
        url: spec.url,
        params: spec.params,
        headers: spec.headers,
        body,
        auth,
        settings: spec.settings,
        docs: spec.docs,
        script_count: spec.script_count,
        assertion_count: spec.assertion_count,
        source,
        rewrite_body,
        rewrite_auth,
    })
}

pub fn save_draft(draft: &mut RequestDraft) -> Result<()> {
    let doc = document_from_draft(draft)?;
    let mut yaml = http::to_yaml(&doc)?;
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }
    atomic_write(&draft.path, &yaml)?;
    draft.source = doc;
    Ok(())
}

#[cfg(test)]
fn draft_yaml(draft: &RequestDraft) -> Result<String> {
    let doc = document_from_draft(draft)?;
    let mut yaml = http::to_yaml(&doc)?;
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }
    Ok(yaml)
}

fn document_from_draft(draft: &RequestDraft) -> Result<Value> {
    let mut doc = if draft.source.is_object() {
        draft.source.clone()
    } else {
        Value::Object(Map::new())
    };
    {
        let info = ensure_object(&mut doc, "info")?;
        info.insert("name".into(), Value::String(draft.name.clone()));
        if !info.contains_key("type") {
            info.insert("type".into(), Value::String(String::from("http")));
        }
        if let Some(seq) = draft.seq {
            info.entry("seq".to_string()).or_insert(Value::from(seq));
        }
    }
    {
        let http_map = ensure_object(&mut doc, "http")?;
        http_map.insert("method".into(), Value::String(draft.method.clone()));
        http_map.insert("url".into(), Value::String(draft.url.clone()));
        if draft.params.is_empty() {
            http_map.remove("params");
        } else {
            http_map.insert("params".into(), params_to_value(&draft.params));
        }
        if draft.headers.is_empty() {
            http_map.remove("headers");
        } else {
            http_map.insert("headers".into(), key_values_to_value(&draft.headers));
        }
        if draft.rewrite_body {
            match body_to_value(&draft.body) {
                Some(body) => {
                    http_map.insert("body".into(), body);
                }
                None => {
                    http_map.remove("body");
                }
            }
        }
        if draft.rewrite_auth {
            http_map.insert("auth".into(), auth_to_value(&draft.auth));
        }
    }
    if !doc.get("settings").map(Value::is_object).unwrap_or(false) {
        let settings = ensure_object(&mut doc, "settings")?;
        settings.insert("encodeUrl".into(), Value::Bool(true));
    }
    Ok(doc)
}

fn ensure_object<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Map<String, Value>> {
    if !value.is_object() {
        bail!("El documento del request no es un objeto");
    }
    let is_object = value.get(key).map(Value::is_object).unwrap_or(false);
    if !is_object {
        value
            .as_object_mut()
            .unwrap()
            .insert(key.to_string(), Value::Object(Map::new()));
    }
    value
        .get_mut(key)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("No se pudo editar {key}"))
}

fn key_values_to_value(entries: &[KeyValue]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|entry| {
                let mut map = Map::new();
                map.insert("name".into(), Value::String(entry.name.clone()));
                map.insert("value".into(), Value::String(entry.value.clone()));
                if !entry.enabled {
                    map.insert("disabled".into(), Value::Bool(true));
                }
                Value::Object(map)
            })
            .collect(),
    )
}

fn params_to_value(params: &[Param]) -> Value {
    Value::Array(
        params
            .iter()
            .map(|param| {
                let mut map = Map::new();
                map.insert("name".into(), Value::String(param.entry.name.clone()));
                map.insert("value".into(), Value::String(param.entry.value.clone()));
                map.insert(
                    "type".into(),
                    Value::String(
                        match param.kind {
                            ParamKind::Query => "query",
                            ParamKind::Path => "path",
                        }
                        .to_string(),
                    ),
                );
                if !param.entry.enabled {
                    map.insert("disabled".into(), Value::Bool(true));
                }
                Value::Object(map)
            })
            .collect(),
    )
}

fn body_to_value(body: &BodyForm) -> Option<Value> {
    match body.kind {
        BodyKind::None | BodyKind::Unsupported => None,
        BodyKind::Form => {
            let mut map = Map::new();
            map.insert(
                "type".into(),
                Value::String(String::from("form-urlencoded")),
            );
            map.insert("data".into(), key_values_to_value(&body.form));
            Some(Value::Object(map))
        }
        kind => {
            let mut map = Map::new();
            map.insert("type".into(), Value::String(kind.label().to_string()));
            map.insert("data".into(), Value::String(body.text.clone()));
            Some(Value::Object(map))
        }
    }
}

fn auth_to_value(auth: &AuthForm) -> Value {
    match auth.kind {
        AuthKind::None => {
            let mut map = Map::new();
            map.insert("type".into(), Value::String(String::from("none")));
            Value::Object(map)
        }
        AuthKind::Inherit => Value::String(String::from("inherit")),
        AuthKind::Bearer => {
            let mut map = Map::new();
            map.insert("type".into(), Value::String(String::from("bearer")));
            map.insert("token".into(), Value::String(auth.token.clone()));
            Value::Object(map)
        }
        AuthKind::Basic => {
            let mut map = Map::new();
            map.insert("type".into(), Value::String(String::from("basic")));
            map.insert("username".into(), Value::String(auth.username.clone()));
            map.insert("password".into(), Value::String(auth.password.clone()));
            Value::Object(map)
        }
        AuthKind::ApiKey => {
            let mut map = Map::new();
            map.insert("type".into(), Value::String(String::from("apikey")));
            map.insert("key".into(), Value::String(auth.key.clone()));
            map.insert("value".into(), Value::String(auth.value.clone()));
            map.insert(
                "placement".into(),
                Value::String(if auth.in_query {
                    String::from("query")
                } else {
                    String::from("header")
                }),
            );
            Value::Object(map)
        }
        AuthKind::Unsupported => Value::Null,
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("yml.saving");
    fs::write(&tmp, content).with_context(|| format!("No se pudo escribir {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("No se pudo guardar {}", path.display()))?;
    Ok(())
}

pub fn unique_request_path(dir: &Path, name: &str) -> PathBuf {
    let mut used = HashSet::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                used.insert(name.to_lowercase());
            }
        }
    }
    let filename = http::unique_name(&mut used, &http::sanitize_file_name(name), ".yml");
    dir.join(filename)
}

pub fn rename_request(draft: &mut RequestDraft, new_name: &str) -> Result<()> {
    let new_name = new_name.trim();
    validate_entry_name(new_name)?;
    let dir = draft
        .path
        .parent()
        .ok_or_else(|| anyhow!("El request no tiene carpeta"))?;
    let new_path = dir.join(format!("{}.yml", http::sanitize_file_name(new_name)));
    let old_path = draft.path.clone();
    draft.name = new_name.to_string();
    if new_path == old_path {
        save_draft(draft)?;
        return Ok(());
    }
    if new_path.exists() {
        bail!("Ya existe {}", new_path.display());
    }
    draft.path = new_path;
    if let Err(err) = save_draft(draft) {
        draft.path = old_path;
        return Err(err);
    }
    fs::remove_file(&old_path)
        .with_context(|| format!("No se pudo quitar {}", old_path.display()))?;
    Ok(())
}

pub fn duplicate_request(path: &Path) -> Result<PathBuf> {
    let mut draft = load_draft(path)?;
    let copy_name = if draft.name.trim().is_empty() {
        String::from("copia")
    } else {
        format!("{} copia", draft.name.trim())
    };
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("El request no tiene carpeta"))?;
    let filename = copy_name.split_whitespace().collect::<Vec<_>>().join("-");
    let dest = unique_request_path(dir, &filename);
    draft.name = copy_name;
    draft.path = dest.clone();
    save_draft(&mut draft)?;
    Ok(dest)
}

pub fn delete_request(path: &Path) -> Result<()> {
    fs::remove_file(path).with_context(|| format!("No se pudo borrar {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mdnav-studio-{label}-{}-{}",
            std::process::id(),
            label.len()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    const SAMPLE: &str = r#"
info:
  name: Crear
  type: http
  seq: 4
http:
  method: POST
  url: "{{base}}/users/:id"
  params:
    - name: id
      value: "7"
      type: path
    - name: verbose
      value: "1"
      type: query
      disabled: true
  headers:
    - name: X-Trace
      value: abc
  body:
    type: json
    data: '{"name":"ana"}'
  auth:
    type: bearer
    token: "{{token}}"
runtime:
  scripts:
    - type: tests
      code: test("ok", function() {});
settings:
  encodeUrl: true
  timeout: 5000
  followRedirects: false
docs: No perder.
"#;

    #[test]
    fn edits_roundtrip_through_opencollection_yaml() {
        let mut draft = draft_from_yaml(PathBuf::from("crear.yml"), SAMPLE).unwrap();
        assert_eq!(draft.method, "POST");
        assert!(!draft.params[1].entry.enabled);
        assert_eq!(draft.settings.timeout, Some(Duration::from_millis(5000)));
        assert_eq!(draft.script_count, 1);

        draft.method = String::from("PUT");
        draft.url = String::from("https://httpbin.org/anything");
        draft.headers.push(KeyValue {
            name: String::from("Accept"),
            value: String::from("application/json"),
            enabled: true,
        });
        draft.params[1].entry.enabled = true;
        draft.body.kind = BodyKind::Form;
        draft.body.form = vec![KeyValue {
            name: String::from("usuario"),
            value: String::from("ana"),
            enabled: true,
        }];
        draft.rewrite_body = true;
        draft.auth.kind = AuthKind::Basic;
        draft.auth.username = String::from("ada");
        draft.auth.password = String::from("s3cret");
        draft.rewrite_auth = true;

        let yaml = draft_yaml(&draft).unwrap();
        let parsed = http::parse_request(&yaml).unwrap();
        assert_eq!(parsed.method, "PUT");
        assert_eq!(parsed.url, "https://httpbin.org/anything");
        assert_eq!(parsed.headers.len(), 2);
        assert!(parsed.params[1].entry.enabled);
        assert!(matches!(parsed.body, Body::Form(_)));
        assert_eq!(
            parsed.auth,
            Auth::Basic {
                username: String::from("ada"),
                password: String::from("s3cret"),
            }
        );
        assert_eq!(parsed.docs.as_deref(), Some("No perder."));
        assert_eq!(parsed.settings.timeout, Some(Duration::from_millis(5000)));
        assert!(!parsed.settings.follow_redirects);
        assert_eq!(parsed.script_count, 1, "runtime survives the rewrite");
    }

    #[test]
    fn unsupported_auth_stays_until_the_user_changes_it() {
        let yaml = "info:\n  name: Q\n  type: http\nhttp:\n  method: GET\n  url: https://example.com\n  auth:\n    type: oauth2\n    grant: x\n";
        let mut draft = draft_from_yaml(PathBuf::from("q.yml"), yaml).unwrap();
        draft.url = String::from("https://example.com/v2");
        let saved = draft_yaml(&draft).unwrap();
        let again = http::parse_request(&saved).unwrap();
        assert!(matches!(again.auth, Auth::Unsupported(_)));
        assert!(saved.contains("oauth2"));
        assert!(saved.contains("grant"));

        draft.auth.cycle(1);
        draft.rewrite_auth = true;
        let replaced = draft_yaml(&draft).unwrap();
        assert!(!replaced.contains("oauth2"));
        assert!(matches!(
            http::parse_request(&replaced).unwrap().auth,
            Auth::None
        ));
    }

    #[test]
    fn editor_edits_url_params_and_method_then_serializes() {
        let mut studio = HttpStudio {
            draft: Some(draft_from_yaml(PathBuf::from("crear.yml"), SAMPLE).unwrap()),
            panel: StudioPanel::Editor,
            ..HttpStudio::default()
        };
        assert_eq!(studio.nav.cursor, EditorCursor::Url);
        studio.move_vertical(1);
        assert_eq!(studio.nav.cursor, EditorCursor::Row(0));
        studio.move_vertical(-1);
        assert_eq!(studio.nav.cursor, EditorCursor::Url);

        assert!(!studio.activate());
        assert!(studio.nav.editing);
        studio.nav.buffer = String::from("https://httpbin.org/get");
        assert!(studio.commit_edit());

        studio.switch_tab(1);
        assert_eq!(studio.nav.tab, EditorTab::Headers);
        assert!(studio.add_row());
        assert!(studio.nav.editing);
        for ch in ['A', 'c', 'c', 'e', 'p', 't'] {
            studio.push_char(ch);
        }
        assert!(studio.commit_edit());
        studio.move_horizontal(1);
        assert_eq!(studio.nav.col, CellCol::Value);
        assert!(!studio.activate());
        studio.nav.buffer = String::from("application/json");
        assert!(studio.commit_edit());

        assert!(studio.cycle_method());
        let method = studio.draft.as_ref().unwrap().method.clone();
        assert_eq!(method, "PUT", "POST cycles to the next method");

        let yaml = draft_yaml(studio.draft.as_ref().unwrap()).unwrap();
        let parsed = http::parse_request(&yaml).unwrap();
        assert_eq!(parsed.url, "https://httpbin.org/get");
        assert_eq!(parsed.method, "PUT");
        assert!(parsed
            .headers
            .iter()
            .any(|header| header.name == "Accept" && header.value == "application/json"));
    }

    #[test]
    fn list_skips_environments_and_folder_files() {
        let dir = temp_dir("list");
        http::init_collection(&dir).unwrap();
        http::create_request(&dir, "Otro").unwrap();
        fs::write(dir.join("folder.yml"), "info:\n  name: folder\n").unwrap();
        let items = list_requests(&dir.join("."));
        let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
        assert!(names.contains(&"Health check"), "{names:?}");
        assert!(names.contains(&"Otro"), "{names:?}");
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| !item.rel.contains("environments")));
        assert!(
            items
                .iter()
                .all(|item| !item.rel.starts_with('/') && !item.rel.contains(':')),
            "la ruta relativa no debe quedar absoluta: {names:?} {:?}",
            items.iter().map(|item| &item.rel).collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_duplicate_and_delete_map_to_files() {
        let dir = temp_dir("files");
        http::init_collection(&dir).unwrap();
        let original = dir.join("health.yml");
        let mut draft = load_draft(&original).unwrap();
        rename_request(&mut draft, "Estado").unwrap();
        assert!(!original.exists());
        assert_eq!(draft.path, dir.join("Estado.yml"));
        let renamed = load_draft(&draft.path).unwrap();
        assert_eq!(renamed.name, "Estado");

        let copy = duplicate_request(&draft.path).unwrap();
        assert!(copy.exists());
        assert_ne!(copy, draft.path);
        let copied = load_draft(&copy).unwrap();
        assert!(copied.name.contains("copia"));
        assert!(
            !copy.file_name().unwrap().to_string_lossy().contains(' '),
            "el archivo duplicado no lleva espacios"
        );
        assert!(rename_request(&mut draft, "a.b").is_err());
        assert!(rename_request(&mut draft, "a/b").is_err());

        delete_request(&copy).unwrap();
        assert!(!copy.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_lookup_treats_a_dotted_root_as_the_same_collection() {
        let dir = temp_dir("envkey");
        let dotted = dir.join(".");
        let mut map = BTreeMap::new();
        map.insert(dotted.display().to_string(), String::from("httpbin"));
        assert_eq!(env_name_in(&map, &dir), Some("httpbin"));
        assert_eq!(collection_env_key(&dotted), collection_env_key(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_library_ships_an_httpbin_example() {
        let dir = temp_dir("lib");
        let collections = ensure_library(&dir).unwrap();
        assert_eq!(collections.len(), 1);
        assert!(collections[0].expanded);
        assert!(!collections[0].requests.is_empty());
        let request = &collections[0].requests[0];
        let draft = load_draft(&request.path).unwrap();
        let (environments, errors) = http::load_environments(&collections[0].path);
        assert!(errors.is_empty(), "{errors:?}");
        let resolved = http::resolve_request(
            &collections[0].path,
            &draft.path,
            &draft.to_spec(),
            environments.first(),
        );
        assert_eq!(resolved.url, "https://httpbin.org/get");
        assert!(validate_entry_name("foo/bar").is_err());
        assert!(validate_entry_name("a.b").is_err());
        assert!(validate_entry_name("Estado").is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_into_the_url_appends_instead_of_leaving_the_field() {
        let mut studio = HttpStudio {
            draft: Some(draft_from_yaml(PathBuf::from("crear.yml"), SAMPLE).unwrap()),
            panel: StudioPanel::Editor,
            ..HttpStudio::default()
        };
        assert!(studio.cursor_is_text());
        studio.begin_typing('n');
        assert!(studio.nav.editing);
        assert!(studio.nav.buffer.ends_with('n'));
    }

    #[test]
    fn footers_say_shift_s_and_list_rename_duplicate_delete() {
        let editor = panel_footer(StudioPanel::Editor, false, false, false, false, "es");
        assert!(editor.contains("Shift+S enviar"), "{editor}");
        assert!(!editor.contains("· S enviar"), "{editor}");
        let editing = panel_footer(StudioPanel::Editor, true, false, false, false, "es");
        assert!(editing.contains("Shift+S enviar"), "{editing}");
        let response = panel_footer(StudioPanel::Response, false, false, false, false, "es");
        assert!(response.contains("Shift+S enviar"), "{response}");
        let request = panel_footer(StudioPanel::Collection, false, false, false, false, "es");
        assert!(request.contains("r renombrar"), "{request}");
        assert!(
            request.contains("y duplicar") && request.contains("x borrar"),
            "{request}"
        );
        let collection = panel_footer(StudioPanel::Collection, false, false, true, false, "es");
        assert!(
            collection.contains("r renombrar") && collection.contains("c vacía"),
            "{collection}"
        );
        let prompt = panel_footer(StudioPanel::Editor, false, false, false, true, "es");
        assert!(
            prompt.contains("g guardar") && prompt.contains("d descartar"),
            "{prompt}"
        );
    }

    #[test]
    fn new_collection_starts_empty_and_can_include_the_example() {
        let dir = temp_dir("emptycol");
        let empty = create_named_collection(&dir, "notas", false).unwrap();
        assert!(empty.join("opencollection.yml").is_file());
        assert!(list_requests(&empty).is_empty());
        let example = create_named_collection(&dir, "demo", true).unwrap();
        assert_eq!(list_requests(&example).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_and_duplicate_a_collection_and_retarget_its_env_key() {
        let dir = temp_dir("colops");
        let original = create_named_collection(&dir, "notas", false).unwrap();
        let mut envs = BTreeMap::new();
        envs.insert(collection_env_key(&original), String::from("local"));
        let renamed = rename_collection(&original, "notas dos").unwrap();
        assert!(!original.exists());
        assert!(renamed.join("opencollection.yml").is_file());
        assert!(!renamed.file_name().unwrap().to_string_lossy().contains(' '));
        assert_eq!(collection_title(&renamed), "notas dos");
        assert!(retarget_env_key(&mut envs, &original, &renamed));
        assert_eq!(env_name_in(&envs, &renamed), Some("local"));
        assert!(env_name_in(&envs, &original).is_none());

        let copy = duplicate_collection(&renamed).unwrap();
        assert!(copy.exists());
        assert!(!copy.file_name().unwrap().to_string_lossy().contains(' '));
        assert!(collection_title(&copy).contains("copia"));
        delete_collection(&copy).unwrap();
        assert!(!copy.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn timer_does_not_autosave_a_dirty_or_open_field() {
        let mut studio = HttpStudio {
            active: true,
            dirty: true,
            dirty_at: Some(Instant::now() - Duration::from_secs(5)),
            ..HttpStudio::default()
        };
        assert!(studio.needs_save_prompt());
        assert!(!studio.autosave_due());
        studio.dirty = false;
        studio.nav.editing = true;
        studio.nav.buffer = String::from("https://httpbin.org/getn");
        assert!(studio.needs_save_prompt());
        studio.cancel_field_edit();
        assert!(!studio.nav.editing);
        assert!(studio.nav.buffer.is_empty());
        assert!(!studio.needs_save_prompt());
    }

    #[test]
    fn palette_http_args_do_not_treat_httpenv_as_http() {
        assert_eq!(http_palette_action("@http"), Some(false));
        assert_eq!(http_palette_action("  @http env  "), Some(true));
        assert_eq!(http_palette_action("@httpenv"), None);
        assert_eq!(http_palette_action("@http extra"), None);
        assert_eq!(http_palette_action("http"), None);
    }

    #[test]
    fn save_failure_keeps_the_buffer() {
        let mut draft = draft_from_yaml(PathBuf::from("missing/nope.yml"), SAMPLE).unwrap();
        draft.url = String::from("https://httpbin.org/status/200");
        let error = save_draft(&mut draft).unwrap_err();
        assert!(
            error.to_string().contains("nope")
                || error.to_string().contains("guardar")
                || error.to_string().contains("escribir")
                || format!("{error:#}").contains("No se pudo")
        );
        assert_eq!(draft.url, "https://httpbin.org/status/200");
    }
}
