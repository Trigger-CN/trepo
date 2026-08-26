use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::state::App;
use crate::domain::{HeadState, RepoProjectState, ScanState, WorkspaceKind};

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 60 || area.height < 12 {
        frame.render_widget(
            Paragraph::new("Terminal too small. Resize to at least 60x12. Press q to quit.")
                .block(Block::default().title(" repo-tui ").borders(Borders::ALL))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);
    render_header(frame, app, vertical[0]);

    if area.width >= 105 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
            .split(vertical[1]);
        render_table(frame, app, body[0]);
        render_inspector(frame, app, body[1]);
    } else {
        render_table(frame, app, vertical[1]);
    }
    render_footer(frame, app, vertical[2]);
    render_repo_batch_overlay(frame, app);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let summary = app.summary();
    let kind = match app.workspace.kind {
        WorkspaceKind::Repo => "repo",
        WorkspaceKind::Git => "git",
    };
    let title = Line::from(vec![
        Span::styled(
            " repo-tui ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {} ({kind})", app.workspace_label())),
    ]);
    let status = if app.search_mode {
        format!("Search: {}_", app.search)
    } else if !app.search.is_empty() {
        format!(
            "Filter: {}  Selected: {}",
            app.search,
            app.selected_project_count()
        )
    } else {
        format!(
            "{} projects  {} selected  {} dirty  {} conflict  {} ahead  {} behind  {} errors",
            summary.total,
            app.selected_project_count(),
            summary.dirty,
            summary.conflicted,
            summary.ahead,
            summary.behind,
            summary.errors
        )
    };
    frame.render_widget(
        Paragraph::new(vec![title, Line::raw(status)])
            .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_table(frame: &mut Frame, app: &App, area: Rect) {
    let indices = app.filtered_indices();
    let visible = area.height.saturating_sub(3) as usize;
    let start = viewport_start(app.selected, visible, indices.len());
    let rows = indices
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .filter_map(|(visible_index, project_index)| {
            let snapshot = app.projects.get(*project_index)?;
            let selected = visible_index == app.selected;
            let style = row_style(snapshot, selected);
            let upstream = snapshot.upstream.as_ref().map_or_else(
                || "-".to_owned(),
                |value| format!("+{} -{}", value.ahead, value.behind),
            );
            let status = match &snapshot.scan {
                ScanState::Pending => "scanning".to_owned(),
                ScanState::Error(_) => "error".to_owned(),
                ScanState::Ready => snapshot.worktree.status_label(),
            };
            Some(
                Row::new(vec![
                    Cell::from(format!(
                        "{}{}",
                        if selected { ">" } else { " " },
                        if app.selected_projects.contains(&snapshot.project.id) {
                            "x"
                        } else {
                            " "
                        }
                    )),
                    Cell::from(status),
                    Cell::from(
                        snapshot
                            .project
                            .relative_path
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    Cell::from(head_label(&snapshot.head)),
                    Cell::from(upstream),
                ])
                .style(style),
            )
        });

    let widths = [
        Constraint::Length(3),
        Constraint::Length(12),
        Constraint::Min(20),
        Constraint::Length(18),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["", "Status", "Project / path", "HEAD", "Upstream"]).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1)
        .block(
            Block::default()
                .title(format!(
                    " Projects ({}/{}) ",
                    indices.len(),
                    app.projects.len()
                ))
                .borders(Borders::ALL),
        );
    frame.render_widget(table, area);
}

fn render_inspector(frame: &mut Frame, app: &App, area: Rect) {
    let lines = match app.selected_project() {
        Some(snapshot) => {
            let mut lines = vec![
                Line::styled(
                    snapshot.project.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::raw(snapshot.project.path.display().to_string()),
                Line::raw(""),
                Line::raw(format!("HEAD: {}", head_label(&snapshot.head))),
                Line::raw(format!("Status: {}", snapshot.worktree.status_label())),
                Line::raw(format!("Staged: {}", snapshot.worktree.staged)),
                Line::raw(format!("Modified: {}", snapshot.worktree.unstaged)),
                Line::raw(format!("Untracked: {}", snapshot.worktree.untracked)),
                Line::raw(format!("Conflicts: {}", snapshot.worktree.conflicted)),
            ];
            if let Some(upstream) = &snapshot.upstream {
                lines.push(Line::raw(""));
                lines.push(Line::raw(format!("Upstream: {}", upstream.name)));
                lines.push(Line::raw(format!(
                    "Ahead/behind: +{} -{}",
                    upstream.ahead, upstream.behind
                )));
            }
            if let ScanState::Error(error) = &snapshot.scan {
                lines.push(Line::raw(""));
                lines.push(Line::styled(error.clone(), Color::Red));
            }
            lines
        }
        None => vec![Line::raw("No matching project")],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(" Inspector ").borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let task = if let Some(task) = &app.repo_batch.task {
        if task.running {
            format!("{} running", task.spec.action.label())
        } else {
            format!("{} finished", task.spec.action.label())
        }
    } else if app.scanning > 0 {
        format!("Scanning {}", app.scanning)
    } else {
        "Ready".to_owned()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                task,
                Style::default().fg(if app.scanning > 0 {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
            Span::raw("   Space Select   a Repo actions   / Search   Enter Open   ? Help"),
        ])),
        area,
    );
}

fn render_repo_batch_overlay(frame: &mut Frame, app: &App) {
    if app.repo_batch.action_menu {
        let area = centered_rect(54, 62, frame.area());
        let lines = crate::domain::RepoBatchAction::ALL
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let style = if index == app.repo_batch.action_selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else if action.is_destructive() {
                    Style::default().fg(Color::LightRed)
                } else {
                    Style::default()
                };
                Line::styled(format!(" {}", action.label()), style)
            })
            .collect::<Vec<_>>();
        render_overlay(frame, area, " Repo batch actions ", lines);
    } else if let Some(form) = &app.repo_batch.form {
        let area = centered_rect(60, 34, frame.area());
        let label = form.action.input_label().unwrap_or("Value");
        render_overlay(
            frame,
            area,
            form.action.label(),
            vec![
                Line::raw(format!("{label}: {}_", form.value)),
                Line::raw(""),
                Line::styled("Enter Review   Esc Cancel", Color::DarkGray),
            ],
        );
    } else if let Some((spec, targets)) = &app.repo_batch.pending {
        let area = centered_rect(72, 70, frame.area());
        let mut lines = vec![
            Line::styled(
                spec.action.label(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(
                "Scope: {}",
                if spec.action.is_workspace_action() {
                    "workspace".to_owned()
                } else {
                    format!("{} project(s)", targets.len())
                }
            )),
            Line::raw(format!("Parameters: {}", batch_parameter(spec))),
            Line::raw("Commands:"),
        ];
        let mut detail = Vec::new();
        detail.extend(
            targets
                .iter()
                .map(|project| Line::raw(format!("  {}", project.relative_path.display()))),
        );
        if spec.action.is_workspace_action() {
            if let Ok(args) = crate::adapters::repo::batch_args(spec, None) {
                detail.push(Line::raw(format!("  repo {}", args.join(" "))));
            }
        } else {
            detail.extend(targets.iter().filter_map(|project| {
                crate::adapters::repo::batch_args(spec, Some(&project.relative_path))
                    .ok()
                    .map(|args| Line::raw(format!("  repo {}", args.join(" "))))
            }));
        }
        let budget = area.height.saturating_sub(9) as usize;
        let max_scroll = detail.len().saturating_sub(budget);
        let scroll = app.repo_batch.scroll.min(max_scroll);
        lines.extend(detail.into_iter().skip(scroll).take(budget));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Run now? y Yes   n No",
            if spec.action.is_destructive() {
                Color::LightRed
            } else {
                Color::Yellow
            },
        ));
        render_overlay(frame, area, " Confirm Repo operation ", lines);
    } else if let Some(task) = &app.repo_batch.task {
        let area = centered_rect(86, 82, frame.area());
        let counts = task.results.iter().fold([0usize; 5], |mut counts, result| {
            let index = match result.state {
                RepoProjectState::Pending => 0,
                RepoProjectState::Running => 1,
                RepoProjectState::Succeeded => 2,
                RepoProjectState::Failed => 3,
                RepoProjectState::Cancelled | RepoProjectState::Skipped => 4,
            };
            counts[index] += 1;
            counts
        });
        let mut lines = vec![
            Line::styled(
                format!(
                    "{}  {}",
                    task.spec.action.label(),
                    if task.running {
                        if task.cancelling {
                            "cancelling"
                        } else {
                            "running"
                        }
                    } else if task.cancelled {
                        "cancelled"
                    } else {
                        "complete"
                    }
                ),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(
                "pending {}  running {}  success {}  failed {}  cancelled {}",
                counts[0], counts[1], counts[2], counts[3], counts[4]
            )),
            Line::raw(format!("Parameters: {}", batch_parameter(&task.spec))),
            Line::raw(format!("Commands started: {}", task.args.len())),
        ];
        let mut detail = Vec::new();
        if let Some((state, message)) = &task.workspace_result {
            detail.push(result_line("workspace", *state, message));
        }
        detail.extend(task.results.iter().map(|result| {
            result_line(
                &result.project.relative_path.display().to_string(),
                result.state,
                &result.message,
            )
        }));
        detail.push(Line::raw(""));
        detail.push(Line::styled("Log", Style::default().fg(Color::Cyan)));
        detail.extend(task.logs.iter().map(|line| Line::raw(line.clone())));
        let budget = area.height.saturating_sub(8) as usize;
        let max_scroll = detail.len().saturating_sub(budget);
        let scroll = app.repo_batch.scroll.min(max_scroll);
        lines.extend(detail.into_iter().skip(scroll).take(budget));
        lines.push(Line::styled(
            if task.running {
                "j/k Scroll   c Cancel (no rollback)"
            } else {
                "j/k Scroll   f Retry failed   Esc Close"
            },
            Color::DarkGray,
        ));
        render_overlay(frame, area, " Repo task ", lines);
    } else if let Some((error, message)) = &app.repo_batch.message {
        let area = centered_rect(60, 24, frame.area());
        render_overlay(
            frame,
            area,
            " Repo action ",
            vec![Line::styled(
                message.clone(),
                if *error {
                    Color::LightRed
                } else {
                    Color::Green
                },
            )],
        );
    }
}

