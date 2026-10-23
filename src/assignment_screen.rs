//! Inline applicability controls for Details → Rules.
use crate::{
    assignments::{self, Os, Scope},
    backend::{Row, Snapshot},
};
use ratatui::{prelude::*, widgets::*};

const MINT: Color = Color::Rgb(97, 218, 179);
const MUTED: Color = Color::Rgb(160, 168, 180);

#[derive(Clone, Debug)]
pub enum Command {
    Os(Os),
    AllMachines,
    ThisMachine,
    CustomMachines,
    ToggleMachine(String),
    Review,
    Reset,
}
#[derive(Default)]
pub struct Areas {
    pub controls: Vec<(Rect, Command)>,
    pub max_scroll: u16,
}

fn text(lines: &mut Vec<Line<'static>>, value: &str, width: usize, style: Style) {
    let mut line = String::new();
    for word in value.split_whitespace() {
        if !line.is_empty() && Line::from(format!("{line} {word}")).width() > width {
            lines.push(Line::styled(std::mem::take(&mut line), style));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        for ch in word.chars() {
            if !line.is_empty() && Line::from(format!("{line}{ch}")).width() > width {
                lines.push(Line::styled(std::mem::take(&mut line), style));
            }
            line.push(ch);
        }
    }
    if !line.is_empty() {
        lines.push(Line::styled(line, style));
    }
}

fn choices(
    lines: &mut Vec<Line<'static>>,
    hits: &mut Vec<(usize, usize, usize, Command)>,
    width: usize,
    options: &[(&str, bool, Command)],
) {
    let mut spans = Vec::new();
    let mut x = 0;
    for (label, selected, command) in options {
        let label = format!(
            "[{}{}]",
            if *selected && !matches!(command, Command::Review) {
                "● "
            } else {
                ""
            },
            label
        );
        let size = Line::from(label.clone()).width();
        if x > 0 && x + size > width {
            lines.push(Line::from(std::mem::take(&mut spans)));
            x = 0;
        }
        hits.push((x, lines.len(), size.min(width), command.clone()));
        spans.push(Span::styled(
            label,
            if *selected {
                Style::default().fg(MINT).bold()
            } else {
                Style::default().fg(Color::LightCyan)
            },
        ));
        spans.push(Span::raw(" "));
        x += size + 1;
    }
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
}

pub fn draw(
    f: &mut Frame,
    area: Rect,
    row: Option<&Row>,
    snapshot: &Snapshot,
    scope: &Scope,
    dirty: bool,
    scroll: u16,
) -> Areas {
    let Some(row) = row else {
        f.render_widget(
            Paragraph::new("Select a file or folder in Files to see its assignments.")
                .wrap(Wrap { trim: false }),
            area,
        );
        return Areas::default();
    };
    let width = usize::from(area.width.saturating_sub(1)).max(1);
    let muted = Style::default().fg(MUTED);
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    text(
        &mut lines,
        &format!("~/{}{}", row.path, if row.directory { "/" } else { "" }),
        width,
        Style::default().bold(),
    );
    if !row.managed {
        lines.push(Line::default());
        text(
            &mut lines,
            "Capture this file or folder first, then choose which operating systems and machines use the saved copy.",
            width,
            muted,
        );
    } else if snapshot.identity.hostname.is_empty() {
        text(&mut lines, "Inspecting machine identity…", width, muted);
    } else {
        lines.push(Line::default());
        text(
            &mut lines,
            "Operating systems · o cycle",
            width,
            Style::default().fg(MINT).bold(),
        );
        choices(
            &mut lines,
            &mut hits,
            width,
            &[
                ("All", scope.os == Os::All, Command::Os(Os::All)),
                ("Darwin", scope.os == Os::Darwin, Command::Os(Os::Darwin)),
                ("Linux", scope.os == Os::Linux, Command::Os(Os::Linux)),
            ],
        );
        lines.push(Line::default());
        text(
            &mut lines,
            "Machines · m toggle · M choose",
            width,
            Style::default().fg(MINT).bold(),
        );
        let this_machine =
            scope.machines.len() == 1 && scope.machines.contains(&snapshot.identity.hostname);
        choices(
            &mut lines,
            &mut hits,
            width,
            &[
                (
                    "All machines",
                    scope.machines.is_empty(),
                    Command::AllMachines,
                ),
                ("This machine", this_machine, Command::ThisMachine),
                (
                    "Choose…",
                    !scope.machines.is_empty() && !this_machine,
                    Command::CustomMachines,
                ),
            ],
        );
        let known: std::collections::BTreeSet<_> = snapshot
            .assignments
            .values()
            .flat_map(|assignment| assignment.machines.iter().cloned())
            .chain(scope.machines.iter().cloned())
            .chain(std::iter::once(snapshot.identity.hostname.clone()))
            .collect();
        if known.len() > 1 {
            text(&mut lines, "Known machines · click to toggle", width, muted);
            let options = known
                .iter()
                .map(|name| {
                    (
                        name.as_str(),
                        scope.machines.contains(name),
                        Command::ToggleMachine(name.clone()),
                    )
                })
                .collect::<Vec<_>>();
            choices(&mut lines, &mut hits, width, &options);
        }
        text(
            &mut lines,
            &format!("Assigned: {}", scope.machine_label()),
            width,
            Style::default(),
        );
        text(
            &mut lines,
            &format!(
                "This machine: {} ({})",
                snapshot.identity.hostname, snapshot.identity.os
            ),
            width,
            muted,
        );
        let parent = std::path::Path::new(&row.path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or_default();
        let includes = scope.allows(&snapshot.identity)
            && assignments::applies(&snapshot.assignments, parent, &snapshot.identity);
        text(
            &mut lines,
            if includes {
                "Assignment includes this machine."
            } else {
                "Assignment skips this machine; originals stay unchanged."
            },
            width,
            Style::default().fg(if includes { MINT } else { Color::Yellow }),
        );
        for (parent, inherited) in snapshot
            .assignments
            .iter()
            .filter(|(p, _)| row.path.starts_with(&format!("{p}/")))
        {
            text(
                &mut lines,
                &format!(
                    "Also restricted by ~/{parent}/: {} · {}",
                    inherited.os.label(),
                    inherited.machine_label()
                ),
                width,
                muted,
            );
        }
        lines.push(Line::default());
        choices(
            &mut lines,
            &mut hits,
            width,
            &[
                (
                    if width < 24 {
                        "s Review"
                    } else {
                        "s Review & save"
                    },
                    dirty,
                    Command::Review,
                ),
                (
                    if width < 24 {
                        "u Reset"
                    } else {
                        "u Reset edits"
                    },
                    false,
                    Command::Reset,
                ),
            ],
        );
        if dirty {
            lines.push(Line::from("Unsaved assignment changes").fg(Color::Yellow));
        }
        lines.push(Line::default());
        text(
            &mut lines,
            "Folder assignments include descendants. Other native ignore rules still apply. Commit and push to share assignments.",
            width,
            muted,
        );
        if snapshot.scoped_out.contains_key(&row.path) {
            lines.push(Line::default());
            text(
                &mut lines,
                "Saved copy is excluded here. It stays visible so you can change its assignment.",
                width,
                muted,
            );
        }
    }
    let max_scroll = lines
        .len()
        .saturating_sub(area.height as usize)
        .min(u16::MAX as usize) as u16;
    let scroll = scroll.min(max_scroll);
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)),
        Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height),
    );
    if max_scroll > 0 {
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area,
            &mut ScrollbarState::new(usize::from(max_scroll) + 1)
                .position(usize::from(scroll))
                .viewport_content_length(area.height as usize),
        );
    }
    let controls = hits
        .into_iter()
        .filter_map(|(x, y, width, command)| {
            let y = y as i64 - i64::from(scroll);
            (y >= 0 && y < i64::from(area.height)).then_some((
                Rect::new(
                    area.x.saturating_add(x as u16),
                    area.y.saturating_add(y.max(0) as u16),
                    width as u16,
                    1,
                ),
                command,
            ))
        })
        .collect();
    Areas {
        controls,
        max_scroll,
    }
}
