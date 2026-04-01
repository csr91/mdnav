use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    layout::Position,
    Frame,
};

use crate::{
    app::{App, Focus, FullscreenPanel, HelpSection, Overlay, PreviewCursor},
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
        pending_cd_span(app),
    ]))
    .block(Block::default().borders(Borders::TOP));

    frame.render_widget(footer, areas[1]);

    match app.overlay {
        Overlay::Help => render_help_popup(frame, app),
        Overlay::MermaidSelect => render_mermaid_select_popup(frame, app),
        Overlay::MermaidOutput => render_mermaid_output_popup(frame, app),
        Overlay::MermaidTerminalView => render_mermaid_terminal_view(frame, app),
        Overlay::WebLink => render_web_link_popup(frame, app),
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

fn pending_cd_span(app: &App) -> Span<'static> {
    let Some(path) = app.pending_cd.as_ref() else {
        return Span::raw("");
    };

    let relative = path.strip_prefix(&app.root).unwrap_or(path);
    let display = if relative.as_os_str().is_empty() {
        String::from("/")
    } else {
        format!("/{}", relative.display().to_string().replace('\\', "/"))
    };

    Span::styled(
        format!("   Go: {display}"),
        Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD),
    )
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
            let selector = if app.selector_path.as_ref() == Some(&item.path) {
                "*"
            } else {
                " "
            };

            ListItem::new(Line::from(vec![Span::raw(format!(
                "{selector}{indent}{marker} {}",
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
        if app.selection.is_some() {
            "Preview [select]"
        } else {
            "Preview [fullscreen]"
        }
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
            .enumerate()
            .skip(app.preview_scroll)
            .map(|(index, line)| styled_preview_line(line, app.selection, index))
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

    if let Some(cursor) = app.selection.map(|selection| selection.cursor) {
        if let Some(position) = preview_cursor_position(app, area, cursor) {
            frame.set_cursor_position(position);
        }
    }
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
        ListItem::new(Line::from("Abrir web link")),
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

fn render_web_link_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(72, 32, frame.area());
    frame.render_widget(Clear, popup_area);

    let link = app
        .web_link_popup
        .as_deref()
        .unwrap_or("Link no disponible");

    let lines = vec![
        Line::from(vec![Span::styled(
            "Link Manual Disponible:",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            link.to_string(),
            Style::default().fg(Color::Yellow),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Enter o Esc para cerrar",
            Style::default().fg(Color::Gray),
        )]),
    ];

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Web Link")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightGreen)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, popup_area);
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
        shortcut_line("Shift+Y", "activar modo seleccion en preview"),
        shortcut_line("Shift+E", "abrir nano sobre el archivo actual"),
        shortcut_line("Shift+G", "dejar pendiente cd al directorio del item"),
        shortcut_line("Shift+0", "pantalla completa del panel enfocado"),
        shortcut_line("Shift+1..5", "ajustar proporcion entre paneles"),
        shortcut_line("q", "salir"),
        Line::from(""),
        Line::from(vec![Span::styled("Preview", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        shortcut_line(", / .", "scroll del documento"),
        shortcut_line("Arrows", "mover cursor en modo seleccion"),
        shortcut_line("Shift+Arrows", "extender seleccion en modo seleccion"),
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

fn styled_preview_line(
    line: &PreviewLine,
    selection: Option<crate::app::SelectionState>,
    line_index: usize,
) -> Line<'static> {
    let base_style = preview_line_style(&line.kind);
    let selected_range = selection_range_for_line(selection, line_index, line.text.chars().count());

    match line.kind {
        PreviewLineKind::MermaidTitle => {
            if let Some((title, hint)) = line.text.split_once("    ") {
                styled_selected_text(
                    &format!("{title}    {hint}"),
                    base_style,
                    selected_range,
                )
            } else {
                styled_selected_text(&line.text, base_style, selected_range)
            }
        }
        _ => styled_selected_text(&line.text, base_style, selected_range),
    }
}

fn preview_line_style(kind: &PreviewLineKind) -> Style {
    match kind {
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
        PreviewLineKind::MermaidTitle => Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    }
}

fn selection_range_for_line(
    selection: Option<crate::app::SelectionState>,
    line_index: usize,
    line_len: usize,
) -> Option<(usize, usize)> {
    let selection = selection?;
    if selection.anchor == selection.cursor {
        return None;
    }

    let (start, end) = normalized_selection_bounds(selection.anchor, selection.cursor);
    if line_index < start.line || line_index > end.line {
        return None;
    }

    let start_col = if line_index == start.line { start.column } else { 0 };
    let end_col = if line_index == end.line { end.column } else { line_len };

    if start_col == end_col {
        None
    } else {
        Some((start_col.min(line_len), end_col.min(line_len)))
    }
}

fn normalized_selection_bounds(left: PreviewCursor, right: PreviewCursor) -> (PreviewCursor, PreviewCursor) {
    if (left.line, left.column) <= (right.line, right.column) {
        (left, right)
    } else {
        (right, left)
    }
}

fn styled_selected_text(text: &str, base: Style, selected: Option<(usize, usize)>) -> Line<'static> {
    let Some((start, end)) = selected else {
        return Line::from(Span::styled(text.to_string(), base));
    };

    let chars = text.chars().collect::<Vec<_>>();
    let prefix = chars.iter().take(start).collect::<String>();
    let middle = chars.iter().skip(start).take(end.saturating_sub(start)).collect::<String>();
    let suffix = chars.iter().skip(end).collect::<String>();
    let selected_style = base.bg(Color::LightCyan).fg(Color::Black);

    let mut spans = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix, base));
    }
    if !middle.is_empty() {
        spans.push(Span::styled(middle, selected_style));
    }
    if !suffix.is_empty() {
        spans.push(Span::styled(suffix, base));
    }

    if spans.is_empty() {
        Line::from(Span::styled(String::new(), base))
    } else {
        Line::from(spans)
    }
}

fn preview_cursor_position(app: &App, area: Rect, cursor: PreviewCursor) -> Option<Position> {
    if cursor.line < app.preview_scroll {
        return None;
    }

    let offset_y = cursor.line.saturating_sub(app.preview_scroll) as u16;
    let inner = if app.fullscreen == FullscreenPanel::Preview {
        area
    } else {
        Rect {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        }
    };

    if offset_y >= inner.height {
        return None;
    }

    let max_x = inner.width.saturating_sub(1) as usize;
    Some(Position::new(
        inner.x.saturating_add(cursor.column.min(max_x) as u16),
        inner.y.saturating_add(offset_y),
    ))
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
