//! Keyboard reference with scoped tabs and a layout that follows terminal width.
use crate::dialog;
use crossterm::event::KeyCode;
use ratatui::{prelude::*, widgets::*};

const MINT: Color = Color::Rgb(97, 218, 179);
const MUTED: Color = Color::Rgb(160, 168, 180);
const TABS: [&str; 3] = ["1 General", "2 Files", "3 Git"];

#[derive(Clone, Debug, Default)]
pub struct View {
    pub tab: usize,
    scroll: u16,
}

#[derive(Default)]
pub struct Areas {
    pub body: Rect,
    pub max_scroll: u16,
    pub controls: Vec<(Rect, KeyCode)>,
}

impl View {
    pub fn new(tab: usize) -> Self {
        Self { tab, scroll: 0 }
    }

    /// Return true when the user closes Help.
    pub fn key(&mut self, key: KeyCode, areas: &Areas) -> bool {
        let tab = match key {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => return true,
            KeyCode::Char('1') => 0,
            KeyCode::Char('2') => 1,
            KeyCode::Char('3') => 2,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char(']') => (self.tab + 1) % TABS.len(),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('[') => {
                (self.tab + TABS.len() - 1) % TABS.len()
            }
            _ => self.tab,
        };
        if tab != self.tab {
            self.tab = tab;
            self.scroll = 0;
            return false;
        }
        let page = areas.body.height.saturating_sub(1).max(1);
        let scroll = self.scroll.min(areas.max_scroll);
        self.scroll = match key {
            KeyCode::Down | KeyCode::Char('j') => scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
            KeyCode::PageDown => scroll.saturating_add(page),
            KeyCode::PageUp => scroll.saturating_sub(page),
            KeyCode::Home => 0,
            KeyCode::End => areas.max_scroll,
            _ => scroll,
        }
        .min(areas.max_scroll);
        false
    }
}

enum Entry {
    Heading(&'static str),
    Key(&'static str, &'static str),
    Note(&'static str),
}
use Entry::{Heading, Key, Note};

const GENERAL: &[Entry] = &[
    Heading("Move around"),
    Key("1 / 2 / 3 / 4", "Status / Files / Git / Machines"),
    Key("0", "Focus Details"),
    Key("Tab", "Focus the next pane"),
    Key("↑ ↓ / j k", "Move through rows"),
    Key("PgUp / PgDn", "Move through a page of rows"),
    Key("[ / ]", "Switch tabs in the focused pane"),
    Key("z", "Expand or restore the focused pane"),
    Key(
        "Enter",
        "Open file details; open actions in Status / Machines",
    ),
    Key("Esc", "Return to Files or cancel the current action"),
    Heading("Workspace"),
    Key(":", "All actions, settings, recovery and packages"),
    Key("Ctrl+R", "Refresh the workspace"),
    Key("?", "Open Help"),
    Key("q / Ctrl+C", "Quit when no write operation is running"),
    Heading("Mouse and text input"),
    Note(
        "Click a pane to focus it. Click tabs and footer shortcuts to activate them. Scroll the pane under the pointer with the wheel.",
    ),
    Note("Text fields receive character shortcuts as text. Pasting is supported."),
];

const FILES: &[Entry] = &[
    Heading("Browse and select"),
    Key("→ / l", "Open a folder"),
    Key("← / h", "Go to the parent folder"),
    Key("Space", "Select a file or folder"),
    Key("/", "Search files"),
    Key("t", "Toggle tracked (Managed) / all files in ~"),
    Key("f", "Cycle All / Managed / Changed"),
    Heading("Assign operating systems and machines"),
    Key("s", "Open Details → Rules for this file or folder"),
    Key(
        "o / m / M",
        "In Rules: cycle OS / all or this machine / choose hostnames",
    ),
    Key(
        "s / u",
        "In Rules: review and save / reset assignment edits",
    ),
    Heading("Save and use settings"),
    Key("c", "Capture: save HOME → source"),
    Key("a", "Apply: use rendered source → HOME"),
    Key("R", "Restore originals from saved copies"),
    Key("e", "Edit the native source file"),
    Key(
        "Ctrl+S / F2",
        "In the editor: review changes / expand or restore",
    ),
    Heading("Ignore and remove"),
    Key("i", "Ignore in both chezmoi and Git"),
    Key("r", "Remove saved copies; keep originals"),
    Key("d / Delete", "Move untracked originals to system Trash"),
    Heading("Review before applying"),
    Key("Enter / y", "Confirm the reviewed operation"),
    Key("Esc", "Cancel the review"),
    Note(
        "Capture skips developer clutter, including .DS_Store, node_modules, target, Python caches and .venv by default.",
    ),
    Note("Native chezmoi previews may evaluate repository templates and hooks."),
];

const GIT: &[Entry] = &[
    Heading("Changes and commits"),
    Key("h / [ / ]", "Switch Changes / Commits"),
    Key("Space", "Stage or unstage the current path"),
    Key("v", "Select multiple Git paths"),
    Key("c", "Commit the staged changes"),
    Key("i", "Ignore in both chezmoi and Git"),
    Key("d", "Discard source changes back to HEAD"),
    Heading("Remote repository"),
    Key("F", "Fetch"),
    Key("L", "Pull (clean, fast-forward only)"),
    Key("P", "Push"),
    Heading("Understand the state"),
    Note(
        "HOME changes, saved source changes, staged changes and unpublished commits describe different steps.",
    ),
    Note(
        "Stage, commit and push are separate actions. HOME matching source does not mean the source is committed or published.",
    ),
];

fn content(tab: usize, width: u16) -> Text<'static> {
    let entries = [GENERAL, FILES, GIT][tab];
    let stacked = width < 48;
    let key_width = if stacked { 0 } else { 17 };
    let mut lines = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match entry {
            Heading(title) => {
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Line::from(*title).fg(MINT).bold());
            }
            Key(key, description) => {
                if stacked {
                    lines.push(Line::from(*key).fg(Color::LightCyan).bold());
                }
                let available = usize::from(width).saturating_sub(key_width).max(1);
                let mut wrapped = vec![String::new()];
                for word in description.split_whitespace() {
                    let line = wrapped.last_mut().unwrap();
                    if !line.is_empty() && Line::from(format!("{line} {word}")).width() > available
                    {
                        wrapped.push(word.into());
                    } else {
                        if !line.is_empty() {
                            line.push(' ');
                        }
                        line.push_str(word);
                    }
                }
                for (index, description) in wrapped.into_iter().enumerate() {
                    let key = if !stacked && index == 0 { *key } else { "" };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{key: <key_width$}"),
                            Style::default().fg(Color::LightCyan).bold(),
                        ),
                        Span::raw(description),
                    ]));
                }
            }
            Note(note) => {
                if index > 0 && !matches!(entries[index - 1], Heading(_)) {
                    lines.push(Line::default());
                }
                lines.push(Line::from(*note).fg(MUTED));
            }
        }
    }
    Text::from(lines)
}

