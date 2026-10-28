//! Rendering. Pure functions of [`App`] state.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Dialog, Load, Panel, TABS};
use crate::paths::tilde;
use crate::source::TrackState;
use crate::track::Outcome;

/// Below this size only the focused panel is shown.
pub const NARROW_WIDTH: u16 = 60;
pub const SHORT_HEIGHT: u16 = 16;

pub fn is_single_panel(area: Rect) -> bool {
    area.width < NARROW_WIDTH || area.height < SHORT_HEIGHT
}

const FOOTER: &str = " q quit · ? help · 1-5 · 0 pending · Space track · m assign · t variant · V values · v range · r refresh";

const HELP: &[(&str, &str)] = &[
    ("?", "toggle this help"),
    ("q / Ctrl-C", "quit"),
    ("1-5", "focus panel Status/Files/Changes/Git/Machines"),
    ("Tab / Shift-Tab", "next / previous panel"),
    ("0", "focus the Pending pane (also Tab after Machines)"),
    ("j / k, ↓ / ↑", "move selection"),
    (
        "Space",
        "Files: track / untrack (adds or drops a pending item)",
    ),
    ("v", "Files: start / end a range selection, then Space"),
    ("m / t", "Files/Changes: assign (m), add a variant ◆ (t)"),
    (
        "i",
        "Files: hide (or un-hide) the path through a pending hide item",
    ),
    (
        "Space/a (Changes)",
        "stage, unstage, apply incoming (y/n); a = all; [ ] tabs",
    ),
    (
        "c A d (Changes)",
        "commit, amend (Ctrl+J: new line), discard; Enter diff",
    ),
    ("f p P", "fetch, pull (fast-forward), push (y/n)"),
    ("Space (Pending)", "check / uncheck the item for the review"),
    ("d (Pending)", "drop the selected pending item"),
    ("Enter (Pending)", "review and apply all checked items"),
    (
        "markers",
        "✓ tracked ▣ folder ◇ holds tracked ? new +/- pending",
    ),
    ("l / → / Enter", "open folder (Files)"),
    (
        "h / ← / Backspace",
        "leave folder (Files); other panels: previous panel",
    ),
    ("< / >, Home / End", "first / last item"),
    (".", "show or hide items matched by hide rules"),
    ("r", "refresh status and folder"),
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

fn draw_values(f: &mut Frame, app: &App, area: Rect) {
    let Some(v) = &app.values else { return };
    let input_h = if v.input.is_some() { 3 } else { 0 };
    let [table, input] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(input_h)]).areas(area);
    let mut lines = vec![Line::styled(
        format!(
            "{:<18} {:<9} {:<16} {:<34} {}",
            "NAME", "KIND", "DEFAULT", "OVERRIDES", "FILES"
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if v.rows.is_empty() {
        lines.push(Line::from("No values yet. n creates a variable."));
    }
    for (i, r) in v.rows.iter().enumerate() {
        let overrides = r
            .overrides
            .iter()
            .map(|o| format!("{}={}", o.target, o.value))
            .collect::<Vec<_>>()
            .join(", ");
        let files = r
            .files
            .iter()
            .map(|f| format!("~/{f}"))
            .collect::<Vec<_>>()
            .join(", ");
        let style = if i == v.selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::styled(
            format!(
                "{:<18} {:<9} {:<16} {:<34} {}",
                r.name, r.kind, r.default, overrides, files
            ),
            style,
        ));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Values · Enter edit default · o add override · n new · Esc back "),
        ),
        table,
    );
    if let Some(i) = &v.input {
        let prompt = match &i.edit {
            super::app::ValueEdit::Default(n) => format!(" {n} default "),
            super::app::ValueEdit::Override(n) => format!(" {n} override: TARGET VALUE "),
            super::app::ValueEdit::New => " new variable: NAME VALUE ".to_string(),
        };
        f.render_widget(
            Paragraph::new(format!("{}█", i.text))
                .block(Block::default().borders(Borders::ALL).title(prompt)),
            input,
        );
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.size = (area.width, area.height);
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    if app.values.is_some() {
        draw_values(f, app, body);
    } else if is_single_panel(area) {
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
                Constraint::Length(6)
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
        Panel::Changes => draw_changes(f, app, block, area, expanded),
        Panel::Machines => draw_machines(f, app, block, area, expanded),
        Panel::Git => {
            let text = "Commits · Branches · Remotes · Stash\n(arrives with git support)";
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

fn draw_machines(f: &mut Frame, app: &mut App, block: Block<'static>, area: Rect, expanded: bool) {
    let Some(reg) = app.registry.clone() else {
        return f.render_widget(Paragraph::new("loading…").block(block), area);
    };
    if !expanded {
        let n = reg.machines.len();
        return f.render_widget(Paragraph::new(format!("{n} machine(s)")).block(block), area);
    }
    let items: Vec<ListItem> = reg
        .machines
        .iter()
        .map(|l| {
            let m = &l.machine;
            let mut text = format!(
                "{}{} {}/{}",
                if l.this_machine { "* " } else { "  " },
                m.name,
                m.platform,
                m.distribution
            );
            if m.retired {
                text.push_str(" (retired)");
            }
            let style = if m.retired {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(Line::styled(text, style))
        })
        .collect();
    if items.is_empty() {
        return f.render_widget(Paragraph::new("(no machines)").block(block), area);
    }
    let mut state = ListState::default().with_selected(Some(app.machines_selected));
    f.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
        &mut state,
    );
}

/// `[darwin,!srv1]`: the values of the includes, then `!` and the excludes.
fn assignment_badge(a: &crate::assign::Assignment) -> String {
    let value = |t: &String| t.split_once(':').map_or(t.as_str(), |(_, v)| v).to_string();
    let mut parts: Vec<String> = a.include.iter().map(value).collect();
    parts.extend(a.exclude.iter().map(|t| format!("!{}", value(t))));
    let joined = parts.join(",");
    if joined.chars().count() > 18 {
        format!("[{}…]", joined.chars().take(17).collect::<String>())
    } else {
        format!("[{joined}]")
    }
}

/// `◆2`: the file has 2 versions (default and one variant); `!` when they clash here.
fn variant_badge(app: &App, rel: &str) -> Option<Span<'static>> {
    let Load::Ready(s) = &app.status else {
        return None;
    };
    let v = s.variants.iter().find(|v| v.path == rel)?;
    let (text, colour) = if v.clash.is_some() {
        (format!(" ◆{}!", v.targets.len() + 1), Color::Red)
    } else {
        (format!(" ◆{}", v.targets.len() + 1), Color::Magenta)
    };
    Some(Span::styled(text, Style::default().fg(colour)))
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
                    None => format!("{} NOT IN REGISTRY: dw machine use", m.name),
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
                Line::from(counts_text(app)),
            ]
        }
    }
}

