use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::state::{App, RangeHistoryGroup, RangeHistoryState};

/// Renders the workspace range-query page: the filter form, or the merged
/// cross-repository commit timeline with the same table language as the Graph
/// page (minus the lanes). Only the visible viewport is materialised, so a
/// capped-but-large result set still draws in constant time.
pub(super) fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 60 || area.height < 12 {
        frame.render_widget(
            Paragraph::new(app.language.text(
                "Terminal too small. Resize to at least 60x12. Press Esc to return.",
                "终端太小，请调整到至少 60x12，按 Esc 返回。",
            ))
            .block(
                Block::default()
                    .title(app.language.text(" Range history ", " 时间范围检索 "))
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    }

    let state = &app.range_history;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);
    render_header(frame, app, state, vertical[0]);
    if let Some(view) = state.view.as_ref() {
        render_commit_view(frame, app, view, vertical[1]);
    } else if state.form.is_some() {
        render_form(frame, app, state, vertical[1]);
    } else {
        render_results(frame, app, state, vertical[1]);
    }
    render_footer(frame, app, state, vertical[2]);
}

/// Renders one located commit: metadata, its full message, then the diffstat
/// and patch produced by `git show`. Only the visible lines are drawn, so a
/// large patch costs the same as a small one.
pub(super) fn render_commit_view(
    frame: &mut Frame,
    app: &App,
    view: &crate::app::state::RangeCommitView,
    area: Rect,
) {
    let title = format!(
        " {} {}  {}  {} ",
        view.project_name,
        view.short_oid,
        app.language.label("Commit"),
        super::text::truncate(
            &view.commit.subject.clone(),
            area.width.saturating_sub(40) as usize,
        )
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    if view.loading {
        frame.render_widget(
            Paragraph::new(app.language.text("Loading commit...", "正在加载提交..."))
                .style(Style::default().fg(Color::Yellow))
                .block(block),
            area,
        );
        return;
    }
    if let Some(error) = view.error.as_ref() {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
        return;
    }

    let width = area.width.saturating_sub(2) as usize;
    let mut lines = vec![
        Line::styled(
            view.commit.subject.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        super::graph::detail_line(
            app.language.label("Commit"),
            view.commit.oid.clone(),
            Color::LightBlue,
        ),
        super::graph::detail_line(
            app.language.label("Author"),
            view.commit.author.clone(),
            Color::Gray,
        ),
        super::graph::detail_line(
            app.language.label("Date"),
            super::graph::calendar_date(view.commit.timestamp),
            Color::Gray,
        ),
        super::graph::detail_line(
            app.language.label("Parents"),
            if view.commit.parents.is_empty() {
                "-".to_owned()
            } else {
                view.commit.parents.join(" ")
            },
            Color::LightBlue,
        ),
        Line::raw(""),
    ];
    for source_line in view.commit.body.split('\n') {
        lines.extend(
            super::text::wrap(source_line.trim_end_matches('\r'), width)
                .into_iter()
                .map(|line| Line::styled(line, Style::default().fg(Color::Gray))),
        );
    }
    lines.push(Line::raw(""));

    if view.text.trim().is_empty() {
        let message = if view.commit.parents.len() > 1 {
            app.language.text(
                "No changes against the first parent.",
                "相对第一个父提交没有变更。",
            )
        } else {
            app.language.text("No textual changes.", "没有文本变更。")
        };
        lines.push(Line::styled(message, Style::default().fg(Color::DarkGray)));
    } else {
        lines.extend(patch_lines(&view.text, width));
    }

    let scroll = u16::try_from(view.scroll).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
}

/// Colours the patch like the Changes diff: additions green, removals red,
/// hunk headers cyan and file headers yellow.
pub(super) fn patch_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| {
            let style = if line.starts_with("+++") || line.starts_with("---") {
                Style::default().fg(Color::Yellow)
            } else if line.starts_with('+') {
                Style::default().fg(Color::Green)
            } else if line.starts_with('-') {
                Style::default().fg(Color::Red)
            } else if line.starts_with("@@") || line.starts_with("diff --git ") {
                Style::default().fg(Color::Cyan)
            } else if line.contains("|") && line.contains("changed") {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            Line::styled(super::text::truncate(line, width), style)
        })
        .collect()
}

fn render_header(frame: &mut Frame, app: &App, state: &RangeHistoryState, area: Rect) {
    let scope = if app.selected_projects.is_empty() {
        app.language.text("all repositories", "全部仓库").to_owned()
    } else {
        format!(
            "{} {}",
            app.selected_projects.len(),
            app.language.text("selected repositories", "个已选仓库")
        )
    };
    let filter = state.spec.summary();
    let filter = if filter.is_empty() {
        app.language.text("no range", "无范围").to_owned()
    } else {
        filter
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " trepo ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {}  /  {filter}  [{}]",
                app.language.text("Range history", "时间范围检索"),
                scope
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_form(frame: &mut Frame, app: &App, state: &RangeHistoryState, area: Rect) {
    let Some(form) = state.form.as_ref() else {
        return;
    };
    let mut lines = Vec::new();
    for (index, (label, value)) in form.fields().iter().enumerate() {
        let marker = if index == form.selected { "> " } else { "  " };
        let style = if index == form.selected {
            super::selection_style()
        } else {
            Style::default()
        };
        lines.push(Line::styled(
            format!("{marker}{}: {value}", app.language.label(label)),
            style,
        ));
    }
    lines.push(Line::raw(""));
    if let Some((error, message)) = &state.message {
        lines.push(Line::styled(
            message.clone(),
            Style::default().fg(if *error { Color::Red } else { Color::Green }),
        ));
    }
    lines.push(Line::styled(
        app.language.text(
            "Enter run   Tab field   Esc cancel",
            "Enter 执行   Tab 字段   Esc 取消",
        ),
        Color::DarkGray,
    ));
    lines.push(Line::styled(
        app.language.text(
            "Dates use YYYY-MM-DD; empty means unbounded",
            "日期格式 YYYY-MM-DD；留空表示不限制",
        ),
        Color::DarkGray,
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(app.language.text(" Range filters ", " 范围过滤 "))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_results(frame: &mut Frame, app: &App, state: &RangeHistoryState, area: Rect) {
    let total = app.range_history_total();
    let error_count = state.projects.iter().filter(|p| p.error.is_some()).count();
    let capped_count = state.projects.iter().filter(|p| p.capped).count();

    if total == 0 {
        let message = if state.loading {
            app.language.text("Loading...", "正在加载...").to_owned()
        } else if let Some((_, text)) = app.range_history_message() {
            text
        } else {
            app.language
                .text("No commits in range", "范围内没有提交")
                .to_owned()
        };
        frame.render_widget(
            Paragraph::new(vec![Line::raw(message), Line::raw("")])
                .block(results_block(app, total, error_count, capped_count))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let groups = app.range_history_groups();
    // Each group is a two-line header plus one line per matched commit, so
    // the whole body is a flat list of unit-height display lines.
    let total_lines = groups
        .iter()
        .map(|group| RangeHistoryGroup::HEADER_HEIGHT + group.count)
        .sum::<usize>();
    let selected = state.selected.min(total - 1);
    let title = format!(
        " {} ({}){} ",
        app.language.text("Range history", "时间范围检索"),
        total,
        result_notes(error_count, capped_count)
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    // The column header stays pinned above the scrolling grouped body and no
    // longer needs a Project column: the owning repository is the group name.
    let header_area = Rect { height: 1, ..inner };
    let inner_width = inner.width as usize;
    let subject_width = subject_column_width(inner_width);
    frame.render_widget(
        Paragraph::new(columns_line(
            " ",
            app.language.label("Commit"),
            app.language.label("Date"),
            app.language.label("Author"),
            app.language.label("Subject"),
            subject_width,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        header_area,
    );
    let body = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };
    if body.height == 0 {
        return;
    }

    // Only the display lines inside the viewport are built; the cached order
    // means scrolling never re-sorts or re-groups.
    let selected_line = display_line_of(groups, selected);
    let start = viewport_start(selected_line, usize::from(body.height), total_lines);
    let end = (start + usize::from(body.height)).min(total_lines);

    let mut cursor_y = body.y;
    let mut line = 0usize;
    for group in groups {
        let group_lines = RangeHistoryGroup::HEADER_HEIGHT + group.count;
        let group_start = line;
        let group_end = line + group_lines;
        line = group_end;
        if group_end <= start {
            continue;
        }
        if group_start >= end {
            break;
        }
        let visible_start = start.max(group_start);
        let visible_end = end.min(group_end);

        let header_start = visible_start.max(group_start);
        let header_end = visible_end.min(group_start + RangeHistoryGroup::HEADER_HEIGHT);
        if header_end > header_start {
            let height = (header_end - header_start) as u16;
            let rect = Rect {
                y: cursor_y,
                height,
                ..body
            };
            let mut lines = Vec::new();
            for header_line in header_start..header_end {
                if header_line == group_start {
                    let count = if group.count == 1 {
                        "1 commit".to_owned()
                    } else {
                        format!("{} commits", group.count)
                    };
                    let mut spans = vec![Span::styled(
                        super::text::truncate(&group.name, inner_width),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )];
                    spans.push(Span::styled(
                        format!("  ({count})"),
                        Style::default().fg(Color::DarkGray),
                    ));
                    if group.capped {
                        spans.push(Span::styled(
                            app.language.text("  capped", "  已截断"),
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                    lines.push(Line::from(spans));
                } else {
                    // The directory gets its own full-width line so it is
                    // never squeezed into a truncated table column; text
                    // helpers keep control characters visible.
                    lines.push(Line::styled(
                        super::text::truncate(&group.directory, inner_width),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            frame.render_widget(Paragraph::new(lines), rect);
            cursor_y += height;
        }

        let commit_start = visible_start.max(group_start + RangeHistoryGroup::HEADER_HEIGHT);
        if visible_end > commit_start {
            let height = (visible_end - commit_start) as u16;
            let rect = Rect {
                y: cursor_y,
                height,
                ..body
            };
            let rows = (commit_start - group_start - RangeHistoryGroup::HEADER_HEIGHT
                ..commit_start - group_start - RangeHistoryGroup::HEADER_HEIGHT
                    + usize::from(height))
                .filter_map(|offset| {
                    let index = group.first_row + offset;
                    let (_, oid, commit) = app.range_history_row(index)?;
                    let is_selected = index == selected;
                    let style = if is_selected {
                        super::selection_style()
                    } else {
                        Style::default()
                    };
                    Some(
                        Row::new([
                            Cell::from(Line::styled(
                                if is_selected { ">" } else { " " },
                                Style::default()
                                    .fg(super::selection_fg(is_selected, Color::Cyan))
                                    .add_modifier(Modifier::BOLD),
                            )),
                            Cell::from(Line::styled(
                                oid,
                                Style::default()
                                    .fg(super::selection_fg(is_selected, Color::LightBlue)),
                            )),
                            Cell::from(Line::styled(
                                super::graph::calendar_date(commit.timestamp),
                                Style::default().fg(super::selection_fg(is_selected, Color::Gray)),
                            )),
                            Cell::from(Line::styled(
                                super::text::truncate(
                                    &commit.author.replace(['\n', '\r'], " "),
                                    14,
                                ),
                                Style::default().fg(super::selection_fg(is_selected, Color::Gray)),
                            )),
                            Cell::from(Line::styled(
                                super::text::truncate(
                                    &commit.subject.replace(['\n', '\r'], " "),
                                    subject_width,
                                ),
                                Style::default().fg(super::selection_fg(is_selected, Color::White)),
                            )),
                        ])
                        .style(style),
                    )
                })
                .collect::<Vec<_>>();
            let widths = [
                Constraint::Length(1),
                Constraint::Length(9),
                Constraint::Length(10),
                Constraint::Length(14),
                Constraint::Min(10),
            ];
            frame.render_widget(Table::new(rows, widths).column_spacing(1), rect);
            cursor_y += height;
        }
    }
}

/// Display line of a flattened row index inside the grouped body.
fn display_line_of(groups: &[RangeHistoryGroup], row_index: usize) -> usize {
    let mut line = 0;
    let mut row = 0;
    for group in groups {
        if row_index < row + group.count {
            return line + RangeHistoryGroup::HEADER_HEIGHT + (row_index - row);
        }
        row += group.count;
        line += RangeHistoryGroup::HEADER_HEIGHT + group.count;
    }
    line.saturating_sub(1)
}

/// Builds the pinned column header so it lines up with the grouped table
/// columns (1 marker, 9 commit, 10 date, 14 author, then subject).
fn columns_line(
    marker: &str,
    oid: &str,
    date: &str,
    author: &str,
    subject: &str,
    subject_width: usize,
    style: Style,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut push = |value: &str, width: usize| {
        let value = super::text::truncate(value, width);
        let padding = width.saturating_sub(super::text::display_width(&value));
        spans.push(Span::styled(value, style));
        spans.push(Span::raw(" ".repeat(padding + 1)));
    };
    push(marker, 1);
    push(oid, 9);
    push(date, 10);
    push(author, 14);
    spans.push(Span::styled(
        super::text::truncate(subject, subject_width),
        style,
    ));
    Line::from(spans)
}

fn results_block(
    app: &App,
    total: usize,
    error_count: usize,
    capped_count: usize,
) -> Block<'static> {
    Block::default()
        .title(format!(
            " {} ({total}){} ",
            app.language.text("Range history", "时间范围检索"),
            result_notes(error_count, capped_count)
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
}

/// Per-repository failure and truncation counts shown in the title, so colour
/// is never the only carrier of that information.
fn result_notes(error_count: usize, capped_count: usize) -> String {
    let mut notes = String::new();
    if error_count > 0 {
        notes.push_str(&format!(", {error_count} failed"));
    }
    if capped_count > 0 {
        notes.push_str(&format!(", {capped_count} capped"));
    }
    notes
}

fn render_footer(frame: &mut Frame, app: &App, state: &RangeHistoryState, area: Rect) {
    let footer = if state.view.is_some() {
        app.language.text(
            "j/k Scroll   PgUp/PgDn Page   l Locate in Graph   Esc Back to results",
            "j/k 滚动   PgUp/PgDn 翻页   l 在提交图中定位   Esc 返回列表",
        )
    } else if state.form.is_some() {
        app.language
            .text("Enter run   Esc cancel", "Enter 执行   Esc 取消")
    } else if state.loading {
        app.language
            .text("Loading...   Esc back", "正在加载...   Esc 返回")
    } else {
        app.language.text(
            "Enter Detail   l Locate   j/k Move   g/G First/Last   f Filter   r Rerun   Esc Back",
            "Enter 详情   l 定位   j/k 移动   g/G 首/末   f 过滤   r 重跑   Esc 返回",
        )
    };
    frame.render_widget(Paragraph::new(Line::styled(footer, Color::DarkGray)), area);
}

fn viewport_start(selected: usize, budget: usize, total: usize) -> usize {
    if total <= budget {
        return 0;
    }
    let start = selected.saturating_sub(budget.saturating_sub(1));
    start.min(total.saturating_sub(budget))
}

fn subject_column_width(width: usize) -> usize {
    // 1 marker + 9 commit + 10 date + 14 author + separators.
    width.saturating_sub(1 + 9 + 10 + 14 + 5).max(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_keeps_the_selected_row_visible_and_bounded() {
        assert_eq!(viewport_start(0, 10, 5), 0);
        assert_eq!(viewport_start(4, 10, 100), 0);
        assert_eq!(viewport_start(10, 10, 100), 1);
        assert_eq!(viewport_start(99, 10, 100), 90);
    }

    #[test]
    fn patch_lines_colour_diff_content() {
        let text =
            " s.txt | 1 +\n\ndiff --git a/s.txt b/s.txt\n+++ b/s.txt\n@@ -0,0 +1 @@\n+s\n-s\n";
        let lines = patch_lines(text, 40);
        // `Line::styled` stores the style on the line, not on its spans.
        let color_at = |index: usize| lines[index].style.fg;
        assert_eq!(color_at(2), Some(Color::Cyan));
        assert_eq!(color_at(3), Some(Color::Yellow));
        assert_eq!(color_at(4), Some(Color::Cyan));
        assert_eq!(color_at(5), Some(Color::Green));
        assert_eq!(color_at(6), Some(Color::Red));
    }

    #[test]
    fn display_lines_account_for_group_headers() {
        let group = |name: &str, count: usize, first_row: usize| RangeHistoryGroup {
            project_index: 0,
            name: name.to_owned(),
            directory: format!("/tmp/{name}"),
            capped: false,
            count,
            first_row,
        };
        let groups = vec![group("alpha", 2, 0), group("beta", 1, 2)];

        // Two header lines precede each group's commit rows.
        assert_eq!(display_line_of(&groups, 0), 2);
        assert_eq!(display_line_of(&groups, 1), 3);
        assert_eq!(display_line_of(&groups, 2), 6);
    }
}
