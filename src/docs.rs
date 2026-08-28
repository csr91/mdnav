use std::{
    collections::BTreeSet,
    fs,
    fs::OpenOptions,
    io,
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
    visit_dir(
        root,
        root,
        expanded_dirs,
        0,
        only_mds,
        sort_mode,
        &mut items,
    )?;
    Ok(items)
}

/// Copies a file or directory without overwriting an existing destination.
/// Directory copies are rolled back when any entry fails, and symbolic links
/// are rejected so a documentation tree cannot unexpectedly escape its root.
pub fn copy_path(source: &Path, dest: &Path) -> Result<()> {
    validate_file_operation(source, dest)?;
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("No se pudo inspeccionar {}", source.display()))?;

    if metadata.file_type().is_symlink() {
        anyhow::bail!("No se copian enlaces simbolicos: {}", source.display());
    }

    if metadata.is_dir() {
        let result = copy_dir_recursive_inner(source, dest);
        if result.is_err() && dest.exists() {
            let _ = fs::remove_dir_all(dest);
        }
        result
    } else if metadata.is_file() {
        copy_file_new(source, dest, &metadata)
    } else {
        anyhow::bail!("Tipo de archivo no soportado: {}", source.display());
    }
}

/// Moves a file or directory. On cross-filesystem moves, it falls back to a
/// checked copy followed by deletion of the original.
pub fn move_path(source: &Path, dest: &Path) -> Result<()> {
    validate_file_operation(source, dest)?;

    match fs::rename(source, dest) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            copy_path(source, dest)?;
            let remove_result = if source.is_dir() {
                fs::remove_dir_all(source)
            } else {
                fs::remove_file(source)
            };
            remove_result.with_context(|| {
                format!(
                    "Se copio a {}, pero no se pudo eliminar el origen {}",
                    dest.display(),
                    source.display()
                )
            })
        }
        Err(error) => Err(error)
            .with_context(|| format!("No se pudo mover {} a {}", source.display(), dest.display())),
    }
}

fn validate_file_operation(source: &Path, dest: &Path) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)
        .with_context(|| format!("No se pudo inspeccionar {}", source.display()))?;
    match fs::symlink_metadata(dest) {
        Ok(_) => anyhow::bail!("El destino ya existe: {}", dest.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("No se pudo inspeccionar {}", dest.display()));
        }
    }

    let source_absolute = source
        .canonicalize()
        .with_context(|| format!("No se pudo resolver {}", source.display()))?;
    let dest_parent = dest
        .parent()
        .context("El destino no tiene directorio padre")?
        .canonicalize()
        .with_context(|| format!("No se pudo resolver el destino {}", dest.display()))?;

    if source_metadata.is_dir() && dest_parent.starts_with(&source_absolute) {
        anyhow::bail!("No se puede copiar o mover una carpeta dentro de si misma");
    }

    Ok(())
}

fn copy_dir_recursive_inner(source: &Path, dest: &Path) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)
        .with_context(|| format!("No se pudo inspeccionar {}", source.display()))?;
    fs::create_dir(dest).with_context(|| format!("No se pudo crear {}", dest.display()))?;

    for entry in
        fs::read_dir(source).with_context(|| format!("No se pudo leer {}", source.display()))?
    {
        let entry = entry?;
        let entry_path = entry.path();
        let target = dest.join(entry.file_name());
        let metadata = fs::symlink_metadata(&entry_path)
            .with_context(|| format!("No se pudo inspeccionar {}", entry_path.display()))?;

        if metadata.file_type().is_symlink() {
            anyhow::bail!("No se copian enlaces simbolicos: {}", entry_path.display());
        }
        if metadata.is_dir() {
            copy_dir_recursive_inner(&entry_path, &target)?;
        } else if metadata.is_file() {
            copy_file_new(&entry_path, &target, &metadata)?;
        } else {
            anyhow::bail!("Tipo de archivo no soportado: {}", entry_path.display());
        }
    }

    fs::set_permissions(dest, source_metadata.permissions())
        .with_context(|| format!("No se pudieron preservar permisos en {}", dest.display()))?;
    Ok(())
}