/// `local 2 · staged 1 · incoming 0 · conflicts 0` (New files count as local).
fn counts_text(app: &App) -> String {
    let n = app.change_counts();
    let blocked = match &app.status {
        Load::Ready(s) => s.variants.iter().filter(|v| v.clash.is_some()).count(),
        _ => 0,
    };
    let mut text = format!(
        "local {} · staged {} · incoming {} · conflicts {}",
        n[0], n[1], n[2], n[3]
    );
    if blocked > 0 {
        text.push_str(&format!(" · blocked {blocked}"));
    }
    text
}

fn draw_changes(f: &mut Frame, app: &mut App, block: Block<'static>, area: Rect, expanded: bool) {
    if !expanded {
        let n = app.change_counts();
        let text = format!("L{} S{} I{} C{}", n[0], n[1], n[2], n[3]);
        return f.render_widget(Paragraph::new(text).block(block), area);
    }
    let counts = app.change_counts();
    let mut tabs = Vec::new();
    for (i, name) in TABS.iter().enumerate() {
        let style = if i == app.changes_tab {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        tabs.push(Span::styled(format!(" {name} {} ", counts[i]), style));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [head, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    f.render_widget(Paragraph::new(Line::from(tabs)), head);
    let items: Vec<ListItem> = app
        .change_rows()
        .iter()
        .map(|c| {
            let (mark, colour) = match (c.state, c.kind) {
                (_, "deleted" | "removed") => ("-", Color::Red),
                (_, "new") => ("+", Color::Green),
                (_, "repo") => ("#", Color::Blue),
                _ => ("~", Color::Yellow),
            };
            let name = if c.kind == "repo" {
                format!("repo:{}", c.path)
            } else {
                format!("~/{}", c.path)
            };
            let mut spans = vec![
                Span::styled(format!("{mark} "), Style::default().fg(colour)),
                Span::raw(name),
            ];
            if c.kind != "repo" {
                spans.extend(variant_badge(app, &c.path));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    if items.is_empty() {
        let msg = match app.status {
            Load::Loading => "loading…",
            _ => "nothing here",
        };
        return f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
            body,
        );
    }
    let mut state = ListState::default().with_selected(Some(app.changes_selected));
    f.render_stateful_widget(
        List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        body,
        &mut state,
    );
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
    let assignments = app.registry.as_ref().map(|r| r.assignments.clone());
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
            let mut spans = vec![
                Span::styled(marker, marker_style),
                pending,
                Span::raw(" "),
                Span::styled(name, style),
            ];
            if let Some(a) = assignments.as_ref().and_then(|a| a.get(&rel)) {
                spans.push(Span::styled(
                    format!(" {}", assignment_badge(a)),
                    Style::default().fg(Color::Cyan),
                ));
            }
            if let Some(b) = variant_badge(app, &rel) {
                spans.push(b);
            }
            let item = ListItem::new(Line::from(spans));
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

/// The versions of a variant file and the one this machine receives (empty for other entries).
fn variant_lines(app: &App, e: &super::files::Entry) -> String {
    let Load::Ready(s) = &app.status else {
        return String::new();
    };
    let rel = app.files.rel_of(e);
    let Some(v) = s.variants.iter().find(|v| v.path == rel) else {
        return String::new();
    };
    let mut out = String::from("Versions (t adds one):\n");
    let mark = |t: &str| {
        if v.receives.as_deref() == Some(t) {
            "  ← this machine receives this"
        } else {
            ""
        }
    };
    out.push_str(&format!("  default{}\n", mark("default")));
    for t in &v.targets {
        out.push_str(&format!("  {t}{}\n", mark(t)));
    }
    if let Some(c) = &v.clash {
        out.push_str(&format!("  BLOCKED on this machine: {c}\n"));
    }
    out.push('\n');
    out
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
    if app.focus == Panel::Changes {
        let body = match &app.diff {
            Load::Loading => ratatui::text::Text::from("loading…"),
            Load::Failed(e) => ratatui::text::Text::from(e.clone()),
            Load::Ready(t) if t.lines.is_empty() => ratatui::text::Text::from("No change selected"),
            Load::Ready(t) => t.clone(),
        };
        let title = app
            .selected_change()
            .map_or(" Diff ".to_string(), |c| format!(" {} ", c.path));
        return f.render_widget(Paragraph::new(body).block(block.title(title)), area);
    }
    let text: String = match app.focus {
        Panel::Files => {
            let head = app.files.selected_entry().map_or(String::new(), |e| {
                format!("{}\n{}\n", state_text(app, e), variant_lines(app, e))
            });
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
        Panel::Changes => String::new(),
        Panel::Git => "Commit and branch details will appear here.".into(),
        Panel::Machines => match &app.machine_preview {
            Some((_, Load::Loading)) => "loading…".into(),
            Some((_, Load::Failed(e))) => e.clone(),
            Some((_, Load::Ready(t))) => t.clone(),
            None => "Enter previews the tracked files the selected machine would receive.".into(),
        },
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
                            match c.action.as_str() {
                                "add" => "+",
                                "change" => "~",
                                _ => "-",
                            },
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
                for d in (i.preview.data.iter().chain(&i.preview.rules))
                    .chain(i.preview.effects.iter().take(8))
                {
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
        Dialog::Commit(b) => {
            let mut l: Vec<Line> = b
                .text
                .split('\n')
                .map(|t| Line::from(t.to_string()))
                .collect();
            if let Some(last) = l.last_mut() {
                last.spans
                    .push(Span::styled("█", Style::default().fg(Color::Yellow)));
            }
            l.push(Line::from(""));
            l.push(Line::styled(
                "Enter commits · Ctrl+J / Alt+Enter new line · Esc cancels",
                Style::default().fg(Color::DarkGray),
            ));
            (if b.amend { " Amend " } else { " Commit " }, l)
        }
        Dialog::ConfirmApply(sel) => (
            " Apply ",
            vec![
                Line::from(match sel {
                    Some(p) if p.len() == 1 => format!("Write ~/{} to HOME?", p[0]),
                    Some(p) => format!("Write {} files to HOME?", p.len()),
                    None => "Write every incoming file to HOME?".to_string(),
                }),
                Line::from("A snapshot is taken first."),
                Line::styled(
                    "y applies · n / Esc cancels",
                    Style::default().fg(Color::Yellow),
                ),
            ],
        ),
        Dialog::ConfirmPush => (
            " Push ",
            vec![
                Line::from("Push the branch to its remote?"),
                Line::from("Public remotes are refused."),
                Line::styled(
                    "y pushes · n / Esc cancels",
                    Style::default().fg(Color::Yellow),
                ),
            ],
        ),
        Dialog::Assign(e) => {
            let what = if e.paths.len() == 1 {
                format!("~/{}", e.paths[0])
            } else {
                format!("{} paths", e.paths.len())
            };
            let mut body: Vec<Line> = Vec::new();
            let mut cursor_line = 0;
            let mut group = "";
            for (i, r) in e.rows.iter().enumerate() {
                if r.group != group {
                    group = r.group;
                    body.push(Line::styled(group, Style::default().fg(Color::Blue)));
                }
                let (mark, colour) = match r.state {
                    _ if e.variant => ("      ", Color::Reset),
                    crate::assign::Tri::None => ("[ ] —  ", Color::DarkGray),
                    crate::assign::Tri::Include => ("[+] incl", Color::Green),
                    crate::assign::Tri::Exclude => ("[x] excl", Color::Red),
                    crate::assign::Tri::Mixed => ("[~] mixed", Color::Yellow),
                };
                let value = r.target.split_once(':').map_or("", |(_, v)| v);
                let mut style = Style::default().fg(colour);
                if i == e.cursor {
                    style = style.add_modifier(Modifier::REVERSED);
                    cursor_line = body.len();
                }
                body.push(Line::from(Span::styled(
                    format!("  {mark:<10} {value}"),
                    style,
                )));
            }
            // Title, blank and footer take 3 lines; the rest scrolls around the cursor.
            let window = usize::from(area.height.saturating_sub(4 + 3)).max(1);
            if body.len() > window {
                let start = cursor_line
                    .saturating_sub(window / 2)
                    .min(body.len() - window);
                body = body.into_iter().skip(start).take(window).collect();
            }
            let mut l = vec![Line::styled(
                if e.variant {
                    format!("New variant of {what} from its current content")
                } else {
                    format!("Assign {what}")
                },
                Style::default().add_modifier(Modifier::BOLD),
            )];
            l.extend(body);
            l.push(Line::from(""));
            l.push(Line::styled(
                if e.variant {
                    "j/k move · Enter add variant item · Esc"
                } else {
                    "Space cycle · i incl · x excl · - none · Enter add item · Esc"
                },
                Style::default().fg(Color::Yellow),
            ));
            (if e.variant { " Variant " } else { " Assign " }, l)
        }
        Dialog::ConfirmDiscard(paths) => (
            " Discard ",
            vec![
                Line::from(format!(
                    "Overwrite {} with the committed version?",
                    if paths.len() == 1 {
                        format!("~/{}", paths[0])
                    } else {
                        format!("{} files", paths.len())
                    }
                )),
                Line::from("A snapshot is taken first."),
                Line::styled(
                    "y discards · n / Esc cancels",
                    Style::default().fg(Color::Yellow),
                ),
            ],
        ),
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
    let h = (HELP.len() as u16 + 2).min(area.height);
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
