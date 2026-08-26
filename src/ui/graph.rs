use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::repository::{action_preview, FormField};
use crate::app::state::{graph_actions, graph_object_label, App, GraphState};
use crate::domain::{Commit, CommitRef, CommitRefKind, RiskLevel};

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
    let topology = topology_rows(&graph.commits);
    let graph_width = topology
        .iter()
        .map(|row| row.cells.len().saturating_mul(2))
        .max()
        .unwrap_or(2)
        .max(7)
        .min(area.width.saturating_sub(59).max(7) as usize) as u16;
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
            Some(
                Row::new(vec![
                    Cell::from(Line::styled(
                        if index == graph.selected { ">" } else { " " },
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(topology),
                    Cell::from(Line::styled(
                        commit.oid.chars().take(8).collect::<String>(),
                        Style::default().fg(Color::LightBlue),
                    )),
                    Cell::from(Line::styled(commit.subject.clone(), subject_style)),
                    Cell::from(refs_line(&commit.refs)),
                    Cell::from(Line::styled(
                        commit.author.clone(),
                        Style::default().fg(Color::Gray),
                    )),
                    Cell::from(Line::styled(
                        calendar_date(commit.timestamp),
                        Style::default().fg(Color::Gray),
                    )),
                    Cell::from(Line::styled(
                        relative_age(commit.timestamp),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
                .style(style),
            )
        });
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(graph_width),
            Constraint::Length(9),
            Constraint::Min(18),
            Constraint::Length(30),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(5),
        ],
    )
    .header(
        Row::new([
            "", "Graph", "Commit", "Subject", "Refs", "Author", "Date", "Age",
        ])
        .style(
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
                Line::styled("Refs", Style::default().fg(Color::DarkGray)),
            ];
            if commit.refs.is_empty() {
                lines.push(Line::raw("  -"));
            } else {
                for reference in &commit.refs {
                    lines.push(Line::from(vec![Span::raw("  "), ref_span(reference)]));
                }
            }
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

#[derive(Debug, Clone)]
struct Lane {
    oid: String,
    color: usize,
}

#[derive(Debug, Clone)]
struct TopologyRow {
    cells: Vec<(char, usize)>,
}

fn topology_rows(commits: &[Commit]) -> Vec<TopologyRow> {
    const UP: u8 = 1;
    const DOWN: u8 = 2;
    const LEFT: u8 = 4;
    const RIGHT: u8 = 8;

    let mut lanes: Vec<Lane> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let positions: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.as_str(), index))
        .collect();
    let mut next_color = 0;

    for commit in commits {
        let (lane, inherited_color, display_lanes, mut next_lanes) =
            if let Some(lane) = lanes.iter().position(|value| value.oid == commit.oid) {
                let inherited_color = lanes[lane].color;
                let mut next_lanes = lanes.clone();
                next_lanes.remove(lane);
                (lane, inherited_color, lanes.clone(), next_lanes)
            } else {
                let color = next_color;
                next_color += 1;
                let lane = lanes.len();
                let mut display_lanes = lanes.clone();
                display_lanes.push(Lane {
                    oid: commit.oid.clone(),
                    color,
                });
                (lane, color, display_lanes, lanes.clone())
            };
        let cell_count = display_lanes.len().max(next_lanes.len()).max(lane + 1);
        let mut connections = vec![0u8; cell_count];
        let mut colors = (0..cell_count)
            .map(|index| {
                display_lanes
                    .get(index)
                    .or_else(|| next_lanes.get(index))
                    .map_or(inherited_color, |value| value.color)
            })
            .collect::<Vec<_>>();

        for (index, connection) in connections.iter_mut().enumerate().take(cell_count) {
            if display_lanes.get(index).is_some() {
                *connection |= UP;
            }
            if next_lanes.get(index).is_some() {
                *connection |= DOWN;
            }
        }

        let mut insert_at = lane.min(next_lanes.len());
        for (parent_index, parent) in commit.parents.iter().enumerate() {
            if !positions.contains_key(parent.as_str())
                || next_lanes.iter().any(|value| value.oid == *parent)
            {
                continue;
            }
            let color = if parent_index == 0 {
                inherited_color
            } else {
                let color = next_color;
                next_color += 1;
                color
            };
            next_lanes.insert(
                insert_at,
                Lane {
                    oid: parent.clone(),
                    color,
                },
            );
            insert_at += 1;
        }

        let next_cell_count = cell_count.max(next_lanes.len());
        connections.resize(next_cell_count, 0);
        colors.resize(next_cell_count, inherited_color);
        for index in cell_count..next_cell_count {
            if next_lanes.get(index).is_some() {
                connections[index] |= DOWN;
                colors[index] = next_lanes[index].color;
            }
        }

        for (parent_index, parent) in commit.parents.iter().enumerate() {
            let Some(target) = next_lanes.iter().position(|value| value.oid == *parent) else {
                continue;
            };
            if target == lane && parent_index == 0 {
                continue;
            }
            let (start, end) = if target < lane {
                (target, lane)
            } else {
                (lane, target)
            };
            if target < lane {
                connections[target] |= RIGHT;
            } else {
                connections[target] |= LEFT;
            }
            for connection in connections.iter_mut().take(end).skip(start + 1) {
                *connection |= LEFT | RIGHT;
            }
        }

        connections[lane] = 0;
        if display_lanes.get(lane).is_some() {
            connections[lane] |= UP;
        }
        if next_lanes.get(lane).is_some() {
            connections[lane] |= DOWN;
        }
        let node = if commit.parents.len() > 1 {
            '◆'
        } else {
            '●'
        };
        let cells = connections
            .into_iter()
            .zip(colors)
            .enumerate()
            .map(|(index, (mask, color))| {
                if index == lane {
                    (node, inherited_color)
                } else {
                    (box_glyph(mask), color)
                }
            })
            .collect();
        rows.push(TopologyRow { cells });
        lanes = next_lanes;
    }
    rows
}

fn box_glyph(mask: u8) -> char {
    match mask {
        0 => ' ',
        1 => '│',
        2 => '│',
        3 => '│',
        4 => '─',
        8 => '─',
        5 => '┘',
        6 => '┐',
        9 => '└',
        10 => '┌',
        12 => '─',
        7 => '┤',
        11 => '├',
        13 => '┴',
        14 => '┬',
        15 => '┼',
        _ => ' ',
    }
}

fn topology_line(row: &TopologyRow) -> Line<'static> {
    Line::from(
        row.cells
            .iter()
            .map(|(glyph, color)| {
                Span::styled(
                    format!("{glyph} "),
                    Style::default()
                        .fg(lane_color(*color))
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect::<Vec<_>>(),
    )
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

fn refs_line(refs: &[CommitRef]) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, reference) in refs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(ref_span(reference));
    }
    Line::from(spans)
}

fn ref_span(reference: &CommitRef) -> Span<'static> {
    let (label, style) = match reference.kind {
        CommitRefKind::Head => (
            "HEAD".to_owned(),
            Style::default().fg(Color::Black).bg(Color::LightGreen),
        ),
        CommitRefKind::LocalBranch => (
            format!("L:{}", reference.name),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        CommitRefKind::RemoteBranch => (
            format!("R:{}", reference.name),
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        CommitRefKind::Tag => (
            format!("T:{}", reference.name),
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ),
        CommitRefKind::Stash => (
            format!("S:{}", reference.name),
            Style::default().fg(Color::White).bg(Color::Magenta),
        ),
    };
    Span::styled(format!(" {label} "), style.add_modifier(Modifier::BOLD))
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
        let rows = topology_rows(&commits);
        assert_eq!(rows.len(), commits.len());
        assert_eq!(rows[0].cells[0].0, '◆');
        assert!(rows[0].cells.iter().any(|(glyph, _)| *glyph == '┐'));
        assert!(rows[2].cells.iter().any(|(glyph, _)| *glyph == '├'));
        assert!(rows.iter().any(|row| row.cells.len() >= 2));
    }

    #[test]
    fn renders_split_without_merge_commit() {
        let commits = vec![
            commit("feature", &["base"]),
            commit("main", &["base"]),
            commit("base", &[]),
        ];
        let rows = topology_rows(&commits);
        assert!(rows[1].cells.iter().any(|(glyph, _)| *glyph == '├'));
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
        let rows = topology_rows(&commits);
        assert!(rows.iter().any(|row| row.cells.len() == 6));
        let colors = rows
            .iter()
            .find(|row| row.cells.len() == 6)
            .unwrap()
            .cells
            .iter()
            .map(|(_, color)| *color)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(colors.len(), 6);
    }

    #[test]
    fn box_glyph_covers_line_junction_combinations() {
        assert_eq!(box_glyph(12), '─');
        assert_eq!(box_glyph(7), '┤');
        assert_eq!(box_glyph(11), '├');
        assert_eq!(box_glyph(13), '┴');
        assert_eq!(box_glyph(14), '┬');
        assert_eq!(box_glyph(15), '┼');
        assert_eq!(box_glyph(5), '┘');
        assert_eq!(box_glyph(6), '┐');
        assert_eq!(box_glyph(9), '└');
        assert_eq!(box_glyph(10), '┌');
    }

    #[test]
    fn box_glyph_uses_solid_single_direction_lines() {
        assert_eq!(box_glyph(1), '│');
        assert_eq!(box_glyph(2), '│');
        assert_eq!(box_glyph(4), '─');
        assert_eq!(box_glyph(8), '─');
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
}