pub fn draw(f: &mut Frame, area: Rect, view: &View) -> Areas {
    let area = area.inner(Margin::new(u16::from(area.width >= 30), 0));
    let regions = Layout::vertical([
        Constraint::Length(2.min(area.height.saturating_sub(2))),
        Constraint::Min(1),
        Constraint::Length(if area.height >= 8 { 3 } else { 1 }),
    ])
    .split(area);
    let labels = if area.width < 30 {
        ["1", "2", "3"]
    } else {
        TABS
    };
    let mut controls = Vec::new();
    let mut x = regions[0].x;
    for (index, label) in labels.into_iter().enumerate() {
        let width = (label.len() as u16).min(regions[0].right().saturating_sub(x));
        let rect = Rect::new(x, regions[0].y, width, regions[0].height.min(1));
        let style = if index == view.tab {
            Style::default().fg(MINT).bold().underlined()
        } else {
            Style::default().fg(MUTED)
        };
        f.render_widget(Paragraph::new(label).style(style), rect);
        controls.push((rect, KeyCode::Char(char::from(b'1' + index as u8))));
        x = x.saturating_add(width + 3);
    }
    // Leave a column for the scrollbar so it never covers text.
    let body = Rect::new(
        regions[1].x,
        regions[1].y,
        regions[1].width.saturating_sub(1),
        regions[1].height,
    );
    let paragraph = Paragraph::new(content(view.tab, body.width)).wrap(Wrap { trim: false });
    let lines = paragraph.line_count(body.width);
    let max_scroll = lines
        .saturating_sub(body.height as usize)
        .min(u16::MAX as usize) as u16;
    let scroll = view.scroll.min(max_scroll);
    f.render_widget(paragraph.scroll((scroll, 0)), body);
    if max_scroll > 0 {
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            regions[1],
            // Ratatui adds the viewport length when sizing the thumb.
            &mut ScrollbarState::new(usize::from(max_scroll) + 1)
                .viewport_content_length(body.height as usize)
                .position(scroll as usize),
        );
    }
    let footer = regions[2];
    if footer.height >= 3 {
        let hint = if max_scroll > 0 && footer.width >= 54 {
            "Tab Topic · ↑↓ / PgUp PgDn / wheel Scroll"
        } else if max_scroll > 0 {
            "Tab Topic · ↑↓ Scroll"
        } else {
            "1–3 / Tab Switch help topic"
        };
        f.render_widget(
            Paragraph::new(hint).fg(MUTED),
            Rect::new(footer.x, footer.y + 1, footer.width, 1),
        );
    }
    let close = if footer.width < 28 {
        "Esc Close"
    } else {
        "Enter / Esc Close"
    };
    controls.extend(dialog::controls(
        f,
        Rect::new(
            footer.x,
            footer.bottom().saturating_sub(1),
            footer.width,
            footer.height.min(1),
        ),
        &[(close, KeyCode::Esc)],
    ));
    if max_scroll > 0 && footer.width >= 40 {
        f.render_widget(
            Paragraph::new(format!(
                "{}–{} of {}",
                scroll + 1,
                (scroll as usize + body.height as usize).min(lines),
                lines
            ))
            .fg(MUTED)
            .right_aligned(),
            Rect::new(
                footer.x + 20,
                footer.bottom().saturating_sub(1),
                footer.width.saturating_sub(20),
                1,
            ),
        );
    }
    Areas {
        body,
        max_scroll,
        controls,
    }
}
