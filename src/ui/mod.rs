mod change_tree;
mod changes;
mod graph;
mod graph_layout;
mod repository;
mod text;
mod workspace;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::state::{App, Screen};

pub fn render(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Workspace => workspace::render(frame, app),
        Screen::Graph => graph::render(frame, app),
        Screen::Changes => changes::render(frame, app),
        Screen::Repository => repository::render(frame, app),
    }
    if app.help {
        render_help(frame, app);
    }
}

fn selection_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::LightCyan)
        .add_modifier(Modifier::BOLD)
}

fn selection_fg(selected: bool, color: Color) -> Color {
    if selected {
        Color::Black
    } else {
        color
    }
}

fn render_help(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 70, frame.area());
    frame.render_widget(Clear, area);
    let language = app.language;
    let text = if language.is_zh() {
        Text::from(vec![
            Line::styled("trepo 按键", Style::default().add_modifier(Modifier::BOLD)),
            Line::raw(""),
            Line::raw("j/k 或方向键    移动选择 / 滚动当前任务"),
            Line::raw("g/G             首项 / 末项"),
            Line::raw("Enter           打开提交图 / 提交信息中换行"),
            Line::raw("Space / A       选择当前 / 全部仓库或改动文件"),
            Line::raw("S/Z/D           对光标仓库 / 已选仓库执行暂存 / 储藏 / 丢弃"),
            Line::raw("d               全部 / 仅改动 / 改动与文件 三态循环"),
            Line::raw("a               打开 Repo 或仓库操作"),
            Line::raw("f, /, x         提交图过滤 / 搜索 / 清除"),
            Line::raw("Tab             切换改动模式或表单字段"),
            Line::raw("z/s/u           储藏 / 暂存 / 取消暂存"),
            Line::raw("m               提交；Ctrl-Enter/S 确认"),
            Line::raw("r               刷新当前页面"),
            Line::raw("Esc / q / ?     返回 / 退出 / 切换帮助"),
            Line::raw(""),
            Line::styled(
                "写操作会显示作用域，并在适用锁下重新校验状态。",
                Color::Yellow,
            ),
        ])
    } else {
        Text::from(vec![
            Line::styled("trepo keys", Style::default().add_modifier(Modifier::BOLD)),
            Line::raw(""),
            Line::raw("j/k or arrows  Move selection / scroll active task"),
            Line::raw("g/G            First / last"),
            Line::raw("Enter          Open graph / newline in commit message"),
            Line::raw("Space / A      Select current / all repositories or Changed files"),
            Line::raw(
                "S/Z/D          Workspace cursor / selected repositories: Stage / Stash / Discard",
            ),
            Line::raw("d              Cycle all / changed / changed with files"),
            Line::raw("a              Open Workspace Repo or Repository actions"),
            Line::raw("f, /, x        Graph filter / query / clear; retry failed Repo task"),
            Line::raw("Tab            Toggle Changes mode or active form field"),
            Line::raw("z/s/u          Stash files / Stage / Unstage in Changes"),
            Line::raw("m              Commit; Ctrl-Enter/S submit, Ctrl-A/U/G options"),
            Line::raw("r              Refresh current page"),
            Line::raw("Esc / q / ?    Back / quit Workspace / toggle help"),
            Line::raw(""),
            Line::styled(
                "Write operations show scope and revalidate state under the applicable locks.",
                Color::Yellow,
            ),
        ])
    };
    let widget = Paragraph::new(text)
        .block(
            Block::default()
                .title(language.text(" Help ", " 帮助 "))
                .borders(Borders::ALL),
        )
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::app::repository::{form_for, RepositoryChoice, RepositoryTab};
    use crate::app::state::{
        App, ChangesMode, ChangesState, GraphState, PendingOperation, RepoBatchForm, RepoBatchTask,
        RepositoryState, Screen, WorkspaceGitTask,
    };
    use crate::domain::{
        BatchOperationItem, BranchEntry, ChangeCode, ChangeEntry, ChangeHunk, ChangeLine,
        ChangePreview, Commit, CommitRef, CommitRefKind, GitOperationKind, HunkSource,
        OperationKind, OperationTarget, Project, ProjectId, RemoteBranchEntry, RemoteEntry,
        RepoBatchAction, RepoBatchSpec, RepoProjectResult, RepoProjectState, RepositoryAction,
        RepositorySnapshot, StashEntry, TagEntry, Workspace, WorkspaceGitAction, WorkspaceGitSpec,
        WorkspaceGitTarget, WorkspaceKind, WorktreeSummary,
    };

    fn app() -> App {
        let path = PathBuf::from("/tmp/demo");
        let project = Project {
            id: ProjectId(path.clone()),
            name: "platform/demo".into(),
            path: path.clone(),
            relative_path: PathBuf::from("demo"),
        };
        App::new(
            Workspace {
                root: path,
                kind: WorkspaceKind::Repo,
                projects: vec![project],
            },
            2,
        )
    }

    fn draw(app: &App, width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
    }

    fn draw_text(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn compact_text(text: &str) -> String {
        text.chars()
            .filter(|value| !value.is_whitespace())
            .collect()
    }

    fn buffer_text(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn assert_rendered_text_style(
        app: &App,
        width: u16,
        height: u16,
        needle: &str,
        fg: Color,
        bg: Color,
    ) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let symbols = needle
            .chars()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let span_width = u16::try_from(symbols.len()).unwrap();
        let mut found_text = false;
        for y in 0..height {
            for x in 0..=width.saturating_sub(span_width) {
                let matches = symbols.iter().enumerate().all(|(offset, symbol)| {
                    buffer[(x + u16::try_from(offset).unwrap(), y)].symbol() == symbol
                });
                if !matches {
                    continue;
                }
                found_text = true;
                let styled = (0..span_width).all(|offset| {
                    let cell = &buffer[(x + offset, y)];
                    cell.fg == fg && cell.bg == bg && cell.modifier.contains(Modifier::BOLD)
                });
                if styled {
                    return;
                }
            }
        }
        assert!(
            found_text,
            "text {needle:?} was not rendered at {width}x{height}"
        );
        panic!("text {needle:?} had no {fg:?}/{bg:?}/bold rendering at {width}x{height}");
    }

    fn assert_text_foreground(app: &App, width: u16, height: u16, needle: &str, fg: Color) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let symbols = needle
            .chars()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let span_width = u16::try_from(symbols.len()).unwrap();
        for y in 0..height {
            for x in 0..=width.saturating_sub(span_width) {
                let matches = symbols.iter().enumerate().all(|(offset, symbol)| {
                    buffer[(x + u16::try_from(offset).unwrap(), y)].symbol() == symbol
                });
                if matches && (0..span_width).all(|offset| buffer[(x + offset, y)].fg == fg) {
                    return;
                }
            }
        }
        panic!("text {needle:?} had no {fg:?} rendering at {width}x{height}");
    }

    fn assert_selected_text(app: &App, width: u16, height: u16, needle: &str) {
        assert_rendered_text_style(app, width, height, needle, Color::Black, Color::LightCyan);
    }

    fn draw_text_and_cursor(
        app: &App,
        width: u16,
        height: u16,
    ) -> (String, ratatui::layout::Position) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        (text, cursor)
    }

    #[test]
    fn workspace_sync_confirmation_shows_scope_and_exact_command() {
        let mut app = app();
        app.repo_batch.pending = Some((
            RepoBatchSpec {
                action: RepoBatchAction::Sync,
                branch: None,
                change: None,
                output: None,
            },
            Vec::new(),
        ));

        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Scope: Entire Repo workspace"));
            assert!(text.contains("repo sync -c -j8"));
            assert!(!text.contains("repo sync -c -j8 --"));
        }
    }

    #[test]
    fn renders_workspace_at_supported_sizes() {
        let app = app();
        for (width, height) in [(80, 24), (120, 40)] {
            assert_selected_text(&app, width, height, "demo");
        }
    }

    #[test]
    fn renders_workspace_git_confirmation_and_results_at_supported_sizes() {
        let mut app = app();
        let project = app.workspace.projects[0].clone();
        let entry = ChangeEntry {
            path: PathBuf::from("src/main.rs"),
            original_path: None,
            index: Some(ChangeCode::Modified),
            worktree: Some(ChangeCode::Modified),
            untracked: false,
            conflicted: false,
        };
        let spec = WorkspaceGitSpec {
            action: WorkspaceGitAction::Discard,
            targets: vec![WorkspaceGitTarget {
                project: project.clone(),
                items: vec![BatchOperationItem {
                    change: entry,
                    expected_token: 42,
                }],
                summary: WorktreeSummary {
                    staged: 1,
                    unstaged: 1,
                    untracked: 0,
                    conflicted: 0,
                },
            }],
        };
        app.workspace_git.pending = Some(spec.clone());
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Confirm Workspace Git operation"));
            assert!(text.contains("Discard target repositories"));
            assert!(text.contains("Frozen repositories: 1"));
            assert!(text.contains("demo  S1 M1 ?0 !0"));
        }
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let text = compact_text(&draw_text(&app, width, height));
            assert!(text.contains("确认WorkspaceGit操作"));
            assert!(text.contains("已冻结仓库:1"));
            assert!(text.contains("丢弃目标仓库改动"));
            assert!(text.contains("现在执行"));
        }
        let mut stage_spec = spec.clone();
        stage_spec.action = WorkspaceGitAction::Stage;
        app.workspace_git.pending = Some(stage_spec);
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let text = compact_text(&draw_text(&app, width, height));
            assert!(text.contains("暂存目标仓库改动"));
            assert!(text.contains("所有已跟踪及未跟踪改动将加入暂存区"));
        }
        app.language = crate::i18n::Language::En;
        app.workspace_git.pending = None;
        app.workspace_git.task = Some(WorkspaceGitTask {
            spec,
            results: vec![RepoProjectResult {
                project,
                state: RepoProjectState::Failed,
                message: "No repositories were changed: stale".into(),
            }],
            running: false,
            generation: 1,
        });
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("pending 0  running 0  success 0  failure 1"));
            assert!(text.contains("No repositories were changed"));
            assert!(text.contains("not transactional"));
        }
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let raw = draw_text(&app, width, height);
            let text = compact_text(&raw);
            assert!(text.contains("WorkspaceGit任务"));
            assert!(text.contains("等待0"));
            assert!(text.contains("失败1"));
            assert!(text.contains("跨仓库操作不具备事务性"));
            assert!(raw.contains("No repositories were changed"));
        }
    }

    #[test]
    fn workspace_changed_files_view_expands_files_in_main_list() {
        let mut app = app();
        app.projects[0].worktree.staged = 1;
        app.projects[0].changes = (0..24)
            .map(|index| ChangeEntry {
                path: PathBuf::from(format!("src/dirty-{index}.rs")),
                original_path: None,
                index: Some(ChangeCode::Modified),
                worktree: None,
                untracked: false,
                conflicted: false,
            })
            .collect();

        app.cycle_workspace_view();
        assert_eq!(
            app.workspace_view,
            crate::app::state::WorkspaceView::Changed
        );
        let changed = draw_text(&app, 120, 40);
        assert!(changed.contains("Changed only"));

        app.cycle_workspace_view();
        assert_eq!(
            app.workspace_view,
            crate::app::state::WorkspaceView::ChangedWithFiles
        );
        let expanded = draw_text(&app, 120, 40);
        assert!(expanded.contains("Changed + files"));
        assert!(expanded.contains("src/"));
        assert!(expanded.contains("M.  "));
        assert!(expanded.contains("dirty-0.rs"));
        assert!(expanded.contains("more tree rows"));
        draw(&app, 80, 24);

        app.cycle_workspace_view();
        assert_eq!(app.workspace_view, crate::app::state::WorkspaceView::All);
    }

    #[test]
    fn renders_repo_batch_overlays_at_supported_sizes() {
        let mut app = app();
        app.selected_projects
            .insert(app.workspace.projects[0].id.clone());
        app.open_repo_batch_menu();
        draw(&app, 80, 24);
        draw(&app, 120, 40);

        app.repo_batch.action_menu = false;
        app.repo_batch.form = Some(RepoBatchForm {
            action: RepoBatchAction::Start,
            value: "topic/x".into(),
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);

        let project = app.workspace.projects[0].clone();
        let spec = RepoBatchSpec {
            action: RepoBatchAction::Sync,
            branch: None,
            change: None,
            output: None,
        };
        app.repo_batch.form = None;
        app.repo_batch.pending = Some((spec.clone(), vec![project.clone()]));
        draw(&app, 80, 24);
        draw(&app, 120, 40);

        app.repo_batch.pending = None;
        app.repo_batch.task = Some(RepoBatchTask {
            spec,
            targets: vec![project.clone()],
            results: vec![RepoProjectResult {
                project,
                state: RepoProjectState::Failed,
                message: "network failure".into(),
            }],
            workspace_result: None,
            args: vec![],
            logs: vec!["[demo] raw repo output".into()],
            running: false,
            cancelling: false,
            cancelled: false,
            generation: 1,
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let raw = draw_text(&app, width, height);
            let text = compact_text(&raw);
            assert!(text.contains("Repo任务"));
            assert!(text.contains("参数:无"));
            assert!(text.contains("失败1"));
            assert!(text.contains("日志"));
            assert!(raw.contains("[demo] raw repo output"));
        }
    }

    #[test]
    fn renders_graph_and_help() {
        let mut app = app();
        let project = app.workspace.projects[0].clone();
        app.screen = Screen::Graph;
        app.help = true;
        app.graph = Some(GraphState {
            project,
            commits: vec![Commit {
                oid: "aaaaaaaa".into(),
                parents: vec![],
                refs: vec![CommitRef {
                    name: "main".into(),
                    kind: CommitRefKind::LocalBranch,
                }],
                author: "Ada".into(),
                timestamp: 1_700_000_000,
                subject: "Initial commit".into(),
                body: "Initial commit".into(),
            }],
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            object_menu: true,
            object_selected: 0,
            action_menu: false,
            action_selected: 0,
            selected_object: None,
            form: None,
            message: None,
            selected_oid: None,
            filter: crate::app::state::GraphFilter::default(),
            filter_form: None,
            filter_error: None,
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);
    }

    #[test]
    fn graph_prioritizes_subject_and_folds_dense_refs_at_supported_sizes() {
        let mut app = app();
        let project = app.workspace.projects[0].clone();
        app.screen = Screen::Graph;
        app.graph = Some(GraphState {
            project,
            commits: vec![Commit {
                oid: "aaaaaaaa".into(),
                parents: vec![],
                refs: vec![
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
                ],
                author: "Ada".into(),
                timestamp: 1_700_000_000,
                subject: "Subject remains readable".into(),
                body: "Body".into(),
            }],
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            object_menu: false,
            object_selected: 0,
            action_menu: false,
            action_selected: 0,
            selected_object: None,
            form: None,
            message: None,
            selected_oid: None,
            filter: crate::app::state::GraphFilter::default(),
            filter_form: None,
            filter_error: None,
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });

        let narrow = draw_text(&app, 80, 24);
        assert!(narrow.contains("Subject remains readable"));
        assert!(narrow.contains("T:v1"));
        assert!(narrow.contains("T:+2"));
        assert!(!narrow.contains("Date"));

        let wide = draw_text(&app, 120, 40);
        assert!(wide.contains("Subject remains readable"));
        assert!(wide.contains("2023-11-14"));
        assert!(wide.contains("Refs (3)"));
        assert!(wide.contains("Tags (3)"));
        assert!(wide.contains("T:v1"));
        assert!(wide.contains("T:v2"));
        assert!(wide.contains("T:v3"));
        for (width, height) in [(80, 24), (120, 40)] {
            assert_selected_text(&app, width, height, "Subject remains readable");
            assert_selected_text(&app, width, height, "●");
            assert_rendered_text_style(&app, width, height, "T:v1", Color::Black, Color::Yellow);
        }
        assert_selected_text(&app, 120, 40, "2023-11-14");
    }

    #[test]
    fn graph_wraps_long_subject_and_preserves_body_lines() {
        let mut app = app();
        let project = app.workspace.projects[0].clone();
        app.screen = Screen::Graph;
        app.graph = Some(GraphState {
            project,
            commits: vec![Commit {
                oid: "aaaaaaaa".into(),
                parents: vec![],
                refs: vec![],
                author: "Ada".into(),
                timestamp: 1_700_000_000,
                subject: "alpha beta gamma delta epsilon zeta eta theta".into(),
                body: "body first\n\nbody third".into(),
            }],
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            object_menu: false,
            object_selected: 0,
            action_menu: false,
            action_selected: 0,
            selected_object: None,
            form: None,
            message: None,
            selected_oid: None,
            filter: crate::app::state::GraphFilter::default(),
            filter_form: None,
            filter_error: None,
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });

        let narrow = draw_text(&app, 80, 24);
        assert!(narrow.contains("alpha beta gamma"));
        assert!(narrow.contains("delta epsilon"));
        let wide = draw_text(&app, 120, 40);
        let lines = wide.lines().collect::<Vec<_>>();
        let first = lines
            .iter()
            .position(|line| line.contains("body first"))
            .unwrap();
        let third = lines
            .iter()
            .position(|line| line.contains("body third"))
            .unwrap();
        assert_eq!(third, first + 2);
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let raw = draw_text(&app, width, height);
            let text = compact_text(&raw);
            assert!(text.contains("全部引用提交图"));
            assert!(text.contains("主题"));
            assert!(text.contains("引用"));
            assert!(!raw.contains("Subject"));
            assert!(!raw.contains("Refs ("));
            if width == 120 {
                assert!(text.contains("提交:aaaaaaaa"));
                assert!(text.contains("作者:Ada"));
                assert!(text.contains("日期:2023-11-14"));
                assert!(text.contains("父提交:-"));
            }
        }
    }

    #[test]
    fn renders_graph_filters_and_empty_results_at_supported_sizes() {
        use crate::app::state::{GraphFilter, GraphFilterForm};

        let mut app = app();
        let project = app.workspace.projects[0].clone();
        app.screen = Screen::Graph;
        app.graph = Some(GraphState {
            project,
            commits: vec![Commit {
                oid: "aaaaaaaa".into(),
                parents: vec![],
                refs: vec![CommitRef {
                    name: "main".into(),
                    kind: CommitRefKind::LocalBranch,
                }],
                author: "Ada".into(),
                timestamp: 1_700_000_000,
                subject: "Initial commit".into(),
                body: "Initial commit body".into(),
            }],
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            object_menu: false,
            object_selected: 0,
            action_menu: false,
            action_selected: 0,
            selected_object: None,
            form: None,
            message: None,
            selected_oid: None,
            filter: GraphFilter {
                branch: "main".into(),
                query: String::new(),
                author: "Ada".into(),
                since: "2023-01-01".into(),
                until: String::new(),
            },
            filter_form: Some(GraphFilterForm {
                draft: GraphFilter {
                    branch: "main".into(),
                    query: "initial".into(),
                    author: "Ada".into(),
                    since: "2023-01-01".into(),
                    until: "2023-12-31".into(),
                },
                selected: 1,
            }),
            filter_error: Some("Until must use YYYY-MM-DD".into()),
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });

        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Graph filters"));
            assert!(text.contains("Branch: main"));
            assert!(text.contains("Query: initial_"));
            assert!(text.contains("Author: Ada"));
            assert!(text.contains("Since: 2023-01-01"));
            assert!(text.contains("Until: 2023-12-31"));
            assert!(text.contains("Until must use YYYY-MM-DD"));
        }

        let graph = app.graph.as_mut().unwrap();
        graph.filter_form = None;
        graph.filter_error = None;
        graph.filter.query = "no-such-commit".into();
        let text = draw_text(&app, 120, 40);
        assert!(text.contains("branch:main  query:no-such-commit  author:Ada"));
        assert!(text.contains("All refs commit graph (0/1)"));
        assert!(text.contains("No matching commits"));
    }

    #[test]
    fn renders_graph_object_action_and_form_overlays_at_supported_sizes() {
        use crate::app::repository::FormField;
        use crate::app::state::{GraphActionChoice, GraphForm, GraphObject, GraphObjectKind};
        let mut app = app();
        let project = app.workspace.projects[0].clone();
        app.screen = Screen::Graph;
        let object = GraphObject {
            kind: GraphObjectKind::LocalBranch,
            name: "feature/x".into(),
            oid: "aaaaaaaa".into(),
        };
        app.graph = Some(GraphState {
            project,
            commits: vec![Commit {
                oid: "aaaaaaaa".into(),
                parents: vec![],
                refs: vec![
                    CommitRef {
                        name: "HEAD".into(),
                        kind: CommitRefKind::Head,
                    },
                    CommitRef {
                        name: "feature/x".into(),
                        kind: CommitRefKind::LocalBranch,
                    },
                    CommitRef {
                        name: "origin/feature/x".into(),
                        kind: CommitRefKind::RemoteBranch,
                    },
                    CommitRef {
                        name: "v1".into(),
                        kind: CommitRefKind::Tag,
                    },
                    CommitRef {
                        name: "stash@{0}".into(),
                        kind: CommitRefKind::Stash,
                    },
                ],
                author: "Ada".into(),
                timestamp: 1_700_000_000,
                subject: "Initial commit".into(),
                body: "Initial commit".into(),
            }],
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            object_menu: false,
            object_selected: 1,
            action_menu: true,
            action_selected: 0,
            selected_object: Some(object.clone()),
            form: None,
            message: None,
            selected_oid: None,
            filter: crate::app::state::GraphFilter::default(),
            filter_form: None,
            filter_error: None,
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Push branch"));
            assert!(text.contains("Force push with lease"));
        }
        {
            let graph = app.graph.as_mut().unwrap();
            graph.action_menu = false;
            graph.object_menu = true;
        }
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let text = compact_text(&draw_text(&app, width, height));
            assert!(text.contains("提交:commitaaaaaaaa"));
            assert!(text.contains("HEAD:HEAD"));
            assert!(text.contains("本地分支:feature/x"));
            assert!(text.contains("远程分支:origin/feature/x"));
            assert!(text.contains("标签:v1"));
            assert!(text.contains("储藏:stash@{0}"));
            assert!(!text.contains("Remotebranch:"));
        }
        app.language = crate::i18n::Language::En;
        app.graph.as_mut().unwrap().action_menu = false;
        app.graph.as_mut().unwrap().form = Some(GraphForm {
            choice: GraphActionChoice::Commit,
            object,
            fields: vec![
                FormField::Text {
                    label: "Commit message",
                    value: "test".into(),
                },
                FormField::Toggle {
                    label: "Sign off",
                    value: false,
                },
                FormField::Toggle {
                    label: "Sign commit",
                    value: false,
                },
            ],
            selected: 0,
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);
    }
    #[test]
    fn renders_changes_and_destructive_confirmation() {
        let mut app = app();
        let project = app.workspace.projects[0].clone();
        let staged = ChangeEntry {
            path: PathBuf::from("staged.rs"),
            original_path: None,
            index: Some(ChangeCode::Modified),
            worktree: None,
            untracked: false,
            conflicted: false,
        };
        let unstaged = ChangeEntry {
            path: PathBuf::from("unstaged.rs"),
            original_path: None,
            index: None,
            worktree: Some(ChangeCode::Modified),
            untracked: false,
            conflicted: false,
        };
        let entry = ChangeEntry {
            path: PathBuf::from("src/main.rs"),
            original_path: None,
            index: Some(ChangeCode::Modified),
            worktree: Some(ChangeCode::Modified),
            untracked: false,
            conflicted: false,
        };
        let mixed = ChangeEntry {
            path: PathBuf::from("mixed.rs"),
            original_path: None,
            index: Some(ChangeCode::Modified),
            worktree: Some(ChangeCode::Modified),
            untracked: false,
            conflicted: false,
        };
        let untracked = ChangeEntry {
            path: PathBuf::from("untracked.rs"),
            original_path: None,
            index: None,
            worktree: None,
            untracked: true,
            conflicted: false,
        };
        let conflicted = ChangeEntry {
            path: PathBuf::from("conflicted.rs"),
            original_path: None,
            index: Some(ChangeCode::Updated),
            worktree: Some(ChangeCode::Updated),
            untracked: false,
            conflicted: true,
        };
        app.screen = Screen::Changes;
        app.changes = Some(ChangesState {
            project,
            return_screen: Screen::Workspace,
            entries: vec![
                staged,
                unstaged,
                mixed,
                entry.clone(),
                untracked,
                conflicted,
            ],
            selected: 3,
            selected_files: std::iter::once(entry.path.clone()).collect(),
            mode: ChangesMode::File,
            selected_hunk: 0,
            selected_hunk_identity: Some((HunkSource::Worktree, 7)),
            selected_line: 0,
            selected_line_identity: None,
            loading: false,
            error: None,
            generation: 1,
            preview: Some(ChangePreview {
                text: "== Worktree ==\n@@ -1 +1 @@\n-old\n+new\n".into(),
                token: 42,
                truncated: false,
                hunks: vec![ChangeHunk {
                    source: HunkSource::Worktree,
                    header: "@@ -1 +1 @@".into(),
                    display_start: 1,
                    display_end: 3,
                    fingerprint: 7,
                }],
                lines: vec![ChangeLine {
                    source: HunkSource::Worktree,
                    hunk_fingerprint: 7,
                    fingerprint: 8,
                    display_line: 3,
                }],
            }),
            preview_path: Some(entry.path.clone()),
            preview_loading: false,
            preview_generation: 1,
            preview_scroll: 0,
            operation_running: false,
            operation_generation: 0,
            confirmation: None,
            message: None,
            commit_message: "subject\n\nbody".into(),
            commit_cursor: "subject\n\nbody".len(),
            commit_editing: false,
            commit_amend: false,
            commit_signoff: false,
            commit_signing: false,
            commit_running: false,
            commit_generation: 0,
        });
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("src/"));
            assert!(text.contains("main.rs"));
            assert!(text.contains("1 selected"));
            assert!(text.contains("[x]"));
        }
        assert_text_foreground(&app, 120, 40, "staged.rs", Color::LightGreen);
        assert_text_foreground(&app, 120, 40, "unstaged.rs", Color::LightRed);
        assert_text_foreground(&app, 120, 40, "mixed.rs", Color::LightMagenta);
        assert_text_foreground(&app, 120, 40, "untracked.rs", Color::Yellow);
        assert_text_foreground(&app, 120, 40, "conflicted.rs", Color::LightRed);
        assert_selected_text(&app, 120, 40, "main.rs");
        app.changes.as_mut().unwrap().mode = ChangesMode::Hunk;
        for (width, height) in [(80, 24), (120, 40)] {
            assert_selected_text(&app, width, height, "main.rs");
            assert_selected_text(&app, width, height, "@@ -1 +1 @@");
            assert_selected_text(&app, width, height, "-old");
            assert_selected_text(&app, width, height, "+new");
        }
        app.changes.as_mut().unwrap().commit_editing = true;
        for (width, height) in [(80, 24), (120, 40)] {
            let (text, cursor) = draw_text_and_cursor(&app, width, height);
            assert!(text.contains(" Message "));
            assert!(text.contains("subject"));
            assert!(text.contains("body"));
            assert!(text.contains(" Options "));
            assert!(text.contains(" Keys "));
            assert!(text.contains("Arrows move"));
            assert!(text.contains("Ctrl-Enter/Ctrl-S commit"));
            let lines = text.lines().collect::<Vec<_>>();
            let message_y = lines
                .iter()
                .position(|line| line.contains(" Message "))
                .unwrap();
            let options_y = lines
                .iter()
                .position(|line| line.contains(" Options "))
                .unwrap();
            let keys_y = lines
                .iter()
                .position(|line| line.contains(" Keys "))
                .unwrap();
            assert!(message_y < usize::from(cursor.y));
            assert!(usize::from(cursor.y) < options_y);
            assert!(options_y < keys_y);
            assert!(cursor.x < width);
        }
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let (raw, cursor) = draw_text_and_cursor(&app, width, height);
            let text = compact_text(&raw);
            assert!(text.contains("提交信息"));
            assert!(text.contains("选项"));
            assert!(text.contains("按键"));
            assert!(text.contains("Ctrl-A修订:关"));
            assert!(text.contains("方向键移动"));
            assert!(text.contains("Ctrl-Enter/Ctrl-S提交"));
            assert!(cursor.x < width);
        }
        app.language = crate::i18n::Language::En;
        let changes = app.changes.as_mut().unwrap();
        changes.commit_editing = false;
        changes.confirmation = Some(PendingOperation::Single {
            kind: OperationKind::RestoreWorktree,
            change: changes.entries[3].clone(),
            target: OperationTarget::Hunk {
                source: HunkSource::Worktree,
                fingerprint: 7,
            },
            expected_token: 42,
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);

        let changes = app.changes.as_mut().unwrap();
        changes.confirmation = Some(PendingOperation::Batch(crate::domain::BatchOperationSpec {
            project: changes.project.clone(),
            items: vec![BatchOperationItem {
                change: changes.entries[3].clone(),
                expected_token: 42,
            }],
            kind: OperationKind::Stash,
        }));
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Confirm batch stash"));
            assert!(text.contains("Files: 1"));
            assert!(text.contains("src/main.rs"));
            assert!(text.contains("including untracked files"));
        }
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let raw = draw_text(&app, width, height);
            let text = compact_text(&raw);
            assert!(text.contains("确认批量储藏"));
            assert!(text.contains("仓库:demo"));
            assert!(text.contains("文件:1"));
            assert!(text.contains("包括未跟踪文件"));
            assert!(raw.contains("src/main.rs"));
        }
    }

    #[test]
    fn changes_long_lines_are_clipped_sanitized_and_cleared_on_redraw() {
        let mut app = app();
        let project = app.workspace.projects[0].clone();
        let long_tail = "LONG_TAIL_MARKER";
        let entry = ChangeEntry {
            path: PathBuf::from(format!("目录/{}-{long_tail}.rs", "很长".repeat(40))),
            original_path: None,
            index: Some(ChangeCode::Modified),
            worktree: Some(ChangeCode::Modified),
            untracked: false,
            conflicted: false,
        };
        app.screen = Screen::Changes;
        app.changes = Some(ChangesState {
            project,
            return_screen: Screen::Workspace,
            entries: vec![entry.clone()],
            selected: 0,
            selected_files: Default::default(),
            mode: ChangesMode::File,
            selected_hunk: 0,
            selected_hunk_identity: None,
            selected_line: 0,
            selected_line_identity: None,
            loading: false,
            error: None,
            generation: 1,
            preview: Some(ChangePreview {
                text: format!("+{}\u{1b}[2J{long_tail}\n+short", "x".repeat(400)),
                token: 1,
                truncated: false,
                hunks: vec![],
                lines: vec![],
            }),
            preview_path: Some(entry.path.clone()),
            preview_loading: false,
            preview_generation: 1,
            preview_scroll: 0,
            operation_running: false,
            operation_generation: 0,
            confirmation: None,
            message: None,
            commit_message: String::new(),
            commit_cursor: 0,
            commit_editing: false,
            commit_amend: false,
            commit_signoff: false,
            commit_signing: false,
            commit_running: false,
            commit_generation: 0,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let first = buffer_text(&terminal, 80, 24);
        assert!(!first.contains('\u{1b}'));
        assert!(!first.contains(long_tail));

        let changes = app.changes.as_mut().unwrap();
        changes.entries[0].path = PathBuf::from("short.rs");
        changes.preview_path = Some(PathBuf::from("short.rs"));
        changes.preview.as_mut().unwrap().text = "+short".into();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let second = buffer_text(&terminal, 80, 24);
        assert!(second.contains("short.rs"));
        assert!(!second.contains(long_tail));
        assert!(!second.contains("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"));
    }

    fn repository_snapshot() -> RepositorySnapshot {
        RepositorySnapshot {
            operation: Some(GitOperationKind::Merge),
            conflicts: vec![PathBuf::from("src/conflict.rs")],
            stashes: vec![StashEntry {
                selector: "stash@{0}".into(),
                oid: "cccccccc".into(),
                subject: "WIP".into(),
            }],
            branches: vec![BranchEntry {
                name: "main".into(),
                oid: "bbbbbbbb".into(),
                upstream: Some("origin/main".into()),
                ahead: 1,
                behind: 0,
                current: true,
            }],
            tags: vec![TagEntry {
                name: "v0.3.0".into(),
                target: "bbbbbbbb".into(),
            }],
            remotes: vec![RemoteEntry {
                name: "origin".into(),
                fetch_url: "https://example.com/repo.git".into(),
                push_url: "https://example.com/repo.git".into(),
            }],
            remote_branches: vec![RemoteBranchEntry {
                name: "origin/main".into(),
                oid: "aaaaaaaa".into(),
            }],
            worktree_token: 0,
            token: 7,
        }
    }

    fn repository_state(app: &App) -> RepositoryState {
        RepositoryState {
            project: app.workspace.projects[0].clone(),
            return_screen: Screen::Workspace,
            snapshot: Some(repository_snapshot()),
            tab: RepositoryTab::Status,
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            action_menu: false,
            action_selected: 0,
            form: None,
            pending: None,
            action_running: false,
            action_generation: 0,
            message: None,
            detail: Some("Operation detail".into()),
        }
    }

    #[test]
    fn renders_repository_tabs_overlays_and_states_at_supported_sizes() {
        let mut app = app();
        app.screen = Screen::Repository;
        app.repository = Some(repository_state(&app));
        for (width, height) in [(80, 24), (120, 40)] {
            assert_selected_text(&app, width, height, "Operation: merge");
        }
        for tab in RepositoryTab::ALL {
            app.repository.as_mut().unwrap().tab = tab;
            draw(&app, 80, 24);
            draw(&app, 120, 40);
        }

        {
            let state = app.repository.as_mut().unwrap();
            state.tab = RepositoryTab::Remotes;
            state.action_menu = true;
        }
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Push branch"));
            assert!(text.contains("Force push with lease"));
        }

        {
            let state = app.repository.as_mut().unwrap();
            state.tab = RepositoryTab::Stashes;
        }
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Create branch from stash"));
            assert!(text.contains("Clear all stashes"));
        }

        {
            let state = app.repository.as_mut().unwrap();
            state.action_menu = false;
            state.form = form_for(
                RepositoryChoice::StashPush,
                state.snapshot.as_ref().unwrap(),
                0,
            )
            .unwrap();
        }
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("Include untracked"));
            assert!(text.contains("Keep index"));
            assert!(text.contains("Staged only"));
        }

        {
            let state = app.repository.as_mut().unwrap();
            state.form = form_for(
                RepositoryChoice::StashClear,
                state.snapshot.as_ref().unwrap(),
                0,
            )
            .unwrap();
        }
        for (width, height) in [(80, 24), (120, 40)] {
            assert!(draw_text(&app, width, height).contains("Enter execute"));
        }

        {
            let state = app.repository.as_mut().unwrap();
            state.form = None;
            state.pending = Some(RepositoryAction::Push {
                remote: "origin".into(),
                branch: "main".into(),
                set_upstream: true,
                force_with_lease: true,
            });
        }
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("rewrite remote branch history"));
            assert!(text.contains("Refspec: main:main"));
            assert!(text.contains("Commit range: aaaaaaaa..bbbbbbbb"));
            assert!(text.contains("Force with lease: on"));
        }
        app.language = crate::i18n::Language::Zh;
        for (width, height) in [(80, 24), (120, 40)] {
            let raw = draw_text(&app, width, height);
            let text = compact_text(&raw);
            assert!(text.contains("可能重写远程分支历史"));
            assert!(text.contains("引用规格:main:main"));
            assert!(text.contains("提交范围:aaaaaaaa..bbbbbbbb"));
            assert!(text.contains("租约强制:开"));
            assert!(!raw.contains("Force with lease: on"));
        }

        {
            let state = app.repository.as_mut().unwrap();
            state.pending = None;
            state.snapshot = None;
            state.loading = true;
        }
        draw(&app, 80, 24);
        {
            let state = app.repository.as_mut().unwrap();
            state.loading = false;
            state.error = Some("repository error".into());
        }
        draw(&app, 120, 40);
        app.repository.as_mut().unwrap().error = None;
        draw(&app, 80, 24);
    }

    #[test]
    fn renders_primary_pages_in_chinese_at_supported_sizes() {
        let mut app = app();
        app.language = crate::i18n::Language::Zh;
        app.help = true;
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains("trepo"));
            assert!(text.contains('按'));
            assert!(text.contains('态'));
        }

        app.help = false;
        app.screen = Screen::Repository;
        app.repository = Some(repository_state(&app));
        app.repository.as_mut().unwrap().action_menu = true;
        for (width, height) in [(80, 24), (120, 40)] {
            let text = draw_text(&app, width, height);
            assert!(text.contains('仓'));
            assert!(text.contains('操'));
            assert!(text.contains('我'));
        }
    }
}
