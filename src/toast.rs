//! Transient background status; never dims or takes focus from the workspace.
use ratatui::{prelude::*, widgets::*};

#[derive(Default)]
pub struct Areas {
    pub bounds: Rect,
    pub cancel: Option<Rect>,
}

pub fn draw(f: &mut Frame, area: Rect, title: &str, message: &str, cancellable: bool) -> Areas {
    if area.is_empty() {
        return Areas::default();
    }
    let margin = u16::from(area.width >= 60 && area.height >= 10);
    let width = area.width.saturating_sub(2 * margin).min(48);
    let paragraph = Paragraph::new(message).wrap(Wrap { trim: false });
    let height = (paragraph.line_count(width.saturating_sub(2)).min(4) as u16 + 2)
        .min(area.height.saturating_sub(2 * margin));
    let bounds = Rect::new(
        area.right() - margin - width,
        area.bottom() - margin - height,
        width,
        height,
    );
    let mut block = Block::bordered()
        .title(title.to_owned())
        .border_style(Style::default().fg(Color::Rgb(97, 218, 179)))
        .style(
            Style::default()
                .fg(Color::Rgb(235, 239, 243))
                .bg(Color::Rgb(24, 26, 31)),
        );
    if cancellable {
        block = block.title_bottom("Esc Cancel");
    }
    let inner = block.inner(bounds);
    f.render_widget(Clear, bounds);
    f.render_widget(block, bounds);
    f.render_widget(paragraph, inner);
    let cancel = (cancellable && bounds.height >= 2 && bounds.width >= 3).then(|| {
        Rect::new(
            bounds.x + 1,
            bounds.bottom() - 1,
            bounds.width.saturating_sub(2).min(10),
            1,
        )
    });
    Areas { bounds, cancel }
}
