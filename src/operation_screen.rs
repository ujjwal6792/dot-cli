//! Human-readable operation reviews and receipts; execution stays in operations.
use crate::{
    config::Config,
    operations::{Action, Outcome, Plan},
};
use crossterm::event::KeyCode;
use ratatui::{prelude::*, widgets::*};
use std::{collections::BTreeMap, path::Path};

const BG: Color = Color::Rgb(24, 26, 31);
const INK: Color = Color::Rgb(235, 239, 243);
const MUTED: Color = Color::Rgb(160, 168, 180);
const MINT: Color = Color::Rgb(97, 218, 179);
const RED: Color = Color::Rgb(255, 140, 140);
const AMBER: Color = Color::Rgb(240, 198, 116);

#[derive(Clone, Debug, Default)]
pub struct View {
    pub tab: usize,
    pub scroll: u16,
}

impl View {
    pub fn key(&mut self, key: KeyCode, max_scroll: u16) {
        let tab = match key {
            KeyCode::Char('1') => 0,
            KeyCode::Char('2') => 1,
            KeyCode::Tab | KeyCode::BackTab => 1 - self.tab,
            _ => self.tab,
        };
        if tab != self.tab {
            self.tab = tab;
            self.scroll = 0;
        } else {
            self.scroll = match key {
                KeyCode::Down | KeyCode::Char('j') => self.scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll.saturating_sub(1),
                KeyCode::PageDown => self.scroll.saturating_add(10),
                KeyCode::PageUp => self.scroll.saturating_sub(10),
                KeyCode::Home => 0,
                KeyCode::End => max_scroll,
                _ => self.scroll,
            }
            .min(max_scroll);
        }
    }
}

#[derive(Default)]
pub struct Areas {
    pub controls: Vec<(Rect, KeyCode)>,
    pub max_scroll: u16,
    pub body: Rect,
}

fn name(action: Action) -> &'static str {
    match action {
        Action::Capture => "Capture",
        Action::Apply => "Apply",
        Action::Restore => "Restore",
        Action::Forget => "Stop tracking",
        Action::Ignore => "Ignore",
        Action::IgnoreDefaults => "Add ignore defaults",
        Action::Enable => "Enable updates",
        Action::Disable => "Disable updates",
        Action::Stage => "Stage",
        Action::Unstage => "Unstage",
        Action::Discard => "Discard",
        Action::Commit => "Commit",
        Action::Fetch => "Fetch",
        Action::Pull => "Pull",
        Action::Push => "Push",
        Action::EditSource => "Edit saved settings",
        Action::Setting => "Change setting",
    }
}

fn file_count(n: usize) -> String {
    format!("{n} {}", if n == 1 { "file" } else { "files" })
}

// Group paths for scanning, but keep every exact target visible and scrollable.
fn files(paths: &[String], label: &str, color: Color) -> Vec<Line<'static>> {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for path in paths {
        let p = Path::new(path);
        let parent = p
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| format!("{}/", p.display()))
            .unwrap_or_else(|| "./".into());
        let file = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        groups.entry(parent).or_default().push(file);
    }
    let mut lines = vec![];
    for (folder, mut names) in groups {
        names.sort();
        lines.push(Line::from(folder).fg(MUTED).bold());
        for file in names {
            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<8}"), Style::default().fg(color)),
                Span::raw(file),
            ]));
        }
    }
    lines
}

