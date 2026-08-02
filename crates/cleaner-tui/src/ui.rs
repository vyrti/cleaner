//! TUI rendering — Norton/Abyss palette and digit action bar.

use super::app::{App, SortMode};
use ratatui::{
    prelude::*,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

const CORE: Style = Style::new().fg(Color::Gray).bg(Color::Blue);
const HEADER: Style = Style::new().fg(Color::Yellow).bg(Color::Blue);
const SELECTED: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
const TEMP_STYLE: Style = Style::new().fg(Color::LightRed).bg(Color::Blue);
const DIR_STYLE: Style = Style::new().fg(Color::LightCyan).bg(Color::Blue);
const FILE_STYLE: Style = Style::new().fg(Color::Gray).bg(Color::Blue);
const CONFIRM: Style = Style::new().fg(Color::Yellow).bg(Color::Blue).add_modifier(Modifier::BOLD);

/// How much chrome to draw around the analyze content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Chrome {
    /// Standalone cleaner: header + list + status + digit bar.
    Full,
    /// Embedded in Abyss: header + list only (host owns status/buttons).
    ContentOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionButton {
    Help,
    Empty2,
    Empty3,
    Sort,
    Clean,
    Delete,
    Refresh,
    Empty8,
    Empty9,
    Quit,
}

impl ActionButton {
    pub fn key(self) -> char {
        match self {
            Self::Help => '1',
            Self::Empty2 => '2',
            Self::Empty3 => '3',
            Self::Sort => '4',
            Self::Clean => '5',
            Self::Delete => '6',
            Self::Refresh => '7',
            Self::Empty8 => '8',
            Self::Empty9 => '9',
            Self::Quit => '0',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Help => "Help",
            Self::Empty2 | Self::Empty3 | Self::Empty8 | Self::Empty9 => "",
            Self::Sort => "Sort",
            Self::Clean => "Clean",
            Self::Delete => "Delete",
            Self::Refresh => "Refresh",
            Self::Quit => "Quit",
        }
    }

    fn disabled(self) -> bool {
        matches!(
            self,
            Self::Empty2 | Self::Empty3 | Self::Empty8 | Self::Empty9
        )
    }
}

const BUTTONS: [ActionButton; 10] = [
    ActionButton::Help,
    ActionButton::Empty2,
    ActionButton::Empty3,
    ActionButton::Sort,
    ActionButton::Clean,
    ActionButton::Delete,
    ActionButton::Refresh,
    ActionButton::Empty8,
    ActionButton::Empty9,
    ActionButton::Quit,
];

#[allow(dead_code)]
pub fn render(f: &mut Frame, app: &App) {
    render_in(f, app, f.area(), Chrome::Full);
}

pub fn render_in(f: &mut Frame, app: &App, area: Rect, chrome: Chrome) {
    f.render_widget(Block::default().style(CORE), area);

    let (content, status_area, button_area) = match chrome {
        Chrome::Full => {
            let button_area = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
            let has_status = status_line(app).is_some();
            let status_area = has_status.then(|| {
                Rect::new(area.x, button_area.y.saturating_sub(1), area.width, 1)
            });
            let content_bottom = status_area.map(|r| r.y).unwrap_or(button_area.y);
            let content = Rect::new(
                area.x,
                area.y,
                area.width,
                content_bottom.saturating_sub(area.y),
            );
            (content, status_area, Some(button_area))
        }
        Chrome::ContentOnly => (area, None, None),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(content);

    render_header(f, app, chunks[0]);
    render_list(f, app, chunks[1]);

    if let Some(status_area) = status_area {
        render_status(f, app, status_area);
    }
    if let Some(button_area) = button_area {
        render_buttons(f, button_area);
    }
}

/// Status / confirm text for Abyss status line or standalone status row.
/// Returns `None` when idle — key hints belong on the button bar, not here.
pub fn status_line(app: &App) -> Option<String> {
    if let Some((phase, current, total)) = app.rebuild_progress() {
        let stage = match phase {
            0 => "scanning",
            1 => "indexing",
            2 => "sizing",
            _ => "finalizing",
        };
        return Some(if total == 0 {
            format!("Rebuilding tree: {stage}...")
        } else {
            format!("Rebuilding tree: {stage} {current}/{total}")
        });
    }
    if app.is_cleaning() {
        return Some("Cleaning... please wait".into());
    }
    if app.is_deleting() {
        return Some("Deleting... please wait".into());
    }
    if app.confirm_clean {
        let (dirs, files, bytes) = app.current_temp_stats();
        let size_str = humansize::format_size(bytes, humansize::BINARY);
        return Some(format!(
            "Clean all temp in '{}'? (y/n) — {} folders, {} files, {}",
            app.current_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| app.current_path.to_string_lossy().to_string()),
            dirs,
            files,
            size_str
        ));
    }
    if app.confirm_delete {
        return Some(if let Some(entry) = app.selected_entry() {
            format!(
                "Delete '{}'? (y/n) — {} will be freed",
                entry.name.to_string_lossy(),
                humansize::format_size(entry.size, humansize::BINARY)
            )
        } else {
            "Delete? (y/n)".into()
        });
    }
    if let Some(index) = &app.index_status {
        if let Some(msg) = &app.status_message {
            return Some(format!("{index} │ {msg}"));
        }
        return Some(index.clone());
    }
    app.status_message.clone()
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    // Path sits in the block title (middle-truncated); stats fill the inner row —
    // same Norton pattern as Abyss panes so long paths never steal Folder/Disk.
    let title_width = area.width.saturating_sub(4) as usize;
    let path = truncate_middle(&app.current_path.to_string_lossy(), title_width);
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
        format!(" │ Disk Used: {used_str} │ Free: {free_str} ({free_pct:.1}%)")
    } else {
        String::new()
    };

    let inner_width = area.width.saturating_sub(2) as usize;
    let stats = fit(
        &format!(
            " Folder: {total_size} │ Sort: {sort_str}{disk_info} │ {} items",
            app.entries.len()
        ),
        inner_width,
    );

    f.render_widget(Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(HEADER)
            .style(CORE)
            .title(Span::styled(format!(" {path} "), HEADER)),
        area,
    );
    if area.height >= 3 && area.width >= 2 {
        let body = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), 1);
        f.render_widget(Paragraph::new(stats).style(HEADER), body);
    }
}

