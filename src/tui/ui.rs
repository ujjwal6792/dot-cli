//! Rendering. Pure functions of [`App`] state.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Dialog, Load, Panel};
use crate::paths::tilde;
use crate::source::TrackState;
use crate::track::Outcome;

/// Below this size only the focused panel is shown.
pub const NARROW_WIDTH: u16 = 60;
pub const SHORT_HEIGHT: u16 = 16;

pub fn is_single_panel(area: Rect) -> bool {
    area.width < NARROW_WIDTH || area.height < SHORT_HEIGHT
}

const FOOTER: &str =
    " q quit · ? help · 1-5 · 0 pending · Space track · v range · . hidden · r refresh";

const HELP: &[(&str, &str)] = &[
    ("1-5", "focus panel Status/Files/Changes/Git/Machines"),
    ("Tab / Shift-Tab", "next / previous panel"),
    ("0", "focus the Pending pane (also Tab after Machines)"),
    ("j / k, ↓ / ↑", "move selection"),
    (
        "Space",
        "Files: track / untrack (adds or drops a pending item)",
    ),
    ("v", "Files: start / end a range selection, then Space"),
    ("Space (Pending)", "check / uncheck the item for the review"),
    ("d (Pending)", "drop the selected pending item"),
    ("Enter (Pending)", "review and apply all checked items"),
    (
        "markers",
        "✓ tracked  ▣ tracked folder  ◇ holds tracked  ? new  + / - pending track / untrack",
    ),
    ("l / → / Enter", "open folder (Files)"),
    (
        "h / ← / Backspace",
        "leave folder (Files); other panels: previous panel",
    ),
    ("< / >, Home / End", "first / last item"),
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
    if panel == Panel::Pending {
        title = format!(" [0] Pending ({}) ", app.pending.items.len());
    }
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
        draw_pending(f, app, pending);
    }

    let hint = if is_single_panel(area) {
        " 0-5 panel · ? help · q quit"
    } else {
        FOOTER
    };
    let (text, style) = match &app.message {
        Some(m) => (format!(" {m}"), Style::default().fg(Color::Yellow)),
        None if app.actions_busy() => (" working…".to_string(), Style::default().fg(Color::Yellow)),
        None => (hint.to_string(), Style::default().fg(Color::DarkGray)),
    };
    f.render_widget(Paragraph::new(text).style(style), footer);
    if app.show_help {
        draw_help(f, area);
    }
    if let Some(d) = &app.dialog {
        draw_dialog(f, d, area);
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
        Panel::Pending => draw_pending_list(f, app, block, area),
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
    let marks = app.pending_marks();
    let range = app.range_rows();
    let items: Vec<ListItem> = app
        .files
        .visible()
        .iter()
        .enumerate()
        .map(|(row, e)| {
            let rel = app.files.rel_of(e);
            let (marker, marker_style) = marker_for(app, e, &rel);
            let pending = match marks.get(&rel) {
                Some(&"track") => Span::styled("+", Style::default().fg(Color::Yellow)),
                Some(_) => Span::styled("-", Style::default().fg(Color::Yellow)),
                None => Span::raw(" "),
            };
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
            let item = ListItem::new(Line::from(vec![
                Span::styled(marker, marker_style),
                pending,
                Span::raw(" "),
                Span::styled(name, style),
            ]));
            if range.is_some_and(|(lo, hi)| (lo..=hi).contains(&row)) {
                item.style(Style::default().bg(Color::DarkGray))
            } else {
                item
            }
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

fn marker_for(app: &App, e: &super::files::Entry, rel: &str) -> (&'static str, Style) {
    if e.hidden {
        return (" ", Style::default());
    }
    match app.tracked.state_of(rel, e.is_dir) {
        TrackState::Tracked => ("✓", Style::default().fg(Color::Green)),
        TrackState::TrackedFolder => ("▣", Style::default().fg(Color::Green)),
        TrackState::ContainsTracked => ("◇", Style::default().fg(Color::Green)),
        TrackState::New => ("?", Style::default().fg(Color::Magenta)),
        TrackState::Untracked => (" ", Style::default()),
    }
}

fn state_text(app: &App, e: &super::files::Entry) -> String {
    let rel = app.files.rel_of(e);
    let marks = app.pending_marks();
    let state = if e.hidden {
        "hidden".to_string()
    } else {
        crate::track::state_label(app.tracked.state_of(&rel, e.is_dir)).to_string()
    };
    match marks.get(&rel) {
        Some(p) => format!("[{state}] pending: {p}"),
        None => format!("[{state}]"),
    }
}

fn draw_main(f: &mut Frame, app: &App, area: Rect) {
    let title = match (app.focus, app.files.selected_entry(), app.selected_item()) {
        (Panel::Files, Some(e), _) => format!(" {} ", e.display()),
        (Panel::Pending, _, Some(i)) => format!(" Pending {} ", i.label()),
        _ => " Main ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border(false))
        .title(title);
    let text: String = match app.focus {
        Panel::Files => {
            let head = app
                .files
                .selected_entry()
                .map_or(String::new(), |e| format!("{}\n\n", state_text(app, e)));
            let body = match &app.preview {
                Load::Loading => "loading…".into(),
                Load::Failed(e) => e.clone(),
                Load::Ready(t) => t.clone(),
            };
            format!("{head}{body}")
        }
        Panel::Pending => app
            .selected_item()
            .map_or_else(|| "No pending actions".to_string(), |i| i.render()),
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

fn draw_pending(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel_block(app, Panel::Pending);
    draw_pending_list(f, app, block, area);
}

fn draw_pending_list(f: &mut Frame, app: &mut App, block: Block<'static>, area: Rect) {
    if app.pending.items.is_empty() {
        return f.render_widget(
            Paragraph::new("No pending actions")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
    }
    let items: Vec<ListItem> = app
        .pending
        .items
        .iter()
        .map(|i| {
            let checked = i.id.is_some_and(|id| !app.unchecked.contains(&id));
            ListItem::new(format!(
                "[{}] {} {}: {}",
                if checked { "x" } else { " " },
                i.label(),
                i.requester,
                i.summary
            ))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(if app.focus == Panel::Pending {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        });
    let mut state = ListState::default();
    state.select(Some(app.pending_selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_dialog(f: &mut Frame, dialog: &Dialog, area: Rect) {
    let (title, lines): (&str, Vec<Line>) = match dialog {
        Dialog::Review(items) => {
            let mut l = vec![Line::styled(
                format!("Apply {} pending item(s)?", items.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )];
            for i in items {
                l.push(Line::from(format!(
                    "{} {} — {}",
                    i.label(),
                    i.requester,
                    i.summary
                )));
                for c in i.preview.files.iter().take(3) {
                    l.push(Line::styled(
                        format!(
                            "    {} {}",
                            if c.action == "add" { "+" } else { "-" },
                            c.source
                        ),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                if i.preview.files.len() > 3 {
                    l.push(Line::styled(
                        format!("    … {} more file(s)", i.preview.files.len() - 3),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                for d in i.preview.data.iter().chain(&i.preview.rules) {
                    l.push(Line::styled(
                        format!("    {d}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            l.push(Line::from(""));
            l.push(Line::styled(
                "Enter / y apply · Esc cancel (results are staged in git, not committed)",
                Style::default().fg(Color::Yellow),
            ));
            (" Review ", l)
        }
        Dialog::Result(results) => {
            let mut l = Vec::new();
            for r in results {
                let (tag, colour) = match r.status {
                    Outcome::Applied => ("applied", Color::Green),
                    Outcome::Stale => ("stale  ", Color::Yellow),
                    Outcome::Failed => ("failed ", Color::Red),
                };
                l.push(Line::from(vec![
                    Span::styled(tag, Style::default().fg(colour)),
                    Span::raw(format!(
                        " {} {}",
                        r.id.map_or(String::new(), |i| format!("#{i}")),
                        r.summary
                    )),
                ]));
                if let Some(m) = &r.message {
                    l.push(Line::styled(
                        format!("    {m}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            l.push(Line::from(""));
            l.push(Line::styled(
                "Stale and failed items are kept in the pending list. Any key closes.",
                Style::default().fg(Color::Yellow),
            ));
            (" Result ", l)
        }
    };
    let w = area.width.saturating_sub(4).min(90);
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(title)),
        rect,
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
