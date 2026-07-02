use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::config::TreeSortMode;

#[derive(Clone, Debug)]
pub struct DocItem {
    pub path: PathBuf,
    pub name: String,
    pub relative: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
    pub is_bookmark: bool,
}

#[derive(Clone, Debug)]
pub struct LinkTarget {
    pub label: String,
    pub raw_target: String,
    pub resolved: Option<PathBuf>,
    pub line_index: usize, // approximate line in rendered preview
}

pub fn collect_markdown_tree(
    root: &Path,
    expanded_dirs: &BTreeSet<PathBuf>,
    only_mds: bool,
    sort_mode: &TreeSortMode,
) -> Result<Vec<DocItem>> {
    let mut items = Vec::new();
    visit_dir(root, root, expanded_dirs, 0, only_mds, sort_mode, &mut items)?;
    Ok(items)
}

/// Recursively copies `source` (a directory) into `dest`, creating `dest` and
/// every subdirectory. Files are copied with their contents; symlinks are
/// followed and permissions are not explicitly preserved. Intended for
/// documentation trees, not for arbitrary large or special filesystems.
pub fn copy_dir_recursive(source: &Path, dest: &Path) -> Result<()> {
    fs::create_dir(dest)
        .with_context(|| format!("No se pudo crear {}", dest.display()))?;

    for entry in fs::read_dir(source)
        .with_context(|| format!("No se pudo leer {}", source.display()))?
    {
        let entry = entry?;
        let entry_path = entry.path();
        let target = dest.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry_path, &target)?;
        } else {
            fs::copy(&entry_path, &target)
                .with_context(|| format!("No se pudo copiar {}", entry_path.display()))?;
        }
    }

    Ok(())
}

/// Builds a navigable directory-only tree under `root` (including `root` itself
/// as the first entry). Only directories present in `expanded_dirs` reveal their
/// children, so the caller controls what is visible. Used by the move/copy
/// destination picker: it starts collapsed and expands as the user navigates.
pub fn collect_dir_tree(root: &Path, expanded_dirs: &BTreeSet<PathBuf>) -> Result<Vec<DocItem>> {
    let mut items = vec![make_item(root, root.to_path_buf(), 0, true)?];
    visit_dir_tree(root, root, expanded_dirs, 1, &mut items)?;
    Ok(items)
}

fn visit_dir_tree(
    root: &Path,
    current: &Path,
    expanded_dirs: &BTreeSet<PathBuf>,
    depth: usize,
    items: &mut Vec<DocItem>,
) -> Result<()> {
    let mut entries = fs::read_dir(current)
        .with_context(|| format!("No se pudo leer {}", current.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    entries.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

    for entry in entries {
        if entry.file_type()?.is_dir() {
            let path = entry.path();
            items.push(make_item(root, path.clone(), depth, true)?);
            if expanded_dirs.contains(&path) {
                visit_dir_tree(root, &path, expanded_dirs, depth + 1, items)?;
            }
        }
    }

    Ok(())
}

/// Returns true when `dir` contains at least one subdirectory. Used by the
/// picker to decide whether to draw an expand marker.
pub fn has_subdirs(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
}

fn visit_dir(
    root: &Path,
    current: &Path,
    expanded_dirs: &BTreeSet<PathBuf>,
    depth: usize,
    only_mds: bool,
    sort_mode: &TreeSortMode,
    items: &mut Vec<DocItem>,
) -> Result<()> {
    let mut entries = fs::read_dir(current)
        .with_context(|| format!("No se pudo leer {}", current.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    entries.sort_by(|left, right| {
        let left_is_dir = left.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let right_is_dir = right.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        (!left_is_dir)
            .cmp(&!right_is_dir)
            .then_with(|| compare_entries(left, right, sort_mode))
    });

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            let item = make_item(root, path.clone(), depth, true)?;
            let should_expand = expanded_dirs.contains(&path);
            items.push(item);

            if should_expand {
                visit_dir(root, &path, expanded_dirs, depth + 1, only_mds, sort_mode, items)?;
            }
        } else if !only_mds || is_markdown_file(&path) {
            items.push(make_item(root, path, depth, false)?);
        }
    }

    Ok(())
}

fn compare_entries(
    left: &fs::DirEntry,
    right: &fs::DirEntry,
    sort_mode: &TreeSortMode,
) -> std::cmp::Ordering {
    match sort_mode {
        TreeSortMode::Name => left.file_name().cmp(&right.file_name()),
        TreeSortMode::Modified => entry_modified(right)
            .cmp(&entry_modified(left))
            .then_with(|| left.file_name().cmp(&right.file_name())),
        TreeSortMode::Size => entry_size(right)
            .cmp(&entry_size(left))
            .then_with(|| left.file_name().cmp(&right.file_name())),
    }
}

fn entry_modified(entry: &fs::DirEntry) -> Option<std::time::SystemTime> {
    entry.metadata().ok()?.modified().ok()
}

fn entry_size(entry: &fs::DirEntry) -> Option<u64> {
    entry.metadata().ok().map(|metadata| metadata.len())
}

fn make_item(root: &Path, path: PathBuf, depth: usize, is_dir: bool) -> Result<DocItem> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("No se pudo relativizar {}", path.display()))?
        .to_path_buf();

    let name = if depth == 0 && relative.as_os_str().is_empty() {
        root.file_name()
            .unwrap_or_else(|| root.as_os_str())
            .to_string_lossy()
            .to_string()
    } else {
        path.file_name()
            .unwrap_or_else(|| path.as_os_str())
            .to_string_lossy()
            .to_string()
    };

    Ok(DocItem {
        path,
        name,
        relative,
        depth,
        is_dir,
        is_bookmark: false,
    })
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

pub fn parent_dir_if_within(root: &Path, path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?.to_path_buf();
    if parent.starts_with(root) {
        Some(parent)
    } else {
        None
    }
}
