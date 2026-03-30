use std::{collections::BTreeSet, path::PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    docs::{collect_markdown_tree, parent_dir_if_within, DocItem},
    markdown::{load_preview, PreviewDocument},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Focus {
    Tree,
    Preview,
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
    pub preview_fullscreen: bool,
    pub split_level: u8,
    pub running: bool,
    pub status: String,
}

impl App {
    pub fn new(root: PathBuf) -> Result<Self> {
        let mut expanded_dirs = BTreeSet::new();
        expanded_dirs.insert(root.clone());

        let items = collect_markdown_tree(&root, &expanded_dirs)?;
        let selected_index = items.iter().position(|item| !item.is_dir).unwrap_or(0);
        let current_file = items.get(selected_index).filter(|item| !item.is_dir).map(|item| item.path.clone());
        let preview = if let Some(path) = &current_file {
            load_preview(path)?
        } else {
            PreviewDocument::default()
        };

        Ok(Self {
            root,
            items,
            selected_index,
            current_file,
            preview,
            preview_scroll: 0,
            expanded_dirs,
            focus: Focus::Tree,
            preview_fullscreen: false,
            split_level: 3,
            running: true,
            status: String::from("Listo"),
        })
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_preview_fullscreen()
            }
            KeyCode::Right | KeyCode::Enter => self.activate_selected()?,
            KeyCode::Left | KeyCode::Backspace => self.collapse_or_parent()?,
            KeyCode::Char('j') if self.focus == Focus::Preview => self.scroll_preview(1),
            KeyCode::Char('k') if self.focus == Focus::Preview => self.scroll_preview(-1),
            KeyCode::Char('.') if self.focus == Focus::Preview => self.scroll_preview(1),
            KeyCode::Char(',') if self.focus == Focus::Preview => self.scroll_preview(-1),
            KeyCode::Char('!') => self.set_split_level(1),
            KeyCode::Char('@') => self.set_split_level(2),
            KeyCode::Char('#') => self.set_split_level(3),
            KeyCode::Char('$') => self.set_split_level(4),
            KeyCode::Char('%') => self.set_split_level(5),
            _ => {}
        }

        Ok(())
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Tree => Focus::Preview,
            Focus::Preview => Focus::Tree,
        };
        self.status = format!("Foco: {:?}", self.focus);
    }

    fn toggle_preview_fullscreen(&mut self) {
        self.preview_fullscreen = !self.preview_fullscreen;
        self.focus = Focus::Preview;
        self.status = if self.preview_fullscreen {
            String::from("Preview completo activado")
        } else {
            String::from("Preview completo desactivado")
        };
    }

    fn set_split_level(&mut self, level: u8) {
        self.split_level = level.clamp(1, 5);
        self.status = format!("Separacion ajustada: {}", self.split_level);
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() || self.focus != Focus::Tree {
            return;
        }

        let max_index = self.items.len().saturating_sub(1) as isize;
        let next = (self.selected_index as isize + delta).clamp(0, max_index) as usize;
        self.selected_index = next;

        if self.focus == Focus::Tree {
            if let Some(item) = self.items.get(self.selected_index) {
                self.status = item.relative.display().to_string();
            }
        }
    }

    fn activate_selected(&mut self) -> Result<()> {
        let Some(item) = self.items.get(self.selected_index).cloned() else {
            return Ok(());
        };

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
            if let Some(index) = self.items.iter().position(|candidate| candidate.path == parent) {
                self.selected_index = index;
                self.status = format!("Padre {}", self.items[index].relative.display());
            }
        }

        Ok(())
    }

    fn scroll_preview(&mut self, delta: isize) {
        let max_scroll = self.preview.lines.len().saturating_sub(1) as isize;
        let next = (self.preview_scroll as isize + delta).clamp(0, max_scroll) as usize;
        self.preview_scroll = next;
    }

    fn reload_items(&mut self) -> Result<()> {
        let selected_path = self.items.get(self.selected_index).map(|item| item.path.clone());
        self.items = collect_markdown_tree(&self.root, &self.expanded_dirs)?;

        if let Some(path) = selected_path {
            if let Some(index) = self.items.iter().position(|item| item.path == path) {
                self.selected_index = index;
            } else {
                self.selected_index = self.items.len().saturating_sub(1);
            }
        }

        Ok(())
    }

    fn open_file(&mut self, path: PathBuf) -> Result<()> {
        self.preview = load_preview(&path)?;
        self.preview_scroll = 0;
        self.current_file = Some(path.clone());
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
                format!(" | primer link: {} -> {} ({resolution})", link.label, link.raw_target)
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
}
