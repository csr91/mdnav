//! The collection tree of the HTTP client: collections at the top, their
//! folders nested below and requests as leaves. The tree is built once from
//! the discovered request paths and flattened into the visible rows on every
//! expand or collapse, so moving around never touches the disk.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpNodeKind {
    Collection,
    Folder,
    Request,
}

/// What a leaf shows besides its path, read from the request file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequestMeta {
    pub name: Option<String>,
    pub method: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpTreeNode {
    pub path: PathBuf,
    pub kind: HttpNodeKind,
    pub label: String,
    /// Uppercase method of a request; `None` for folders and for requests
    /// that could not be read.
    pub method: Option<String>,
    pub children: Vec<HttpTreeNode>,
}

/// One visible line of the tree pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpTreeRow {
    pub path: PathBuf,
    pub kind: HttpNodeKind,
    pub label: String,
    pub method: Option<String>,
    pub depth: usize,
    pub expanded: bool,
    /// Index into `HttpTree::collections` of the collection holding the row.
    pub collection: usize,
}

/// Builds the node of the collection at `root` from its request files. Only
/// folders holding requests show up, ordered like the markdown explorer:
/// folders first, then requests, each group by file name.
pub fn build_collection(
    root: &Path,
    requests: &[PathBuf],
    meta: impl Fn(&Path) -> RequestMeta,
) -> HttpTreeNode {
    let requests: Vec<&Path> = requests.iter().map(PathBuf::as_path).collect();
    HttpTreeNode {
        path: root.to_path_buf(),
        kind: HttpNodeKind::Collection,
        label: file_label(root),
        method: None,
        children: build_children(root, &requests, &meta),
    }
}

