use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct DocItem {
    pub path: PathBuf,
    pub name: String,
    pub relative: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
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
) -> Result<Vec<DocItem>> {
    let mut items = Vec::new();
    visit_dir(root, root, expanded_dirs, 0, only_mds, &mut items)?;
    Ok(items)
}

fn visit_dir(
    root: &Path,
    current: &Path,
    expanded_dirs: &BTreeSet<PathBuf>,
    depth: usize,
    only_mds: bool,
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
            .then_with(|| left.file_name().cmp(&right.file_name()))
    });

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            let item = make_item(root, path.clone(), depth, true)?;
            let should_expand = expanded_dirs.contains(&path);
            items.push(item);

            if should_expand {
                visit_dir(root, &path, expanded_dirs, depth + 1, only_mds, items)?;
            }
        } else if !only_mds || is_markdown_file(&path) {
            items.push(make_item(root, path, depth, false)?);
        }
    }

    Ok(())
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
