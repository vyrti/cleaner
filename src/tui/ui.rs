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

    let disk_info = if app.disk_total > 0 {
        let disk_used = app.disk_total.saturating_sub(app.disk_free);
        let used_str = humansize::format_size(disk_used, humansize::BINARY);
        let free_str = humansize::format_size(app.disk_free, humansize::BINARY);
        let free_pct = (app.disk_free as f64 / app.disk_total as f64) * 100.0;
        format!(
            " │ Disk Used: {} │ Free: {} ({:.1}%)",
            used_str, free_str, free_pct
        )
    } else {
        "".to_string()
    };

    let header = Paragraph::new(format!(
        " {} │ Folder: {} │ Sort: {}{} │ {} items",
        path_str,
        total_size,
        sort_str,
        disk_info,
        app.entries.len()
    ))
    .block(Block::default().borders(Borders::ALL).title(" Cleaner "));

    f.render_widget(header, area);
}

fn render_list(f: &mut Frame, app: &App, area: Rect) {
    let visible_rows = usize::from(area.height.saturating_sub(2)).max(1);
    let start = app.selected.saturating_add(1).saturating_sub(visible_rows);
    let end = start.saturating_add(visible_rows).min(app.entries.len());
    let items: Vec<ListItem> = app
        .entries
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(relative_index, entry)| {
            let index = start + relative_index;
            let size_str = humansize::format_size(entry.size, humansize::BINARY);
            let prefix = if entry.is_dir { "▸ " } else { "  " };
            let temp_marker = if entry.is_temp { " [TEMP]" } else { "" };

            let text = format!(
                "{}{:<40} {:>10}{}",
                prefix,
                entry.name.to_string_lossy(),
                size_str,
                temp_marker
            );

            let style = if index == app.selected {
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
    state.select((start < end).then_some(app.selected.saturating_sub(start)));

    f.render_stateful_widget(list, area, &mut state);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let text = if let Some((phase, current, total)) = app.rebuild_progress() {
        let stage = match phase {
            0 => "scanning",
            1 => "indexing",
            2 => "sizing",
            _ => "finalizing",
        };
        if total == 0 {
            format!(" ⏳ Rebuilding tree: {stage}...")
        } else {
            format!(" ⏳ Rebuilding tree: {stage} {current}/{total}")
        }
    } else if app.is_cleaning() {
        " ⏳ Cleaning... please wait".to_string()
    } else if app.is_deleting() {
        " ⏳ Deleting... please wait".to_string()
    } else if app.confirm_clean {
        let (dirs, files, bytes) = app.current_temp_stats();
        let size_str = humansize::format_size(bytes, humansize::BINARY);
        format!(
            " Clean all temp files in '{}'? (y/n) - Will delete: {} folders, {} files, {} size",
            app.current_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| app.current_path.to_string_lossy().to_string()),
            dirs,
            files,
            size_str
        )
    } else if app.confirm_delete {
        if let Some(entry) = app.selected_entry() {
            format!(
                " Delete '{}'? (y/n) - {} will be freed",
                entry.name.to_string_lossy(),
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

    let text = if let Some(index) = &app.index_status {
        format!(" {index} │ {}", text.trim_start())
    } else {
        text
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::patterns::PatternMatcher;
    use crate::tui::tree::{DirEntry, DirTree};
    use foldhash::{HashMap, HashMapExt};
    use ratatui::{backend::TestBackend, Terminal};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn app() -> App {
        let root = PathBuf::from("test-root");
        let matcher = Arc::new(PatternMatcher::new(Arc::new(Config {
            directories: vec!["target".into()],
            files: vec![".pyc".into()],
            days: None,
            force: false,
        })));
        let mut children = HashMap::new();
        children.insert(
            root.clone(),
            vec![
                DirEntry {
                    name: "target".into(),
                    size: 4096,
                    is_dir: true,
                    is_temp: true,
                },
                DirEntry {
                    name: "main.rs".into(),
                    size: 20,
                    is_dir: false,
                    is_temp: false,
                },
            ],
        );
        App::new_with_tree(root, matcher, DirTree::from_children(children), false)
    }

    fn screen(app: &App) -> String {
        let backend = TestBackend::new(100, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn renders_header_entries_and_default_help() {
        let output = screen(&app());
        assert!(output.contains("Cleaner"));
        assert!(output.contains("Sort: size"));
        assert!(output.contains("target"));
        assert!(output.contains("[TEMP]"));
        assert!(output.contains("Enter:open"));
    }

    #[test]
    fn renders_delete_clean_and_status_footers() {
        let mut app = app();
        app.confirm_delete = true;
        assert!(screen(&app).contains("Delete 'target'?"));
        app.confirm_delete = false;
        app.confirm_clean = true;
        assert!(screen(&app).contains("Clean all temp files"));
        app.confirm_clean = false;
        app.status_message = Some("Refreshed".into());
        assert!(screen(&app).contains("Refreshed"));
    }

    #[test]
    fn renders_disk_information_and_name_sort() {
        let mut app = app();
        app.sort_mode = SortMode::Name;
        app.disk_total = 1000;
        app.disk_free = 250;
        let output = screen(&app);
        assert!(output.contains("Sort: name"));
        assert!(output.contains("25.0%"));
    }

    #[test]
    fn renders_only_rows_visible_near_selection() {
        let mut app = app();
        app.entries = Arc::new(
            (0..50)
                .map(|index| DirEntry {
                    name: format!("entry-{index:02}").into(),
                    size: index,
                    is_dir: false,
                    is_temp: false,
                })
                .collect(),
        );
        app.selected = 49;
        let output = screen(&app);
        assert!(output.contains("entry-49"));
        assert!(!output.contains("entry-00"));
    }
}
