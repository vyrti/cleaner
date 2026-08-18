use ratatui::prelude::*;

pub const CORE: Style = Style::new().fg(Color::Gray).bg(Color::Blue);
pub const HEADER: Style = Style::new().fg(Color::Yellow).bg(Color::Blue);
pub const SELECTED: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
pub const TEMP_STYLE: Style = Style::new().fg(Color::LightRed).bg(Color::Blue);
pub const DIR_STYLE: Style = Style::new().fg(Color::LightCyan).bg(Color::Blue);
pub const FILE_STYLE: Style = Style::new().fg(Color::Gray).bg(Color::Blue);
pub const CONFIRM: Style = Style::new()
    .fg(Color::Yellow)
    .bg(Color::Blue)
    .add_modifier(Modifier::BOLD);