pub fn review(f: &mut Frame, area: Rect, c: &Config, plan: &Plan, view: &View) -> Areas {
    let capture = plan.action == Action::Capture;
    let forget = plan.action == Action::Forget;
    let ignore = matches!(plan.action, Action::Ignore | Action::IgnoreDefaults);
    let title = if capture {
        "Save a copy of your settings".into()
    } else if forget {
        "Remove saved copies from dot".into()
    } else {
        format!("Review {}", name(plan.action).to_lowercase())
    };
    let mut header = vec![
        Line::from(title).bold(),
        Line::from(if capture {
            format!(
                "{} selected  ·  Current settings → Saved copies",
                file_count(plan.paths.len())
            )
        } else {
            format!("{} selected", file_count(plan.paths.len()))
        })
        .fg(MINT),
    ];
    if capture {
        header.push(Line::from("Your apps will keep using their current settings.").fg(MUTED));
        header.push(Line::from(format!("Save to  {}", c.source.display())).fg(MUTED));
    } else if forget {
        header.push(Line::from("Your original files stay on this computer.").fg(MINT));
        header.push(Line::from("Dot will stop tracking these files.").fg(MUTED));
    }
    if ignore {
        header.push(
            Line::from(if plan.action == Action::Ignore {
                "Ignore in chezmoi and Git; stop tracking saved copies."
            } else {
                "Add developer defaults to .chezmoiignore and .gitignore."
            })
            .fg(MINT),
        );
        header.push(
            Line::from("No files will be deleted. Read Changes for the rules and how to undo.")
                .fg(MUTED),
        );
    }
    let body = if view.tab == 0 {
        let mut lines = vec![
            Line::from(if capture {
                "These files will be copied into your settings folder."
            } else if forget {
                "Only the saved copies below will be removed from your settings folder."
            } else {
                "This review covers the following exact targets."
            })
            .fg(MUTED),
            Line::from(""),
        ];
        if forget {
            lines.push(
                Line::from(
                    "Publishing this change also removes these copies from the shared repository.",
                )
                .fg(MUTED),
            );
            lines.push(Line::from(""));
        }
        lines.extend(files(
            &plan.paths,
            if ignore {
                "IGNORE"
            } else if capture {
                "COPY"
            } else if forget {
                "REMOVE"
            } else {
                "TARGET"
            },
            if forget { AMBER } else { MINT },
        ));
        if plan.paths.is_empty() {
            lines.push(Line::from(
                "This action operates on repository state. Read Changes for its scope.",
            ));
        }
        Text::from(lines)
    } else {
        let mut lines = vec![
            Line::from(if capture {
                "Review the content that will be saved."
            } else {
                "Review the changes before confirming."
            })
            .fg(MUTED),
            Line::from(""),
        ];
        if let Some(payload) = &plan.payload
            && plan.action == Action::Commit
        {
            lines.push(Line::from(format!("Commit message: {payload}")));
            lines.push(Line::from(""));
        }
        for line in plan.preview.lines() {
            let color = if line.starts_with("+++")
                || line.starts_with("---")
                || line.starts_with("diff ")
                || line.starts_with("NEW source from HOME:")
            {
                INK
            } else if line.starts_with('+') {
                MINT
            } else if line.starts_with('-') {
                RED
            } else if line.starts_with("@@") {
                AMBER
            } else {
                INK
            };
            let display = if let Some(path) = line.strip_prefix("NEW source from HOME: ") {
                format!("New saved copy: {path}")
            } else {
                line.to_owned()
            };
            lines.push(Line::from(display).fg(color));
        }
        if plan.preview.is_empty() {
            lines.push(Line::from("No content differences to display.").fg(MUTED));
        }
        Text::from(lines)
    };
    let confirm = if forget {
        "y Remove copies".into()
    } else {
        format!("y {}", name(plan.action))
    };
    draw(
        f,
        area,
        header,
        ["1 Files", "2 Changes"],
        body,
        view,
        Controls {
            buttons: &[(&confirm, KeyCode::Char('y')), ("Esc Cancel", KeyCode::Esc)],
            accent: if forget { AMBER } else { MINT },
        },
    )
}

pub fn result(f: &mut Frame, area: Rect, c: &Config, outcome: &Outcome, view: &View) -> Areas {
    let incomplete = outcome.failed.is_some() || !outcome.not_attempted.is_empty();
    let capture = outcome.action == Action::Capture;
    let forget = outcome.action == Action::Forget;
    let ignore = outcome.action == Action::Ignore;
    let accent = if incomplete { AMBER } else { MINT };
    let title = if incomplete {
        format!("{} stopped", name(outcome.action))
    } else if capture {
        "Your settings are saved".into()
    } else if forget {
        "Saved copies removed from dot".into()
    } else if ignore {
        "Ignored in chezmoi and Git".into()
    } else {
        format!("{} complete", name(outcome.action))
    };
    let done = if capture {
        "saved"
    } else if forget {
        "no longer tracked"
    } else if ignore {
        "ignored"
    } else {
        "completed"
    };
    let mut header = vec![
        Line::from(title).bold().fg(accent),
        Line::from(if incomplete {
            format!(
                "{} {done}  ·  {} failed  ·  {} skipped",
                outcome.succeeded.len(),
                usize::from(outcome.failed.is_some()),
                outcome.not_attempted.len()
            )
        } else if outcome.succeeded.is_empty() {
            "Completed successfully".into()
        } else {
            format!("{} {done}", file_count(outcome.succeeded.len()))
        }),
    ];
    if capture {
        header.push(Line::from("Your apps still use their current settings.").fg(MUTED));
        header.push(Line::from(format!("Saved copies  {}", c.source.display())).fg(MUTED));
    } else if forget || ignore {
        header.push(Line::from("Your original files are still on this computer.").fg(MUTED));
    }
    let body = if view.tab == 1 {
        Text::from(serde_json::to_string_pretty(outcome).unwrap_or_default())
    } else {
        let mut lines = vec![];
        // Failures come first, even when hundreds of files succeeded.
        if incomplete {
            lines.push(Line::from("Needs attention").bold().fg(AMBER));
            if let Some(failed) = &outcome.failed {
                lines.push(Line::from(format!("Failed: {failed}")).fg(RED));
            }
            if !outcome.message.is_empty() {
                lines.push(Line::from(outcome.message.clone()).fg(RED));
            }
            lines.push(
                Line::from(
                    "Completed changes remain in place. Fix the issue, then review a new plan.",
                )
                .fg(MUTED),
            );
            lines.push(Line::from(""));
        } else if capture {
            lines.push(
                Line::from("Next: review your saved copies in Git, then commit when ready.")
                    .fg(MUTED),
            );
            lines.push(Line::from(""));
        } else if forget {
            lines.push(
                Line::from("Review the removal in Git before committing or publishing it.")
                    .fg(MUTED),
            );
            lines.push(Line::from(""));
        } else if !outcome.message.is_empty() {
            lines.push(Line::from(outcome.message.clone()));
            lines.push(Line::from(""));
        }
        if !outcome.succeeded.is_empty() {
            lines.push(
                Line::from(if capture {
                    "Saved files"
                } else if forget {
                    "No longer tracked"
                } else {
                    "Completed files"
                })
                .bold(),
            );
            lines.extend(files(
                &outcome.succeeded,
                if capture {
                    "SAVED"
                } else if forget {
                    "REMOVED"
                } else {
                    "DONE"
                },
                MINT,
            ));
        }
        if !outcome.not_attempted.is_empty() {
            lines.push(Line::from(""));
            lines.push(
                Line::from(format!(
                    "{} skipped",
                    file_count(outcome.not_attempted.len())
                ))
                .bold(),
            );
            lines.extend(files(&outcome.not_attempted, "SKIPPED", AMBER));
        }
        Text::from(lines)
    };
    draw(
        f,
        area,
        header,
        ["1 Files", "2 Details"],
        body,
        view,
        Controls {
            buttons: &[
                ("Enter Done", KeyCode::Enter),
                ("g Review in Git", KeyCode::Char('g')),
            ],
            accent,
        },
    )
}

