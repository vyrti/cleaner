use super::layout::{fit, pad_right};
use ratatui::{
    prelude::*,
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionButton {
    Help,
    Empty2,
    Sort,
    Deep,
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
            Self::Sort => '3',
            Self::Deep => '4',
            Self::Clean => '5',
            Self::Delete => '6',
            Self::Refresh => '7',
            Self::Empty8 => '8',
            Self::Empty9 => '9',
            Self::Quit => '0',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Help => "Help",
            Self::Empty2 | Self::Empty8 | Self::Empty9 => "",
            Self::Sort => "Sort",
            Self::Deep => "Deep",
            Self::Clean => "Clean",
            Self::Delete => "Delete",
            Self::Refresh => "Refresh",
            Self::Quit => "Quit",
        }
    }

    pub fn disabled(self) -> bool {
        matches!(self, Self::Empty2 | Self::Empty8 | Self::Empty9)
    }
}

pub const BUTTONS: [ActionButton; 10] = [
    ActionButton::Help,
    ActionButton::Empty2,
    ActionButton::Sort,
    ActionButton::Deep,
    ActionButton::Clean,
    ActionButton::Delete,
    ActionButton::Refresh,
    ActionButton::Empty8,
    ActionButton::Empty9,
    ActionButton::Quit,
];

pub fn render_buttons(f: &mut Frame, area: Rect) {
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
