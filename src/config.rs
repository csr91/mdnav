use std::{fs, path::PathBuf};

use anyhow::{Context, Result};

use crate::strings;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub only_mds: bool,
    pub editor: String,
    pub language: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            only_mds: true,
            editor: String::from("nano"),
            language: String::from("es"),
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
        format!(
            "# mdnav user config\nonly_mds = {}\neditor = \"{}\"\nlanguage = \"{}\"\n",
            if self.only_mds { "true" } else { "false" },
            self.editor,
            self.language
        )
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

        if key == "only_mds" {
            config.only_mds = matches!(value, "true" | "1" | "yes" | "on");
        }

        if key == "editor" && matches!(value, "nano" | "vim") {
            config.editor = value.to_string();
        }

        if key == "language" && matches!(value, "es" | "en") {
            config.language = value.to_string();
        }
    }

    config
}

pub fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("No se pudo resolver la carpeta de config del usuario")?;
    Ok(base.join("mdnav").join("config.toml"))
}