fn build_children(
    dir: &Path,
    requests: &[&Path],
    meta: &dyn Fn(&Path) -> RequestMeta,
) -> Vec<HttpTreeNode> {
    let mut folders: BTreeMap<OsString, Vec<&Path>> = BTreeMap::new();
    let mut leaves: Vec<&Path> = Vec::new();
    for &path in requests {
        let Ok(relative) = path.strip_prefix(dir) else {
            continue;
        };
        let mut components = relative.components();
        let Some(first) = components.next() else {
            continue;
        };
        if components.next().is_some() {
            folders
                .entry(first.as_os_str().to_os_string())
                .or_default()
                .push(path);
        } else {
            leaves.push(path);
        }
    }
    leaves.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

    let folder_nodes = folders.into_iter().map(|(name, inner)| {
        let path = dir.join(name);
        HttpTreeNode {
            label: file_label(&path),
            kind: HttpNodeKind::Folder,
            method: None,
            children: build_children(&path, &inner, meta),
            path,
        }
    });
    let leaf_nodes = leaves.into_iter().map(|path| {
        let RequestMeta { name, method } = meta(path);
        let label = name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        HttpTreeNode {
            path: path.to_path_buf(),
            kind: HttpNodeKind::Request,
            label,
            method,
            children: Vec::new(),
        }
    });
    folder_nodes.chain(leaf_nodes).collect()
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// The visible rows: children show only below expanded nodes.
pub fn flatten(collections: &[HttpTreeNode], expanded: &BTreeSet<PathBuf>) -> Vec<HttpTreeRow> {
    fn visit(
        node: &HttpTreeNode,
        depth: usize,
        collection: usize,
        expanded: &BTreeSet<PathBuf>,
        rows: &mut Vec<HttpTreeRow>,
    ) {
        let is_expanded = node.kind != HttpNodeKind::Request && expanded.contains(&node.path);
        rows.push(HttpTreeRow {
            path: node.path.clone(),
            kind: node.kind,
            label: node.label.clone(),
            method: node.method.clone(),
            depth,
            expanded: is_expanded,
            collection,
        });
        if is_expanded {
            for child in &node.children {
                visit(child, depth + 1, collection, expanded, rows);
            }
        }
    }

    let mut rows = Vec::new();
    for (index, collection) in collections.iter().enumerate() {
        visit(collection, 0, index, expanded, &mut rows);
    }
    rows
}

/// Expands every folder between `collection` and `path`, the collection
/// included, so `path` becomes visible. `path` itself is left as is.
pub fn expand_ancestors(expanded: &mut BTreeSet<PathBuf>, collection: &Path, path: &Path) {
    for dir in path.ancestors().skip(1) {
        if !dir.starts_with(collection) {
            break;
        }
        expanded.insert(dir.to_path_buf());
    }
}

/// Tree pane state: the built tree, which nodes are open and the cursor.
#[derive(Default)]
pub struct HttpTree {
    pub collections: Vec<HttpTreeNode>,
    pub expanded: BTreeSet<PathBuf>,
    pub rows: Vec<HttpTreeRow>,
    /// Index into `rows`.
    pub cursor: usize,
}

impl HttpTree {
    /// Replaces the tree, keeping the cursor on the same path when it still
    /// exists.
    pub fn set_collections(&mut self, collections: Vec<HttpTreeNode>) {
        let selected = self.selected().map(|row| row.path.clone());
        self.collections = collections;
        self.rebuild_rows(selected.as_deref());
    }

    pub fn selected(&self) -> Option<&HttpTreeRow> {
        self.rows.get(self.cursor)
    }

    /// The selected row when it is a request.
    pub fn selected_request(&self) -> Option<&Path> {
        self.selected()
            .filter(|row| row.kind == HttpNodeKind::Request)
            .map(|row| row.path.as_path())
    }

    /// The collection of the selected row.
    pub fn active_collection(&self) -> Option<&Path> {
        let row = self.selected()?;
        self.collections
            .get(row.collection)
            .map(|node| node.path.as_path())
    }

    pub fn collection_roots(&self) -> Vec<PathBuf> {
        self.collections
            .iter()
            .map(|node| node.path.clone())
            .collect()
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    /// `l`: opens a collapsed node, or steps into the first child of an open
    /// one. Returns false on a request, where there is nothing to open.
    pub fn expand(&mut self) -> bool {
        let Some(row) = self.selected().cloned() else {
            return false;
        };
        if row.kind == HttpNodeKind::Request {
            return false;
        }
        if row.expanded {
            let has_child = self
                .rows
                .get(self.cursor + 1)
                .is_some_and(|next| next.depth > row.depth);
            if has_child {
                self.cursor += 1;
            }
        } else {
            self.expanded.insert(row.path.clone());
            self.rebuild_rows(Some(&row.path));
        }
        true
    }

    /// Opens the node at `path` without moving the cursor.
    pub fn expand_path(&mut self, path: &Path) {
        let selected = self.selected().map(|row| row.path.clone());
        self.expanded.insert(path.to_path_buf());
        self.rebuild_rows(selected.as_deref());
    }

    /// `h`: closes an open node, otherwise jumps to the parent row.
    pub fn collapse(&mut self) {
        let Some(row) = self.selected().cloned() else {
            return;
        };
        if row.expanded {
            self.expanded.remove(&row.path);
            self.rebuild_rows(Some(&row.path));
        } else if let Some(parent) = self.rows[..self.cursor]
            .iter()
            .rposition(|candidate| candidate.depth < row.depth)
        {
            self.cursor = parent;
        }
    }

    /// Expands the ancestors of `path` and puts the cursor on it. Returns
    /// false when no collection holds it.
    pub fn reveal(&mut self, path: &Path) -> bool {
        let Some(collection) = self
            .collections
            .iter()
            .find(|node| path.starts_with(&node.path))
            .map(|node| node.path.clone())
        else {
            return false;
        };
        expand_ancestors(&mut self.expanded, &collection, path);
        self.rebuild_rows(Some(path));
        self.selected().is_some_and(|row| row.path == path)
    }

    fn rebuild_rows(&mut self, selected: Option<&Path>) {
        self.rows = flatten(&self.collections, &self.expanded);
        let position = selected.and_then(|path| self.rows.iter().position(|row| row.path == path));
        self.cursor = position.unwrap_or(self.cursor.min(self.rows.len().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(collection: &Path, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(collection.to_path_buf(), |path, part| path.join(part))
    }

    fn sample_tree() -> (PathBuf, HttpTree) {
        let root = PathBuf::from("repo").join("api");
        let requests = [
            "zeta.yml",
            "users/list.yml",
            "users/admin/ban.yml",
            "alpha.yml",
            "auth/login.yml",
            "users/create.yml",
        ]
        .map(|relative| request(&root, relative));
        let node = build_collection(&root, &requests, |path| RequestMeta {
            name: (path.file_stem().unwrap() == "login").then(|| String::from("Log in")),
            method: Some(String::from("GET")),
        });
        let mut tree = HttpTree::default();
        tree.set_collections(vec![node]);
        (root, tree)
    }

    fn labels(nodes: &[HttpTreeNode]) -> Vec<&str> {
        nodes.iter().map(|node| node.label.as_str()).collect()
    }

    #[test]
    fn nests_folders_and_orders_folders_before_requests_by_name() {
        let (root, tree) = sample_tree();
        let collection = &tree.collections[0];
        assert_eq!(collection.kind, HttpNodeKind::Collection);
        assert_eq!(collection.label, "api");
        assert_eq!(
            labels(&collection.children),
            ["auth", "users", "alpha", "zeta"]
        );

        let users = &collection.children[1];
        assert_eq!(users.kind, HttpNodeKind::Folder);
        assert_eq!(users.path, root.join("users"));
        assert_eq!(labels(&users.children), ["admin", "create", "list"]);
        assert_eq!(labels(&users.children[0].children), ["ban"]);
        assert_eq!(users.children[0].children[0].kind, HttpNodeKind::Request);
    }

    #[test]
    fn leaves_use_the_request_name_and_fall_back_to_the_file_stem() {
        let (_, tree) = sample_tree();
        let auth = &tree.collections[0].children[0];
        assert_eq!(labels(&auth.children), ["Log in"]);
        assert_eq!(auth.children[0].method.as_deref(), Some("GET"));
        assert_eq!(tree.collections[0].children[2].label, "alpha");
    }

    #[test]
    fn flatten_shows_children_only_below_expanded_nodes() {
        let (root, mut tree) = sample_tree();
        assert_eq!(tree.rows.len(), 1, "everything starts collapsed");

        tree.expanded.insert(root.clone());
        tree.rebuild_rows(None);
        let visible: Vec<_> = tree.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(visible, ["api", "auth", "users", "alpha", "zeta"]);
        assert!(tree.rows[0].expanded);
        assert_eq!(tree.rows[1].depth, 1);
        assert!(tree.rows.iter().all(|row| row.collection == 0));
    }

    #[test]
    fn reveal_expands_the_ancestors_of_a_preselected_request() {
        let (root, mut tree) = sample_tree();
        let target = request(&root, "users/admin/ban.yml");

        assert!(tree.reveal(&target));
        assert_eq!(tree.selected_request(), Some(target.as_path()));
        assert_eq!(tree.active_collection(), Some(root.as_path()));
        for dir in [
            root.clone(),
            root.join("users"),
            root.join("users").join("admin"),
        ] {
            assert!(tree.expanded.contains(&dir), "{} expanded", dir.display());
        }
        assert!(!tree.expanded.contains(&root.join("auth")));
        assert_eq!(tree.selected().unwrap().depth, 3);
    }

    #[test]
    fn expand_ancestors_stops_at_the_collection() {
        let root = PathBuf::from("repo").join("api");
        let mut expanded = BTreeSet::new();
        expand_ancestors(&mut expanded, &root, &request(&root, "a/b/c.yml"));
        let expected: BTreeSet<PathBuf> = [root.clone(), root.join("a"), root.join("a").join("b")]
            .into_iter()
            .collect();
        assert_eq!(expanded, expected);
    }

    #[test]
    fn expand_and_collapse_navigate_like_a_file_tree() {
        let (root, mut tree) = sample_tree();
        assert!(tree.expand(), "opens the collection");
        assert!(tree.rows[0].expanded);
        assert!(tree.expand(), "steps into the first child");
        assert_eq!(tree.selected().unwrap().path, root.join("auth"));

        tree.collapse();
        assert_eq!(tree.cursor, 0, "a closed folder jumps to its parent");
        tree.collapse();
        assert_eq!(tree.rows.len(), 1, "an open node closes");

        tree.move_cursor(5);
        assert_eq!(tree.cursor, 0, "the cursor stays inside the rows");
    }
}
