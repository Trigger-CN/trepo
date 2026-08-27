use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::repository::{action_preview, FormField};
use crate::app::state::{graph_actions, graph_object_label, App, GraphState};
use crate::domain::{CommitRef, CommitRefKind, RiskLevel};

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 60 || area.height < 12 {
        frame.render_widget(
            Paragraph::new("Terminal too small. Resize to at least 60x12. Press Esc to return.")
                .block(Block::default().title(" Graph ").borders(Borders::ALL)),
            area,
        );
        return;
    }
    let Some(graph) = &app.graph else {
        frame.render_widget(Paragraph::new("No repository selected"), area);
        return;
    };

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " repo-tui ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {}  /  Graph{}",
                graph.project.name,
                if graph.filter.is_active() {
                    format!("  [{}]", graph.filter.summary())
                } else {
                    String::new()
                }
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        vertical[0],
    );

    if area.width >= 100 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(78), Constraint::Percentage(22)])
            .split(vertical[1]);
        render_commits(frame, graph, body[0]);
        render_detail(frame, graph, body[1]);
    } else {
        render_commits(frame, graph, vertical[1]);
    }

    let footer = if graph.loading {
        Span::styled("Loading commits...", Style::default().fg(Color::Yellow))
    } else if app
        .repository
        .as_ref()
        .is_some_and(|state| state.action_running)
    {
        Span::styled(
            "Running Git operation...",
            Style::default().fg(Color::Yellow),
        )
    } else if let Some((error, message)) = &graph.message {
        Span::styled(
            message.clone(),
            Style::default().fg(if *error { Color::Red } else { Color::Green }),
        )
    } else {
        Span::raw("Enter Objects  j/k Move  f Filter  / Search  x Clear  r Reload  Esc Back")
    };
    frame.render_widget(Paragraph::new(Line::from(footer)), vertical[2]);
    if graph.object_menu {
        render_object_menu(frame, app, graph);
    }
    if graph.action_menu {
        render_action_menu(frame, graph);
    }
    if graph.form.is_some() {
        render_graph_form(frame, graph);
    }
    if graph.filter_form.is_some() {
        render_filter_form(frame, graph);
    }
    if app
        .repository
        .as_ref()
        .is_some_and(|state| state.pending.is_some())
    {
        render_confirmation(frame, app);
    }
}

fn render_filter_form(frame: &mut Frame, graph: &GraphState) {
    let Some(form) = graph.filter_form.as_ref() else {
        return;
    };
    let area = centered_rect(70, 56, frame.area());
    frame.render_widget(Clear, area);
    let mut lines = form
        .fields()
        .into_iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let suffix = if index == form.selected { "_" } else { "" };
            Line::styled(
                format!(
                    "{} {label}: {value}{suffix}",
                    if index == form.selected { ">" } else { " " }
                ),
                if index == form.selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                },
            )
        })
        .collect::<Vec<_>>();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Branch matches local/remote refs. Dates use YYYY-MM-DD UTC.",
        Color::DarkGray,
    ));
    if let Some(error) = &graph.filter_error {
        lines.push(Line::styled(error.clone(), Color::LightRed));
    }
    lines.push(Line::raw("Tab/Up/Down Field   Enter Apply   Esc Cancel"));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Graph filters ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_object_menu(frame: &mut Frame, app: &App, graph: &GraphState) {
    let area = centered_rect(62, 70, frame.area());
    frame.render_widget(Clear, area);
    let objects = app.graph_objects();
    let items = objects.iter().enumerate().map(|(index, object)| {
        ListItem::new(graph_object_label(object)).style(menu_style(index == graph.object_selected))
    });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Objects on selected node ")
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_action_menu(frame: &mut Frame, graph: &GraphState) {
    let Some(object) = graph.selected_object.as_ref() else {
        return;
    };
    let area = centered_rect(66, 76, frame.area());
    frame.render_widget(Clear, area);
    let items = graph_actions(object.kind)
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            ListItem::new(choice.label()).style(menu_style(index == graph.action_selected))
        });
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(format!(" Actions for {} ", graph_object_label(object)))
                .borders(Borders::ALL),
        ),
        area,
    );
}

