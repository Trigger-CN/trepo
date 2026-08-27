use std::collections::{HashMap, HashSet};

use crate::domain::Commit;

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GraphEdgeKind {
    Direct,
    Indirect,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GraphEdge {
    pub target: String,
    pub kind: GraphEdgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GraphNode {
    pub oid: String,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PipeKind {
    Starts,
    Continues,
    Terminates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pipe {
    pub from_lane: usize,
    pub to_lane: usize,
    pub color: usize,
    pub kind: PipeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TopologyCell {
    pub glyph: char,
    pub color: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TopologyRow {
    pub cells: Vec<TopologyCell>,
    pub pipes: Vec<Pipe>,
    pub hidden_lanes: usize,
    pub has_missing_edge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Lane {
    oid: String,
    color: usize,
}

pub(super) fn graph_nodes(commits: &[Commit]) -> Vec<GraphNode> {
    let positions = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.as_str(), index))
        .collect::<HashMap<_, _>>();
    commits
        .iter()
        .enumerate()
        .map(|(index, commit)| GraphNode {
            oid: commit.oid.clone(),
            edges: commit
                .parents
                .iter()
                .map(|parent| GraphEdge {
                    target: parent.clone(),
                    kind: match positions.get(parent.as_str()) {
                        Some(parent_index) if *parent_index > index => GraphEdgeKind::Direct,
                        Some(_) => GraphEdgeKind::Indirect,
                        None => GraphEdgeKind::Missing,
                    },
                })
                .collect(),
        })
        .collect()
}

pub(super) fn topology_rows(commits: &[Commit], max_visible_lanes: usize) -> Vec<TopologyRow> {
    let nodes = graph_nodes(commits);
    layout_nodes(&nodes, max_visible_lanes.max(1))
}

fn layout_nodes(nodes: &[GraphNode], max_visible_lanes: usize) -> Vec<TopologyRow> {
    let future = nodes
        .iter()
        .map(|node| node.oid.as_str())
        .collect::<HashSet<_>>();
    let mut lanes = Vec::<Lane>::new();
    let mut rows = Vec::with_capacity(nodes.len());
    let mut next_color = 0;

    for node in nodes {
        let (node_lane, node_color, display_lanes) =
            if let Some(index) = lanes.iter().position(|lane| lane.oid == node.oid) {
                (index, lanes[index].color, lanes.clone())
            } else {
                let color = next_color;
                next_color += 1;
                let mut display = lanes.clone();
                display.push(Lane {
                    oid: node.oid.clone(),
                    color,
                });
                (display.len() - 1, color, display)
            };

        let mut next_lanes = display_lanes.clone();
        next_lanes.remove(node_lane);
        let direct_edges = node
            .edges
            .iter()
            .filter(|edge| {
                edge.kind == GraphEdgeKind::Direct && future.contains(edge.target.as_str())
            })
            .collect::<Vec<_>>();
        let mut insert_at = node_lane.min(next_lanes.len());
        for (parent_index, edge) in direct_edges.iter().enumerate() {
            if next_lanes.iter().any(|lane| lane.oid == edge.target) {
                continue;
            }
            let color = if parent_index == 0 {
                node_color
            } else {
                let color = next_color;
                next_color += 1;
                color
            };
            next_lanes.insert(
                insert_at,
                Lane {
                    oid: edge.target.clone(),
                    color,
                },
            );
            insert_at += 1;
        }

        let mut pipes = Vec::new();
        for (from_lane, lane) in display_lanes.iter().enumerate() {
            if from_lane == node_lane {
                continue;
            }
            if let Some(to_lane) = next_lanes.iter().position(|next| next.oid == lane.oid) {
                pipes.push(Pipe {
                    from_lane,
                    to_lane,
                    color: lane.color,
                    kind: PipeKind::Continues,
                });
            } else {
                pipes.push(Pipe {
                    from_lane,
                    to_lane: from_lane,
                    color: lane.color,
                    kind: PipeKind::Terminates,
                });
            }
        }
        for edge in &direct_edges {
            if let Some(to_lane) = next_lanes.iter().position(|lane| lane.oid == edge.target) {
                pipes.push(Pipe {
                    from_lane: node_lane,
                    to_lane,
                    color: next_lanes[to_lane].color,
                    kind: PipeKind::Starts,
                });
            }
        }
        pipes.sort_by_key(|pipe| match pipe.kind {
            PipeKind::Continues => 0,
            PipeKind::Starts => 1,
            PipeKind::Terminates => 2,
        });

        let full_width = display_lanes.len().max(next_lanes.len()).max(node_lane + 1);
        let mut masks = vec![0u8; full_width];
        let mut colors = vec![node_color; full_width];
        for (index, lane) in display_lanes.iter().enumerate() {
            masks[index] |= UP;
            colors[index] = lane.color;
        }
        for (index, lane) in next_lanes.iter().enumerate() {
            masks[index] |= DOWN;
            if masks[index] == DOWN {
                colors[index] = lane.color;
            }
        }
        for pipe in &pipes {
            add_pipe(&mut masks, &mut colors, *pipe);
        }
        masks[node_lane] = 0;
        colors[node_lane] = node_color;
        let node_glyph = if node
            .edges
            .iter()
            .any(|edge| edge.kind == GraphEdgeKind::Missing)
        {
            '◉'
        } else if direct_edges.len() > 1 {
            '◆'
        } else {
            '●'
        };
        let mut cells = masks
            .into_iter()
            .zip(colors)
            .enumerate()
            .map(|(index, (mask, color))| TopologyCell {
                glyph: if index == node_lane {
                    node_glyph
                } else {
                    box_glyph(mask)
                },
                color,
            })
            .collect::<Vec<_>>();
        let hidden_lanes = cells.len().saturating_sub(max_visible_lanes);
        cells.truncate(max_visible_lanes);
        rows.push(TopologyRow {
            cells,
            pipes,
            hidden_lanes,
            has_missing_edge: node
                .edges
                .iter()
                .any(|edge| edge.kind == GraphEdgeKind::Missing),
        });
        lanes = next_lanes;
    }
    rows
}

fn add_pipe(masks: &mut [u8], colors: &mut [usize], pipe: Pipe) {
    if pipe.from_lane >= masks.len() || pipe.to_lane >= masks.len() {
        return;
    }
    if pipe.kind != PipeKind::Starts {
        masks[pipe.from_lane] |= UP;
    }
    if pipe.kind != PipeKind::Terminates {
        masks[pipe.to_lane] |= DOWN;
    }
    colors[pipe.from_lane] = pipe.color;
    colors[pipe.to_lane] = pipe.color;
    if pipe.from_lane == pipe.to_lane {
        return;
    }
    let (start, end) = if pipe.from_lane < pipe.to_lane {
        masks[pipe.from_lane] |= RIGHT;
        masks[pipe.to_lane] |= LEFT;
        (pipe.from_lane, pipe.to_lane)
    } else {
        masks[pipe.from_lane] |= LEFT;
        masks[pipe.to_lane] |= RIGHT;
        (pipe.to_lane, pipe.from_lane)
    };
    for mask in masks.iter_mut().take(end).skip(start + 1) {
        *mask |= LEFT | RIGHT;
    }
}

pub(super) fn box_glyph(mask: u8) -> char {
    match mask {
        0 => ' ',
        1..=3 => '│',
        4 | 8 | 12 => '─',
        5 => '┘',
        6 => '┐',
        7 => '┤',
        9 => '└',
        10 => '┌',
        11 => '├',
        13 => '┴',
        14 => '┬',
        15 => '┼',
        _ => ' ',
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
    fn continuing_pipes_follow_lanes_that_shift_left() {
        let commits = vec![
            commit("merge", &["main", "side"]),
            commit("main", &[]),
            commit("side", &["side-base"]),
            commit("side-base", &[]),
        ];
        let rows = topology_rows(&commits, 10);
        assert!(rows[1].pipes.iter().any(|pipe| {
            pipe.kind == PipeKind::Continues && pipe.from_lane == 1 && pipe.to_lane == 0
        }));
        assert!(rows[1]
            .cells
            .iter()
            .any(|cell| matches!(cell.glyph, '┘' | '┐' | '┤' | '┴' | '┼')));
    }

    #[test]
    fn marks_missing_parents_instead_of_drawing_a_fake_direct_edge() {
        let commits = vec![commit("tip", &["missing-parent"])];
        let nodes = graph_nodes(&commits);
        assert_eq!(nodes[0].edges[0].kind, GraphEdgeKind::Missing);
        let rows = topology_rows(&commits, 10);
        assert!(rows[0].has_missing_edge);
        assert_eq!(rows[0].cells[0].glyph, '◉');
        assert!(rows[0].pipes.is_empty());
    }

    #[test]
    fn reports_hidden_lane_count_when_projection_is_capped() {
        let commits = vec![
            commit("octopus", &["a", "b", "c", "d", "e"]),
            commit("a", &["base"]),
            commit("b", &["base"]),
            commit("c", &["base"]),
            commit("d", &["base"]),
            commit("e", &["base"]),
            commit("base", &[]),
        ];
        let rows = topology_rows(&commits, 3);
        assert_eq!(rows[0].cells.len(), 3);
        assert_eq!(rows[0].hidden_lanes, 2);
    }

    #[test]
    fn box_glyph_covers_solid_line_junctions() {
        assert_eq!(box_glyph(1), '│');
        assert_eq!(box_glyph(12), '─');
        assert_eq!(box_glyph(7), '┤');
        assert_eq!(box_glyph(11), '├');
        assert_eq!(box_glyph(13), '┴');
        assert_eq!(box_glyph(14), '┬');
        assert_eq!(box_glyph(15), '┼');
    }
}
