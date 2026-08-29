use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::change_tree::{change_tree_rows, ChangeTreeRow};
use crate::app::state::{App, ChangesMode, ChangesState, PendingOperation};
use crate::domain::{ChangeEntry, OperationKind, OperationTarget};

fn change_file_style(entry: &ChangeEntry) -> Style {
    if entry.conflicted {
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD)
    } else if entry.untracked {
        Style::default().fg(Color::Yellow)
    } else {
        match (entry.index.is_some(), entry.worktree.is_some()) {
            (true, true) => Style::default().fg(Color::LightMagenta),
            (true, false) => Style::default().fg(Color::LightGreen),
            (false, true) => Style::default().fg(Color::LightRed),
            (false, false) => Style::default(),
        }
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 60 || area.height < 14 {
        frame.render_widget(
            Paragraph::new(app.language.text(
                "Terminal too small. Resize to at least 60x14. Press Esc to return.",
                "终端太小，请调整到至少 60x14，按 Esc 返回。",
            ))
            .block(
                Block::default()
                    .title(app.language.text(" Changes ", " 改动 "))
                    .borders(Borders::ALL),
            ),
            area,
        );
        return;
    }
    let Some(changes) = &app.changes else {
        frame.render_widget(
            Paragraph::new(app.language.text("No repository selected", "未选择仓库")),
            area,
        );
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
    render_header(frame, app, changes, vertical[0]);
    if area.width >= 100 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(vertical[1]);
        render_files(frame, app, changes, body[0]);
        render_preview(frame, app, changes, body[1]);
    } else {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(vertical[1]);
        render_files(frame, app, changes, body[0]);
        render_preview(frame, app, changes, body[1]);
    }
    render_footer(frame, app, changes, vertical[2]);
    if changes.confirmation.is_some() {
        render_confirmation(frame, app, changes);
    }
    if changes.commit_editing {
        render_commit_dialog(frame, app, changes);
    }
}

