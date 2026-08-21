//! Deep Clean rendering.
//!
//! Replaces the browser's header and list while the view is open. The status
//! line and digit bar keep rendering, so the screen never loses its footer.

use super::layout::{display_width, fit, pad_left, pad_right};
use super::theme::{CONFIRM, CORE, DIR_STYLE, FILE_STYLE, HEADER, SELECTED, TEMP_STYLE};
use crate::app::{visible_rows, App, DeepPhase, DeepState, DESTRUCTIVE_WORD};
use cleaner_core::sysclean::Tier;
use ratatui::{
    prelude::*,
    text::Span,
    widgets::{Block, Borders, Paragraph},
};

/// Width of the right-hand size column.
const SIZE_WIDTH: usize = 10;
/// Width of the ` [x] ` marker column.
const MARK_WIDTH: usize = 5;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let Some(state) = app.deep.as_ref() else {
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    render_header(f, app, state, chunks[0]);
    render_list(f, state, chunks[1]);
}

fn render_header(f: &mut Frame, app: &App, state: &DeepState, area: Rect) {
    let marked = humansize::format_size(state.marked_bytes(), humansize::BINARY);
    let free = humansize::format_size(app.disk_free, humansize::BINARY);

    let summary = match &state.phase {
        DeepPhase::Probing => {
            let (current, total) = state.progress.get_stage_progress();
            format!(" Measuring {current}/{total}...")
        }
        _ => format!(
            " {} marked, {marked} selected  │  {} rows  │  Free: {free}",
            state.marked_count(),
            state.items.len()
        ),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .style(CORE)
        .title(Span::styled(" Deep Clean ", HEADER));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(fit(&summary, inner.width as usize)).style(HEADER),
        inner,
    );
}

fn render_list(f: &mut Frame, state: &DeepState, area: Rect) {
    let block = Block::default().borders(Borders::ALL).style(CORE);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.phase == DeepPhase::Probing {
        f.render_widget(
            Paragraph::new(" Measuring caches, applications and system junk...").style(FILE_STYLE),
            inner,
        );
        return;
    }

    let rows = visible_rows(state);
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new(" Nothing to clean here. Press h to show empty entries.")
                .style(FILE_STYLE),
            inner,
        );
        return;
    }

    // Build the display lines first: a section heading whenever the section
    // changes, then the rows underneath it.
    let mut lines: Vec<(Option<usize>, Line)> = Vec::new();
    let mut current_section: Option<&str> = None;
    let width = inner.width as usize;

    for (index, candidate) in state.items.iter().enumerate() {
        let section = candidate.section();
        if current_section != Some(section) {
            if !state.show_absent && section_is_empty(state, section) {
                continue;
            }
            current_section = Some(section);
            lines.push((None, section_line(state, section, width)));
        }
        // `rows` is ascending, so this stays cheap as the catalog grows.
        if rows.binary_search(&index).is_err() {
            continue;
        }
        lines.push((Some(index), row_line(state, index, width)));
    }

    // Keep the cursor in view. Unlike the browser list, this view tracks a real
    // window rather than pinning the cursor to the last visible row.
    let height = inner.height as usize;
    let cursor_line = lines
        .iter()
        .position(|(index, _)| *index == Some(state.cursor))
        .unwrap_or(0);
    let start = cursor_line.saturating_sub(height.saturating_sub(1) / 2);
    let start = start.min(lines.len().saturating_sub(height));

    let visible: Vec<Line> = lines
        .into_iter()
        .skip(start)
        .take(height)
        .map(|(_, line)| line)
        .collect();

    f.render_widget(Paragraph::new(visible), inner);
}

/// True when every row in a section is absent, so the heading would head
/// nothing.
fn section_is_empty(state: &DeepState, section: &str) -> bool {
    !state
        .items
        .iter()
        .any(|candidate| candidate.section() == section && candidate.present)
}

