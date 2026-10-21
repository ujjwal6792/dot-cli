//! Geometry and frame shared by action dialogs. Render the workspace first.
use crossterm::event::KeyCode;
use ratatui::{prelude::*, widgets::*};

pub fn bounds(area: Rect, width: u16, height: u16) -> Rect {
    let margin_x = if area.width >= 80 {
        2
    } else if area.width >= 30 {
        1
    } else {
        0
    };
    let margin_y = u16::from(area.height >= 12);
    let width = width.min(area.width.saturating_sub(2 * margin_x));
    let height = height.min(area.height.saturating_sub(2 * margin_y));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

pub fn operation_bounds(area: Rect) -> Rect {
    bounds(area, 100, 30)
}

pub fn dim(f: &mut Frame) {
    let area = f.area();
    f.buffer_mut().set_style(
        area,
        Style::default()
            .fg(Color::Rgb(105, 113, 126))
            .remove_modifier(Modifier::BOLD),
    );
}

pub fn frame(f: &mut Frame, area: Rect, title: &str) -> Rect {
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(title.to_owned())
        .border_style(Style::default().fg(Color::Rgb(97, 218, 179)))
        .style(
            Style::default()
                .fg(Color::Rgb(235, 239, 243))
                .bg(Color::Rgb(24, 26, 31)),
        );
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

/// Render exactly the same rectangles used for mouse activation.
pub fn controls(f: &mut Frame, area: Rect, controls: &[(&str, KeyCode)]) -> Vec<(Rect, KeyCode)> {
    let mut x = area.x;
    let mut y = area.y;
    let mut hits = vec![];
    for (label, key) in controls {
        let width = label.chars().count() as u16;
        if x > area.x && x.saturating_add(width) > area.right() {
            x = area.x;
            y = y.saturating_add(1);
        }
        if y >= area.bottom() || area.width == 0 {
            break;
        }
        let rect = Rect::new(x, y, width.min(area.right().saturating_sub(x)), 1);
        f.render_widget(Paragraph::new(*label).fg(Color::LightCyan), rect);
        hits.push((rect, *key));
        x = x.saturating_add(width + 3);
    }
    hits
}