fn batch_parameter(spec: &crate::domain::RepoBatchSpec) -> String {
    spec.branch
        .as_deref()
        .or(spec.change.as_deref())
        .map(str::to_owned)
        .or_else(|| {
            spec.output
                .as_ref()
                .map(|value| value.display().to_string())
        })
        .unwrap_or_else(|| "(none)".to_owned())
}

fn result_line(label: &str, state: RepoProjectState, message: &str) -> Line<'static> {
    let color = match state {
        RepoProjectState::Succeeded => Color::Green,
        RepoProjectState::Failed => Color::LightRed,
        RepoProjectState::Running => Color::Yellow,
        RepoProjectState::Cancelled | RepoProjectState::Skipped => Color::DarkGray,
        RepoProjectState::Pending => Color::Gray,
    };
    Line::from(vec![
        Span::styled(format!("{:<9}", state.label()), color),
        Span::raw(format!(" {label}: {message}")),
    ])
}

fn render_overlay(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
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

fn row_style(snapshot: &crate::domain::ProjectSnapshot, selected: bool) -> Style {
    let mut style = if matches!(snapshot.scan, ScanState::Error(_)) {
        Style::default().fg(Color::Red)
    } else if snapshot.worktree.conflicted > 0 {
        Style::default().fg(Color::LightRed)
    } else if snapshot.worktree.is_dirty() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    if selected {
        style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
    }
    style
}

fn head_label(head: &HeadState) -> String {
    match head {
        HeadState::Branch(name) => name.clone(),
        HeadState::Detached(oid) => format!("detached {oid}"),
        HeadState::Unborn(name) => format!("unborn {name}"),
        HeadState::Unknown => "-".to_owned(),
    }
}

fn viewport_start(selected: usize, visible: usize, total: usize) -> usize {
    if visible == 0 || total <= visible {
        0
    } else {
        selected.saturating_sub(visible / 2).min(total - visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_selection_in_view() {
        assert_eq!(viewport_start(0, 10, 100), 0);
        assert_eq!(viewport_start(50, 10, 100), 45);
        assert_eq!(viewport_start(99, 10, 100), 90);
    }
}
