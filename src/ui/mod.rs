mod changes;
mod graph;
mod repository;
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
        render_help(frame);
    }
}

fn render_help(frame: &mut Frame) {
    let area = centered_rect(70, 70, frame.area());
    frame.render_widget(Clear, area);
    let text = Text::from(vec![
        Line::styled(
            "repo-tui keys",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw("j/k or arrows  Move selection / scroll active Repo task"),
        Line::raw("g/G            First / last"),
        Line::raw("Enter          Open graph / select active overlay item"),
        Line::raw("Space / A      Select project / all filtered projects"),
        Line::raw("a              Open Workspace Repo or Repository actions"),
        Line::raw("c / f          Cancel Repo task / retry failed projects"),
        Line::raw("Tab            Toggle Changes mode or active form field"),
        Line::raw("s/u/d          Stage / unstage / discard selection"),
        Line::raw("m              Commit staged changes; Ctrl-A/U/G toggle options"),
        Line::raw("/ / r          Search projects / refresh current page"),
        Line::raw("Esc / q / ?    Back / quit Workspace / toggle help"),
        Line::raw(""),
        Line::styled(
            "Write operations show scope and revalidate state under the applicable locks.",
            Color::Yellow,
        ),
    ]);
    let widget = Paragraph::new(text)
        .block(Block::default().title(" Help ").borders(Borders::ALL))
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
        RepositoryState, Screen,
    };
    use crate::domain::{
        BranchEntry, ChangeCode, ChangeEntry, ChangeHunk, ChangeLine, ChangePreview, Commit,
        CommitRef, CommitRefKind, GitOperationKind, HunkSource, OperationKind, OperationTarget,
        Project, ProjectId, RemoteBranchEntry, RemoteEntry, RepoBatchAction, RepoBatchSpec,
        RepoProjectResult, RepoProjectState, RepositoryAction, RepositorySnapshot, StashEntry,
        TagEntry, Workspace, WorkspaceKind,
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

    #[test]
    fn renders_workspace_at_supported_sizes() {
        let app = app();
        draw(&app, 80, 24);
        draw(&app, 120, 40);
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
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);
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
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);
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
        let entry = ChangeEntry {
            path: PathBuf::from("src/main.rs"),
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
            mode: ChangesMode::Hunk,
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
            confirmation: Some(PendingOperation {
                kind: OperationKind::RestoreWorktree,
                change: entry,
                target: OperationTarget::Hunk {
                    source: HunkSource::Worktree,
                    fingerprint: 7,
                },
                expected_token: 42,
            }),
            message: None,
            commit_message: String::new(),
            commit_editing: true,
            commit_amend: false,
            commit_signoff: false,
            commit_signing: false,
            commit_running: false,
            commit_generation: 0,
        });
        draw(&app, 80, 24);
        draw(&app, 120, 40);
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
        draw(&app, 80, 24);
        draw(&app, 120, 40);

        {
            let state = app.repository.as_mut().unwrap();
            state.action_menu = false;
            state.form =
                form_for(RepositoryChoice::Push, state.snapshot.as_ref().unwrap(), 0).unwrap();
        }
        draw(&app, 80, 24);
        draw(&app, 120, 40);

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
        draw(&app, 80, 24);
        draw(&app, 120, 40);

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
}
