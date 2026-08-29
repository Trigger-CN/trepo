use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::change_tree::{change_tree_rows, ChangeTreeRow};
use crate::app::state::{App, WorkspaceView};
use crate::domain::{HeadState, RepoProjectState, ScanState, WorkspaceGitAction, WorkspaceKind};

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
    render_workspace_git_overlay(frame, app);
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
        format!("{}: {}_", app.language.text("Search", "搜索"), app.search)
    } else {
        let filter = match (app.language.is_zh(), app.workspace_view) {
            (_, WorkspaceView::All) => "",
            (false, WorkspaceView::Changed) => "  Changed only",
            (false, WorkspaceView::ChangedWithFiles) => "  Changed + files",
            (true, WorkspaceView::Changed) => "  仅改动",
            (true, WorkspaceView::ChangedWithFiles) => "  改动与文件",
        };
        if !app.search.is_empty() {
            format!(
                "{}: {}{}  {}: {}",
                app.language.text("Filter", "过滤"),
                app.search,
                filter,
                app.language.text("Selected", "已选"),
                app.selected_project_count()
            )
        } else if app.language.is_zh() {
            format!(
                "{} 个仓库{}  已选 {}  改动 {}  冲突 {}  领先 {}  落后 {}  错误 {}",
                summary.total,
                filter,
                app.selected_project_count(),
                summary.dirty,
                summary.conflicted,
                summary.ahead,
                summary.behind,
                summary.errors
            )
        } else {
            format!(
                "{} projects{}  {} selected  {} dirty  {} conflict  {} ahead  {} behind  {} errors",
                summary.total,
                filter,
                app.selected_project_count(),
                summary.dirty,
                summary.conflicted,
                summary.ahead,
                summary.behind,
                summary.errors
            )
        }
    };
    frame.render_widget(
        Paragraph::new(vec![title, Line::raw(status)])
            .block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn render_table(frame: &mut Frame, app: &App, area: Rect) {
    let indices = app.filtered_indices();
    let row_budget = area.height.saturating_sub(3) as usize;
    let project_width = area.width.saturating_sub(49).max(20) as usize;
    let row_heights = indices
        .iter()
        .map(|project_index| {
            app.projects.get(*project_index).map_or(1, |snapshot| {
                workspace_row_height(snapshot, app.workspace_view, row_budget)
            })
        })
        .collect::<Vec<_>>();
    let start = variable_viewport_start(app.selected, row_budget, &row_heights);
    let mut used_rows = 0;
    let rows = indices
        .iter()
        .enumerate()
        .zip(row_heights.iter().copied())
        .skip(start)
        .take_while(|(_, height)| {
            let fits = used_rows == 0 || used_rows + height <= row_budget;
            if fits {
                used_rows += height;
            }
            fits
        })
        .filter_map(|((visible_index, project_index), row_height)| {
            let snapshot = app.projects.get(*project_index)?;
            let selected = visible_index == app.selected;
            let style = row_style(snapshot, selected);
            let upstream = snapshot.upstream.as_ref().map_or_else(
                || "-".to_owned(),
                |value| format!("+{} -{}", value.ahead, value.behind),
            );
            let status = match &snapshot.scan {
                ScanState::Pending => app.language.text("scanning", "扫描中").to_owned(),
                ScanState::Error(_) => app.language.text("error", "错误").to_owned(),
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
                    Cell::from(workspace_project_lines(
                        snapshot,
                        app.workspace_view,
                        project_width,
                        row_budget,
                    )),
                    Cell::from(head_label(&snapshot.head)),
                    Cell::from(upstream),
                ])
                .height(u16::try_from(row_height).unwrap_or(u16::MAX))
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
            Row::new([
                "",
                app.language.text("Status", "状态"),
                app.language.text("Project / path", "仓库 / 路径"),
                "HEAD",
                app.language.text("Upstream", "上游"),
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
                    " {} ({}/{}){} ",
                    app.language.text("Projects", "仓库"),
                    indices.len(),
                    app.projects.len(),
                    match (app.language.is_zh(), app.workspace_view) {
                        (_, WorkspaceView::All) => "",
                        (false, WorkspaceView::Changed) => " changed only",
                        (false, WorkspaceView::ChangedWithFiles) => " changed + files",
                        (true, WorkspaceView::Changed) => " 仅改动",
                        (true, WorkspaceView::ChangedWithFiles) => " 改动与文件",
                    }
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
                Line::raw(format!(
                    "{}: {}",
                    app.language.text("Status", "状态"),
                    snapshot.worktree.status_label()
                )),
                Line::raw(format!(
                    "{}: {}",
                    app.language.text("Staged", "已暂存"),
                    snapshot.worktree.staged
                )),
                Line::raw(format!(
                    "{}: {}",
                    app.language.text("Modified", "已修改"),
                    snapshot.worktree.unstaged
                )),
                Line::raw(format!(
                    "{}: {}",
                    app.language.text("Untracked", "未跟踪"),
                    snapshot.worktree.untracked
                )),
                Line::raw(format!(
                    "{}: {}",
                    app.language.text("Conflicts", "冲突"),
                    snapshot.worktree.conflicted
                )),
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
            } else if snapshot.changes.is_empty() {
                lines.push(Line::raw(""));
                lines.push(Line::raw(
                    app.language.text("Changed files: none", "改动文件：无"),
                ));
            } else {
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    format!(
                        "{} ({})",
                        app.language.text("Changed files", "改动文件"),
                        snapshot.changes.len()
                    ),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                let tree_rows = change_tree_rows(&snapshot.changes);
                let available_rows = area.height.saturating_sub(lines.len() as u16 + 2) as usize;
                let row_budget = if tree_rows.len() > available_rows {
                    available_rows.saturating_sub(1)
                } else {
                    available_rows
                };
                for row in tree_rows.iter().take(row_budget) {
                    lines.push(change_tree_line(
                        row,
                        &snapshot.changes,
                        area.width.saturating_sub(4) as usize,
                    ));
                }
                let remaining = tree_rows.len().saturating_sub(row_budget);
                if remaining > 0 {
                    lines.push(Line::styled(
                        format!("... {remaining} more tree rows"),
                        Color::DarkGray,
                    ));
                }
            }
            lines
        }
        None => vec![Line::raw(
            app.language.text("No matching project", "没有匹配的仓库"),
        )],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(app.language.text(" Inspector ", " 检查器 "))
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let task = if app.workspace_git.preparing {
        app.language
            .text(
                "Preparing Workspace Git confirmation",
                "正在准备工作区 Git 确认",
            )
            .to_owned()
    } else if app.scanning > 0 {
        format!(
            "{} {}",
            app.language.text("Scanning", "扫描中"),
            app.scanning
        )
    } else {
        app.language.text("Ready", "就绪").to_owned()
    };
    let keys = app.language.text(
        "   Space Select   S Stage   Z Stash   D Discard   d View cycle   a Repo actions   / Search",
        "   Space 选择   S 暂存   Z 储藏   D 丢弃   d 视图循环   a Repo 操作   / 搜索",
    );
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
            Span::raw(keys),
        ])),
        area,
    );
}