fn render_list(f: &mut Frame, app: &App, area: Rect) {
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(CORE)
            .style(CORE),
        area,
    );

    let inner_width = area.width.saturating_sub(2);
    if inner_width == 0 || area.height < 3 {
        return;
    }

    let header_area = Rect::new(area.x + 1, area.y + 1, inner_width, 1);
    f.render_widget(
        Paragraph::new(columns("Name", "Size", "", inner_width)).style(HEADER),
        header_area,
    );

    // Borders + Name/Size header row.
    let visible_rows = usize::from(area.height.saturating_sub(3)).max(1);
    let start = app.selected.saturating_add(1).saturating_sub(visible_rows);
    let end = start.saturating_add(visible_rows).min(app.entries.len());

    for (row, entry) in app
        .entries
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let index = start + row;
        let size_str = humansize::format_size(entry.size, humansize::BINARY);
        let prefix = if entry.is_dir { "▸ " } else { "  " };
        let temp_marker = if entry.is_temp { " [TEMP]" } else { "" };
        let name = format!(
            "{prefix}{}{temp_marker}",
            entry.name.to_string_lossy()
        );
        let text = columns(&name, &size_str, "", inner_width);
        let style = if index == app.selected {
            SELECTED.add_modifier(Modifier::BOLD)
        } else if entry.is_temp {
            TEMP_STYLE
        } else if entry.is_dir {
            DIR_STYLE
        } else {
            FILE_STYLE
        };
        let row_area = Rect::new(area.x + 1, area.y + 2 + row as u16, inner_width, 1);
        f.render_widget(Paragraph::new(text).style(style), row_area);
    }
}

/// Name left, Size right; optional third column (unused, kept for Abyss-like layout).
fn columns(name: &str, size: &str, _extra: &str, width: u16) -> String {
    let width = width as usize;
    if width < 14 {
        return fit(name, width);
    }
    let size_width = 10;
    let name_width = width.saturating_sub(size_width + 1);
    format!(
        "{} {}",
        pad_right(&fit_filename(name, name_width), name_width),
        pad_left(size, size_width),
    )
}

fn fit(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if display_width(value) <= width {
        return value.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    format!("{}…", take_prefix_columns(value, width - 1))
}

fn fit_filename(value: &str, width: usize) -> String {
    truncate_middle(value, width)
}

fn display_width(value: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(value)
}

fn take_prefix_columns(value: &str, width: usize) -> String {
    let mut used = 0;
    let mut end = 0;
    for (idx, ch) in value.char_indices() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        end = idx + ch.len_utf8();
    }
    value[..end].to_owned()
}

fn take_suffix_columns(value: &str, width: usize) -> String {
    let mut used = 0;
    let mut start = value.len();
    for (idx, ch) in value.char_indices().rev() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        start = idx;
    }
    value[start..].to_owned()
}

fn pad_right(value: &str, width: usize) -> String {
    let value = fit(value, width);
    let padding = width.saturating_sub(display_width(&value));
    format!("{value}{}", " ".repeat(padding))
}

fn pad_left(value: &str, width: usize) -> String {
    let value = fit(value, width);
    let padding = width.saturating_sub(display_width(&value));
    format!("{}{value}", " ".repeat(padding))
}

