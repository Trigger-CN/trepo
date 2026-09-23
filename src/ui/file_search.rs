use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::state::{App, FileSearchState};

/// Renders the workspace file-search page: the one-field query form, the
/// per-repository path results, or the opened file's commit history. Only the
/// visible viewport is materialised, so a wide match set still draws in
/// constant time.
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
                    .title(app.language.text(" File search ", " 文件检索 "))
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    }

    let state = &app.file_search;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);
    render_header(frame, app, state, vertical[0]);
    if let Some(view) = state
        .history
        .as_ref()
        .and_then(|history| history.commit_view.as_ref())
    {
        super::range_history::render_commit_view(frame, app, view, vertical[1]);
    } else if let Some(history) = state.history.as_ref() {
        render_history(frame, app, history, vertical[1]);
    } else if state.form.is_some() {
        render_form(frame, app, state, vertical[1]);
    } else {
        render_results(frame, app, state, vertical[1]);
    }
    render_footer(frame, app, state, vertical[2]);
}

fn render_header(frame: &mut Frame, app: &App, state: &FileSearchState, area: Rect) {
    let scope = if app.selected_projects.is_empty() {
        app.language.text("all repositories", "全部仓库").to_owned()
    } else {
        format!(
            "{} {}",
            app.selected_projects.len(),
            app.language.text("selected repositories", "个已选仓库")
        )
    };
    let query = if state.spec.query.is_empty() {
        app.language.text("no query", "无查询").to_owned()
    } else {
        state.spec.summary()
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
                "  {}  /  {query}  [{scope}]",
                app.language.text("File search", "文件检索"),
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_form(frame: &mut Frame, app: &App, state: &FileSearchState, area: Rect) {
    let Some(form) = state.form.as_ref() else {
        return;
    };
    let mut lines = vec![Line::styled(
        format!("> {}: {}", app.language.label("Path"), form.query),
        super::selection_style(),
    )];
    lines.push(Line::raw(""));
    if let Some((error, message)) = &state.message {
        lines.push(Line::styled(
            message.clone(),
            Style::default().fg(if *error { Color::Red } else { Color::Green }),
        ));
    }
    lines.push(Line::styled(
        app.language
            .text("Enter search   Esc cancel", "Enter 检索   Esc 取消"),
        Color::DarkGray,
    ));
    lines.push(Line::styled(
        app.language.text(
            "A file name, a path fragment, or a full path; matching is case-insensitive",
            "可输入文件名、路径片段或完整路径；匹配不区分大小写",
        ),
        Color::DarkGray,
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(app.language.text(" Find file ", " 查找文件 "))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_results(frame: &mut Frame, app: &App, state: &FileSearchState, area: Rect) {
    let total = app.file_search_total();
    let error_count = state.projects.iter().filter(|p| p.error.is_some()).count();
    let capped_count = state.projects.iter().filter(|p| p.capped).count();

    if total == 0 {
        let message = if state.loading {
            app.language.text("Searching...", "正在检索...").to_owned()
        } else if let Some((_, text)) = app.file_search_message() {
            text
        } else {
            app.language
                .text("No matching files", "没有匹配的文件")
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

    let block = results_block(app, total, error_count, capped_count);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    // The column header stays pinned above the scrolling body: the owning
    // repository is named by each group heading rather than by a column.
    let header_area = Rect { height: 1, ..inner };
    let body = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            app.language.text("  File", "  文件"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        header_area,
    );
    if body.height == 0 {
        return;
    }

    // Flat display lines: each repository contributes a two-line heading
    // (name, then its full directory) followed by one line per match.
    const HEADER_HEIGHT: usize = 2;
    /// One display line of the grouped body.
    #[derive(Clone, Copy, PartialEq)]
    enum DisplayLine {
        /// First heading line: the repository name.
        Name(usize),
        /// Second heading line: that repository's full directory.
        Directory(usize),
        /// One matched file of a repository.
        File(usize, usize),
    }
    let mut display = Vec::<DisplayLine>::new();
    for (project_index, project) in state.projects.iter().enumerate() {
        display.push(DisplayLine::Name(project_index));
        display.push(DisplayLine::Directory(project_index));
        for file_index in 0..project.files.len() {
            display.push(DisplayLine::File(project_index, file_index));
        }
    }
    // Display position of the selected match: everything before it is the
    // preceding repositories' headings and their match lines.
    let selected_line = {
        let (project_index, _) = app
            .file_search_rows()
            .get(state.selected)
            .copied()
            .unwrap_or((0, 0));
        let mut line = 0usize;
        for (index, project) in state.projects.iter().enumerate() {
            if index == project_index {
                let offset = app
                    .file_search_rows()
                    .iter()
                    .take(state.selected)
                    .filter(|(value, _)| *value == project_index)
                    .count();
                line += HEADER_HEIGHT + offset;
                break;
            }
            line += HEADER_HEIGHT + project.files.len();
        }
        line
    };
    let start = viewport_start(selected_line, usize::from(body.height), display.len());
    let end = (start + usize::from(body.height)).min(display.len());

    let mut cursor_y = body.y;
    for (line_index, line) in display.iter().enumerate().take(end).skip(start) {
        let rect = Rect {
            y: cursor_y,
            height: 1,
            ..body
        };
        match *line {
            DisplayLine::Name(project_index) => {
                let Some(project) = state.projects.get(project_index) else {
                    continue;
                };
                let count = project.files.len();
                let mut spans = vec![
                    Span::styled(
                        super::text::truncate(&project.project_name, inner.width as usize),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  ({count})"), Style::default().fg(Color::DarkGray)),
                ];
                if project.capped {
                    spans.push(Span::styled(
                        app.language.text("  capped", "  已截断"),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                if let Some(error) = project.error.as_ref() {
                    spans.push(Span::styled(
                        format!("  !{}", super::text::truncate(error, 40)),
                        Style::default().fg(Color::Red),
                    ));
                }
                frame.render_widget(Paragraph::new(Line::from(spans)), rect);
            }
            DisplayLine::Directory(project_index) => {
                let Some(project) = state.projects.get(project_index) else {
                    continue;
                };
                // The directory owns a full-width line of its own, so it is
                // never squeezed into a truncated table column.
                let directory = app
                    .workspace
                    .projects
                    .iter()
                    .find(|candidate| candidate.id == project.project_id)
                    .map_or_else(
                        || project.project_name.clone(),
                        |owner| owner.path.display().to_string(),
                    );
                frame.render_widget(
                    Paragraph::new(Line::styled(
                        super::text::truncate(&directory, inner.width as usize),
                        Style::default().fg(Color::DarkGray),
                    )),
                    rect,
                );
            }
            DisplayLine::File(project_index, file_index) => {
                let Some(project) = state.projects.get(project_index) else {
                    continue;
                };
                let Some(file) = project.files.get(file_index) else {
                    continue;
                };
                let is_selected = line_index == selected_line;
                let style = if is_selected {
                    super::selection_style()
                } else {
                    Style::default()
                };
                let marker = if is_selected { ">" } else { " " };
                let row = Row::new([Cell::from(Line::styled(
                    format!(
                        "{marker} {}",
                        super::text::truncate(file, inner.width.saturating_sub(3) as usize)
                    ),
                    Style::default().fg(super::selection_fg(is_selected, Color::White)),
                ))])
                .style(style);
                frame.render_widget(Table::new(vec![row], [Constraint::Min(10)]), rect);
            }
        }
        cursor_y += 1;
    }
}

/// One file's commit history: newest first, with the path recorded in each
/// commit so a rename shows the file's historical name.
fn render_history(
    frame: &mut Frame,
    app: &App,
    history: &crate::app::state::FileHistoryView,
    area: Rect,
) {
    let title = format!(
        " {}  {} ",
        app.language.text("File history", "文件历史"),
        super::text::truncate(
            &format!("{}:{}", history.project.name, history.path),
            area.width.saturating_sub(24) as usize
        )
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    if history.loading {
        frame.render_widget(
            Paragraph::new(app.language.text("Loading history...", "正在加载历史..."))
                .style(Style::default().fg(Color::Yellow))
                .block(block),
            area,
        );
        return;
    }
    if let Some(error) = history.error.as_ref() {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
        return;
    }
    if history.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(
                app.language
                    .text("No commits touch this file.", "没有提交涉及该文件。"),
            )
            .style(Style::default().fg(Color::DarkGray))
            .block(block),
            area,
        );
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }
    let header_area = Rect { height: 1, ..inner };
    let body = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };
    // 1 marker + 9 commit + 10 date + 14 author + 4 status + separators.
    let subject_width = (inner.width as usize)
        .saturating_sub(1 + 9 + 10 + 14 + 4 + 5)
        .max(8);
    let widths = [
        Constraint::Length(1),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(4),
        Constraint::Min(8),
    ];
    let labels: [&'static str; 6] = [
        " ",
        app.language.label("Commit"),
        app.language.label("Date"),
        app.language.label("Author"),
        app.language.label("Status"),
        app.language.label("Subject"),
    ];
    let header = labels
        .into_iter()
        .map(|label| {
            Cell::from(Line::styled(
                label,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Table::new(vec![Row::new(header)], widths).column_spacing(1),
        header_area,
    );

    let budget = usize::from(body.height);
    let start = viewport_start(history.selected, budget, history.entries.len());
    let end = (start + budget).min(history.entries.len());
    let rows = history.entries[start..end]
        .iter()
        .enumerate()
        .map(|(offset, entry)| {
            let index = start + offset;
            let is_selected = index == history.selected;
            let style = if is_selected {
                super::selection_style()
            } else {
                Style::default()
            };
            Row::new([
                Cell::from(Line::styled(
                    if is_selected { ">" } else { " " },
                    Style::default()
                        .fg(super::selection_fg(is_selected, Color::Cyan))
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(Line::styled(
                    entry.commit.oid.chars().take(8).collect::<String>(),
                    Style::default().fg(super::selection_fg(is_selected, Color::LightBlue)),
                )),
                Cell::from(Line::styled(
                    super::graph::calendar_date(entry.commit.timestamp),
                    Style::default().fg(super::selection_fg(is_selected, Color::Gray)),
                )),
                Cell::from(Line::styled(
                    super::text::truncate(&entry.commit.author.replace(['\n', '\r'], " "), 14),
                    Style::default().fg(super::selection_fg(is_selected, Color::Gray)),
                )),
                Cell::from(Line::styled(
                    super::text::truncate(&entry.status, 4),
                    Style::default().fg(super::selection_fg(is_selected, Color::Yellow)),
                )),
                Cell::from(Line::styled(
                    {
                        // The recorded path keeps a rename readable even when
                        // the subject itself is truncated by the column.
                        let subject = entry.commit.subject.replace(['\n', '\r'], " ");
                        if entry.path == history.path {
                            super::text::truncate(&subject, subject_width)
                        } else {
                            let provenance = format!(" (was {})", entry.path);
                            format!(
                                "{}{}",
                                super::text::truncate(
                                    &subject,
                                    subject_width
                                        .saturating_sub(super::text::display_width(&provenance))
                                ),
                                super::text::truncate(&provenance, subject_width.saturating_sub(8)),
                            )
                        }
                    },
                    Style::default().fg(super::selection_fg(is_selected, Color::White)),
                )),
            ])
            .style(style)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Table::new(rows, widths).column_spacing(1), body);
}

fn results_block(
    app: &App,
    total: usize,
    error_count: usize,
    capped_count: usize,
) -> Block<'static> {
    let mut notes = String::new();
    if error_count > 0 {
        notes.push_str(&format!(", {error_count} failed"));
    }
    if capped_count > 0 {
        notes.push_str(&format!(", {capped_count} capped"));
    }
    Block::default()
        .title(format!(
            " {} ({total}){notes} ",
            app.language.text("File search", "文件检索")
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn render_footer(frame: &mut Frame, app: &App, state: &FileSearchState, area: Rect) {
    let footer = if state
        .history
        .as_ref()
        .is_some_and(|history| history.commit_view.is_some())
    {
        app.language.text(
            "j/k Scroll   PgUp/PgDn Page   Esc Back to history",
            "j/k 滚动   PgUp/PgDn 翻页   Esc 返回历史",
        )
    } else if state.history.is_some() {
        app.language.text(
            "Enter Detail   l Locate   j/k Move   g/G First/Last   Esc Back to results",
            "Enter 详情   l 定位   j/k 移动   g/G 首/末   Esc 返回列表",
        )
    } else if state.form.is_some() {
        app.language
            .text("Enter search   Esc cancel", "Enter 检索   Esc 取消")
    } else if state.loading {
        app.language
            .text("Searching...   Esc back", "正在检索...   Esc 返回")
    } else {
        app.language.text(
            "Enter History   j/k Move   g/G First/Last   f Filter   r Rerun   Esc Back",
            "Enter 历史   j/k 移动   g/G 首/末   f 过滤   r 重跑   Esc 返回",
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
}
