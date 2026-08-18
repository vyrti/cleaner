use super::theme::{CORE, HEADER};
use cleaner_core::tree::ScanProgress;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use std::path::Path;

pub fn draw_scan_progress(frame: &mut Frame, area: Rect, root: &Path, progress: &ScanProgress) {
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
