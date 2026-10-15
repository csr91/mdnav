use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::strings;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TreeInfoMode {
    Size,
    Lines,
    #[default]
    #[serde(other)]
    Off,
}

impl TreeInfoMode {
    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::Size,
            Self::Size => Self::Lines,
            Self::Lines => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TreeSortMode {
    Modified,
    Size,
    #[default]
    #[serde(other)]
    Name,
}

impl TreeSortMode {
    pub fn next(&self) -> Self {
        match self {
            Self::Name => Self::Modified,
            Self::Modified => Self::Size,
            Self::Size => Self::Name,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AppConfig {
    pub only_mds: bool,
    pub editor: String,
    pub language: String,
    pub show_bookmarks: bool,
    pub bookmarks: Vec<String>,
    pub tree_info: TreeInfoMode,
    pub tree_sort: TreeSortMode,
    pub show_git_status: bool,
    /// Names of the last palette commands the user ran, most recent first.
    pub recent_commands: Vec<String>,
    /// Recently opened files, most recent first.
    pub recent_files: Vec<String>,
    /// Active HTTP environment per collection, keyed by the collection root.
    pub http_environments: BTreeMap<String, String>,
}

/// How many recent commands the palette keeps and shows by default.
pub const RECENT_COMMANDS_LIMIT: usize = 5;
pub const RECENT_FILES_LIMIT: usize = 12;

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
            recent_commands: Vec::new(),
            recent_files: Vec::new(),
            http_environments: BTreeMap::new(),
        }
    }
}

impl AppConfig {
    pub fn strings(&self) -> &'static strings::Strings {
        strings::get(&self.language)
    }

    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("No se pudo leer {}", path.display()))?;

        parse_config(&content).with_context(|| format!("No se pudo interpretar {}", path.display()))
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("No se pudo crear {}", parent.display()))?;
        }

        let content = self.to_toml()?;
        fs::write(&path, content)
            .with_context(|| format!("No se pudo escribir {}", path.display()))?;

        Ok(path)
    }

    fn to_toml(&self) -> Result<String> {
        let body =
            toml::to_string_pretty(self).context("No se pudo serializar la configuracion")?;
        Ok(format!("# mdnav user config\n{body}"))
    }

    fn normalize(&mut self) {
        if !matches!(self.editor.as_str(), "nano" | "vim") {
            self.editor = AppConfig::default().editor;
        }
        if !matches!(self.language.as_str(), "es" | "en") {
            self.language = AppConfig::default().language;
        }
        let mut unique_bookmarks = Vec::with_capacity(self.bookmarks.len());
        for bookmark in self.bookmarks.drain(..) {
            if !bookmark.trim().is_empty() && !unique_bookmarks.contains(&bookmark) {
                unique_bookmarks.push(bookmark);
            }
        }
        self.bookmarks = unique_bookmarks;

        let mut unique_commands = Vec::with_capacity(self.recent_commands.len());
        for command in self.recent_commands.drain(..) {
            if !command.trim().is_empty() && !unique_commands.contains(&command) {
                unique_commands.push(command);
            }
        }
        unique_commands.truncate(RECENT_COMMANDS_LIMIT);
        self.recent_commands = unique_commands;

        let mut unique_files = Vec::with_capacity(self.recent_files.len());
        for file in self.recent_files.drain(..) {
            if !file.trim().is_empty() && !unique_files.contains(&file) {
                unique_files.push(file);
            }
        }
        unique_files.truncate(RECENT_FILES_LIMIT);
        self.recent_files = unique_files;
    }

    /// Records `name` as the most recently used palette command, keeping the
    /// list deduplicated and capped at `RECENT_COMMANDS_LIMIT`.
    pub fn record_recent_command(&mut self, name: &str) {
        self.recent_commands.retain(|command| command != name);
        self.recent_commands.insert(0, name.to_string());
        self.recent_commands.truncate(RECENT_COMMANDS_LIMIT);
    }

    pub fn record_recent_file(&mut self, path: &str) {
        self.recent_files.retain(|file| file != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(RECENT_FILES_LIMIT);
    }
}

fn parse_config(content: &str) -> Result<AppConfig> {
    // v0.1.10 and older stored one `bookmark = ...` line per bookmark. Repeated
    // TOML keys are invalid, so read that legacy representation once and save
    // it in the standard `bookmarks = [...]` form on the next change.
    if content.lines().any(|line| {
        line.split_once('=')
            .map(|(key, _)| key.trim() == "bookmark")
            .unwrap_or(false)
    }) {
        return Ok(parse_legacy_config(content));
    }

    let mut config: AppConfig = toml::from_str(content).context("TOML invalido")?;
    config.normalize();
    Ok(config)
}