fn copy_file_new(source: &Path, dest: &Path, metadata: &fs::Metadata) -> Result<()> {
    let result = (|| -> Result<()> {
        let mut input = fs::File::open(source)
            .with_context(|| format!("No se pudo abrir {}", source.display()))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)
            .with_context(|| format!("No se pudo crear {}", dest.display()))?;
        io::copy(&mut input, &mut output)
            .with_context(|| format!("No se pudo copiar {}", source.display()))?;
        fs::set_permissions(dest, metadata.permissions())
            .with_context(|| format!("No se pudieron preservar permisos en {}", dest.display()))?;
        Ok(())
    })();

    if result.is_err() && dest.exists() {
        let _ = fs::remove_file(dest);
    }
    result
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

    entries.sort_by_key(|left| left.file_name());

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
                visit_dir(
                    root,
                    &path,
                    expanded_dirs,
                    depth + 1,
                    only_mds,
                    sort_mode,
                    items,
                )?;
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
            .unwrap_or(root.as_os_str())
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mdnav-{label}-{}-{id}", std::process::id()));
            fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn markdown_tree_filters_files_and_expands_selected_directories() {
        let temp = TestDir::new("tree");
        let guide = temp.path().join("guide");
        fs::create_dir(&guide).unwrap();
        fs::write(temp.path().join("README.md"), "# Home").unwrap();
        fs::write(temp.path().join("notes.txt"), "hidden").unwrap();
        fs::write(guide.join("intro.MD"), "# Intro").unwrap();

        let collapsed =
            collect_markdown_tree(temp.path(), &BTreeSet::new(), true, &TreeSortMode::Name)
                .unwrap();
        assert_eq!(
            collapsed
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["guide", "README.md"]
        );

        let expanded = collect_markdown_tree(
            temp.path(),
            &BTreeSet::from([guide]),
            true,
            &TreeSortMode::Name,
        )
        .unwrap();
        assert_eq!(
            expanded
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["guide", "intro.MD", "README.md"]
        );
    }

    #[test]
    fn copy_path_copies_directories_and_preserves_existing_destinations() {
        let temp = TestDir::new("copy");
        let source = temp.path().join("source");
        let nested = source.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(source.join("one.md"), "one").unwrap();
        fs::write(nested.join("two.md"), "two").unwrap();

        let destination = temp.path().join("destination");
        copy_path(&source, &destination).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("one.md")).unwrap(),
            "one"
        );
        assert_eq!(
            fs::read_to_string(destination.join("nested").join("two.md")).unwrap(),
            "two"
        );

        let existing = temp.path().join("existing.md");
        fs::write(&existing, "keep").unwrap();
        assert!(copy_path(&source.join("one.md"), &existing).is_err());
        assert_eq!(fs::read_to_string(existing).unwrap(), "keep");
    }

    #[test]
    fn copy_path_rejects_a_destination_inside_the_source() {
        let temp = TestDir::new("descendant");
        let source = temp.path().join("source");
        let child = source.join("child");
        fs::create_dir_all(&child).unwrap();

        let destination = child.join("copy");
        let error = copy_path(&source, &destination).expect_err("descendant must be rejected");

        assert!(error.to_string().contains("dentro de si misma"));
        assert!(!destination.exists());
    }

    #[test]
    fn move_path_moves_a_file_without_overwriting() {
        let temp = TestDir::new("move");
        let source = temp.path().join("source.md");
        let destination = temp.path().join("destination.md");
        fs::write(&source, "content").unwrap();

        move_path(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read_to_string(destination).unwrap(), "content");
    }

    #[cfg(unix)]
    #[test]
    fn directory_copy_rejects_symlinks_and_removes_partial_destination() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new("symlink");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file.md"), "content").unwrap();
        symlink(source.join("file.md"), source.join("link.md")).unwrap();

        let error = copy_path(&source, &destination).expect_err("symlink must be rejected");

        assert!(error.to_string().contains("enlaces simbolicos"));
        assert!(!destination.exists());
    }
}