struct Controls<'a> {
    buttons: &'a [(&'a str, KeyCode)],
    accent: Color,
}

fn draw(
    f: &mut Frame,
    content: Rect,
    header: Vec<Line<'static>>,
    tabs: [&str; 2],
    body: Text<'static>,
    view: &View,
    controls: Controls<'_>,
) -> Areas {
    let Controls { buttons, accent } = controls;
    let width = content.width;
    let height = content.height;
    let header = Paragraph::new(header).wrap(Wrap { trim: false });
    let header_height = (header.line_count(width) as u16).min(if height < 10 {
        1
    } else if height < 16 {
        2
    } else {
        7
    });
    let button_width = buttons
        .iter()
        .map(|(label, _)| label.chars().count())
        .sum::<usize>()
        + buttons.len().saturating_sub(1) * 3;
    let button_rows = if button_width > width as usize { 2 } else { 1 };
    let regions = Layout::vertical([
        Constraint::Length(header_height + u16::from(height >= 16)),
        Constraint::Length(if height < 16 { 1 } else { 2 }),
        Constraint::Min(1),
        Constraint::Length(button_rows + 1),
    ])
    .split(content);
    f.render_widget(header, regions[0]);
    let mut areas = Areas {
        body: regions[2],
        ..Areas::default()
    };
    let mut x = regions[1].x;
    for (i, label) in tabs.into_iter().enumerate() {
        let display = if width < 48 {
            label.to_owned()
        } else {
            format!("  {label}  ")
        };
        let w = (display.chars().count() as u16).min(regions[1].right().saturating_sub(x));
        let rect = Rect::new(x, regions[1].y, w, 1);
        f.render_widget(
            Paragraph::new(display).style(if i == view.tab {
                Style::default().fg(BG).bg(accent).bold()
            } else {
                Style::default().fg(MUTED)
            }),
            rect,
        );
        areas
            .controls
            .push((rect, KeyCode::Char(if i == 0 { '1' } else { '2' })));
        x = x.saturating_add(w + 1);
    }
    let paragraph = Paragraph::new(body).wrap(Wrap { trim: false });
    areas.max_scroll = paragraph
        .line_count(regions[2].width)
        .saturating_sub(regions[2].height as usize)
        .min(u16::MAX as usize) as u16;
    f.render_widget(
        paragraph.scroll((view.scroll.min(areas.max_scroll), 0)),
        regions[2],
    );
    let button_area = Rect::new(
        regions[3].x,
        regions[3].y,
        regions[3].width,
        regions[3].height.saturating_sub(1),
    );
    let controls = crate::dialog::controls(f, button_area, buttons);
    if let Some((rect, _)) = controls.first() {
        f.buffer_mut()
            .set_style(*rect, Style::default().fg(BG).bg(accent).bold());
    }
    areas.controls.extend(controls);
    let hint = if width < 48 {
        "↑/↓ Scroll · Tab View"
    } else {
        "↑/↓ Scroll   PgUp/PgDn Page   Tab Switch view"
    };
    let hint = if areas.max_scroll > 0 {
        format!(
            "{hint}   {}/{}",
            view.scroll.min(areas.max_scroll) + 1,
            areas.max_scroll + 1
        )
    } else {
        hint.into()
    };
    f.render_widget(
        Paragraph::new(hint).fg(MUTED),
        Rect::new(
            regions[3].x,
            regions[3].bottom().saturating_sub(1),
            regions[3].width,
            1,
        ),
    );
    areas
}
