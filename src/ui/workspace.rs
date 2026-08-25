use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::state::App;
use crate::domain::{HeadState, ScanState, WorkspaceKind};

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
        format!("Filter: {}", app.search)
    } else {
        format!(
            "{} projects  {} dirty  {} conflict  {} ahead  {} behind  {} errors",
            summary.total,
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
                    Cell::from(if selected { ">" } else { " " }),
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
        Constraint::Length(2),
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
    let task = if app.scanning > 0 {
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
            Span::raw("   Enter Open   / Search   r Refresh   ? Help   q Quit"),
        ])),
        area,
    );
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
