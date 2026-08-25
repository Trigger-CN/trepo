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
