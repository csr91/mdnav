use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Focus};
use crate::markdown::{PreviewLine, PreviewLineKind};

pub fn render(frame: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(frame.area());

    if app.preview_fullscreen {
        render_preview(frame, areas[0], app);
    } else if frame.area().width <= 60 {
        match app.focus {
            Focus::Tree => render_tree(frame, areas[0], app),
            Focus::Preview => render_preview(frame, areas[0], app),
        }
    } else {
        let body = split_body(frame.area().width, areas[0], app.split_level);
        render_tree(frame, body[0], app);
        render_preview(frame, body[1], app);
    }

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("q salir  "),
        Span::raw("Enter abrir  "),
        Span::raw("Ctrl+Enter preview  "),
        Span::raw("Tab/Shift+Tab foco  "),
        Span::raw(",/. scroll  "),
        Span::raw("Shift+1..5 split  "),
        Span::styled(&app.status, Style::default().fg(Color::Yellow)),
    ]))
    .block(Block::default().borders(Borders::TOP));

    frame.render_widget(footer, areas[1]);
}

fn split_body(width: u16, area: Rect, split_level: u8) -> Vec<Rect> {
    if width > 100 {
        let nav_percentage = match split_level {
            1 => 25,
            2 => 30,
            3 => 35,
            4 => 40,
            _ => 45,
        };
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(nav_percentage),
                Constraint::Percentage(100 - nav_percentage),
            ])
            .split(area)
            .to_vec()
    } else {
        let nav_percentage = match split_level {
            1 => 18,
            2 => 22,
            3 => 26,
            4 => 30,
            _ => 36,
        };
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(nav_percentage),
                Constraint::Percentage(100 - nav_percentage),
            ])
            .split(area)
            .to_vec()
    }
}

fn render_tree(frame: &mut Frame, area: Rect, app: &App) {
    let visible_height = area.height.saturating_sub(2) as usize;
    let (start, end, local_selected) =
        tree_window(app.items.len(), app.selected_index, visible_height.max(1));

    let items = app
        .items
        .iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|item| {
            let indent = "  ".repeat(item.depth);
            let marker = if item.is_dir {
                if app.expanded_dirs.contains(&item.path) {
                    "v"
                } else {
                    ">"
                }
            } else {
                "-"
            };

            ListItem::new(Line::from(vec![Span::raw(format!(
                "{indent}{marker} {}",
                item.name
            ))]))
        })
        .collect::<Vec<_>>();

    let title = if app.focus == Focus::Tree {
        "Docs [focus]"
    } else {
        "Docs"
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!("{title} [{}/{}]", app.selected_index.saturating_add(1), app.items.len()))
                .borders(Borders::ALL)
                .border_style(border_style(app.focus == Focus::Tree)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(" ");

    let mut state = ListState::default();
    state.select(Some(local_selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_preview(frame: &mut Frame, area: Rect, app: &App) {
    let title = if app.preview_fullscreen {
        "Preview completo [Ctrl+Enter]"
    } else if app.focus == Focus::Preview {
        "Preview [focus]"
    } else {
        "Preview"
    };

    let lines = if app.preview.lines.is_empty() {
        vec![Line::from("Selecciona un archivo Markdown para ver el contenido")]
    } else {
        app.preview
            .lines
            .iter()
            .skip(app.preview_scroll)
            .map(styled_preview_line)
            .collect::<Vec<_>>()
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style(app.focus == Focus::Preview)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn border_style(is_focused: bool) -> Style {
    if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

fn styled_preview_line(line: &PreviewLine) -> Line<'static> {
    let style = match line.kind {
        PreviewLineKind::Normal => Style::default().fg(Color::Gray),
        PreviewLineKind::Heading(1) => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        PreviewLineKind::Heading(2) => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        PreviewLineKind::Heading(3) => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        PreviewLineKind::Heading(_) => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        PreviewLineKind::CodeFence => Style::default().fg(Color::LightBlue),
        PreviewLineKind::MermaidPlaceholder => Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::ITALIC),
    };

    Line::from(Span::styled(line.text.clone(), style))
}

fn tree_window(total_items: usize, selected_index: usize, visible_height: usize) -> (usize, usize, usize) {
    if total_items <= visible_height {
        return (0, total_items, selected_index);
    }

    let half = visible_height / 2;
    let mut start = selected_index.saturating_sub(half);
    let mut end = start + visible_height;

    if end > total_items {
        end = total_items;
        start = end.saturating_sub(visible_height);
    }

    let local_selected = selected_index.saturating_sub(start);
    (start, end, local_selected)
}
