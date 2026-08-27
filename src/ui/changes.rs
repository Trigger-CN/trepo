use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::change_tree::{change_tree_rows, ChangeTreeRow};
use crate::app::state::{App, ChangesMode, ChangesState};
use crate::domain::OperationTarget;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 60 || area.height < 14 {
        frame.render_widget(
            Paragraph::new("Terminal too small. Resize to at least 60x14. Press Esc to return.")
                .block(Block::default().title(" Changes ").borders(Borders::ALL)),
            area,
        );
        return;
    }
    let Some(changes) = &app.changes else {
        frame.render_widget(Paragraph::new("No repository selected"), area);
        return;
    };

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    render_header(frame, changes, vertical[0]);
    if area.width >= 100 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(vertical[1]);
        render_files(frame, changes, body[0]);
        render_preview(frame, changes, body[1]);
    } else {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(vertical[1]);
        render_files(frame, changes, body[0]);
        render_preview(frame, changes, body[1]);
    }
    render_footer(frame, changes, vertical[2]);
    if changes.confirmation.is_some() {
        render_confirmation(frame, changes);
    }
    if changes.commit_editing {
        render_commit_dialog(frame, changes);
    }
}

fn render_header(frame: &mut Frame, changes: &ChangesState, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " repo-tui ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}  /  Changes", changes.project.name)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_files(frame: &mut Frame, changes: &ChangesState, area: Rect) {
    if let Some(error) = &changes.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(Color::Red))
                .block(Block::default().title(" Files ").borders(Borders::ALL))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let tree_rows = change_tree_rows(&changes.entries);
    let selected_row = tree_rows
        .iter()
        .position(|row| matches!(row, ChangeTreeRow::File { entry_index, .. } if *entry_index == changes.selected))
        .unwrap_or(0);
    let visible = area.height.saturating_sub(3) as usize;
    let start = viewport_start(selected_row, visible, tree_rows.len());
    let rows = tree_rows
        .into_iter()
        .skip(start)
        .take(visible)
        .map(|row| match row {
            ChangeTreeRow::Directory { .. } => Row::new(vec![
                Cell::from(" "),
                Cell::from(" "),
                Cell::from(" "),
                Cell::from(row.display()),
            ])
            .style(Style::default().fg(Color::DarkGray)),
            ChangeTreeRow::File { entry_index, .. } => {
                let entry = &changes.entries[entry_index];
                let selected = entry_index == changes.selected;
                let checked = changes.selected_files.contains(&entry.path);
                let style = if selected {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else if entry.conflicted {
                    Style::default().fg(Color::LightRed)
                } else if entry.untracked {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(if selected { ">" } else { " " }),
                    Cell::from(if checked { "[x]" } else { "[ ]" }),
                    Cell::from(entry.status_label()),
                    Cell::from(row.display()),
                ])
                .style(style)
            }
        });
    let title = if changes.loading {
        " Files (loading) ".to_owned()
    } else {
        format!(
            " Files ({}; {} selected) ",
            changes.entries.len(),
            changes.selected_files.len()
        )
    };
    let border_style = if changes.mode == ChangesMode::File {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(12),
        ],
    )
    .header(
        Row::new(["", "Sel", "XY", "Tree"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1)
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style),
    );
    frame.render_widget(table, area);
}

