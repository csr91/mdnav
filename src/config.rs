use std::{fs, path::PathBuf};

use anyhow::{Context, Result};

use crate::strings;

#[derive(Clone, Debug, PartialEq)]
pub enum TreeInfoMode {
    Off,
    Size,
    Lines,
}

impl TreeInfoMode {
    pub fn next(&self) -> Self {
        match self {
            Self::Off   => Self::Size,
            Self::Size  => Self::Lines,
            Self::Lines => Self::Off,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off   => "off",
            Self::Size  => "size",
            Self::Lines => "lines",
        }
    }
}

impl Default for TreeInfoMode {
    fn default() -> Self { Self::Off }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TreeSortMode {
    Name,
    Modified,
    Size,
}

impl TreeSortMode {
    pub fn next(&self) -> Self {
        match self {
            Self::Name     => Self::Modified,
            Self::Modified => Self::Size,
            Self::Size     => Self::Name,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Name     => "name",
            Self::Modified => "modified",
            Self::Size     => "size",
        }
    }
}

impl Default for TreeSortMode {
    fn default() -> Self { Self::Name }
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub only_mds: bool,
    pub editor: String,
    pub language: String,
    pub show_bookmarks: bool,
    pub bookmarks: Vec<String>,
    pub tree_info: TreeInfoMode,
    pub tree_sort: TreeSortMode,
    pub show_git_status: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            only_mds: true,
            editor: String::from("nano"),
            language: String::from("es"),
            show_bookmarks: true,
            bookmarks: Vec::new(),
            tree_info: TreeInfoMode::Off,
            tree_sort: TreeSortMode::Name,
            show_git_status: true,
        }
    }
}

impl AppConfig {
    pub fn strings(&self) -> &'static strings::Strings {
        strings::get(&self.language)
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("No se pudo leer {}", path.display()))?;

        Ok(parse_config(&content))
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("No se pudo crear {}", parent.display()))?;
        }

        fs::write(&path, self.to_toml())
            .with_context(|| format!("No se pudo escribir {}", path.display()))?;

        Ok(path)
    }

    fn to_toml(&self) -> String {
        let mut s = format!(
            "# mdnav user config\nonly_mds = {}\neditor = \"{}\"\nlanguage = \"{}\"\nshow_bookmarks = {}\ntree_info = \"{}\"\ntree_sort = \"{}\"\nshow_git_status = {}\n",
            if self.only_mds { "true" } else { "false" },
            self.editor,
            self.language,
            if self.show_bookmarks { "true" } else { "false" },
            self.tree_info.as_str(),
            self.tree_sort.as_str(),
            if self.show_git_status { "true" } else { "false" },
        );
        for bm in &self.bookmarks {
            s.push_str(&format!("bookmark = \"{}\"\n", bm));
        }
        s
    }
}

fn parse_config(content: &str) -> AppConfig {
    let mut config = AppConfig::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };

        let key = key.trim();
        let value = value.trim().trim_matches('"');

        match key {
            "only_mds"       => config.only_mds = matches!(value, "true" | "1" | "yes" | "on"),
            "editor" if matches!(value, "nano" | "vim") => config.editor = value.to_string(),
            "language" if matches!(value, "es" | "en") => config.language = value.to_string(),
            "show_bookmarks" => config.show_bookmarks = matches!(value, "true" | "1" | "yes" | "on"),
            "bookmark" if !value.is_empty() => config.bookmarks.push(value.to_string()),
            "tree_info" => config.tree_info = match value {
                "size"  => TreeInfoMode::Size,
                "lines" => TreeInfoMode::Lines,
                _       => TreeInfoMode::Off,
            },
            "tree_sort" => config.tree_sort = match value {
                "modified" => TreeSortMode::Modified,
                "size"     => TreeSortMode::Size,
                _          => TreeSortMode::Name,
            },
            "show_git_status" => config.show_git_status = matches!(value, "true" | "1" | "yes" | "on"),
            _ => {}
        }
    }

    config
}

pub fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("No se pudo resolver la carpeta de config del usuario")?;
    Ok(base.join("mdnav").join("config.toml"))
}
