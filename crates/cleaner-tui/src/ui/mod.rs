//! TUI rendering — Norton/Abyss palette and digit action bar.

mod buttons;
mod deep;
mod layout;
mod progress;
mod theme;

#[cfg(test)]
mod tests;

pub use buttons::{ActionButton, BUTTONS};
pub use layout::status_line;
pub use progress::draw_scan_progress;
pub use theme::{CONFIRM, CORE, DIR_STYLE, FILE_STYLE, HEADER, SELECTED, TEMP_STYLE};

use crate::app::App;
use ratatui::{prelude::*, widgets::Block};

/// How much chrome to draw around the analyze content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Chrome {
    /// Standalone cleaner: header + list + status + digit bar.
    Full,
    /// Embedded in Abyss: header + list only (host owns status/buttons).
    ContentOnly,
}

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
            let status_area = has_status
                .then(|| Rect::new(area.x, button_area.y.saturating_sub(1), area.width, 1));
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

    // Deep Clean takes over the content area entirely; the status line and
    // digit bar below it keep rendering.
    if app.in_deep() {
        deep::render(f, app, content);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(5)])
            .split(content);

        layout::render_header(f, app, chunks[0]);
        layout::render_list(f, app, chunks[1]);
    }

    if let Some(status_area) = status_area {
        layout::render_status(f, app, status_area);
    }
    if let Some(button_area) = button_area {
        buttons::render_buttons(f, button_area);
    }
}