fn render_workspace_git_overlay(frame: &mut Frame, app: &App) {
    if app.workspace_git.preparing {
        let area = centered_rect(60, 24, frame.area());
        render_overlay(
            frame,
            area,
            app.language.text(" Workspace Git ", " Workspace Git "),
            vec![
                Line::styled(
                    app.language
                        .text("Freezing repository state...", "正在冻结仓库状态..."),
                    Color::Yellow,
                ),
                Line::raw(app.language.text(
                    "All repositories are read before confirmation.",
                    "确认前会读取全部仓库状态。",
                )),
            ],
        );
    } else if let Some(spec) = &app.workspace_git.pending {
        let area = centered_rect(82, 76, frame.area());
        let mut lines = vec![
            Line::styled(
                app.language.action(spec.action.label()),
                Style::default()
                    .fg(if spec.action == WorkspaceGitAction::Discard {
                        Color::LightRed
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(
                "{}: {}",
                app.language.text("Frozen repositories", "已冻结仓库"),
                spec.targets.len()
            )),
            Line::raw(match spec.action {
                WorkspaceGitAction::Stage => app.language.text(
                    "All tracked and untracked changes will be added to the index.",
                    "所有已跟踪及未跟踪改动将加入暂存区。",
                ),
                WorkspaceGitAction::Stash => app.language.text(
                    "Each stash includes tracked, staged, and untracked changes.",
                    "每个储藏都包含已跟踪、已暂存和未跟踪改动。",
                ),
                WorkspaceGitAction::Discard => app.language.text(
                    "Tracked index/worktree and untracked files will be permanently cleared.",
                    "已跟踪的暂存区/工作区改动及未跟踪文件将被永久清除。",
                ),
            }),
            Line::raw(""),
        ];
        let detail = spec
            .targets
            .iter()
            .map(|target| {
                Line::raw(format!(
                    "{}  S{} M{} ?{} !{}",
                    target.project.relative_path.display(),
                    target.summary.staged,
                    target.summary.unstaged,
                    target.summary.untracked,
                    target.summary.conflicted
                ))
            })
            .collect::<Vec<_>>();
        let budget = usize::from(area.height.saturating_sub(9));
        let max_scroll = detail.len().saturating_sub(budget);
        let scroll = app.workspace_git.scroll.min(max_scroll);
        lines.extend(detail.into_iter().skip(scroll).take(budget));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            app.language
                .text("Run now? y Yes   n/Esc No", "现在执行？y 是   n/Esc 否"),
            if spec.action == WorkspaceGitAction::Discard {
                Color::LightRed
            } else {
                Color::Yellow
            },
        ));
        render_overlay(
            frame,
            area,
            app.language.text(
                " Confirm Workspace Git operation ",
                " 确认 Workspace Git 操作 ",
            ),
            lines,
        );
    } else if let Some(task) = &app.workspace_git.task {
        let area = centered_rect(84, 76, frame.area());
        let counts = task.results.iter().fold([0usize; 4], |mut counts, result| {
            let index = match result.state {
                RepoProjectState::Pending => 0,
                RepoProjectState::Running => 1,
                RepoProjectState::Succeeded => 2,
                _ => 3,
            };
            counts[index] += 1;
            counts
        });
        let mut lines = vec![
            Line::styled(
                format!(
                    "{}  {}",
                    app.language.action(task.spec.action.label()),
                    app.language
                        .label(if task.running { "running" } else { "complete" })
                ),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(
                "{} {}  {} {}  {} {}  {} {}",
                app.language.label("pending"),
                counts[0],
                app.language.label("running"),
                counts[1],
                app.language.label("success"),
                counts[2],
                app.language.label("failure"),
                counts[3]
            )),
            Line::raw(app.language.text(
                "Cross-repository operations are not transactional; completed work is not rolled back.",
                "跨仓库操作不具备事务性；已完成的操作不会回滚。",
            )),
            Line::raw(""),
        ];
        let detail = task
            .results
            .iter()
            .map(|result| {
                result_line(
                    app,
                    &result.project.relative_path.display().to_string(),
                    result.state,
                    &result.message,
                )
            })
            .collect::<Vec<_>>();
        let budget = usize::from(area.height.saturating_sub(8));
        let max_scroll = detail.len().saturating_sub(budget);
        let scroll = app.workspace_git.scroll.min(max_scroll);
        lines.extend(detail.into_iter().skip(scroll).take(budget));
        lines.push(Line::styled(
            if task.running {
                app.language.text("j/k Scroll", "j/k 滚动")
            } else {
                app.language
                    .text("j/k Scroll   Esc Close", "j/k 滚动   Esc 关闭")
            },
            Color::DarkGray,
        ));
        render_overlay(
            frame,
            area,
            app.language
                .text(" Workspace Git task ", " Workspace Git 任务 "),
            lines,
        );
    } else if let Some((error, message)) = &app.workspace_git.message {
        let area = centered_rect(64, 24, frame.area());
        render_overlay(
            frame,
            area,
            app.language.text(" Workspace Git ", " Workspace Git "),
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
                Line::styled(format!(" {}", app.language.action(action.label())), style)
            })
            .collect::<Vec<_>>();
        render_overlay(
            frame,
            area,
            app.language.text(" Repo batch actions ", " Repo 批量操作 "),
            lines,
        );
    } else if let Some(form) = &app.repo_batch.form {
        let area = centered_rect(60, 34, frame.area());
        let label = app
            .language
            .label(form.action.input_label().unwrap_or("Value"));
        render_overlay(
            frame,
            area,
            app.language.action(form.action.label()),
            vec![
                Line::raw(format!("{label}: {}_", form.value)),
                Line::raw(""),
                Line::styled(
                    app.language
                        .text("Enter Review   Esc Cancel", "Enter 检查   Esc 取消"),
                    Color::DarkGray,
                ),
            ],
        );
    } else if let Some((spec, targets)) = &app.repo_batch.pending {
        let area = centered_rect(72, 70, frame.area());
        let mut lines = vec![
            Line::styled(
                app.language.action(spec.action.label()),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(
                "{}: {}",
                app.language.label("Scope"),
                if spec.action.is_workspace_action()
                    || (spec.action == crate::domain::RepoBatchAction::Sync && targets.is_empty())
                {
                    app.language
                        .text("Entire Repo workspace", "整个 Repo 工作区")
                        .to_owned()
                } else {
                    format!(
                        "{} {}",
                        targets.len(),
                        app.language.text("selected project(s)", "个所选仓库")
                    )
                }
            )),
            Line::raw(format!(
                "{}: {}",
                app.language.label("Parameters"),
                batch_parameter(app, spec)
            )),
            Line::raw(format!("{}:", app.language.label("Commands"))),
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
        } else if spec.action == crate::domain::RepoBatchAction::Sync {
            let paths = targets
                .iter()
                .map(|project| project.relative_path.as_path())
                .collect::<Vec<_>>();
            if let Ok(args) = crate::adapters::repo::sync_args(&paths) {
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
            app.language
                .text("Run now? y Yes   n No", "现在执行？y 是   n 否"),
            if spec.action.is_destructive() {
                Color::LightRed
            } else {
                Color::Yellow
            },
        ));
        render_overlay(
            frame,
            area,
            app.language
                .text(" Confirm Repo operation ", " 确认 Repo 操作 "),
            lines,
        );
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
                    app.language.action(task.spec.action.label()),
                    app.language.label(if task.running {
                        if task.cancelling {
                            "cancelling"
                        } else {
                            "running"
                        }
                    } else if task.cancelled {
                        "cancelled"
                    } else {
                        "complete"
                    })
                ),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(format!(
                "{} {}  {} {}  {} {}  {} {}  {} {}",
                app.language.label("pending"),
                counts[0],
                app.language.label("running"),
                counts[1],
                app.language.label("success"),
                counts[2],
                app.language.label("failed"),
                counts[3],
                app.language.label("cancelled"),
                counts[4]
            )),
            Line::raw(format!(
                "{}: {}",
                app.language.label("Parameters"),
                batch_parameter(app, &task.spec)
            )),
            Line::raw(format!(
                "{}: {}",
                app.language.label("Commands started"),
                task.args.len()
            )),
        ];
        let mut detail = Vec::new();
        if let Some((state, message)) = &task.workspace_result {
            detail.push(result_line(app, "workspace", *state, message));
        }
        detail.extend(task.results.iter().map(|result| {
            result_line(
                app,
                &result.project.relative_path.display().to_string(),
                result.state,
                &result.message,
            )
        }));
        detail.push(Line::raw(""));
        detail.push(Line::styled(
            app.language.label("Log"),
            Style::default().fg(Color::Cyan),
        ));
        detail.extend(task.logs.iter().map(|line| Line::raw(line.clone())));
        let budget = area.height.saturating_sub(8) as usize;
        let max_scroll = detail.len().saturating_sub(budget);
        let scroll = app.repo_batch.scroll.min(max_scroll);
        lines.extend(detail.into_iter().skip(scroll).take(budget));
        lines.push(Line::styled(
            if task.running {
                app.language.text(
                    "j/k Scroll   c Cancel (no rollback)",
                    "j/k 滚动   c 取消（不回滚）",
                )
            } else {
                app.language.text(
                    "j/k Scroll   f Retry failed   Esc Close",
                    "j/k 滚动   f 重试失败项   Esc 关闭",
                )
            },
            Color::DarkGray,
        ));
        render_overlay(
            frame,
            area,
            app.language.text(" Repo task ", " Repo 任务 "),
            lines,
        );
    } else if let Some((error, message)) = &app.repo_batch.message {
        let area = centered_rect(60, 24, frame.area());
        render_overlay(
            frame,
            area,
            app.language.text(" Repo action ", " Repo 操作 "),
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

fn batch_parameter(app: &App, spec: &crate::domain::RepoBatchSpec) -> String {
    spec.branch
        .as_deref()
        .or(spec.change.as_deref())
        .map(str::to_owned)
        .or_else(|| {
            spec.output
                .as_ref()
                .map(|value| value.display().to_string())
        })
        .unwrap_or_else(|| app.language.label("none").to_owned())
}

fn result_line(app: &App, label: &str, state: RepoProjectState, message: &str) -> Line<'static> {
    let color = match state {
        RepoProjectState::Succeeded => Color::Green,
        RepoProjectState::Failed => Color::LightRed,
        RepoProjectState::Running => Color::Yellow,
        RepoProjectState::Cancelled | RepoProjectState::Skipped => Color::DarkGray,
        RepoProjectState::Pending => Color::Gray,
    };
    Line::from(vec![
        Span::styled(format!("{:<9}", app.language.label(state.label())), color),
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

fn change_tree_line(
    row: &ChangeTreeRow,
    changes: &[crate::domain::ChangeEntry],
    width: usize,
) -> Line<'static> {
    match row {
        ChangeTreeRow::Directory { .. } => Line::styled(
            super::text::truncate(&row.display(), width),
            Color::DarkGray,
        ),
        ChangeTreeRow::File { entry_index, .. } => {
            let change = &changes[*entry_index];
            let text = super::text::truncate(
                &format!("{}  {}", change.status_label(), row.display()),
                width,
            );
            let color = if change.conflicted {
                Color::LightRed
            } else if change.untracked {
                Color::Yellow
            } else {
                Color::Cyan
            };
            Line::styled(text, color)
        }
    }
}

fn row_style(snapshot: &crate::domain::ProjectSnapshot, selected: bool) -> Style {
    if selected {
        return super::selection_style();
    }
    if matches!(snapshot.scan, ScanState::Error(_)) {
        Style::default().fg(Color::Red)
    } else if snapshot.worktree.conflicted > 0 {
        Style::default().fg(Color::LightRed)
    } else if snapshot.worktree.is_dirty() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    }
}

fn head_label(head: &HeadState) -> String {
    match head {
        HeadState::Branch(name) => name.clone(),
        HeadState::Detached(oid) => format!("detached {oid}"),
        HeadState::Unborn(name) => format!("unborn {name}"),
        HeadState::Unknown => "-".to_owned(),
    }
}

fn workspace_row_height(
    snapshot: &crate::domain::ProjectSnapshot,
    view: WorkspaceView,
    row_budget: usize,
) -> usize {
    if !view.expands_files() || snapshot.changes.is_empty() {
        return 1;
    }
    let tree_rows = change_tree_rows(&snapshot.changes);
    let tree_budget = row_budget.saturating_sub(1).min(6);
    1 + tree_rows.len().min(tree_budget)
}

fn workspace_project_lines(
    snapshot: &crate::domain::ProjectSnapshot,
    view: WorkspaceView,
    width: usize,
    row_budget: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::raw(super::text::truncate(
        &snapshot.project.relative_path.to_string_lossy(),
        width,
    ))];
    if !view.expands_files() || snapshot.changes.is_empty() {
        return lines;
    }

    let tree_rows = change_tree_rows(&snapshot.changes);
    let tree_budget = row_budget.saturating_sub(1).min(6);
    let visible_tree_rows = if tree_rows.len() > tree_budget {
        tree_budget.saturating_sub(1)
    } else {
        tree_budget
    };
    lines.extend(
        tree_rows
            .iter()
            .take(visible_tree_rows)
            .map(|row| change_tree_line(row, &snapshot.changes, width)),
    );
    let remaining = tree_rows.len().saturating_sub(visible_tree_rows);
    if remaining > 0 && tree_budget > 0 {
        lines.push(Line::styled(
            super::text::truncate(&format!("... {remaining} more tree rows"), width),
            Color::DarkGray,
        ));
    }
    lines
}

fn variable_viewport_start(selected: usize, budget: usize, heights: &[usize]) -> usize {
    if budget == 0 || heights.is_empty() {
        return 0;
    }
    let selected = selected.min(heights.len() - 1);
    let mut start = selected;
    let mut used = heights[selected].min(budget);
    while start > 0 && used + heights[start - 1] <= budget {
        start -= 1;
        used += heights[start];
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_selection_in_variable_height_view() {
        assert_eq!(variable_viewport_start(0, 10, &[1; 100]), 0);
        assert_eq!(variable_viewport_start(3, 5, &[1, 3, 1, 4]), 2);
        assert_eq!(variable_viewport_start(2, 6, &[1, 3, 2, 4]), 0);
    }

    #[test]
    fn selected_rows_override_status_colors() {
        let project = crate::domain::Project {
            id: crate::domain::ProjectId("/tmp/demo".into()),
            name: "demo".into(),
            path: "/tmp/demo".into(),
            relative_path: "demo".into(),
        };
        for (scan, worktree, normal_fg) in [
            (
                ScanState::Error("failed".into()),
                crate::domain::WorktreeSummary::default(),
                Color::Red,
            ),
            (
                ScanState::Ready,
                crate::domain::WorktreeSummary {
                    conflicted: 1,
                    ..Default::default()
                },
                Color::LightRed,
            ),
            (
                ScanState::Ready,
                crate::domain::WorktreeSummary {
                    unstaged: 1,
                    ..Default::default()
                },
                Color::Yellow,
            ),
        ] {
            let snapshot = crate::domain::ProjectSnapshot {
                project: project.clone(),
                head: HeadState::Unknown,
                upstream: None,
                worktree,
                changes: Vec::new(),
                scan,
                generation: 0,
            };
            assert_eq!(row_style(&snapshot, false).fg, Some(normal_fg));
            assert_eq!(row_style(&snapshot, true), super::super::selection_style());
        }
    }
}