fn parse_legacy_config(content: &str) -> AppConfig {
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
            "only_mds" => config.only_mds = matches!(value, "true" | "1" | "yes" | "on"),
            "editor" if matches!(value, "nano" | "vim") => config.editor = value.to_string(),
            "language" if matches!(value, "es" | "en") => config.language = value.to_string(),
            "show_bookmarks" => {
                config.show_bookmarks = matches!(value, "true" | "1" | "yes" | "on")
            }
            "bookmark" if !value.is_empty() => config.bookmarks.push(value.to_string()),
            "tree_info" => {
                config.tree_info = match value {
                    "size" => TreeInfoMode::Size,
                    "lines" => TreeInfoMode::Lines,
                    _ => TreeInfoMode::Off,
                }
            }
            "tree_sort" => {
                config.tree_sort = match value {
                    "modified" => TreeSortMode::Modified,
                    "size" => TreeSortMode::Size,
                    _ => TreeSortMode::Name,
                }
            }
            "show_git_status" => {
                config.show_git_status = matches!(value, "true" | "1" | "yes" | "on")
            }
            _ => {}
        }
    }

    config.normalize();
    config
}

pub fn config_path() -> Result<PathBuf> {
    let base =
        dirs::config_dir().context("No se pudo resolver la carpeta de config del usuario")?;
    Ok(base.join("mdnav").join("config.toml"))
}

/// Directory where deleted items are parked so `undo` can bring them back.
/// Lives next to the config file, never inside the user's docs tree.
pub fn trash_dir() -> Result<PathBuf> {
    let path = config_path()?
        .parent()
        .context("No se pudo resolver la carpeta de config del usuario")?
        .join("trash");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_windows_paths_and_quotes() {
        let config = AppConfig {
            bookmarks: vec![
                String::from(r#"C:\Users\Cesar\docs"#),
                String::from(r#"C:\docs\project \"alpha\""#),
            ],
            ..AppConfig::default()
        };

        let encoded = config.to_toml().expect("config should serialize");
        let decoded = parse_config(&encoded).expect("config should parse");

        assert_eq!(decoded, config);
        assert!(encoded.contains("bookmarks = ["));
    }

    #[test]
    fn recent_commands_are_deduplicated_and_capped() {
        let mut config = AppConfig::default();
        for name in ["files", "git", "toc", "find", "sort", "delete"] {
            config.record_recent_command(name);
        }
        config.record_recent_command("git");

        assert_eq!(config.recent_commands.len(), RECENT_COMMANDS_LIMIT);
        assert_eq!(config.recent_commands[0], "git");
        assert_eq!(
            config
                .recent_commands
                .iter()
                .filter(|c| *c == "git")
                .count(),
            1
        );
    }

    #[test]
    fn recent_files_are_deduplicated_and_capped() {
        let mut config = AppConfig::default();
        for i in 0..20 {
            config.record_recent_file(&format!("docs/{i}.md"));
        }
        config.record_recent_file("docs/3.md");

        assert_eq!(config.recent_files.len(), RECENT_FILES_LIMIT);
        assert_eq!(config.recent_files[0], "docs/3.md");
        assert_eq!(
            config
                .recent_files
                .iter()
                .filter(|f| *f == "docs/3.md")
                .count(),
            1
        );
    }

    #[test]
    fn reads_legacy_repeated_bookmarks() {
        let legacy = r#"
only_mds = false
editor = "vim"
language = "en"
bookmark = "C:\docs\one"
bookmark = "C:\docs\two"
tree_info = "lines"
tree_sort = "modified"
show_git_status = false
"#;

        let config = parse_config(legacy).expect("legacy config should parse");

        assert!(!config.only_mds);
        assert_eq!(config.editor, "vim");
        assert_eq!(config.language, "en");
        assert_eq!(config.bookmarks, vec![r"C:\docs\one", r"C:\docs\two"]);
        assert_eq!(config.tree_info, TreeInfoMode::Lines);
        assert_eq!(config.tree_sort, TreeSortMode::Modified);
        assert!(!config.show_git_status);
    }

    #[test]
    fn invalid_toml_is_reported() {
        let error = parse_config("only_mds = definitely").expect_err("invalid TOML must fail");
        assert!(error.to_string().contains("TOML invalido"));
    }

    #[test]
    fn invalid_supported_values_fall_back_to_defaults() {
        let config = parse_config(
            r#"
editor = "unknown"
language = "pt"
bookmarks = ["", "docs", "docs"]
tree_info = "unknown"
tree_sort = "unknown"
"#,
        )
        .expect("valid TOML should parse");

        assert_eq!(config.editor, "nano");
        assert_eq!(config.language, "es");
        assert_eq!(config.bookmarks, vec!["docs"]);
        assert_eq!(config.tree_info, TreeInfoMode::Off);
        assert_eq!(config.tree_sort, TreeSortMode::Name);
    }
}