fn render_preview(frame: &mut Frame, changes: &ChangesState, area: Rect) {
    let title = changes.preview_path.as_ref().map_or_else(
        || " Diff ".to_owned(),
        |path| match changes.mode {
            ChangesMode::Hunk => changes
                .preview
                .as_ref()
                .and_then(|preview| preview.hunks.get(changes.selected_hunk))
                .map_or_else(
                    || format!(" Diff: {} [hunk] ", path.display()),
                    |hunk| {
                        format!(
                            " Diff: {} [hunk {}/{} {}] ",
                            path.display(),
                            changes.selected_hunk + 1,
                            changes
                                .preview
                                .as_ref()
                                .map_or(0, |preview| preview.hunks.len()),
                            hunk.source.label()
                        )
                    },
                ),
            ChangesMode::Line => changes
                .preview
                .as_ref()
                .and_then(|preview| preview.lines.get(changes.selected_line))
                .map_or_else(
                    || format!(" Diff: {} [line] ", path.display()),
                    |line| {
                        format!(
                            " Diff: {} [line {}/{} {}] ",
                            path.display(),
                            changes.selected_line + 1,
                            changes
                                .preview
                                .as_ref()
                                .map_or(0, |preview| preview.lines.len()),
                            line.source.label()
                        )
                    },
                ),
            ChangesMode::File => format!(" Diff: {} [file] ", path.display()),
        },
    );
    let border_style = if changes.mode != ChangesMode::File {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);
    if changes.preview_loading {
        frame.render_widget(Paragraph::new("Loading diff...").block(block), area);
        return;
    }
    let Some(preview) = &changes.preview else {
        frame.render_widget(
            Paragraph::new(if changes.entries.is_empty() {
                "Working tree is clean."
            } else {
                "No diff is available."
            })
            .block(block),
            area,
        );
        return;
    };
    let selected_range = match changes.mode {
        ChangesMode::Hunk => preview
            .hunks
            .get(changes.selected_hunk)
            .map(|hunk| hunk.display_start..=hunk.display_end),
        ChangesMode::Line => preview
            .lines
            .get(changes.selected_line)
            .map(|line| line.display_line..=line.display_line),
        ChangesMode::File => None,
    };
    let lines = preview.text.lines().enumerate().map(|(index, line)| {
        let mut style = if line.starts_with("+++") || line.starts_with("---") {
            Style::default().fg(Color::Yellow)
        } else if line.starts_with('+') {
            Style::default().fg(Color::Green)
        } else if line.starts_with('-') {
            Style::default().fg(Color::Red)
        } else if line.starts_with("@@") || line.starts_with("== ") {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        if selected_range
            .as_ref()
            .is_some_and(|range| range.contains(&index))
        {
            style = style.bg(Color::DarkGray);
            if line.starts_with("@@") {
                style = style.add_modifier(Modifier::BOLD);
            }
        }
        Line::styled(line.to_owned(), style)
    });
    let scroll = u16::try_from(changes.preview_scroll).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(Text::from_iter(lines))
            .block(block)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame, changes: &ChangesState, area: Rect) {
    let first = if changes.commit_running {
        Span::styled("Committing...", Style::default().fg(Color::Yellow))
    } else if changes.operation_running {
        Span::styled("Writing...", Style::default().fg(Color::Yellow))
    } else if let Some((is_error, message)) = &changes.message {
        Span::styled(
            message.clone(),
            Style::default().fg(if *is_error { Color::Red } else { Color::Green }),
        )
    } else {
        Span::raw(
            "Space Select   A All   s Stage   u Unstage   d Discard   m Commit   PgUp/PgDn Diff",
        )
    };
    let mode = match changes.mode {
        ChangesMode::File => "FILE",
        ChangesMode::Hunk => "HUNK",
        ChangesMode::Line => "LINE",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(first),
            Line::raw(format!(
                "[{mode}] Tab Mode   j/k Move   g/G First/last   r Refresh   Esc Back   ? Help"
            )),
        ]),
        area,
    );
}

fn render_confirmation(frame: &mut Frame, changes: &ChangesState) {
    let Some(pending) = &changes.confirmation else {
        return;
    };
    let area = centered_rect(64, 30, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                match pending.target {
                    OperationTarget::File => {
                        "This permanently discards all worktree changes in the file."
                    }
                    OperationTarget::Hunk { .. } => {
                        "This permanently discards the selected worktree hunk."
                    }
                    OperationTarget::Line { .. } => {
                        "This permanently discards the selected worktree line."
                    }
                },
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::raw(format!("File: {}", pending.change.path.display())),
            Line::raw(match pending.target {
                OperationTarget::File => "Scope: entire file".to_owned(),
                OperationTarget::Hunk { source, .. } => {
                    format!("Scope: selected {} hunk", source.label())
                }
                OperationTarget::Line { source, .. } => {
                    format!("Scope: selected {} line", source.label())
                }
            }),
            Line::raw(""),
            Line::raw("Press y to discard or n/Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(" Confirm destructive operation ")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_commit_dialog(frame: &mut Frame, changes: &ChangesState) {
    let area = centered_rect(76, 58, frame.area());
    frame.render_widget(Clear, area);
    let title = if changes.commit_amend {
        " Commit (amend) "
    } else {
        " Commit "
    };
    let options = format!(
        "[Ctrl-A] amend: {}   [Ctrl-U] sign-off: {}   [Ctrl-G] signing: {}",
        if changes.commit_amend { "on" } else { "off" },
        if changes.commit_signoff { "on" } else { "off" },
        if changes.commit_signing { "on" } else { "off" },
    );
    let mut text = vec![Line::styled(
        "Commit message:",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    if changes.commit_message.is_empty() {
        text.push(Line::raw("_"));
    } else {
        let mut lines = changes.commit_message.split('\n').peekable();
        while let Some(line) = lines.next() {
            text.push(Line::raw(if lines.peek().is_none() {
                format!("{line}_")
            } else {
                line.to_owned()
            }));
        }
    }
    text.extend([
        Line::raw(""),
        Line::raw(options),
        Line::raw(""),
        Line::raw("Enter newline   Ctrl-Enter/Ctrl-S commit   Esc cancel"),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let width = area.width.saturating_mul(percent_x) / 100;
    let height = area.height.saturating_mul(percent_y) / 100;
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.max(1),
        height.max(1),
    )
}

fn viewport_start(selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible {
        0
    } else {
        selected.saturating_sub(visible / 2).min(total - visible)
    }
}
