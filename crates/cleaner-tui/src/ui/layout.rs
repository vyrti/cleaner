use super::theme::{CONFIRM, CORE, DIR_STYLE, FILE_STYLE, HEADER, SELECTED, TEMP_STYLE};
use crate::app::{App, SortMode};
use ratatui::{
    prelude::*,
    text::Span,
    widgets::{Block, Borders, Clear, Paragraph},
};

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
    app.status_message.clone()
}

pub fn render_header(f: &mut Frame, app: &App, area: Rect) {
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

pub fn render_list(f: &mut Frame, app: &App, area: Rect) {
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
        let name = format!("{prefix}{}{temp_marker}", entry.name.to_string_lossy());
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

pub fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let text = status_line(app).unwrap_or_default();
    let style = if app.confirm_delete || app.confirm_clean {
        CONFIRM
    } else {
        CORE
    };
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(format!(" {text}")).style(style), area);
}

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

pub fn fit(value: &str, width: usize) -> String {
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

pub fn display_width(value: &str) -> usize {
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

pub fn pad_right(value: &str, width: usize) -> String {
    let value = fit(value, width);
    let padding = width.saturating_sub(display_width(&value));
    format!("{value}{}", " ".repeat(padding))
}

pub fn pad_left(value: &str, width: usize) -> String {
    let value = fit(value, width);
    let padding = width.saturating_sub(display_width(&value));
    format!("{}{value}", " ".repeat(padding))
}

pub fn truncate_middle(value: &str, width: usize) -> String {
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
