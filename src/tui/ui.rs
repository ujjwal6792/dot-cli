//! Rendering. Pure functions of [`App`] state.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use super::app::{App, Load, Panel};
use crate::paths::tilde;

/// Below this size only the focused panel is shown.
pub const NARROW_WIDTH: u16 = 60;
pub const SHORT_HEIGHT: u16 = 16;

pub fn is_single_panel(area: Rect) -> bool {
    area.width < NARROW_WIDTH || area.height < SHORT_HEIGHT
}

const FOOTER: &str =
    " q quit · ? help · 1-5 panels · j/k move · h/l back/open · . hidden · r refresh";

const HELP: &[(&str, &str)] = &[
    ("1-5", "focus panel Status/Files/Changes/Git/Machines"),
    ("Tab / Shift-Tab", "next / previous panel"),
    ("j / k, ↓ / ↑", "move selection"),
    ("l / → / Enter", "open folder (Files)"),
    (
        "h / ← / Backspace",
        "leave folder (Files); other panels: previous panel",
    ),
    ("g / G", "first / last item"),
    (".", "show or hide items matched by hide rules"),
    ("r", "refresh status and folder"),
    ("?", "toggle this help"),
    ("q / Ctrl-C", "quit"),
];

fn border(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn panel_block(app: &App, panel: Panel) -> Block<'static> {
    let mut title = format!(" [{}] {} ", panel.number(), panel.title());
    if panel == Panel::Files {
        title = format!(
            " [2] Files {}{}",
            tilde(&app.files.root, &app.files.cwd),
            if app.files.show_hidden {
                " (+hidden) "
            } else {
                " "
            }
        );
    }
    let focused = app.focus == panel;
    Block::default()
        .borders(Borders::ALL)
        .border_style(border(focused))
        .title(title)
        .title_style(if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        })
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.size = (area.width, area.height);
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    if is_single_panel(area) {
        draw_panel(f, app, app.focus, body, true);
    } else {
        let left_w = (area.width / 3).clamp(28, 44);
        let [left, right] =
            Layout::horizontal([Constraint::Length(left_w), Constraint::Min(10)]).areas(body);
        draw_left(f, app, left);
        let [main, pending] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(6)]).areas(right);
        draw_main(f, app, main);
        draw_pending(f, pending);
    }

    let hint = if is_single_panel(area) {
        " 1-5 panel · ? help · q quit"
    } else {
        FOOTER
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        footer,
    );
    if app.show_help {
        draw_help(f, area);
    }
}

fn draw_left(f: &mut Frame, app: &mut App, area: Rect) {
    let constraints: Vec<Constraint> = Panel::ALL
        .iter()
        .map(|p| {
            if *p == app.focus {
                Constraint::Min(5)
            } else if *p == Panel::Status {
                Constraint::Length(5)
            } else {
                Constraint::Length(3)
            }
        })
        .collect();
    let rects = Layout::vertical(constraints).split(area);
    for (panel, rect) in Panel::ALL.iter().zip(rects.iter()) {
        draw_panel(f, app, *panel, *rect, *panel == app.focus);
    }
}

fn draw_panel(f: &mut Frame, app: &mut App, panel: Panel, area: Rect, expanded: bool) {
    let block = panel_block(app, panel);
    match panel {
        Panel::Files if expanded => draw_files(f, app, block, area),
        Panel::Files => {
            let n = app.files.visible().len();
            f.render_widget(Paragraph::new(format!("{n} item(s)")).block(block), area);
        }
        Panel::Status => {
            f.render_widget(Paragraph::new(status_lines(app)).block(block), area);
        }
        other => {
            let text = match other {
                Panel::Changes => {
                    "Local · Staged · Incoming · Conflicts\n(arrives with the change model)"
                }
                Panel::Git => "Commits · Branches · Remotes · Stash\n(arrives with git support)",
                _ => "Registry · assignments · preview as\n(arrives with machine management)",
            };
            let shown = if expanded {
                text
            } else {
                text.lines().next().unwrap_or("")
            };
            f.render_widget(
                Paragraph::new(shown)
                    .style(Style::default().fg(Color::DarkGray))
                    .block(block),
                area,
            );
        }
    }
}