fn section_line(state: &DeepState, section: &str, width: usize) -> Line<'static> {
    let collapsed = state.collapsed.contains(section);
    let (count, bytes) = state
        .items
        .iter()
        .filter(|candidate| candidate.section() == section)
        .fold((0usize, 0u64), |(count, bytes), candidate| {
            (count + 1, bytes.saturating_add(candidate.size))
        });

    let marker = if collapsed { "▸" } else { "─" };
    let size = humansize::format_size(bytes, humansize::BINARY);
    let label = format!(" {marker} {section} ({count})");
    // Display width, not byte length: the rule characters are multi-byte, and
    // using `len()` here pulls the size column out of line with the rows.
    let filler = width
        .saturating_sub(display_width(&label))
        .saturating_sub(SIZE_WIDTH + 1);

    Line::from(vec![Span::styled(
        format!(
            "{label} {}{}",
            "─".repeat(filler),
            pad_left(&size, SIZE_WIDTH)
        ),
        DIR_STYLE.add_modifier(Modifier::BOLD),
    )])
}

fn row_line(state: &DeepState, index: usize, width: usize) -> Line<'static> {
    let candidate = &state.items[index];
    let target = &candidate.target;
    let marked = state.marked.get(index).copied().unwrap_or(false);

    let mark = if !target.selectable() {
        "  ·  "
    } else if marked {
        " [x] "
    } else {
        " [ ] "
    };

    let size = if candidate.size == 0 {
        "—".to_string()
    } else {
        humansize::format_size(candidate.size, humansize::BINARY)
    };

    // Split the row exactly: mark + label + detail + size fills the width, so
    // the size column lines up with the one in the section rule.
    let body = width.saturating_sub(MARK_WIDTH + SIZE_WIDTH);
    let label_width = body.clamp(0, 34);
    let detail_width = body.saturating_sub(label_width);

    let text = format!(
        "{mark}{}{}{}",
        pad_right(&target.label, label_width),
        pad_right(&fit(&target.detail, detail_width), detail_width),
        pad_left(&size, SIZE_WIDTH),
    );

    let style = if index == state.cursor {
        SELECTED.add_modifier(Modifier::BOLD)
    } else if target.tier == Tier::Destructive {
        TEMP_STYLE.add_modifier(Modifier::BOLD)
    } else if target.tier == Tier::NeedsRoot || !target.selectable() {
        FILE_STYLE
    } else if marked {
        DIR_STYLE
    } else {
        FILE_STYLE
    };

    Line::from(vec![Span::styled(fit(&text, width), style)])
}

/// Status line while Deep Clean is open. Returned by [`super::status_line`].
pub fn status(state: &DeepState) -> Option<String> {
    match &state.phase {
        DeepPhase::Probing => Some("Measuring... 0/q to leave".into()),
        DeepPhase::Running => Some("Cleaning... please wait".into()),
        DeepPhase::Done(summary) => Some(format!("{summary} — enter to re-measure, esc to leave")),
        DeepPhase::Confirm => {
            let bytes = humansize::format_size(state.marked_bytes(), humansize::BINARY);
            Some(format!(
                "Run {} target(s), freeing about {bytes}? (y/n)",
                state.marked_count()
            ))
        }
        DeepPhase::Typing => Some(format!(
            "DESTRUCTIVE. Type {DESTRUCTIVE_WORD} then enter to confirm, esc to cancel: {}",
            state.typed
        )),
        DeepPhase::Ready => {
            let hint = if state.marked_count() > 0 {
                "enter run"
            } else {
                "a mark safe"
            };
            Some(format!(
                "space toggle  a mark safe  u unmark  ←/→ fold  h hidden  {hint}  esc back"
            ))
        }
    }
}

/// Style for the status line while Deep Clean is open.
pub fn status_style(state: &DeepState) -> Option<Style> {
    matches!(state.phase, DeepPhase::Confirm | DeepPhase::Typing).then_some(CONFIRM)
}
