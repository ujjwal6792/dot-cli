//! First-run copy and terminal branding. SVG sources and credits: assets/branding/.
use ratatui::{prelude::*, widgets::*};
use std::path::Path;

const MINT: Color = Color::Rgb(97, 218, 179);
const MUTED: Color = Color::Rgb(160, 168, 180);
const INK: Color = Color::Rgb(235, 239, 243);
const BACKGROUND: Color = Color::Rgb(24, 26, 31);

// Half-block adaptations use ordinary terminal characters, not icon-font glyphs.
const DOT: [&str; 5] = [
    " ▄██▄  ▄██▄ ",
    " ██      ██ ",
    " ██  ██  ██ ",
    " ██      ██ ",
    " ▀██▀  ▀██▀ ",
];
// Nerd Fonts: nf-md-home (chezmoi has no dedicated glyph), nf-dev-git.
const CHEZMOI_ICON: &str = "\u{f02dc}";
const GIT_ICON: &str = "\u{e702}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupKind {
    Created,
    Existing,
    Cloned,
}

#[derive(Clone, Debug)]
pub struct SetupReport {
    pub kind: SetupKind,
    pub details: String,
}

impl SetupReport {
    pub fn summary(&self) -> &'static str {
        match self.kind {
            SetupKind::Created => {
                "Created a folder for your saved settings. No settings have been saved yet."
            }
            SetupKind::Existing => {
                "Dot is ready to use the folder you already have for saved settings."
            }
            SetupKind::Cloned => "Downloaded your settings repository to this computer.",
        }
    }

    pub fn message(&self, source: &Path) -> String {
        format!(
            "Welcome to dot\n\n{}\nSettings folder: {}\nYour apps' current settings haven't been changed.\n\n{}\n\nRun dot to browse your files and get started.",
            self.summary(),
            source.display(),
            self.details,
        )
    }
}

/// Returns the click target for continuing and the maximum body scroll offset.
pub fn draw(
    f: &mut Frame,
    area: Rect,
    source: &Path,
    report: &SetupReport,
    scroll: u16,
) -> (Rect, u16) {
    f.render_widget(
        Block::default().style(Style::default().fg(INK).bg(BACKGROUND)),
        area,
    );
    let width = area
        .width
        .saturating_sub(if area.width >= 60 { 6 } else { 2 })
        .min(86);
    let height = area.height.min(25);
    let content = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let compact = area.height < 22 || width < 60;
    let sections = Layout::vertical([
        Constraint::Length(if compact {
            2
        } else if area.height >= 26 {
            6
        } else {
            5
        }),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .split(content);
    if compact {
        f.render_widget(
            Paragraph::new("dot  /  Welcome").bold().fg(MINT),
            sections[0],
        );
    } else {
        let labels = [
            "",
            "Welcome to dot",
            "Keep your app settings in one place.",
            "Save copies. Review changes. Reuse them.",
            "",
        ];
        let lines: Vec<_> = DOT
            .iter()
            .zip(labels)
            .enumerate()
            .map(|(i, (mark, label))| {
                Line::from(vec![
                    Span::styled(*mark, Style::default().fg(MINT)),
                    Span::raw("   "),
                    Span::styled(
                        label,
                        if i == 1 {
                            Style::default().fg(INK).bold()
                        } else {
                            Style::default().fg(MUTED)
                        },
                    ),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines), sections[0]);
    }

    let mut lines = vec![
        Line::from(report.summary()),
        Line::from("Your apps' current settings haven't been changed.").fg(MINT),
        Line::from(vec![
            Span::styled("Settings folder: ", Style::default().fg(MUTED)),
            Span::raw(source.display().to_string()),
        ]),
        Line::from(""),
        Line::from("Start with one settings file").bold(),
    ];
    let steps = [
        (
            "1  Choose a file",
            "In Files, use arrows to browse and Space to select.",
        ),
        (
            "2  Save a copy",
            "Press c (Capture). Review the copy, then y to save it.",
        ),
        (
            "3  Later, use a saved copy",
            "Press a (Apply). Review the changes, then y to use them.",
        ),
    ];
    for (title, body) in steps {
        lines.push(Line::from(title).fg(MINT));
        lines.push(Line::from(format!("   {body}")));
    }
    lines.push(Line::from(""));
    lines.push(
        Line::from("Developer clutter is ignored automatically. Use i to ignore more.").fg(MUTED),
    );
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(sections[1].width)
        .saturating_sub(sections[1].height as usize)
        .min(u16::MAX as usize) as u16;
    f.render_widget(paragraph.scroll((scroll.min(max_scroll), 0)), sections[1]);

    // Terminals cannot reliably report font coverage. Keep names visible and
    // provide an explicit text-only option rather than guessing font support.
    let icons = content.width >= 32 && std::env::var("DOT_ICONS").as_deref() != Ok("none");
    let branding = if icons {
        Line::from(vec![
            Span::styled("Powered by: ", Style::default().fg(MUTED)),
            Span::styled(CHEZMOI_ICON, Style::default().fg(INK)),
            Span::raw(" chezmoi   "),
            Span::styled(GIT_ICON, Style::default().fg(Color::Rgb(240, 100, 75))),
            Span::raw(" Git"),
        ])
    } else {
        Line::from(if content.width < 24 {
            "chezmoi + Git"
        } else {
            "Powered by: chezmoi + Git"
        })
        .fg(MUTED)
    };
    f.render_widget(Paragraph::new(branding), sections[2]);
    let controls = if content.width < 28 {
        "[Enter] Open files\n↑/↓ Read · q Quit"
    } else if content.width < 48 {
        "[Enter] Open files\n↑/↓ Read · d Details · q Quit"
    } else if max_scroll > 0 {
        "[Enter] Open files   [Esc] Explore on your own\n↑/↓ Scroll guide   d Setup details   q Quit"
    } else {
        "[Enter] Open files   [Esc] Explore on your own\nd Setup details   q Quit"
    };
    f.render_widget(Paragraph::new(controls).fg(MINT), sections[3]);
    (
        Rect::new(sections[3].x, sections[3].y, sections[3].width.min(18), 1),
        max_scroll,
    )
}