fn render_graph_form(frame: &mut Frame, graph: &GraphState) {
    let Some(form) = graph.form.as_ref() else {
        return;
    };
    let area = centered_rect(72, 68, frame.area());
    frame.render_widget(Clear, area);
    let mut lines = form
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let prefix = if index == form.selected { ">" } else { " " };
            let hint = if matches!(field, FormField::Toggle { .. }) {
                " [Space]"
            } else {
                ""
            };
            Line::styled(
                format!(
                    "{prefix} {}: {}{hint}",
                    field.label(),
                    field.display_value()
                ),
                if index == form.selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                },
            )
        })
        .collect::<Vec<_>>();
    if let Some((error, message)) = &graph.message {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            message.clone(),
            Style::default().fg(if *error { Color::Red } else { Color::Green }),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw("Enter execute   Esc cancel"));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(format!(" {} ", form.choice.label()))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_confirmation(frame: &mut Frame, app: &App) {
    let Some(state) = app.repository.as_ref() else {
        return;
    };
    let Some(action) = state.pending.as_ref() else {
        return;
    };
    let area = centered_rect(70, 46, frame.area());
    frame.render_widget(Clear, area);
    let risk = match action.risk() {
        RiskLevel::RemoteWrite => "This action writes to a remote repository.",
        RiskLevel::Destructive => "This action can discard local or reference state.",
        _ => "Confirm this repository operation.",
    };
    let mut lines = vec![
        Line::styled(
            risk,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw(format!("Action: {}", action.label())),
    ];
    if let Some(snapshot) = state.snapshot.as_ref() {
        lines.extend(action_preview(action, snapshot).into_iter().map(Line::raw));
    }
    lines.extend([
        Line::raw(""),
        Line::raw("Press y to continue or n/Esc to cancel."),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Confirm operation ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn menu_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
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

#[derive(Debug, Clone, Copy)]
struct CommitTableLayout {
    graph_width: u16,
    refs_width: u16,
    show_oid: bool,
    show_date: bool,
    show_author: bool,
    show_age: bool,
}

fn commit_table_layout(width: u16, desired_graph_width: u16) -> CommitTableLayout {
    let show_oid = width >= 72;
    let show_date = width >= 88;
    let show_author = width >= 112;
    let show_age = width >= 130;
    let refs_width = if width >= 88 {
        20
    } else if width >= 72 {
        18
    } else {
        16
    };
    let column_count = 4
        + usize::from(show_oid)
        + usize::from(show_date)
        + usize::from(show_author)
        + usize::from(show_age);
    let fixed_width = 2u16
        .saturating_add(18)
        .saturating_add(refs_width)
        .saturating_add(if show_oid { 9 } else { 0 })
        .saturating_add(if show_date { 10 } else { 0 })
        .saturating_add(if show_author { 10 } else { 0 })
        .saturating_add(if show_age { 5 } else { 0 })
        .saturating_add(column_count.saturating_sub(1) as u16)
        .saturating_add(2);
    let graph_width = desired_graph_width
        .max(7)
        .min(width.saturating_sub(fixed_width).clamp(7, 24));
    CommitTableLayout {
        graph_width,
        refs_width,
        show_oid,
        show_date,
        show_author,
        show_age,
    }
}

fn render_commits(frame: &mut Frame, graph: &crate::app::state::GraphState, area: Rect) {
    if let Some(error) = &graph.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(Color::Red))
                .block(
                    Block::default()
                        .title(" Commit graph ")
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let indices = graph.filtered_indices();
    let selected_visible = indices
        .iter()
        .position(|index| *index == graph.selected)
        .unwrap_or(0);
    let visible = area.height.saturating_sub(3) as usize;
    let start = viewport_start(selected_visible, visible, indices.len());
    let topology = super::graph_layout::topology_rows(&graph.commits, 10);
    let desired_graph_width = topology
        .iter()
        .map(|row| row.cells.len().saturating_mul(2) + if row.hidden_lanes > 0 { 3 } else { 0 })
        .max()
        .unwrap_or(2)
        .max(7) as u16;
    let table_layout = commit_table_layout(area.width, desired_graph_width);
    let rows = indices
        .iter()
        .copied()
        .skip(start)
        .take(visible)
        .filter_map(|index| {
            let commit = graph.commits.get(index)?;
            let style = if index == graph.selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            let topology = topology
                .get(index)
                .map(topology_line)
                .unwrap_or_else(|| Line::raw(""));
            let subject_style = if commit.refs.is_empty() {
                Style::default().fg(Color::White)
            } else {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            };
            let mut cells = vec![
                Cell::from(Line::styled(
                    if index == graph.selected { ">" } else { " " },
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(topology),
            ];
            if table_layout.show_oid {
                cells.push(Cell::from(Line::styled(
                    commit.oid.chars().take(8).collect::<String>(),
                    Style::default().fg(Color::LightBlue),
                )));
            }
            cells.push(Cell::from(Line::styled(
                commit.subject.clone(),
                subject_style,
            )));
            cells.push(Cell::from(main_refs_line(&commit.refs)));
            if table_layout.show_date {
                cells.push(Cell::from(Line::styled(
                    calendar_date(commit.timestamp),
                    Style::default().fg(Color::Gray),
                )));
            }
            if table_layout.show_author {
                cells.push(Cell::from(Line::styled(
                    commit.author.clone(),
                    Style::default().fg(Color::Gray),
                )));
            }
            if table_layout.show_age {
                cells.push(Cell::from(Line::styled(
                    relative_age(commit.timestamp),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Some(Row::new(cells).style(style))
        });
    let mut constraints = vec![
        Constraint::Length(2),
        Constraint::Length(table_layout.graph_width),
    ];
    let mut headers = vec!["", "Graph"];
    if table_layout.show_oid {
        constraints.push(Constraint::Length(9));
        headers.push("Commit");
    }
    constraints.push(Constraint::Min(18));
    headers.push("Subject");
    constraints.push(Constraint::Length(table_layout.refs_width));
    headers.push("Refs");
    if table_layout.show_date {
        constraints.push(Constraint::Length(10));
        headers.push("Date");
    }
    if table_layout.show_author {
        constraints.push(Constraint::Length(10));
        headers.push("Author");
    }
    if table_layout.show_age {
        constraints.push(Constraint::Length(5));
        headers.push("Age");
    }
    let table = Table::new(rows, constraints)
        .header(
            Row::new(headers).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .column_spacing(1)
        .block(
            Block::default()
                .title(format!(
                    " All refs commit graph ({}/{}) ",
                    indices.len(),
                    graph.commits.len()
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(table, area);
}

fn calendar_date(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

fn relative_age(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(timestamp, |value| value.as_secs() as i64);
    let seconds = now.saturating_sub(timestamp).max(0);
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3_599 => format!("{}m", seconds / 60),
        3_600..=86_399 => format!("{}h", seconds / 3_600),
        86_400..=2_592_000 => format!("{}d", seconds / 86_400),
        2_592_001..=31_536_000 => format!("{}mo", seconds / 2_592_000),
        _ => format!("{}y", seconds / 31_536_000),
    }
}

fn render_detail(frame: &mut Frame, graph: &crate::app::state::GraphState, area: Rect) {
    let selected = graph
        .filtered_indices()
        .contains(&graph.selected)
        .then(|| graph.commits.get(graph.selected))
        .flatten();
    let lines = selected.map_or_else(
        || {
            vec![Line::raw(if graph.loading {
                "Loading..."
            } else if graph.filter.is_active() {
                "No matching commits"
            } else {
                "No commits"
            })]
        },
        |commit| {
            let mut lines = vec![
                Line::styled(
                    commit.subject.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                detail_line("Commit", commit.oid.clone(), Color::LightBlue),
                detail_line("Author", commit.author.clone(), Color::Gray),
                detail_line("Date", calendar_date(commit.timestamp), Color::Gray),
                detail_line("Age", relative_age(commit.timestamp), Color::DarkGray),
                detail_line(
                    "Parents",
                    if commit.parents.is_empty() {
                        "-".to_owned()
                    } else {
                        commit.parents.join(" ")
                    },
                    Color::LightBlue,
                ),
            ];
            lines.extend(ref_detail_lines(&commit.refs));
            lines.push(Line::raw(""));
            lines.push(Line::raw(commit.body.clone()));
            lines
        },
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Commit detail ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn topology_line(row: &super::graph_layout::TopologyRow) -> Line<'static> {
    let mut spans = row
        .cells
        .iter()
        .map(|cell| {
            Span::styled(
                format!("{} ", cell.glyph),
                Style::default()
                    .fg(lane_color(cell.color))
                    .add_modifier(Modifier::BOLD),
            )
        })
        .collect::<Vec<_>>();
    if row.hidden_lanes > 0 {
        spans.push(Span::styled(
            format!("~{}", row.hidden_lanes),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}
fn lane_color(index: usize) -> Color {
    const COLORS: [Color; 8] = [
        Color::Cyan,
        Color::LightMagenta,
        Color::LightGreen,
        Color::Yellow,
        Color::LightBlue,
        Color::LightRed,
        Color::Green,
        Color::Magenta,
    ];
    COLORS[index % COLORS.len()]
}

fn main_refs_line(refs: &[CommitRef]) -> Line<'static> {
    let mut spans = Vec::new();
    let mut append = |span: Span<'static>| {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(span);
    };
    for kind in [
        CommitRefKind::Head,
        CommitRefKind::LocalBranch,
        CommitRefKind::Stash,
    ] {
        for reference in refs.iter().filter(|reference| reference.kind == kind) {
            append(ref_span(reference));
        }
    }
    for kind in [CommitRefKind::RemoteBranch, CommitRefKind::Tag] {
        let matching = refs
            .iter()
            .filter(|reference| reference.kind == kind)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            append(ref_count_span(kind, matching.len() - 1));
        }
        if let Some(reference) = matching.first() {
            append(ref_span(reference));
        }
    }
    Line::from(spans)
}

fn ref_detail_lines(refs: &[CommitRef]) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        format!("Refs ({})", refs.len()),
        Style::default().fg(Color::DarkGray),
    )];
    if refs.is_empty() {
        lines.push(Line::raw("  -"));
        return lines;
    }
    for (kind, label) in [
        (CommitRefKind::Head, "HEAD"),
        (CommitRefKind::LocalBranch, "Local branches"),
        (CommitRefKind::RemoteBranch, "Remote branches"),
        (CommitRefKind::Tag, "Tags"),
        (CommitRefKind::Stash, "Stashes"),
    ] {
        let matching = refs
            .iter()
            .filter(|reference| reference.kind == kind)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        lines.push(Line::styled(
            format!("{label} ({})", matching.len()),
            Style::default().fg(Color::DarkGray),
        ));
        lines.extend(
            matching
                .into_iter()
                .map(|reference| Line::from(vec![Span::raw("  "), ref_span(reference)])),
        );
    }
    lines
}

fn ref_count_span(kind: CommitRefKind, count: usize) -> Span<'static> {
    let prefix = match kind {
        CommitRefKind::RemoteBranch => "R",
        CommitRefKind::Tag => "T",
        _ => "Refs",
    };
    Span::styled(
        format!(" {prefix}:+{count} "),
        ref_style(kind).add_modifier(Modifier::BOLD),
    )
}

fn ref_span(reference: &CommitRef) -> Span<'static> {
    let label = match reference.kind {
        CommitRefKind::Head => "HEAD".to_owned(),
        CommitRefKind::LocalBranch => format!("L:{}", reference.name),
        CommitRefKind::RemoteBranch => format!("R:{}", reference.name),
        CommitRefKind::Tag => format!("T:{}", reference.name),
        CommitRefKind::Stash => format!("S:{}", reference.name),
    };
    Span::styled(
        format!(" {label} "),
        ref_style(reference.kind).add_modifier(Modifier::BOLD),
    )
}

fn ref_style(kind: CommitRefKind) -> Style {
    match kind {
        CommitRefKind::Head => Style::default().fg(Color::Black).bg(Color::LightGreen),
        CommitRefKind::LocalBranch => Style::default().fg(Color::Black).bg(Color::Cyan),
        CommitRefKind::RemoteBranch => Style::default().fg(Color::White).bg(Color::Blue),
        CommitRefKind::Tag => Style::default().fg(Color::Black).bg(Color::Yellow),
        CommitRefKind::Stash => Style::default().fg(Color::White).bg(Color::Magenta),
    }
}

fn detail_line(label: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(color)),
    ])
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

    use crate::domain::Commit;
    fn commit(oid: &str, parents: &[&str]) -> Commit {
        Commit {
            oid: oid.into(),
            parents: parents.iter().map(|value| (*value).to_owned()).collect(),
            refs: vec![],
            author: "A".into(),
            timestamp: 0,
            subject: oid.into(),
            body: oid.into(),
        }
    }

    #[test]
    fn formats_commit_timestamps_as_utc_calendar_dates() {
        assert_eq!(calendar_date(0), "1970-01-01");
        assert_eq!(calendar_date(1_700_000_000), "2023-11-14");
        assert_eq!(calendar_date(-86_400), "1969-12-31");
        assert_eq!(calendar_date(951_782_400), "2000-02-29");
    }

    #[test]
    fn renders_branch_split_and_merge_edges() {
        let commits = vec![
            commit("merge", &["feature", "main"]),
            commit("feature", &["base"]),
            commit("main", &["base"]),
            commit("base", &[]),
        ];
        let rows = super::super::graph_layout::topology_rows(&commits, 10);
        assert_eq!(rows.len(), commits.len());
        assert_eq!(rows[0].cells[0].glyph, '◆');
        assert!(rows[0].cells.iter().any(|cell| cell.glyph == '┐'));
        assert!(rows[2].cells.iter().any(|cell| cell.glyph == '├'));
        assert!(rows.iter().any(|row| row.cells.len() >= 2));
    }

    #[test]
    fn renders_split_without_merge_commit() {
        let commits = vec![
            commit("feature", &["base"]),
            commit("main", &["base"]),
            commit("base", &[]),
        ];
        let rows = super::super::graph_layout::topology_rows(&commits, 10);
        assert!(rows[1].cells.iter().any(|cell| cell.glyph == '├'));
        assert!(rows[1].cells.len() >= 2);
    }

    #[test]
    fn keeps_more_than_four_parallel_lanes() {
        let commits = vec![
            commit("octopus", &["a", "b", "c", "d", "e", "f"]),
            commit("a", &["base"]),
            commit("b", &["base"]),
            commit("c", &["base"]),
            commit("d", &["base"]),
            commit("e", &["base"]),
            commit("f", &["base"]),
            commit("base", &[]),
        ];
        let rows = super::super::graph_layout::topology_rows(&commits, 10);
        assert!(rows.iter().any(|row| row.cells.len() == 6));
        let colors = rows
            .iter()
            .find(|row| row.cells.len() == 6)
            .unwrap()
            .cells
            .iter()
            .map(|cell| cell.color)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(colors.len(), 6);
    }

    #[test]
    fn ref_kinds_have_distinct_badge_styles() {
        let refs = [
            CommitRefKind::Head,
            CommitRefKind::LocalBranch,
            CommitRefKind::RemoteBranch,
            CommitRefKind::Tag,
            CommitRefKind::Stash,
        ];
        let backgrounds = refs
            .into_iter()
            .map(|kind| {
                ref_span(&CommitRef {
                    name: "ref".into(),
                    kind,
                })
                .style
                .bg
                .unwrap()
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(backgrounds.len(), 5);
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn folds_dense_refs_without_discarding_detail_entries() {
        let refs = vec![
            CommitRef {
                name: "HEAD".into(),
                kind: CommitRefKind::Head,
            },
            CommitRef {
                name: "main".into(),
                kind: CommitRefKind::LocalBranch,
            },
            CommitRef {
                name: "v1".into(),
                kind: CommitRefKind::Tag,
            },
            CommitRef {
                name: "v2".into(),
                kind: CommitRefKind::Tag,
            },
            CommitRef {
                name: "v3".into(),
                kind: CommitRefKind::Tag,
            },
        ];
        let summary = line_text(&main_refs_line(&refs));
        assert!(summary.contains("HEAD"));
        assert!(summary.contains("L:main"));
        assert!(summary.contains("T:v1"));
        assert!(summary.contains("T:+2"));
        assert!(summary.find("T:+2").unwrap() < summary.find("T:v1").unwrap());
        assert!(!summary.contains("T:v2"));
        assert!(!summary.contains("T:v3"));

        let detail = ref_detail_lines(&refs)
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(detail.contains("Refs (5)"));
        assert!(detail.contains("Local branches (1)"));
        assert!(detail.contains("Tags (3)"));
        assert!(detail.contains("T:v1"));
        assert!(detail.contains("T:v2"));
        assert!(detail.contains("T:v3"));
        assert_eq!(refs.len(), 5);
    }

    #[test]
    fn topology_line_marks_hidden_lanes() {
        let row = super::super::graph_layout::TopologyRow {
            cells: vec![super::super::graph_layout::TopologyCell {
                glyph: '●',
                color: 0,
            }],
            pipes: vec![],
            hidden_lanes: 4,
            has_missing_edge: false,
        };
        assert!(line_text(&topology_line(&row)).contains("~4"));
    }

    #[test]
    fn responsive_columns_preserve_subject_before_metadata() {
        let narrow = commit_table_layout(80, 30);
        assert_eq!(narrow.graph_width, 24);
        assert!(narrow.show_oid);
        assert!(!narrow.show_date);
        assert!(!narrow.show_author);
        assert!(!narrow.show_age);

        let split_wide = commit_table_layout(93, 30);
        assert_eq!(split_wide.graph_width, 24);
        assert!(split_wide.show_oid);
        assert!(split_wide.show_date);
        assert!(!split_wide.show_author);
        assert!(!split_wide.show_age);

        let very_wide = commit_table_layout(140, 30);
        assert!(very_wide.show_author);
        assert!(very_wide.show_age);
    }
}
