use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, Focus, FullscreenPanel, HelpSection, Overlay},
    config::config_path,
};
use crate::markdown::{PreviewLine, PreviewLineKind};

pub fn render(frame: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(frame.area());

    if app.fullscreen == FullscreenPanel::Tree {
        render_tree(frame, areas[0], app);
    } else if app.fullscreen == FullscreenPanel::Preview {
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
        Span::styled("== For Help Type: ? ==", Style::default().fg(Color::Yellow)),
        footer_item_span(app),
    ]))
    .block(Block::default().borders(Borders::TOP));

    frame.render_widget(footer, areas[1]);

    match app.overlay {
        Overlay::Help => render_help_popup(frame, app),
        Overlay::MermaidSelect => render_mermaid_select_popup(frame, app),
        Overlay::MermaidOutput => render_mermaid_output_popup(frame, app),
        Overlay::MermaidTerminalView => render_mermaid_terminal_view(frame, app),
        Overlay::None => {}
    }
}

fn footer_item_span(app: &App) -> Span<'static> {
    let Some(item) = app.items.get(app.selected_index) else {
        return Span::styled(app.status.clone(), Style::default().fg(Color::Yellow));
    };

    let is_open_file = app
        .current_file
        .as_ref()
        .map(|current| current == &item.path)
        .unwrap_or(false);

    let color = if is_open_file {
        Color::Green
    } else {
        Color::Yellow
    };

    Span::styled(item.name.clone(), Style::default().fg(color))
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

    let title = if app.fullscreen == FullscreenPanel::Tree {
        "Docs [fullscreen]"
    } else if app.focus == Focus::Tree {
        "Docs [focus]"
    } else {
        "Docs"
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    "{title} [{}/{}]",
                    app.selected_index.saturating_add(1),
                    app.items.len()
                ))
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
    let title = if app.fullscreen == FullscreenPanel::Preview {
        "Preview [fullscreen]"
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

    let paragraph = if app.fullscreen == FullscreenPanel::Preview {
        Paragraph::new(lines).wrap(Wrap { trim: false })
    } else {
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(border_style(app.focus == Focus::Preview)),
            )
            .wrap(Wrap { trim: false })
    };

    frame.render_widget(paragraph, area);
}

fn render_mermaid_select_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(60, 40, frame.area());
    frame.render_widget(Clear, popup_area);

    let items = app
        .preview
        .mermaid_diagrams
        .iter()
        .enumerate()
        .map(|(index, diagram)| {
            let label = format!("{} - {}", index + 1, diagram.title);
            ListItem::new(Line::from(label))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .title("Seleccionar Mermaid")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightMagenta)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Magenta)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    state.select(Some(app.mermaid_selected_index));
    frame.render_stateful_widget(list, popup_area, &mut state);

    let help = Paragraph::new("Up/Down elegir | Enter abrir | Esc cancelar")
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Gray));
    let help_area = Rect {
        x: popup_area.x,
        y: popup_area.y + popup_area.height.saturating_sub(2),
        width: popup_area.width,
        height: 1,
    };
    frame.render_widget(help, help_area);
}

fn render_mermaid_output_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(44, 28, frame.area());
    frame.render_widget(Clear, popup_area);

    let items = vec![
        ListItem::new(Line::from("Render terminal")),
        ListItem::new(Line::from("Abrir HTML")),
    ];

    let list = List::new(items)
        .block(
            Block::default()
                .title("Salida Mermaid")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightCyan)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    state.select(Some(app.mermaid_output_selected_index));
    frame.render_stateful_widget(list, popup_area, &mut state);
}