fn status_lines(app: &App) -> Vec<Line<'static>> {
    match &app.status {
        Load::Loading => vec![Line::from("loading…")],
        Load::Failed(e) => vec![Line::styled(
            e.lines().next().unwrap_or("error").to_string(),
            Style::default().fg(Color::Red),
        )],
        Load::Ready(s) => {
            let machine = match &s.machine {
                Some(m) => match &m.registered {
                    Some(r) => format!("{} ({}/{})", m.name, r.platform, r.distribution),
                    None => format!("{} (unregistered)", m.name),
                },
                None => "no machine".into(),
            };
            let git = match &s.repo {
                Some(r) => {
                    let sync = match (r.ahead, r.behind) {
                        (Some(a), Some(b)) => format!(" ↑{a} ↓{b}"),
                        _ => " (no upstream)".into(),
                    };
                    format!("{}{sync}", r.branch.as_deref().unwrap_or("?"))
                }
                None => "not initialised".into(),
            };
            vec![
                Line::from(machine),
                Line::from(git),
                Line::from(format!("vault: {} · pending: {}", s.vault, s.pending)),
            ]
        }
    }
}

fn draw_files(f: &mut Frame, app: &mut App, block: Block<'static>, area: Rect) {
    if app.files.loading {
        return f.render_widget(Paragraph::new("loading…").block(block), area);
    }
    if let Some(e) = &app.files.error {
        return f.render_widget(
            Paragraph::new(e.clone())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true })
                .block(block),
            area,
        );
    }
    let items: Vec<ListItem> = app
        .files
        .visible()
        .iter()
        .map(|e| {
            let mut name = e.display();
            if e.is_dir {
                name.push('/');
            } else if e.is_symlink {
                name.push('@');
            }
            let mut style = Style::default();
            if e.is_dir {
                style = style.fg(Color::Blue).add_modifier(Modifier::BOLD);
            }
            if e.hidden {
                style = style.fg(Color::DarkGray).add_modifier(Modifier::DIM);
            }
            ListItem::new(Span::styled(name, style))
        })
        .collect();
    if items.is_empty() {
        return f.render_widget(Paragraph::new("(empty)").block(block), area);
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    app.files.list_state.select(Some(app.files.selected));
    f.render_stateful_widget(list, area, &mut app.files.list_state);
}

fn draw_main(f: &mut Frame, app: &App, area: Rect) {
    let title = match (app.focus, app.files.selected_entry()) {
        (Panel::Files, Some(e)) => format!(" {} ", e.display()),
        _ => " Main ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border(false))
        .title(title);
    let text: String = match app.focus {
        Panel::Files => match &app.preview {
            Load::Loading => "loading…".into(),
            Load::Failed(e) => e.clone(),
            Load::Ready(t) => t.clone(),
        },
        Panel::Status => "Machine, branch and sync state of this computer.".into(),
        Panel::Changes => "Diffs of the selected change will appear here (delta).".into(),
        Panel::Git => "Commit and branch details will appear here.".into(),
        Panel::Machines => "Assignments of the selected machine will appear here.".into(),
    };
    f.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
        area,
    );
}

fn draw_pending(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border(false))
        .title(" Pending ");
    f.render_widget(
        Paragraph::new("No pending actions")
            .style(Style::default().fg(Color::DarkGray))
            .block(block),
        area,
    );
}

fn draw_help(f: &mut Frame, area: Rect) {
    let w = area.width.saturating_sub(4).min(76);
    let h = (HELP.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("{k:<18}"), Style::default().fg(Color::Yellow)),
                Span::raw(*d),
            ])
        })
        .collect();
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help (Esc to close) "),
        ),
        rect,
    );
}