fn truncate_middle(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_owned();
    }
    if width < 5 {
        return fit(value, width);
    }
    let left = (width - 1) / 2;
    let right = width - left - 1;
    let start = take_prefix_columns(value, left);
    let end = take_suffix_columns(value, right);
    format!("{start}…{end}")
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let text = status_line(app).unwrap_or_default();
    let style = if app.confirm_delete || app.confirm_clean {
        CONFIRM
    } else {
        CORE
    };
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(format!(" {text}")).style(style), area);
}

fn render_buttons(f: &mut Frame, area: Rect) {
    f.render_widget(
        Block::default().style(Style::new().fg(Color::Black).bg(Color::Cyan)),
        area,
    );
    let base = area.width / BUTTONS.len() as u16;
    let mut x = area.x;
    for (index, button) in BUTTONS.into_iter().enumerate() {
        let remaining = area.right().saturating_sub(x);
        let width = if index + 1 == BUTTONS.len() {
            remaining
        } else {
            base.min(remaining)
        };
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let disabled = button.disabled();
        let label = button.label();
        let content = Line::from(vec![
            Span::styled(
                button.key().to_string(),
                if disabled {
                    Style::new().fg(Color::DarkGray).bg(Color::Black)
                } else {
                    Style::new().fg(Color::White).bg(Color::Black)
                },
            ),
            Span::styled(
                pad_right(
                    &fit(label, width.saturating_sub(1) as usize),
                    width.saturating_sub(1) as usize,
                ),
                if disabled {
                    Style::new().fg(Color::DarkGray).bg(Color::Cyan)
                } else {
                    Style::new().fg(Color::Black).bg(Color::Cyan)
                },
            ),
        ]);
        f.render_widget(Paragraph::new(content), rect);
        x = x.saturating_add(width);
    }
}

pub fn draw_scan_progress(frame: &mut Frame, area: Rect, root: &std::path::Path, progress: &cleaner_core::tree::ScanProgress) {
    frame.render_widget(Block::default().style(CORE), area);
    let files = progress.get_files();
    let dirs = progress.get_dirs();
    let bytes = progress.get_bytes();
    let size_str = humansize::format_size(bytes, humansize::BINARY);
    let errors = progress.get_errors();
    let phase = progress.get_phase();
    let (stage_current, stage_total) = progress.get_stage_progress();
    let (phase_name, phase_title) = match phase {
        0 => ("Scanning filesystem", "Scanning"),
        1 => ("Indexing entries", "Indexing"),
        2 => ("Calculating folder sizes", "Sizing"),
        3 => ("Finalizing navigation", "Finalizing"),
        _ => ("Finalizing", "Finalizing"),
    };
    let stage_line = if phase == 0 || stage_total == 0 {
        String::new()
    } else {
        let percent = stage_current.saturating_mul(100) / stage_total;
        format!(
            "\n  Progress: {}/{} folders ({}%)",
            stage_current.min(stage_total),
            stage_total,
            percent.min(100)
        )
    };

    let text = format!(
        "\n\n  {}: {}{}\n\n  {} folders\n  {} files\n  {}\n  {} errors\n\n  Press q/Esc to cancel",
        phase_name,
        root.display(),
        stage_line,
        dirs,
        files,
        size_str,
        errors
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(HEADER)
        .style(CORE)
        .title(format!(" Cleaner - {phase_title} "))
        .title_style(HEADER);
    frame.render_widget(Paragraph::new(text).style(CORE).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleaner_core::config::Config;
    use cleaner_core::patterns::PatternMatcher;
    use cleaner_core::tree::{DirEntry, DirTree};
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
    fn renders_header_entries_and_digit_bar() {
        let output = screen(&app());
        assert!(output.contains("test-root"));
        assert!(output.contains("Folder:"));
        assert!(output.contains("Sort: size"));
        assert!(output.contains("Name"));
        assert!(output.contains("target"));
        assert!(output.contains("[TEMP]"));
        assert!(output.contains("Clean"));
        assert!(output.contains("Delete"));
        assert!(output.contains("Quit"));
    }

    #[test]
    fn renders_delete_clean_and_status() {
        let mut app = app();
        app.confirm_delete = true;
        assert!(screen(&app).contains("Delete 'target'?"));
        app.confirm_delete = false;
        app.confirm_clean = true;
        assert!(screen(&app).contains("Clean all temp"));
        app.confirm_clean = false;
        app.status_message = Some("Refreshed".into());
        assert!(screen(&app).contains("Refreshed"));
    }

    #[test]
    fn content_only_omits_digit_labels() {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = app();
        terminal
            .draw(|frame| render_in(frame, &app, frame.area(), Chrome::ContentOnly))
            .unwrap();
        let output: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(output.contains("target"));
        assert!(output.contains("test-root"));
        assert!(output.contains("Folder:"));
        // Digit bar Quit label should not appear as a dedicated bar in content-only.
        assert!(!output.contains("0Quit") && !output.contains("0 Exit"));
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