fn render_header(frame: &mut Frame, app: &App, changes: &ChangesState, area: Rect) {
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
                "  {}  /  {}",
                changes.project.name,
                app.language.text("Changes", "改动")
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_files(frame: &mut Frame, app: &App, changes: &ChangesState, area: Rect) {
    let tree_width = area.width.saturating_sub(13) as usize;
    if let Some(error) = &changes.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(Color::Red))
                .block(
                    Block::default()
                        .title(app.language.text(" Files ", " 文件 "))
                        .borders(Borders::ALL),
                )
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
                Cell::from(super::text::truncate(&row.display(), tree_width)),
            ])
            .style(Style::default().fg(Color::DarkGray)),
            ChangeTreeRow::File { entry_index, .. } => {
                let entry = &changes.entries[entry_index];
                let selected = entry_index == changes.selected;
                let checked = changes.selected_files.contains(&entry.path);
                let row_style = if selected {
                    super::selection_style()
                } else {
                    Style::default()
                };
                let file_style = if selected {
                    Style::default()
                } else {
                    change_file_style(entry)
                };
                Row::new(vec![
                    Cell::from(if selected { ">" } else { " " }),
                    Cell::from(if checked { "[x]" } else { "[ ]" }),
                    Cell::from(entry.status_label()).style(file_style),
                    Cell::from(super::text::truncate(&row.display(), tree_width)).style(file_style),
                ])
                .style(row_style)
            }
        });
    let title = if changes.loading {
        app.language
            .text(" Files (loading) ", " 文件（加载中） ")
            .to_owned()
    } else {
        format!(
            " {} ({}; {} {}) ",
            app.language.text("Files", "文件"),
            changes.entries.len(),
            changes.selected_files.len(),
            app.language.text("selected", "已选")
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
        Row::new([
            "",
            app.language.text("Sel", "选"),
            "XY",
            app.language.text("Tree", "树"),
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
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style),
    );
    frame.render_widget(table, area);
}

fn render_preview(frame: &mut Frame, app: &App, changes: &ChangesState, area: Rect) {
    let title = changes.preview_path.as_ref().map_or_else(
        || format!(" {} ", app.language.label("Diff")),
        |path| match changes.mode {
            ChangesMode::Hunk => changes
                .preview
                .as_ref()
                .and_then(|preview| preview.hunks.get(changes.selected_hunk))
                .map_or_else(
                    || {
                        format!(
                            " {}: {} [{}] ",
                            app.language.label("Diff"),
                            path.display(),
                            app.language.label("hunk")
                        )
                    },
                    |hunk| {
                        format!(
                            " {}: {} [{} {}/{} {}] ",
                            app.language.label("Diff"),
                            path.display(),
                            app.language.label("hunk"),
                            changes.selected_hunk + 1,
                            changes
                                .preview
                                .as_ref()
                                .map_or(0, |preview| preview.hunks.len()),
                            app.language.label(hunk.source.label())
                        )
                    },
                ),
            ChangesMode::Line => changes
                .preview
                .as_ref()
                .and_then(|preview| preview.lines.get(changes.selected_line))
                .map_or_else(
                    || {
                        format!(
                            " {}: {} [{}] ",
                            app.language.label("Diff"),
                            path.display(),
                            app.language.label("line")
                        )
                    },
                    |line| {
                        format!(
                            " {}: {} [{} {}/{} {}] ",
                            app.language.label("Diff"),
                            path.display(),
                            app.language.label("line"),
                            changes.selected_line + 1,
                            changes
                                .preview
                                .as_ref()
                                .map_or(0, |preview| preview.lines.len()),
                            app.language.label(line.source.label())
                        )
                    },
                ),
            ChangesMode::File => format!(
                " {}: {} [{}] ",
                app.language.label("Diff"),
                path.display(),
                app.language.label("file")
            ),
        },
    );
    let border_style = if changes.mode != ChangesMode::File {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let block = Block::default()
        .title(super::text::truncate(
            &title,
            area.width.saturating_sub(2) as usize,
        ))
        .borders(Borders::ALL)
        .border_style(border_style);
    if changes.preview_loading {
        frame.render_widget(
            Paragraph::new(app.language.text("Loading diff...", "正在加载差异...")).block(block),
            area,
        );
        return;
    }
    let Some(preview) = &changes.preview else {
        frame.render_widget(
            Paragraph::new(if changes.entries.is_empty() {
                app.language
                    .text("Working tree is clean.", "工作区是干净的。")
            } else {
                app.language
                    .text("No diff is available.", "没有可用的差异。")
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
    let line_width = area.width.saturating_sub(2) as usize;
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
        let selected = selected_range
            .as_ref()
            .is_some_and(|range| range.contains(&index));
        if selected {
            style = super::selection_style();
            if line.starts_with("@@") {
                style = style.add_modifier(Modifier::BOLD);
            }
        }
        Line::styled(super::text::truncate(line, line_width), style)
    });
    let scroll = u16::try_from(changes.preview_scroll).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(Text::from_iter(lines))
            .block(block)
            .scroll((scroll, 0)),
        area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, changes: &ChangesState, area: Rect) {
    let first = if changes.commit_running {
        Span::styled(
            app.language.text("Committing...", "正在提交..."),
            Style::default().fg(Color::Yellow),
        )
    } else if changes.operation_running {
        Span::styled(
            app.language.text("Writing...", "正在写入..."),
            Style::default().fg(Color::Yellow),
        )
    } else if let Some((is_error, message)) = &changes.message {
        Span::styled(
            message.clone(),
            Style::default().fg(if *is_error { Color::Red } else { Color::Green }),
        )
    } else {
        Span::raw(app.language.text(
            "Space Select   A All   z Stash   s Stage   u Unstage   d Discard   m Commit",
            "Space 选择   A 全选   z 储藏   s 暂存   u 取消暂存   d 丢弃   m 提交",
        ))
    };
    let mode = match changes.mode {
        ChangesMode::File => app.language.label("file"),
        ChangesMode::Hunk => app.language.label("hunk"),
        ChangesMode::Line => app.language.label("line"),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(first),
            Line::raw(format!(
                "[{mode}] {}",
                app.language.text(
                    "Tab Mode   j/k Move   g/G First/last   r Refresh   Esc Back   ? Help",
                    "Tab 模式   j/k 移动   g/G 首/末   r 刷新   Esc 返回   ? 帮助",
                )
            )),
        ]),
        area,
    );
}

fn render_confirmation(frame: &mut Frame, app: &App, changes: &ChangesState) {
    let Some(pending) = &changes.confirmation else {
        return;
    };
    let area = centered_rect(76, 62, frame.area());
    let mut lines = Vec::new();
    let (title, action) = match pending {
        PendingOperation::Single { change, target, .. } => {
            lines.push(Line::styled(
                match target {
                    OperationTarget::File => app.language.text(
                        "This permanently discards all worktree changes in the file.",
                        "此操作会永久丢弃该文件的全部工作区改动。",
                    ),
                    OperationTarget::Hunk { .. } => app.language.text(
                        "This permanently discards the selected worktree hunk.",
                        "此操作会永久丢弃所选工作区域块。",
                    ),
                    OperationTarget::Line { .. } => app.language.text(
                        "This permanently discards the selected worktree line.",
                        "此操作会永久丢弃所选工作区行。",
                    ),
                },
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));
            lines.push(Line::raw(super::text::truncate(
                &format!(
                    "{}: {}",
                    app.language.text("File", "文件"),
                    change.path.display()
                ),
                area.width.saturating_sub(2) as usize,
            )));
            lines.push(Line::raw(match target {
                OperationTarget::File => format!(
                    "{}: {}",
                    app.language.label("Scope"),
                    app.language.text("entire file", "整个文件")
                ),
                OperationTarget::Hunk { source, .. } => format!(
                    "{}: {} {}",
                    app.language.label("Scope"),
                    app.language.text("selected", "所选"),
                    app.language.label(source.label())
                ),
                OperationTarget::Line { source, .. } => format!(
                    "{}: {} {}",
                    app.language.label("Scope"),
                    app.language.text("selected", "所选"),
                    app.language.label(source.label())
                ),
            }));
            (
                app.language
                    .text(" Confirm destructive operation ", " 确认破坏性操作 "),
                app.language.text("discard", "丢弃"),
            )
        }
        PendingOperation::Batch(spec) => {
            let destructive = spec.kind == OperationKind::Discard;
            lines.push(Line::styled(
                match spec.kind {
                    OperationKind::Stash => app.language.text(
                        "Stash frozen files, including untracked files.",
                        "储藏已冻结的文件，包括未跟踪文件。",
                    ),
                    OperationKind::Discard => app.language.text(
                        "Permanently discard staged, worktree, and untracked changes.",
                        "永久丢弃已暂存、工作区和未跟踪改动。",
                    ),
                    _ => app
                        .language
                        .text("Run the frozen file batch.", "执行已冻结的文件批处理。"),
                },
                Style::default()
                    .fg(if destructive {
                        Color::Red
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(format!(
                "{}: {}   {}: {}",
                app.language.text("Repository", "仓库"),
                spec.project.relative_path.display(),
                app.language.label("Files"),
                spec.items.len()
            )));
            lines.push(Line::raw(""));
            let budget = usize::from(area.height.saturating_sub(9));
            for item in spec.items.iter().take(budget) {
                lines.push(Line::raw(super::text::truncate(
                    &format!(
                        "  {}  {}",
                        item.change.status_label(),
                        item.change.path.display()
                    ),
                    area.width.saturating_sub(2) as usize,
                )));
            }
            if spec.items.len() > budget {
                lines.push(Line::raw(format!(
                    "  ... {} {}",
                    spec.items.len() - budget,
                    app.language.text("more files", "个更多文件")
                )));
            }
            (
                if destructive {
                    app.language
                        .text(" Confirm batch discard ", " 确认批量丢弃 ")
                } else {
                    app.language.text(" Confirm batch stash ", " 确认批量储藏 ")
                },
                if destructive {
                    app.language.text("discard", "丢弃")
                } else {
                    app.language.text("stash", "储藏")
                },
            )
        }
    };
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!(
        "{} {action}{}",
        app.language.text("Press y to", "按 y "),
        app.language
            .text(" or n/Esc to cancel.", "，按 n/Esc 取消。")
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_commit_dialog(frame: &mut Frame, app: &App, changes: &ChangesState) {
    let area = centered_rect(84, 75, frame.area());
    frame.render_widget(Clear, area);
    let title = if changes.commit_amend {
        app.language.text(" Commit (amend) ", " 提交（修订） ")
    } else {
        app.language.text(" Commit ", " 提交 ")
    };
    let outer = Block::default().title(title).borders(Borders::ALL);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(inner);

    let editor_block = Block::default()
        .title(format!(" {} ", app.language.label("Message")))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let editor_inner = editor_block.inner(sections[0]);
    let (cursor_line, cursor_byte) = commit_cursor_position(changes);
    let visible_height = usize::from(editor_inner.height.max(1));
    let vertical_scroll = cursor_line.saturating_sub(visible_height.saturating_sub(1));
    let line_start = changes.commit_message[..cursor_byte]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let cursor_width = Line::raw(&changes.commit_message[line_start..cursor_byte]).width();
    let visible_width = usize::from(editor_inner.width.max(1));
    let horizontal_scroll = cursor_width.saturating_sub(visible_width.saturating_sub(1));
    let text = changes
        .commit_message
        .split('\n')
        .map(Line::raw)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(text).block(editor_block).scroll((
            u16::try_from(vertical_scroll).unwrap_or(u16::MAX),
            u16::try_from(horizontal_scroll).unwrap_or(u16::MAX),
        )),
        sections[0],
    );

    let enabled = |value| app.language.label(if value { "on" } else { "off" });
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!(
                "{}: {}   {}: {}",
                app.language.text("Ctrl-A amend", "Ctrl-A 修订"),
                enabled(changes.commit_amend),
                app.language.text("Ctrl-U sign-off", "Ctrl-U 作者签署"),
                enabled(changes.commit_signoff)
            )),
            Line::raw(format!(
                "{}: {}",
                app.language.text("Ctrl-G signing", "Ctrl-G 提交签名"),
                enabled(changes.commit_signing)
            )),
        ])
        .block(
            Block::default()
                .title(format!(" {} ", app.language.label("Options")))
                .borders(Borders::TOP),
        ),
        sections[1],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(app.language.text(
                "Arrows move   Home/End line   Backspace/Delete edit",
                "方向键移动   Home/End 行首/行尾   Backspace/Delete 编辑",
            )),
            Line::raw(app.language.text(
                "Enter newline   Ctrl-Enter/Ctrl-S commit   Esc cancel",
                "Enter 换行   Ctrl-Enter/Ctrl-S 提交   Esc 取消",
            )),
        ])
        .block(
            Block::default()
                .title(format!(" {} ", app.language.label("Keys")))
                .borders(Borders::TOP),
        ),
        sections[2],
    );

    if editor_inner.width > 0 && editor_inner.height > 0 {
        let x = editor_inner.x
            + u16::try_from(cursor_width.saturating_sub(horizontal_scroll))
                .unwrap_or(u16::MAX)
                .min(editor_inner.width - 1);
        let y = editor_inner.y
            + u16::try_from(cursor_line.saturating_sub(vertical_scroll))
                .unwrap_or(u16::MAX)
                .min(editor_inner.height - 1);
        frame.set_cursor_position((x, y));
    }
}

fn commit_cursor_position(changes: &ChangesState) -> (usize, usize) {
    let mut cursor = changes.commit_cursor.min(changes.commit_message.len());
    while !changes.commit_message.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let before = &changes.commit_message[..cursor];
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    (line, cursor)
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
