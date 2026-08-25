use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::state::App;
use crate::domain::{Commit, CommitRef, CommitRefKind};

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
            Span::raw(format!("  {}  /  Graph", graph.project.name)),
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
        "Loading commits..."
    } else {
        "j/k Move  g/G Ends  r Reload  Esc Back  L local  R remote  T tag  S stash"
    };
    frame.render_widget(Paragraph::new(footer), vertical[2]);
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

    let visible = area.height.saturating_sub(3) as usize;
    let start = viewport_start(graph.selected, visible, graph.commits.len());
    let topology = topology_rows(&graph.commits);
    let graph_width = topology
        .iter()
        .map(|row| row.cells.len().saturating_mul(2))
        .max()
        .unwrap_or(2)
        .max(7)
        .min(area.width.saturating_sub(48).max(7) as usize) as u16;
    let rows = graph
        .commits
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, commit)| {
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
                    relative_age(commit.timestamp),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
            .style(style)
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
            Constraint::Length(5),
        ],
    )
    .header(
        Row::new(["", "Graph", "Commit", "Subject", "Refs", "Author", "Age"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1)
    .block(
        Block::default()
            .title(format!(" All refs commit graph ({}) ", graph.commits.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(table, area);
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
    let lines = graph.commits.get(graph.selected).map_or_else(
        || {
            vec![Line::raw(if graph.loading {
                "Loading..."
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
                detail_line("Timestamp", commit.timestamp.to_string(), Color::DarkGray),
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
    let mut lanes: Vec<Lane> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let positions: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.as_str(), index))
        .collect();
    let mut next_color = 0;

    for commit in commits {
        let lane = lanes
            .iter()
            .position(|value| value.oid == commit.oid)
            .unwrap_or_else(|| {
                let color = next_color;
                next_color += 1;
                lanes.insert(
                    0,
                    Lane {
                        oid: commit.oid.clone(),
                        color,
                    },
                );
                0
            });
        rows.push(TopologyRow {
            cells: lanes
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let glyph = if index == lane {
                        if commit.parents.len() > 1 {
                            '◆'
                        } else {
                            '●'
                        }
                    } else {
                        '│'
                    };
                    (glyph, value.color)
                })
                .collect(),
        });

        let inherited_color = lanes[lane].color;
        lanes.remove(lane);
        let mut insert_at = lane.min(lanes.len());
        for (parent_index, parent) in commit.parents.iter().enumerate() {
            if !positions.contains_key(parent.as_str())
                || lanes.iter().any(|value| value.oid == *parent)
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
            lanes.insert(
                insert_at,
                Lane {
                    oid: parent.clone(),
                    color,
                },
            );
            insert_at += 1;
        }
    }
    rows
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
    fn renders_basic_topology_lanes() {
        let commits = vec![
            commit("merge", &["left", "right"]),
            commit("left", &["base"]),
            commit("right", &["base"]),
            commit("base", &[]),
        ];
        let rows = topology_rows(&commits);
        assert_eq!(rows.len(), commits.len());
        assert_eq!(rows[0].cells[0].0, '◆');
        assert!(rows.iter().any(|row| row.cells.len() >= 2));
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
