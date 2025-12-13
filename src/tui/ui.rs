//! TUI rendering

use super::app::{App, SortMode};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

const TEMP_COLOR: Color = Color::Red;
const DIR_COLOR: Color = Color::Blue;
const FILE_COLOR: Color = Color::White;

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(5),    // List
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    render_header(f, app, chunks[0]);
    render_list(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let path_str = app.current_path.to_string_lossy();
    let total_size = humansize::format_size(app.total_size, humansize::BINARY);
    let sort_str = match app.sort_mode {
        SortMode::Size => "size",
        SortMode::Name => "name",
    };

    let header = Paragraph::new(format!(
        " {} │ Total: {} │ Sort: {} │ {} items",
        path_str,
        total_size,
        sort_str,
        app.entries.len()
    ))
    .block(Block::default().borders(Borders::ALL).title(" Cleaner "));

    f.render_widget(header, area);
}

fn render_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let size_str = humansize::format_size(entry.size, humansize::BINARY);
            let prefix = if entry.is_dir { "▸ " } else { "  " };
            let temp_marker = if entry.is_temp { " [TEMP]" } else { "" };

            let text = format!(
                "{}{:<40} {:>10}{}",
                prefix, entry.name, size_str, temp_marker
            );

            let style = if i == app.selected {
                Style::default().bg(Color::DarkGray).bold()
            } else if entry.is_temp {
                Style::default().fg(TEMP_COLOR)
            } else if entry.is_dir {
                Style::default().fg(DIR_COLOR)
            } else {
                Style::default().fg(FILE_COLOR)
            };

            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::DarkGray));

    let mut state = ListState::default();
    state.select(Some(app.selected));

    f.render_stateful_widget(list, area, &mut state);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let text = if app.is_cleaning() {
        " ⏳ Cleaning... please wait".to_string()
    } else if app.is_deleting() {
        " ⏳ Deleting... please wait".to_string()
    } else if app.confirm_clean {
        format!(
            " Clean all temp files in '{}'? (y/n)",
            app.current_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| app.current_path.to_string_lossy().to_string())
        )
    } else if app.confirm_delete {
        if let Some(entry) = app.selected_entry() {
            format!(
                " Delete '{}'? (y/n) - {} will be freed",
                entry.name,
                humansize::format_size(entry.size, humansize::BINARY)
            )
        } else {
            " Delete? (y/n)".to_string()
        }
    } else if let Some(ref msg) = app.status_message {
        format!(" {} │ c:clean  d:delete  s:sort  r:refresh  q:quit", msg)
    } else {
        " ↑↓:nav  Enter:open  ←:back  c:clean  d:delete  s:sort  r:refresh  q:quit".to_string()
    };

    let style = if app.confirm_delete || app.confirm_clean {
        Style::default().fg(Color::Yellow).bold()
    } else {
        Style::default()
    };

    let footer = Paragraph::new(text)
        .style(style)
        .block(Block::default().borders(Borders::ALL));

    f.render_widget(footer, area);
}
