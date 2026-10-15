use ratatui::{
    layout::Position,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::markdown::{PreviewLine, PreviewLineKind};
use crate::{
    app::{
        App, CreateKind, CreateStep, FileOpKind, Focus, FullscreenPanel, GitState, GitStatusKind,
        HelpSection, Overlay, PreviewCursor, ResponseTab, HTTP_MENU_ITEMS, RESPONSE_TABS,
    },
    config::{config_path, TreeInfoMode},
    http::{self, HttpResponse},
    strings::Strings,
};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn render(frame: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(frame.area());

    if app.json_viewer_active {
        render_json_viewer(frame, areas[0], app);
    } else if app.fullscreen == FullscreenPanel::Tree {
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

    let s = app.config.strings();
    let footer = if app.json_viewer_active {
        Paragraph::new(Line::from(Span::styled(
            "JSON VIEWER  F5 procesar  Tab cambiar panel  y copiar  Shift+0 vista limpia  Esc cerrar",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::TOP))
    } else if app.overlay == Overlay::Rename {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "Rename: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(app.rename_input.clone(), Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Yellow)),
            Span::styled(
                "  Enter=confirmar  Esc=cancelar",
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.overlay == Overlay::CommandPalette {
        Paragraph::new(Line::from(vec![
            Span::styled(
                ":",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(app.palette_query.clone(), Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.selection.is_some()
        && (app.status.starts_with("Text copied!")
            || app.status.starts_with("Clipboard copy failed"))
    {
        let color = if app.status.starts_with("Text copied!") {
            Color::Green
        } else {
            Color::Red
        };
        Paragraph::new(Line::from(Span::styled(
            app.status.clone(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::TOP))
    } else if app.selection.as_ref().map(|s| s.anchored).unwrap_or(false) {
        let (label, hint, color) = (
            s.select_on.to_string(),
            s.select_copy_hint.to_string(),
            Color::Green,
        );
        Paragraph::new(Line::from(vec![
            Span::styled(
                label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(hint, Style::default().fg(Color::Gray)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.pending_delete.is_some() {
        Paragraph::new(Line::from(vec![Span::styled(
            app.status.clone(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.pending_go_up {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "Go up?  ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "← de nuevo para subir un nivel  Esc=cancelar",
                Style::default().fg(Color::Gray),
            ),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.selection.is_some() {
        Paragraph::new(Line::from(vec![
            Span::styled(
                s.select_cursor.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                s.select_anchor_hint.to_string(),
                Style::default().fg(Color::Gray),
            ),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else {
        Paragraph::new(Line::from(context_footer_spans(app, s)))
            .block(Block::default().borders(Borders::TOP))
    };

    frame.render_widget(footer, areas[1]);
    render_mode_pill(frame, areas[1], app);

    match app.overlay {
        Overlay::Help => render_help_popup(frame, app),
        Overlay::MermaidSelect => render_mermaid_select_popup(frame, app),
        Overlay::MermaidOutput => render_mermaid_output_popup(frame, app),
        Overlay::MermaidTerminalView => render_mermaid_terminal_view(frame, app),
        Overlay::WebLink => render_web_link_popup(frame, app),
        Overlay::Search => render_search_popup(frame, app),
        Overlay::Grep => render_grep_popup(frame, app),
        Overlay::DestPicker => render_dest_picker_popup(frame, app),
        Overlay::Toc => render_toc_popup(frame, app),
        Overlay::CommandPalette => render_command_palette(frame, app),
        Overlay::RecentFiles => render_recent_files_popup(frame, app),
        Overlay::Marks => render_marks_popup(frame, app),
        Overlay::Find => render_find_popup(frame, app),
        Overlay::Create => render_create_popup(frame, app),
        Overlay::Git => render_git_popup(frame, app),
        Overlay::Alert => render_alert_popup(frame, app),
        Overlay::HttpMenu => render_http_menu_popup(frame, app),
        Overlay::HttpEnv => render_http_env_popup(frame, app),
        Overlay::HttpInput => render_http_input_popup(frame, app),
        Overlay::Rename => {}
        Overlay::None => {}
    }
}

fn render_mode_pill(frame: &mut Frame, area: Rect, app: &App) {
    let (label, color) = mode_label(app);
    let width = (label.len() as u16 + 4).min(area.width);
    let pill_area = Rect {
        x: area.x + area.width.saturating_sub(width),
        y: area.y,
        width,
        height: 1,
    };
    frame.render_widget(Clear, pill_area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        pill_area,
    );
}

fn mode_label(app: &App) -> (&'static str, Color) {
    match app.overlay {
        Overlay::CommandPalette => ("COMMAND", Color::Yellow),
        Overlay::Marks => ("MARKS", Color::Cyan),
        Overlay::Grep => ("GREP", Color::Green),
        Overlay::Find => ("FIND", Color::Magenta),
        Overlay::Search => ("FILES", Color::Cyan),
        Overlay::Git => ("GIT", Color::Yellow),
        Overlay::Help => ("HELP", Color::Yellow),
        Overlay::RecentFiles => ("RECENT", Color::Cyan),
        Overlay::Toc => ("TOC", Color::Magenta),
        Overlay::Create | Overlay::Rename | Overlay::DestPicker | Overlay::Alert => {
            ("ACTION", Color::Yellow)
        }
        Overlay::MermaidSelect | Overlay::MermaidOutput | Overlay::MermaidTerminalView => {
            ("MERMAID", Color::Cyan)
        }
        Overlay::WebLink => ("LINK", Color::Cyan),
        Overlay::HttpMenu | Overlay::HttpEnv | Overlay::HttpInput => ("HTTP", Color::Green),
        Overlay::None => {
            if app.json_viewer_active {
                return ("JSON", Color::Green);
            }
            if let Some(selection) = app.selection.as_ref() {
                return if selection.anchored {
                    ("VISUAL", Color::Green)
                } else {
                    ("SELECT", Color::Cyan)
                };
            }

            if app.focus == Focus::Preview && app.http.view.is_some() {
                return if app.http.response_focus {
                    ("RESPONSE", Color::Green)
                } else {
                    ("HTTP", Color::Green)
                };
            }

            match app.fullscreen {
                FullscreenPanel::Tree => ("TREE", Color::Cyan),
                FullscreenPanel::Preview => ("PREVIEW", Color::Cyan),
                FullscreenPanel::None => match app.focus {
                    Focus::Tree => ("TREE", Color::Cyan),
                    Focus::Preview => ("PREVIEW", Color::Cyan),
                },
            }
        }
    }
}

fn context_footer_spans(app: &App, s: &Strings) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let focus = match app.focus {
        Focus::Tree => "TREE",
        Focus::Preview if app.http_response_focused() => "RESPONSE",
        Focus::Preview if app.http.view.is_some() => "HTTP",
        Focus::Preview => "PREVIEW",
    };
    spans.push(Span::styled(
        format!("{focus}  "),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));

    let hint = match app.focus {
        Focus::Tree => "↑↓/jk move · Enter/l open · h parent · Tab preview · : command · ? help  ",
        Focus::Preview if app.http_response_focused() => {
            "↑↓/jk scroll · []/hl tabs · y copy body · S send · Tab tree  "
        }
        Focus::Preview if app.http.view.is_some() => {
            "S send · Tab response · ↑↓ scroll · E edit · :httpenv env  "
        }
        Focus::Preview => "↑↓/jk scroll · [] links · T toc · Y select · Tab tree · : command  ",
    };
    spans.push(Span::styled(hint, Style::default().fg(Color::Gray)));

    // A request keeps running while the user browses other files.
    if app.http_busy() && app.current_request_in_flight().is_none() {
        spans.push(Span::styled(
            "⟳ HTTP en curso  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if app.http.view.is_some() {
        // Request results and HTTP feedback matter more than file metadata.
        spans.push(Span::styled(
            app.status.clone(),
            Style::default().fg(Color::Yellow),
        ));
    } else if let Some(metadata) = app.current_file_metadata() {
        spans.push(Span::styled(metadata, Style::default().fg(Color::DarkGray)));
    } else {
        spans.push(Span::styled(
            s.help_hint.to_string(),
            Style::default().fg(Color::Yellow),
        ));
        spans.push(footer_item_span(app));
    }
    spans.push(pending_cd_span(app));
    spans
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
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
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

    let n_bookmarks = app.items.iter().take_while(|i| i.is_bookmark).count();
    let has_separator = n_bookmarks > 0 && app.items.len() > n_bookmarks;
    let adjusted_selected = if has_separator && local_selected >= n_bookmarks {
        local_selected + 1
    } else {
        local_selected
    };

    let items = app
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .flat_map(|(idx, item)| {
            let mut result = Vec::new();

            // separator between bookmark section and regular items
            if idx == n_bookmarks && n_bookmarks > 0 {
                result.push(ListItem::new(Line::from(Span::styled(
                    " ─────────────────────",
                    Style::default().fg(Color::DarkGray),
                ))));
            }

            if item.is_bookmark {
                let selector = if app.selector_path.as_ref() == Some(&item.path) {
                    "*"
                } else {
                    " "
                };
                let marker = item_marker(app, item);
                result.push(ListItem::new(Line::from(vec![
                    Span::raw(selector.to_string()),
                    Span::styled(
                        format!("{marker} {}", item.name),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])));
            } else {
                let indent = "  ".repeat(item.depth);
                let marker = item_marker(app, item);
                let selector = if app.selector_path.as_ref() == Some(&item.path) {
                    "*"
                } else {
                    " "
                };
                let (git_label, git_style) = if app.config.show_git_status {
                    git_status_style(app.git_status_for_item(item))
                } else {
                    ("", Style::default())
                };
                let prefix_part = format!("{selector}{indent}");
                let name_part = format!("{prefix_part}{git_label}{marker} {}", item.name);

                let info = if !item.is_dir && app.config.tree_info != TreeInfoMode::Off {
                    app.tree_info_cache.get(&item.path).map(|s| s.as_str())
                } else {
                    None
                };

                if let Some(info_str) = info {
                    let content_width = area.width.saturating_sub(2) as usize;
                    let name_len = name_part.chars().count();
                    let info_len = info_str.chars().count();
                    let padding = content_width.saturating_sub(name_len + info_len).max(1);
                    result.push(ListItem::new(Line::from(vec![
                        Span::raw(prefix_part),
                        Span::styled(git_label.to_string(), git_style),
                        Span::raw(format!("{marker} {}", item.name)),
                        Span::raw(" ".repeat(padding)),
                        Span::styled(info_str.to_string(), Style::default().fg(Color::DarkGray)),
                    ])));
                } else {
                    result.push(ListItem::new(Line::from(vec![
                        Span::raw(prefix_part),
                        Span::styled(git_label.to_string(), git_style),
                        Span::raw(format!("{marker} {}", item.name)),
                    ])));
                }
            }

            result
        })
        .collect::<Vec<_>>();

    let s = app.config.strings();
    let title = if app.fullscreen == FullscreenPanel::Tree {
        s.tree_fullscreen
    } else if app.focus == Focus::Tree {
        s.tree_focus
    } else {
        s.tree_title
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
    state.select(Some(adjusted_selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn git_status_style(status: Option<GitStatusKind>) -> (&'static str, Style) {
    match status {
        Some(GitStatusKind::Ignored) => ("! ", Style::default().fg(Color::DarkGray)),
        Some(GitStatusKind::Untracked) => ("? ", Style::default().fg(Color::Cyan)),
        Some(GitStatusKind::Modified) => ("M ", Style::default().fg(Color::Yellow)),
        Some(GitStatusKind::Staged) => ("A ", Style::default().fg(Color::Green)),
        Some(GitStatusKind::Renamed) => ("R ", Style::default().fg(Color::Cyan)),
        Some(GitStatusKind::Deleted) => ("D ", Style::default().fg(Color::Red)),
        Some(GitStatusKind::Conflicted) => (
            "U ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        None => ("  ", Style::default()),
    }
}

fn item_marker(app: &App, item: &crate::docs::DocItem) -> &'static str {
    if item.is_dir {
        if app.expanded_dirs.contains(&item.path) {
            "v"
        } else {
            ">"
        }
    } else {
        match item
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase())
            .as_deref()
        {
            Some("md" | "markdown") => "M",
            Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp") => "I",
            Some("rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "sh" | "ps1") => "C",
            Some("json" | "toml" | "yaml" | "yml") => "D",
            _ => ".",
        }
    }
}

fn render_preview(frame: &mut Frame, area: Rect, app: &App) {
    // Selection mode keeps the single pane: its cursor math assumes it.
    if app.http.view.is_some() && app.selection.is_none() {
        // The request takes what it needs (up to half), the response the rest.
        let request_height = (app.preview.lines.len() as u16)
            .saturating_add(2)
            .clamp(5, (area.height / 2).max(5));
        let panes = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(request_height), Constraint::Min(0)])
            .split(area);
        let request_focused = app.focus == Focus::Preview && !app.http.response_focus;
        render_document(frame, panes[0], app, request_focused, true);
        render_response(frame, panes[1], app);
        return;
    }

    render_document(
        frame,
        area,
        app,
        app.focus == Focus::Preview,
        app.fullscreen != FullscreenPanel::Preview,
    );
}

fn render_json_viewer(frame: &mut Frame, area: Rect, app: &App) {
    if app.fullscreen == FullscreenPanel::Preview {
        let lines = if let Some(error) = &app.json_viewer_error {
            vec![Line::from(Span::styled(
                error,
                Style::default().fg(Color::Red),
            ))]
        } else if !app.json_viewer_processed {
            vec![Line::from(Span::styled(
                "Pulsa F5 para procesar el JSON",
                Style::default().fg(Color::DarkGray),
            ))]
        } else {
            app.json_viewer_pretty
                .lines()
                .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::White))))
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.preview_scroll as u16, 0)),
            area,
        );
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);
    let button = Paragraph::new("[ Procesar JSON ]   F5  (Enter con foco en la salida)")
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(button, sections[0]);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sections[1]);

    let raw_lines = app
        .json_viewer_raw
        .lines()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::Gray))))
        .collect::<Vec<_>>();
    let pretty_lines = if !app.json_viewer_processed {
        vec![Line::from(Span::styled(
            "Pulsa F5 o Enter para procesar el JSON",
            Style::default().fg(Color::DarkGray),
        ))]
    } else if let Some(error) = &app.json_viewer_error {
        vec![
            Line::from(Span::styled(
                "JSON incompleto o invalido:",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(error, Style::default().fg(Color::Red))),
        ]
    } else {
        app.json_viewer_pretty
            .lines()
            .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::White))))
            .collect::<Vec<_>>()
    };

    let raw = Paragraph::new(raw_lines)
        .block(
            Block::default()
                .title(" Texto de entrada ")
                .borders(Borders::ALL)
                .border_style(if app.focus == Focus::Tree {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll as u16, 0));
    let pretty = Paragraph::new(pretty_lines)
        .block(
            Block::default()
                .title(" JSON formateado ")
                .borders(Borders::ALL)
                .border_style(if app.focus == Focus::Preview {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll as u16, 0));

    frame.render_widget(raw, panes[0]);
    frame.render_widget(pretty, panes[1]);
}

fn render_document(frame: &mut Frame, area: Rect, app: &App, focused: bool, bordered: bool) {
    let s = app.config.strings();
    let base_title = if app.current_file.is_none() && app.http.view.is_none() {
        "Bienvenido a mdnav"
    } else if app.http.view.is_some() && app.selection.is_none() {
        "Request"
    } else if app.fullscreen == FullscreenPanel::Preview {
        if app.selection.is_some() {
            s.preview_select
        } else {
            s.preview_fullscreen
        }
    } else if focused {
        s.preview_focus
    } else {
        s.preview_title
    };
    let title = app
        .current_file_breadcrumb()
        .map(|breadcrumb| format!("{base_title} · {breadcrumb}"))
        .unwrap_or_else(|| base_title.to_string());

    let lines = if app.current_file.is_none() && app.http.view.is_none() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Explora tus documentos desde el arbol",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  Up/Down  ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Moverse por el arbol", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled(
                    "  Enter  ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Abrir archivo o expandir carpeta",
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "  Tab    ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Cambiar entre arbol y vista previa",
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "  ?      ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Ver todos los atajos", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled(
                    "  @      ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Abrir JSON Viewer", Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Selecciona un archivo para comenzar.",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else if app.preview.lines.is_empty() {
        vec![Line::from(Span::styled(
            s.preview_empty,
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        // Line index of the active link (if any)
        let active_link_line = app
            .preview_link_cursor
            .and_then(|i| app.preview.links.get(i))
            .map(|l| l.line_index);

        app.preview
            .lines
            .iter()
            .enumerate()
            .skip(app.preview_scroll)
            .map(|(index, line)| {
                styled_preview_line(
                    line,
                    app.selection,
                    index,
                    active_link_line,
                    app.active_find_query.as_str(),
                )
            })
            .collect::<Vec<_>>()
    };

    let paragraph = if bordered {
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(border_style(focused)),
            )
            .wrap(Wrap { trim: false })
    } else {
        Paragraph::new(lines).wrap(Wrap { trim: false })
    };

    frame.render_widget(paragraph, area);

    if let Some(cursor) = app.selection.map(|selection| selection.cursor) {
        if let Some(position) = preview_cursor_position(app, area, cursor) {
            frame.set_cursor_position(position);
        }
    }
}

fn render_response(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Preview && app.http.response_focus;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(focused));

    if let Some(in_flight) = app.current_request_in_flight() {
        let elapsed = in_flight.started.elapsed();
        let spinner = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
        let lines = vec![
            Line::from(Span::styled(
                format!("{spinner} Enviando… {}", http::format_elapsed(elapsed)),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                in_flight.label.clone(),
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Esc abandona el request",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .block(block.title(" Response "))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let Some(response) = app.current_http_response() else {
        let mut lines = vec![
            Line::from(Span::styled(
                "Todavia no se envio",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            shortcut_line("Shift+S", "enviar el request"),
            shortcut_line("Tab", "pasar de request a response"),
            shortcut_line(":httpenv", "elegir environment"),
            shortcut_line("Shift+E", "editar el YAML"),
        ];
        if app.http_busy() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Hay otro request en curso",
                Style::default().fg(Color::Yellow),
            )));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .block(block.title(" Response "))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    };

    let block = block.title(response_title(response));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(response_tabs_line(app, response)),
        sections[0],
    );
    frame.render_widget(
        Paragraph::new(
            response_tab_lines(app, response)
                .into_iter()
                .skip(app.http.response_scroll)
                .collect::<Vec<_>>(),
        )
        .wrap(Wrap { trim: false }),
        sections[1],
    );
}

fn status_color(status: u16) -> Color {
    match status {
        200..=299 => Color::Green,
        300..=399 => Color::Cyan,
        400..=499 => Color::Yellow,
        _ => Color::Red,
    }
}

fn response_title(response: &HttpResponse) -> Line<'static> {
    let elapsed = http::format_elapsed(response.elapsed);
    let (status, color) = match (&response.error, response.status) {
        (None, Some(code)) => (
            format!("{code} {}", response.reason).trim().to_string(),
            status_color(code),
        ),
        _ => (String::from("error"), Color::Red),
    };
    let mut details = format!(" · {elapsed}");
    if response.error.is_none() {
        details.push_str(&format!(" · {}", http::format_size(response.size)));
    }
    Line::from(vec![
        Span::raw(" Response · "),
        Span::styled(
            status,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{details} "), Style::default().fg(Color::Gray)),
    ])
}

fn response_tabs_line(app: &App, response: &HttpResponse) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, tab) in RESPONSE_TABS.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        }
        let label = match tab {
            ResponseTab::Body => String::from("Body"),
            ResponseTab::Headers => format!("Headers ({})", response.headers.len()),
            ResponseTab::Info => String::from("Info"),
        };
        let style = if *tab == app.http.response_tab {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    Line::from(spans)
}

fn response_tab_lines(app: &App, response: &HttpResponse) -> Vec<Line<'static>> {
    let gray = Style::default().fg(Color::Gray);
    match app.http.response_tab {
        ResponseTab::Body => {
            if let Some(error) = &response.error {
                return vec![
                    Line::from(Span::styled(
                        error.clone(),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Revisa la URL, el environment activo y que el servidor este levantado.",
                        gray,
                    )),
                ];
            }
            if response.body_lines.is_empty() {
                return vec![Line::from(Span::styled("(sin body)", gray))];
            }
            let mut lines: Vec<Line<'static>> = response
                .body_lines
                .iter()
                .enumerate()
                .map(|(index, line)| styled_preview_line(line, None, index, None, ""))
                .collect();
            if response.truncated {
                lines.push(Line::from(Span::styled(
                    format!(
                        "… body truncado: se muestran los primeros {}",
                        http::format_size(response.body_text.len())
                    ),
                    Style::default().fg(Color::Yellow),
                )));
            }
            lines
        }
        ResponseTab::Headers => {
            if response.headers.is_empty() {
                return vec![Line::from(Span::styled("(sin headers)", gray))];
            }
            response
                .headers
                .iter()
                .map(|(name, value)| {
                    Line::from(vec![
                        Span::styled(format!("{name}: "), Style::default().fg(Color::Cyan)),
                        Span::styled(value.clone(), Style::default().fg(Color::White)),
                    ])
                })
                .collect()
        }
        ResponseTab::Info => {
            let status = match response.status {
                Some(code) => format!("{code} {}", response.reason),
                None => String::from("sin respuesta"),
            };
            [
                ("Status", status),
                ("Tiempo", http::format_elapsed(response.elapsed)),
                ("Tamaño", http::format_size(response.size)),
                ("Metodo", response.method.clone()),
                ("URL", response.url.clone()),
                (
                    "Content-Type",
                    response
                        .content_type
                        .clone()
                        .unwrap_or_else(|| String::from("-")),
                ),
                (
                    "Truncado",
                    if response.truncated { "si" } else { "no" }.to_string(),
                ),
            ]
            .into_iter()
            .map(|(label, value)| {
                Line::from(vec![
                    Span::styled(format!("{label:<14}"), gray),
                    Span::styled(value, Style::default().fg(Color::White)),
                ])
            })
            .collect()
        }
    }
}

fn render_http_menu_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(60, 45, frame.area());
    frame.render_widget(Clear, popup_area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(popup_area);

    let items: Vec<ListItem> = HTTP_MENU_ITEMS
        .iter()
        .map(|(name, description)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{name:<8}"),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {description}"), Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .title(" HTTP  (↑↓ navigate | Enter run | Esc close) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut state = ListState::default();
    state.select(Some(app.http.menu_cursor));
    frame.render_stateful_widget(list, sections[0], &mut state);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  new/init: carpeta seleccionada · import: JSON seleccionado",
            Style::default().fg(Color::DarkGray),
        ))),
        sections[1],
    );
}

fn render_http_env_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(50, 45, frame.area());
    frame.render_widget(Clear, popup_area);

    let active = app.http.env_root.as_ref().and_then(|root| {
        app.config
            .http_environments
            .get(&root.display().to_string())
    });
    let mut items = vec![ListItem::new(Line::from(Span::styled(
        format!(
            "{} (sin environment)",
            if active.is_none() { "●" } else { " " }
        ),
        Style::default().fg(Color::Gray),
    )))];
    items.extend(app.http.env_options.iter().map(|name| {
        let marker = if active == Some(name) { "●" } else { " " };
        ListItem::new(Line::from(Span::styled(
            format!("{marker} {name}"),
            Style::default().fg(Color::White),
        )))
    }));

    let title = if app.http.env_options.is_empty() {
        " Environment · no hay environments/*.yml "
    } else {
        " Environment  (Enter activar | Esc cerrar) "
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut state = ListState::default();
    state.select(Some(app.http.env_cursor));
    frame.render_stateful_widget(list, popup_area, &mut state);
}

fn render_http_input_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(60, 20, frame.area());
    frame.render_widget(Clear, popup_area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(2)])
        .split(popup_area);

    frame.render_widget(
        Paragraph::new(format!("{}_", app.http.input))
            .block(
                Block::default()
                    .title(" Request nuevo  (Enter crear | Esc cancelar) ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .style(Style::default().fg(Color::White)),
        sections[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Se crea <nombre>.yml en la carpeta seleccionada",
            Style::default().fg(Color::DarkGray),
        ))),
        sections[1],
    );
}

fn render_mermaid_select_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
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
                .title(s.mermaid_select_title)
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

    let help = Paragraph::new(s.mermaid_select_hint)
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
    let s = app.config.strings();
    let popup_area = centered_rect(44, 28, frame.area());
    frame.render_widget(Clear, popup_area);

    let items = vec![
        ListItem::new(Line::from(s.mermaid_render_terminal)),
        ListItem::new(Line::from(s.mermaid_open_html)),
        ListItem::new(Line::from(s.mermaid_open_web)),
    ];

    let list = List::new(items)
        .block(
            Block::default()
                .title(s.mermaid_output_title)
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

    let selected_node = app
        .mermaid_selected_node
        .and_then(|i| app.mermaid_canvas.nodes.get(i));

    let normal_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let highlight_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let lines = (0..inner_height)
        .map(|row| {
            let canvas_y = app.mermaid_canvas_y + row;
            let source = app
                .mermaid_canvas
                .lines
                .get(canvas_y)
                .map(|s| s.as_str())
                .unwrap_or("");

            let chars: Vec<char> = source
                .chars()
                .skip(app.mermaid_canvas_x)
                .take(inner_width)
                .collect();

            // If there's a selected node and this row intersects its box, build colored spans
            if let Some(node) = selected_node {
                if canvas_y >= node.y && canvas_y < node.y + node.height {
                    let node_col_start = node.x.saturating_sub(app.mermaid_canvas_x);
                    let node_col_end = (node.x + node.width).saturating_sub(app.mermaid_canvas_x);

                    if node_col_start < inner_width {
                        let before: String =
                            chars[..node_col_start.min(chars.len())].iter().collect();
                        let hl_start = node_col_start.min(chars.len());
                        let hl_end = node_col_end.min(chars.len());
                        let highlighted: String = chars[hl_start..hl_end].iter().collect();
                        let after: String = chars[hl_end..].iter().collect();

                        let mut spans = Vec::new();
                        if !before.is_empty() {
                            spans.push(Span::styled(before, normal_style));
                        }
                        if !highlighted.is_empty() {
                            spans.push(Span::styled(highlighted, highlight_style));
                        }
                        if !after.is_empty() {
                            spans.push(Span::styled(after, normal_style));
                        }
                        return Line::from(spans);
                    }
                }
            }

            Line::from(Span::styled(chars.iter().collect::<String>(), normal_style))
        })
        .collect::<Vec<_>>();

    let node_hint = selected_node
        .map(|n| {
            if n.url.is_some() {
                format!("  │  {} [Enter=link]", n.label)
            } else {
                format!("  │  {}", n.label)
            }
        })
        .unwrap_or_default();

    let s = app.config.strings();
    let title = format!("{}{}", s.mermaid_view_title, node_hint);

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightCyan)),
        )
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, area);
}

fn render_help_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let area = frame.area();
    let popup_area = Rect {
        x: area.x.saturating_add(2),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(4).max(1),
        height: area.height.saturating_sub(3).max(1),
    };
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" Help  v{} ", env!("CARGO_PKG_VERSION")))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(block, popup_area);

    let inner = Rect {
        x: popup_area.x.saturating_add(2),
        y: popup_area.y.saturating_add(1),
        width: popup_area.width.saturating_sub(4),
        height: popup_area.height.saturating_sub(2),
    };

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(inner);

    render_help_tabs(frame, sections[0], app.help_section, s);

    match app.help_section {
        HelpSection::Shortcuts => render_shortcuts_help(frame, sections[1], s),
        HelpSection::Settings => {
            frame.render_widget(
                Paragraph::new(settings_lines(app, s)).wrap(Wrap { trim: false }),
                sections[1],
            );
        }
    }

    let footer = match app.help_section {
        HelpSection::Shortcuts => s.help_footer_shortcuts,
        HelpSection::Settings => s.help_footer_settings,
    };
    frame.render_widget(
        Paragraph::new(footer)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center),
        sections[2],
    );
}

fn render_shortcuts_help(frame: &mut Frame, area: Rect, s: &'static Strings) {
    let groups = shortcut_groups(s);
    let columns = if area.width >= 110 {
        3
    } else if area.width >= 72 {
        2
    } else {
        1
    };
    let constraints = vec![Constraint::Ratio(1, columns as u32); columns];
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    let mut column_lines = vec![Vec::new(); columns];
    for (index, group) in groups.iter().enumerate() {
        let target = index % columns;
        if !column_lines[target].is_empty() {
            column_lines[target].push(Line::from(""));
        }
        column_lines[target].extend(shortcut_group_lines(group));
    }

    for (index, pane) in panes.iter().enumerate() {
        frame.render_widget(
            Paragraph::new(column_lines[index].clone()).wrap(Wrap { trim: false }),
            *pane,
        );
    }
}

fn render_web_link_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(72, 32, frame.area());
    frame.render_widget(Clear, popup_area);

    let s = app.config.strings();
    let link = app
        .web_link_popup
        .as_deref()
        .unwrap_or("Link no disponible");

    let lines = vec![
        Line::from(vec![Span::styled(
            s.weblink_available,
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
            s.weblink_close,
            Style::default().fg(Color::Gray),
        )]),
    ];

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(s.weblink_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightGreen)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, popup_area);
}

fn render_help_tabs(frame: &mut Frame, area: Rect, selected: HelpSection, s: &'static Strings) {
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Min(1),
        ])
        .split(area);

    frame.render_widget(
        help_tab(s.tab_shortcuts, selected == HelpSection::Shortcuts),
        tabs[0],
    );
    frame.render_widget(
        help_tab(s.tab_settings, selected == HelpSection::Settings),
        tabs[1],
    );
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

struct ShortcutGroup {
    title: &'static str,
    items: Vec<(&'static str, &'static str)>,
}

fn shortcut_groups(s: &'static Strings) -> Vec<ShortcutGroup> {
    vec![
        ShortcutGroup {
            title: s.nav_section,
            items: vec![
                ("Enter", s.sc_enter),
                ("Tab / Shift+Tab", s.sc_tab),
                ("Shift+E", s.sc_shift_e),
                ("Shift+R", s.sc_shift_r),
                ("Shift+B", s.sc_shift_b),
                ("u", s.sc_u),
                ("Ctrl+Shift+C", s.sc_ctrl_shift_c),
                ("Shift+G", s.sc_shift_g),
                ("Shift+0", s.sc_shift_0),
                ("Shift+1..5", s.sc_shift_1_5),
            ],
        },
        ShortcutGroup {
            title: s.preview_section,
            items: vec![
                (", / .", s.sc_comma_dot),
                ("PgUp / PgDn", s.sc_pgupdn),
                ("[ / ]", s.sc_brackets),
                ("Shift+T", s.sc_shift_t),
                ("Shift+M", s.sc_shift_m),
            ],
        },
        ShortcutGroup {
            title: "Selection",
            items: vec![
                ("Shift+Y", s.sc_shift_y),
                ("Arrows / hjkl", s.sc_arrows),
                ("Shift+Arrows", s.sc_shift_arrows),
            ],
        },
        ShortcutGroup {
            title: "Command palette",
            items: vec![
                (":", s.sc_colon),
                (":grep", "search text in visible files"),
                (":marks", "open saved bookmarks"),
                (":q", s.sc_q),
                (":@commands", "show all commands"),
            ],
        },
        ShortcutGroup {
            title: "HTTP",
            items: vec![
                ("Shift+S", "send the open request"),
                ("Tab", "tree → request → response"),
                ("[ / ]  h / l", "response tabs"),
                ("y", "copy the response body"),
                ("Esc", "abandon the request in flight"),
                (":http", "send, env, new, init, import"),
                (":httpenv", "choose the environment"),
            ],
        },
        ShortcutGroup {
            title: "Help",
            items: vec![("?", s.sc_question)],
        },
    ]
}

fn shortcut_group_lines(group: &ShortcutGroup) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![Span::styled(
            group.title,
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];
    lines.extend(
        group
            .items
            .iter()
            .map(|(keys, description)| shortcut_line(keys, description)),
    );
    lines
}

fn shortcut_line(keys: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{keys:<18}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description.to_string(), Style::default().fg(Color::Gray)),
    ])
}

fn settings_lines(app: &App, s: &'static Strings) -> Vec<Line<'static>> {
    let only_mds_toggle = if app.config.only_mds { "ON" } else { "OFF" };
    let only_mds_style = if app.config.only_mds {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let editor_value = app.config.editor.clone();
    let language_value = app.config.language.clone();
    let val_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let cursor = |i: usize| {
        if app.settings_cursor == i {
            "▶ "
        } else {
            "  "
        }
    };

    vec![
        Line::from(vec![Span::styled(
            s.settings_title,
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::raw(cursor(0)),
            Span::styled(
                s.settings_only_mds_label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(only_mds_toggle, only_mds_style),
        ]),
        Line::from(vec![
            Span::raw(cursor(1)),
            Span::styled(
                s.settings_editor_label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(editor_value, val_style),
        ]),
        Line::from(vec![
            Span::raw(cursor(2)),
            Span::styled(
                s.settings_language_label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(language_value, val_style),
        ]),
        Line::from(vec![
            Span::raw(cursor(3)),
            Span::styled(
                s.settings_bookmarks_label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if app.config.show_bookmarks {
                    "ON"
                } else {
                    "OFF"
                },
                if app.config.show_bookmarks {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                },
            ),
        ]),
        Line::from(""),
        Line::from(s.settings_only_mds_desc),
        Line::from(s.settings_editor_desc),
        Line::from(s.settings_language_desc),
        Line::from(""),
        Line::from(s.settings_stored),
        Line::from(format!(
            "{}{}",
            s.settings_config_path,
            config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "unavailable".to_string())
        )),
    ]
}

fn border_style(is_focused: bool) -> Style {
    if is_focused {
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn styled_preview_line(
    line: &PreviewLine,
    selection: Option<crate::app::SelectionState>,
    line_index: usize,
    active_link_line: Option<usize>,
    active_find_query: &str,
) -> Line<'static> {
    let mut base_style = preview_line_style(&line.kind);

    if active_link_line == Some(line_index) {
        base_style = base_style.bg(Color::DarkGray);
    }

    // syntax highlighted code line
    if !line.highlights.is_empty() {
        let spans: Vec<Span> = line
            .highlights
            .iter()
            .map(|(text, rgb)| {
                Span::styled(
                    text.clone(),
                    Style::default().fg(Color::Rgb(rgb[0], rgb[1], rgb[2])),
                )
            })
            .collect();
        return Line::from(spans);
    }

    let selected_range = selection_range_for_line(selection, line_index, line.text.chars().count());

    match line.kind {
        PreviewLineKind::MermaidTitle => {
            if let Some((title, hint)) = line.text.split_once("    ") {
                styled_find_and_selection(
                    &format!("{title}    {hint}"),
                    base_style,
                    selected_range,
                    active_find_query,
                )
            } else {
                styled_find_and_selection(&line.text, base_style, selected_range, active_find_query)
            }
        }
        _ => styled_find_and_selection(&line.text, base_style, selected_range, active_find_query),
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
    if !selection.anchored || selection.anchor == selection.cursor {
        return None;
    }

    let (start, end) = normalized_selection_bounds(selection.anchor, selection.cursor);
    if line_index < start.line || line_index > end.line {
        return None;
    }

    let start_col = if line_index == start.line {
        start.column
    } else {
        0
    };
    let end_col = if line_index == end.line {
        end.column
    } else {
        line_len
    };

    if start_col == end_col {
        None
    } else {
        Some((start_col.min(line_len), end_col.min(line_len)))
    }
}

fn normalized_selection_bounds(
    left: PreviewCursor,
    right: PreviewCursor,
) -> (PreviewCursor, PreviewCursor) {
    if (left.line, left.column) <= (right.line, right.column) {
        (left, right)
    } else {
        (right, left)
    }
}

fn styled_selected_text(
    text: &str,
    base: Style,
    selected: Option<(usize, usize)>,
) -> Line<'static> {
    let Some((start, end)) = selected else {
        return Line::from(Span::styled(text.to_string(), base));
    };

    let chars = text.chars().collect::<Vec<_>>();
    let prefix = chars.iter().take(start).collect::<String>();
    let middle = chars
        .iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect::<String>();
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

fn styled_find_and_selection(
    text: &str,
    base: Style,
    selected_range: Option<(usize, usize)>,
    active_find_query: &str,
) -> Line<'static> {
    if selected_range.is_some() || active_find_query.trim().is_empty() {
        return styled_selected_text(text, base, selected_range);
    }

    let lower_query = active_find_query.to_lowercase();
    let matches = find_match_chars(text, &lower_query);
    if matches.is_empty() {
        return Line::from(Span::styled(text.to_string(), base));
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut cursor = 0;
    for (start_char, len_chars) in matches {
        if start_char > cursor {
            spans.push(Span::styled(
                chars[cursor..start_char].iter().collect::<String>(),
                base,
            ));
        }
        let end_char = start_char + len_chars;
        spans.push(Span::styled(
            chars[start_char..end_char].iter().collect::<String>(),
            base.bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ));
        cursor = end_char;
    }
    if cursor < chars.len() {
        spans.push(Span::styled(
            chars[cursor..].iter().collect::<String>(),
            base,
        ));
    }
    Line::from(spans)
}

fn find_match_chars(text: &str, lower_query: &str) -> Vec<(usize, usize)> {
    let query_len = lower_query.chars().count();
    if query_len == 0 {
        return Vec::new();
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut matches = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let candidate = chars
            .iter()
            .skip(start)
            .take(query_len)
            .collect::<String>()
            .to_lowercase();
        if candidate == lower_query {
            matches.push((start, query_len));
            start += query_len;
        } else {
            start += 1;
        }
    }
    matches
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

fn tree_window(
    total_items: usize,
    selected_index: usize,
    visible_height: usize,
) -> (usize, usize, usize) {
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

fn render_toc_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let popup_area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, popup_area);

    let items = app
        .toc_entries
        .iter()
        .map(|(_, text)| {
            // Indent by heading level (# count at start)
            let level = text.chars().take_while(|&c| c == '#').count();
            let label = text.trim_start_matches('#').trim();
            let indent = "  ".repeat(level.saturating_sub(1));
            ListItem::new(Line::from(format!("{indent}{label}")))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .title(s.toc_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightGreen)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    state.select(Some(app.toc_cursor));
    frame.render_stateful_widget(list, popup_area, &mut state);
}

fn render_search_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let popup_area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(popup_area);

    // Input box
    let input = Paragraph::new(format!("/{}", app.search_query))
        .block(
            Block::default()
                .title(s.search_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(input, sections[0]);

    // Results list
    let items = if app.search_results.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if app.search_query.is_empty() {
                "Type to search files..."
            } else {
                s.search_no_results
            },
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.search_results
            .iter()
            .filter_map(|&i| app.items.get(i))
            .map(|item| {
                let indent = "  ".repeat(item.depth);
                let marker = if item.is_dir { ">" } else { "-" };
                ListItem::new(Line::from(format!("{indent}{marker} {}", item.name)))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(s.search_results_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.search_results.is_empty() {
        state.select(Some(app.search_cursor));
    }
    frame.render_stateful_widget(list, sections[1], &mut state);
}

fn render_grep_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(76, 68, frame.area());
    frame.render_widget(Clear, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(popup_area);

    let input = Paragraph::new(format!("grep {}", app.grep_query))
        .block(
            Block::default()
                .title("Grep visible files  (Esc close | Enter jump)")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(input, sections[0]);

    let items: Vec<ListItem> = if app.grep_results.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if app.grep_query.is_empty() {
                "Type to search all visible files..."
            } else {
                "No results"
            },
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.grep_results
            .iter()
            .map(|result| {
                let label = result
                    .path
                    .strip_prefix(&app.root)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| result.path.display().to_string());
                ListItem::new(Line::from(vec![
                    Span::styled(label, Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!(":{}  ", result.line_index + 1),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(result.line_text.chars().take(90).collect::<String>()),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!("{} result(s)", app.grep_results.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.grep_results.is_empty() {
        state.select(Some(app.grep_cursor));
    }
    frame.render_stateful_widget(list, sections[1], &mut state);
}

fn render_dest_picker_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let popup_area = centered_rect(78, 62, frame.area());
    frame.render_widget(Clear, popup_area);

    // Tree on the left, contents of the highlighted destination on the right.
    // Narrow terminals keep the tree only: a 20-column preview is useless.
    let panes = if popup_area.width >= 60 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(popup_area)
            .to_vec()
    } else {
        vec![popup_area]
    };

    let title = if app.file_op_kind == Some(FileOpKind::Copy) {
        s.dest_copy_title
    } else {
        s.dest_move_title
    };

    // Collapsible directory tree
    let items = app
        .picker_dirs
        .iter()
        .map(|item| {
            let indent = "  ".repeat(item.depth);
            let marker = if app.picker_expanded.contains(&item.path) {
                "▾"
            } else {
                "▸"
            };
            ListItem::new(Line::from(format!("{indent}{marker} {}", item.name)))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.picker_dirs.is_empty() {
        state.select(Some(app.picker_cursor));
    }
    frame.render_stateful_widget(list, panes[0], &mut state);

    if let Some(&preview_area) = panes.get(1) {
        render_dest_picker_preview(frame, app, preview_area);
    }
}

fn render_dest_picker_preview(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" contenido ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    if let Some(error) = &app.picker_preview_error {
        let paragraph = Paragraph::new(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::Red),
        )))
        .wrap(Wrap { trim: false })
        .block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    if app.picker_preview.is_empty() {
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "(vacia)",
            Style::default().fg(Color::DarkGray),
        )))
        .block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    let mut lines: Vec<Line> = app
        .picker_preview
        .iter()
        .map(|entry| {
            let marker = if entry.is_dir { "▸ " } else { "  " };
            let style = if entry.collides {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if entry.is_dir {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray)
            };
            let suffix = if entry.collides {
                "  ← ya existe"
            } else {
                ""
            };
            Line::from(Span::styled(
                format!("{marker}{}{suffix}", entry.name),
                style,
            ))
        })
        .collect();

    let hidden = app
        .picker_preview_total
        .saturating_sub(app.picker_preview.len());
    if hidden > 0 {
        lines.push(Line::from(Span::styled(
            format!("… {hidden} mas"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_command_palette(frame: &mut Frame, app: &App) {
    if app.palette_show_all {
        render_command_catalog(frame, app);
        return;
    }

    let filtered = app.palette_filtered();
    if filtered.is_empty() {
        return;
    }

    let total_area = frame.area();
    let list_height = (filtered.len() as u16 + 2).min(total_area.height.saturating_sub(2));
    let list_width = 50u16.min(total_area.width);
    let list_area = Rect {
        x: 0,
        y: total_area.height.saturating_sub(list_height + 1),
        width: list_width,
        height: list_height,
    };

    frame.render_widget(Clear, list_area);

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|(name, desc)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<10}", name),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {desc}"), Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(if app.palette_query.is_empty() {
                    " recientes "
                } else {
                    " sugerencias "
                })
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !filtered.is_empty() {
        state.select(Some(app.palette_cursor));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
}

struct CommandCatalogRow {
    command_index: Option<usize>,
    title: &'static str,
    description: &'static str,
}

fn render_command_catalog(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_area = Rect {
        x: area.x.saturating_add(2),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(4).max(1),
        height: area.height.saturating_sub(3).max(1),
    };
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Command palette ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    frame.render_widget(block, popup_area);

    let inner = Rect {
        x: popup_area.x.saturating_add(2),
        y: popup_area.y.saturating_add(1),
        width: popup_area.width.saturating_sub(4),
        height: popup_area.height.saturating_sub(2),
    };

    let header = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 2,
    };
    let header_line = Line::from(vec![
        Span::styled(
            "Todos los comandos",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "   Up/Down choose | Enter run | Esc close",
            Style::default().fg(Color::Gray),
        ),
    ]);
    frame.render_widget(Paragraph::new(header_line), header);

    let content = Rect {
        x: inner.x,
        y: inner.y.saturating_add(2),
        width: inner.width,
        height: inner.height.saturating_sub(2),
    };

    let panes = if content.width >= 96 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content)
            .to_vec()
    } else {
        vec![content]
    };

    let mut columns = (0..panes.len()).map(|_| Vec::new()).collect::<Vec<_>>();
    let mut command_index = 0;
    for (group_index, (group, commands)) in app.palette_command_groups().into_iter().enumerate() {
        let column_index = if panes.len() > 1 { group_index % 2 } else { 0 };
        if !columns[column_index].is_empty() {
            columns[column_index].push(CommandCatalogRow {
                command_index: None,
                title: "",
                description: "",
            });
        }
        columns[column_index].push(CommandCatalogRow {
            command_index: None,
            title: group,
            description: "",
        });
        for (name, description) in commands {
            columns[column_index].push(CommandCatalogRow {
                command_index: Some(command_index),
                title: name,
                description,
            });
            command_index += 1;
        }
    }

    for (index, pane) in panes.iter().enumerate() {
        render_command_catalog_column(frame, *pane, app, &columns[index]);
    }
}

fn render_command_catalog_column(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    rows: &[CommandCatalogRow],
) {
    let height = area.height as usize;
    let selected_row = rows
        .iter()
        .enumerate()
        .find_map(|(index, row)| (row.command_index == Some(app.palette_cursor)).then_some(index))
        .unwrap_or(0);
    let start = selected_row.saturating_sub(height.saturating_sub(1));

    let lines = rows
        .iter()
        .skip(start)
        .take(height)
        .map(|row| {
            let Some(command_index) = row.command_index else {
                if row.title.is_empty() {
                    return Line::from("");
                }
                return Line::from(Span::styled(
                    row.title,
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ));
            };

            let selected = command_index == app.palette_cursor;
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let name_style = if selected {
                style
            } else {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            };

            Line::from(vec![
                Span::styled(if selected { "> " } else { "  " }, style),
                Span::styled(format!("{:<12}", row.title), name_style),
                Span::styled(format!(" {}", row.description), style),
            ])
        })
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_recent_files_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(68, 55, frame.area());
    frame.render_widget(Clear, popup_area);

    let items = app
        .config
        .recent_files
        .iter()
        .map(|path| {
            let path_buf = std::path::PathBuf::from(path);
            let label = path_buf
                .strip_prefix(&app.root)
                .map(|relative| relative.display().to_string())
                .unwrap_or_else(|_| path.clone());
            ListItem::new(Line::from(vec![
                Span::styled("M ", Style::default().fg(Color::Yellow)),
                Span::raw(label),
            ]))
        })
        .collect::<Vec<_>>();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" recent files  Enter=open | Esc=close ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.config.recent_files.is_empty() {
        state.select(Some(app.recent_file_cursor));
    }
    frame.render_stateful_widget(list, popup_area, &mut state);
}

fn render_marks_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(72, 62, frame.area());
    frame.render_widget(Clear, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(4)])
        .split(popup_area);

    let input = Paragraph::new(format!("marks {}", app.marks_query))
        .block(
            Block::default()
                .title(" bookmarks  Enter=open | Esc=close ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(input, sections[0]);

    let items = if app.marks_results.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if app.marks_query.is_empty() {
                "No bookmarks yet"
            } else {
                "No matching bookmarks"
            },
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.marks_results
            .iter()
            .map(|mark| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<18}", mark.label),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        mark.path.display().to_string(),
                        Style::default().fg(Color::Gray),
                    ),
                ]))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" {} mark(s) ", app.marks_results.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.marks_results.is_empty() {
        state.select(Some(app.marks_cursor));
    }
    frame.render_stateful_widget(list, sections[1], &mut state);
}

fn render_alert_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(60, 30, frame.area());
    frame.render_widget(Clear, popup_area);

    let lines: Vec<Line> = app
        .alert_message
        .lines()
        .map(|line| {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::White),
            ))
        })
        .chain(std::iter::once(Line::from(Span::styled(
            "Esc / Enter para cerrar",
            Style::default().fg(Color::DarkGray),
        ))))
        .collect();

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .title(" Aviso ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
    );

    frame.render_widget(paragraph, popup_area);
}

fn render_find_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let popup_area = centered_rect(65, 65, frame.area());
    frame.render_widget(Clear, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(popup_area);

    let input = Paragraph::new(format!("/{}", app.find_query))
        .block(
            Block::default()
                .title(s.find_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(input, sections[0]);

    let items: Vec<ListItem> = if app.find_results.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if app.find_query.is_empty() {
                s.find_placeholder
            } else {
                s.find_no_results
            },
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.find_results
            .iter()
            .filter_map(|&i| app.preview.lines.get(i).map(|l| (i, l)))
            .map(|(i, line)| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:>4} ", i + 1),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(line.text.chars().take(80).collect::<String>()),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    "{} {}",
                    app.find_results.len(),
                    s.find_results_suffix
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.find_results.is_empty() {
        state.select(Some(app.find_cursor));
    }
    frame.render_stateful_widget(list, sections[1], &mut state);
}

fn render_create_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let popup_area = centered_rect(50, 40, frame.area());
    frame.render_widget(Clear, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Length(3)])
        .split(popup_area);

    // Kind chooser
    let kind_items: Vec<ListItem> = vec![
        ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                s.create_folder,
                if app.create_kind == CreateKind::Folder {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
        ])),
        ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                s.create_file,
                if app.create_kind == CreateKind::File {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
        ])),
    ];

    let kind_block_style = if app.create_step == CreateStep::ChooseKind {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let kind_list = List::new(kind_items)
        .block(
            Block::default()
                .title(s.create_type_title)
                .borders(Borders::ALL)
                .border_style(kind_block_style),
        )
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));

    let mut kind_state = ListState::default();
    kind_state.select(Some(match app.create_kind {
        CreateKind::Folder => 0,
        CreateKind::File => 1,
    }));
    frame.render_stateful_widget(kind_list, sections[0], &mut kind_state);

    // Name input
    let name_style = if app.create_step == CreateStep::EnterName {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let prompt = match app.create_kind {
        CreateKind::Folder => s.create_folder_name,
        CreateKind::File => s.create_file_name,
    };
    let name_input = Paragraph::new(format!("{}_", app.create_name))
        .block(
            Block::default()
                .title(format!("{prompt}  ({}))", s.create_enter))
                .borders(Borders::ALL)
                .border_style(name_style),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(name_input, sections[1]);
}

fn render_git_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let popup_area = centered_rect(65, 70, frame.area());
    frame.render_widget(Clear, popup_area);

    match app.git_state {
        GitState::CommandList => {
            let sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(popup_area);

            let cmds = App::git_commands();
            let items: Vec<ListItem> = cmds
                .iter()
                .map(|(name, desc, _)| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{:<12}", name),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("  {desc}"), Style::default().fg(Color::Gray)),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .title(s.git_title)
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow)),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::Yellow)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");

            let mut state = ListState::default();
            state.select(Some(app.git_cursor));
            frame.render_stateful_widget(list, sections[0], &mut state);

            let hint = Paragraph::new(Line::from(Span::styled(
                s.git_hint,
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(hint, sections[1]);
        }
        GitState::Output => {
            let sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(popup_area);

            let visible_height = sections[0].height.saturating_sub(2) as usize;
            let start = app.git_output_scroll;
            let end = (start + visible_height).min(app.git_output.len());
            let visible_lines: Vec<ListItem> = app.git_output[start..end]
                .iter()
                .map(|line| ListItem::new(Line::from(Span::raw(line.clone()))))
                .collect();

            let total = app.git_output.len();
            let list = List::new(visible_lines).block(
                Block::default()
                    .title(format!(
                        "Salida  ({}/{}  {})",
                        start + 1,
                        total,
                        s.git_output_suffix
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            );
            frame.render_widget(list, sections[0]);

            let hint = Paragraph::new(Line::from(Span::styled(
                s.git_output_hint,
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(hint, sections[1]);
        }
        GitState::CommitInput => {
            let sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(2)])
                .split(popup_area);

            let input = Paragraph::new(format!("{}_", app.git_commit_input))
                .block(
                    Block::default()
                        .title(s.git_commit_title)
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .style(Style::default().fg(Color::White));
            frame.render_widget(input, sections[0]);

            let hint = Paragraph::new(Line::from(Span::styled(
                s.git_commit_hint,
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(hint, sections[1]);
        }
    }
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
