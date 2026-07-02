use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    layout::Position,
    Frame,
};

use crate::{
    app::{App, CreateKind, CreateStep, FileOpKind, Focus, FullscreenPanel, GitState, GitStatusKind, HelpSection, Overlay, PreviewCursor},
    config::{config_path, TreeInfoMode},
    strings::Strings,
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

    let s = app.config.strings();
    let footer = if app.overlay == Overlay::Rename {
        Paragraph::new(Line::from(vec![
            Span::styled("Rename: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(app.rename_input.clone(), Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Yellow)),
            Span::styled("  Enter=confirmar  Esc=cancelar", Style::default().fg(Color::DarkGray)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.overlay == Overlay::CommandPalette {
        Paragraph::new(Line::from(vec![
            Span::styled(":", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(app.palette_query.clone(), Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.selection.as_ref().map(|s| s.anchored).unwrap_or(false) {
        let (label, hint, color) = if app.status.starts_with("Copiado!") {
            (app.status.clone(), String::new(), Color::Green)
        } else {
            (s.select_on.to_string(), s.select_copy_hint.to_string(), Color::Green)
        };
        Paragraph::new(Line::from(vec![
            Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(hint, Style::default().fg(Color::Gray)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.pending_delete.is_some() {
        Paragraph::new(Line::from(vec![
            Span::styled(app.status.clone(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.pending_go_up {
        Paragraph::new(Line::from(vec![
            Span::styled("Go up?  ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("← de nuevo para subir un nivel  Esc=cancelar", Style::default().fg(Color::Gray)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else if app.selection.is_some() {
        Paragraph::new(Line::from(vec![
            Span::styled(s.select_cursor.to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(s.select_anchor_hint.to_string(), Style::default().fg(Color::Gray)),
        ]))
        .block(Block::default().borders(Borders::TOP))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled(s.help_hint.to_string(), Style::default().fg(Color::Yellow)),
            footer_item_span(app),
            pending_cd_span(app),
        ]))
        .block(Block::default().borders(Borders::TOP))
    };

    frame.render_widget(footer, areas[1]);

    match app.overlay {
        Overlay::Help => render_help_popup(frame, app),
        Overlay::MermaidSelect => render_mermaid_select_popup(frame, app),
        Overlay::MermaidOutput => render_mermaid_output_popup(frame, app),
        Overlay::MermaidTerminalView => render_mermaid_terminal_view(frame, app),
        Overlay::WebLink => render_web_link_popup(frame, app),
        Overlay::Search => render_search_popup(frame, app),
        Overlay::DestPicker => render_dest_picker_popup(frame, app),
        Overlay::Toc => render_toc_popup(frame, app),
        Overlay::CommandPalette => render_command_palette(frame, app),
        Overlay::Find => render_find_popup(frame, app),
        Overlay::Create => render_create_popup(frame, app),
        Overlay::Git => render_git_popup(frame, app),
        Overlay::Rename => {}
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
                let selector = if app.selector_path.as_ref() == Some(&item.path) { "*" } else { " " };
                let marker = if item.is_dir { ">" } else { "-" };
                result.push(ListItem::new(Line::from(vec![
                    Span::raw(format!("{selector}")),
                    Span::styled(
                        format!("{marker} {}", item.name),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                ])));
            } else {
                let indent = "  ".repeat(item.depth);
                let marker = if item.is_dir {
                    if app.expanded_dirs.contains(&item.path) { "v" } else { ">" }
                } else { "-" };
                let selector = if app.selector_path.as_ref() == Some(&item.path) { "*" } else { " " };
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
        Some(GitStatusKind::Conflicted) => ("U ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        None => ("  ", Style::default()),
    }
}

fn render_preview(frame: &mut Frame, area: Rect, app: &App) {
    let s = app.config.strings();
    let title = if app.fullscreen == FullscreenPanel::Preview {
        if app.selection.is_some() {
            s.preview_select
        } else {
            s.preview_fullscreen
        }
    } else if app.focus == Focus::Preview {
        s.preview_focus
    } else {
        s.preview_title
    };

    let lines = if app.preview.lines.is_empty() {
        vec![Line::from(s.preview_empty)]
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
                styled_preview_line(line, app.selection, index, active_link_line)
            })
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

    let normal_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
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
                    let node_col_end =
                        (node.x + node.width).saturating_sub(app.mermaid_canvas_x);

                    if node_col_start < inner_width {
                        let before: String = chars[..node_col_start.min(chars.len())].iter().collect();
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
    let title = format!("{}{}",s.mermaid_view_title, node_hint);

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
    let popup_area = centered_rect(68, 56, frame.area());
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!("Help  v{}", env!("CARGO_PKG_VERSION")))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(2)])
        .split(inner);

    render_help_tabs(frame, sections[0], app.help_section, s);

    let lines = match app.help_section {
        HelpSection::Shortcuts => shortcut_lines(s),
        HelpSection::Settings => settings_lines(app, s),
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, sections[1]);

    let s = app.config.strings();
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
        .constraints([Constraint::Length(18), Constraint::Length(18), Constraint::Min(1)])
        .split(area);

    frame.render_widget(help_tab(s.tab_shortcuts, selected == HelpSection::Shortcuts), tabs[0]);
    frame.render_widget(help_tab(s.tab_settings, selected == HelpSection::Settings), tabs[1]);
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

fn shortcut_lines(s: &'static Strings) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![Span::styled(s.nav_section, Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        shortcut_line("Enter", s.sc_enter),
        shortcut_line("Tab / Shift+Tab", s.sc_tab),
        shortcut_line("Shift+Y", s.sc_shift_y),
        shortcut_line("Shift+E", s.sc_shift_e),
        shortcut_line("Shift+R", s.sc_shift_r),
        shortcut_line("Shift+B", s.sc_shift_b),
        shortcut_line("Ctrl+Shift+C", s.sc_ctrl_shift_c),
        shortcut_line("Shift+G", s.sc_shift_g),
        shortcut_line("Shift+0", s.sc_shift_0),
        shortcut_line("Shift+1..5", s.sc_shift_1_5),
        shortcut_line("q", s.sc_q),
        Line::from(""),
        Line::from(vec![Span::styled(s.preview_section, Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        shortcut_line(", / .", s.sc_comma_dot),
        shortcut_line("PgUp / PgDn", s.sc_pgupdn),
        shortcut_line("Arrows / hjkl", s.sc_arrows),
        shortcut_line("Shift+Arrows", s.sc_shift_arrows),
        shortcut_line(":", s.sc_colon),
        shortcut_line("Shift+T", s.sc_shift_t),
        shortcut_line("[ / ]", s.sc_brackets),
        shortcut_line("Shift+M", s.sc_shift_m),
        shortcut_line("?", s.sc_question),
    ]
}

fn shortcut_line(keys: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{keys:<18}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(description.to_string(), Style::default().fg(Color::Gray)),
    ])
}

fn settings_lines(app: &App, s: &'static Strings) -> Vec<Line<'static>> {
    let only_mds_toggle = if app.config.only_mds { "ON" } else { "OFF" };
    let only_mds_style = if app.config.only_mds {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };
    let editor_value = app.config.editor.clone();
    let language_value = app.config.language.clone();
    let val_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);

    let cursor = |i: usize| if app.settings_cursor == i { "▶ " } else { "  " };

    vec![
        Line::from(vec![Span::styled(s.settings_title, Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD))]),
        Line::from(""),
        Line::from(vec![
            Span::raw(cursor(0)),
            Span::styled(s.settings_only_mds_label, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(only_mds_toggle, only_mds_style),
        ]),
        Line::from(vec![
            Span::raw(cursor(1)),
            Span::styled(s.settings_editor_label, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(editor_value, val_style),
        ]),
        Line::from(vec![
            Span::raw(cursor(2)),
            Span::styled(s.settings_language_label, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(language_value, val_style),
        ]),
        Line::from(vec![
            Span::raw(cursor(3)),
            Span::styled(s.settings_bookmarks_label, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(
                if app.config.show_bookmarks { "ON" } else { "OFF" },
                if app.config.show_bookmarks {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
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
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

fn styled_preview_line(
    line: &PreviewLine,
    selection: Option<crate::app::SelectionState>,
    line_index: usize,
    active_link_line: Option<usize>,
) -> Line<'static> {
    let mut base_style = preview_line_style(&line.kind);

    if active_link_line == Some(line_index) {
        base_style = base_style.bg(Color::DarkGray);
    }

    // syntax highlighted code line
    if !line.highlights.is_empty() {
        let spans: Vec<Span> = line.highlights.iter()
            .map(|(text, rgb)| Span::styled(
                text.clone(),
                Style::default().fg(Color::Rgb(rgb[0], rgb[1], rgb[2])),
            ))
            .collect();
        return Line::from(spans);
    }

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
            s.search_no_results,
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

fn render_dest_picker_popup(frame: &mut Frame, app: &App) {
    let s = app.config.strings();
    let popup_area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, popup_area);

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
    frame.render_stateful_widget(list, popup_area, &mut state);
}

fn render_command_palette(frame: &mut Frame, app: &App) {
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
                Span::styled(format!("{:<10}", name), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(format!("  {desc}"), Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !filtered.is_empty() {
        state.select(Some(app.palette_cursor));
    }
    frame.render_stateful_widget(list, list_area, &mut state);
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
            if app.find_query.is_empty() { s.find_placeholder } else { s.find_no_results },
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.find_results
            .iter()
            .filter_map(|&i| app.preview.lines.get(i).map(|l| (i, l)))
            .map(|(i, line)| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:>4} ", i + 1), Style::default().fg(Color::DarkGray)),
                    Span::raw(line.text.chars().take(80).collect::<String>()),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!("{} {}",  app.find_results.len(), s.find_results_suffix))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(Style::default().bg(Color::Green).fg(Color::Black).add_modifier(Modifier::BOLD))
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
            Span::styled(s.create_folder, if app.create_kind == CreateKind::Folder {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            }),
        ])),
        ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(s.create_file, if app.create_kind == CreateKind::File {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            }),
        ])),
    ];

    let kind_block_style = if app.create_step == CreateStep::ChooseKind {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let kind_list = List::new(kind_items)
        .block(Block::default().title(s.create_type_title).borders(Borders::ALL).border_style(kind_block_style))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black));

    let mut kind_state = ListState::default();
    kind_state.select(Some(match app.create_kind { CreateKind::Folder => 0, CreateKind::File => 1 }));
    frame.render_stateful_widget(kind_list, sections[0], &mut kind_state);

    // Name input
    let name_style = if app.create_step == CreateStep::EnterName {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let prompt = match app.create_kind { CreateKind::Folder => s.create_folder_name, CreateKind::File => s.create_file_name };
    let name_input = Paragraph::new(format!("{}_", app.create_name))
        .block(Block::default().title(format!("{prompt}  ({}))", s.create_enter)).borders(Borders::ALL).border_style(name_style))
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
                        Span::styled(format!("{:<12}", name), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("  {desc}"), Style::default().fg(Color::Gray)),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().title(s.git_title).borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)))
                .highlight_style(Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD))
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
            let list = List::new(visible_lines)
                .block(Block::default()
                    .title(format!("Salida  ({}/{}  {})", start + 1, total, s.git_output_suffix))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)));
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
                .block(Block::default()
                    .title(s.git_commit_title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)))
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