fn render_mermaid_terminal_view(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    let lines = mermaid_canvas_viewport(
        &app.mermaid_terminal_canvas,
        app.mermaid_canvas_x,
        app.mermaid_canvas_y,
        inner_width,
        inner_height,
    )
    .into_iter()
    .map(|line| {
        Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
    })
    .collect::<Vec<_>>();

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Mermaid terminal view [Esc]  arrows/hjkl move")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightCyan)),
        )
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn render_help_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(68, 56, frame.area());
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title("Help")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(2)])
        .split(inner);

    render_help_tabs(frame, sections[0], app.help_section);

    let lines = match app.help_section {
        HelpSection::Shortcuts => shortcut_lines(),
        HelpSection::Settings => settings_lines(app),
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, sections[1]);

    let footer = match app.help_section {
        HelpSection::Shortcuts => "Left/Right switch sections | ? or Esc close",
        HelpSection::Settings => "Enter toggle | Left/Right switch sections | ? or Esc close",
    };
    frame.render_widget(
        Paragraph::new(footer)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center),
        sections[2],
    );
}

fn render_help_tabs(frame: &mut Frame, area: Rect, selected: HelpSection) {
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(18), Constraint::Length(18), Constraint::Min(1)])
        .split(area);

    frame.render_widget(help_tab("Shortcuts", selected == HelpSection::Shortcuts), tabs[0]);
    frame.render_widget(help_tab("Settings", selected == HelpSection::Settings), tabs[1]);
}

fn help_tab(title: &str, active: bool) -> Paragraph<'static> {
    let style = if active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    Paragraph::new(title.to_string())
        .alignment(Alignment::Center)
        .style(style)
        .block(Block::default().borders(Borders::ALL))
}

fn shortcut_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![Span::styled("Navigation", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        shortcut_line("Enter", "abrir archivo o expandir carpeta"),
        shortcut_line("Tab / Shift+Tab", "cambiar foco entre arbol y preview"),
        shortcut_line("Shift+0", "pantalla completa del panel enfocado"),
        shortcut_line("Shift+1..5", "ajustar proporcion entre paneles"),
        shortcut_line("q", "salir"),
        Line::from(""),
        Line::from(vec![Span::styled("Preview", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        shortcut_line(", / .", "scroll del documento"),
        shortcut_line("Shift+M", "abrir acciones Mermaid"),
        shortcut_line("?", "abrir o cerrar este menu"),
    ]
}

fn shortcut_line(keys: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{keys:<18}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(description.to_string(), Style::default().fg(Color::Gray)),
    ])
}

fn settings_lines(app: &App) -> Vec<Line<'static>> {
    let toggle = if app.config.only_mds { "ON" } else { "OFF" };
    let toggle_style = if app.config.only_mds {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };

    vec![
        Line::from(vec![Span::styled("User Config", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Only Mds        ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(toggle, toggle_style),
        ]),
        Line::from(""),
        Line::from("When ON, the tree shows only .md files."),
        Line::from("When OFF, mdnav lists every file in the directory tree."),
        Line::from(""),
        Line::from("This setting is stored per user."),
        Line::from(format!(
            "Config path: {}",
            config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "unavailable".to_string())
        )),
        Line::from(""),
        Line::from("Press Enter to toggle this setting."),
    ]
}

fn border_style(is_focused: bool) -> Style {
    if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

fn styled_preview_line(line: &PreviewLine) -> Line<'static> {
    match line.kind {
        PreviewLineKind::MermaidTitle => {
            if let Some((title, hint)) = line.text.split_once("    ") {
                Line::from(vec![
                    Span::styled(
                        title.to_string(),
                        Style::default()
                            .fg(Color::LightCyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("    "),
                    Span::styled(
                        hint.to_string(),
                        Style::default()
                            .fg(Color::Gray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ])
            } else {
                Line::from(Span::styled(
                    line.text.clone(),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ))
            }
        }
        _ => {
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
                PreviewLineKind::MermaidEdge => Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                PreviewLineKind::MermaidTitle => unreachable!(),
            };

            Line::from(Span::styled(line.text.clone(), style))
        }
    }
}

fn mermaid_canvas_viewport(
    canvas: &[String],
    offset_x: usize,
    offset_y: usize,
    width: usize,
    height: usize,
) -> Vec<String> {
    let mut view = Vec::with_capacity(height);

    for row in 0..height {
        let source_index = offset_y + row;
        if let Some(source) = canvas.get(source_index) {
            let line = source.chars().skip(offset_x).take(width).collect::<String>();
            view.push(line);
        } else {
            view.push(String::new());
        }
    }

    view
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

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}
