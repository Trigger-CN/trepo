use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use tokio::sync::mpsc;

use crate::adapters::git;
use crate::domain::{
    BatchOperationItem, Project, RepoProjectState, WorkspaceGitAction, WorkspaceGitSpec,
    WorkspaceGitTarget, WorktreeSummary,
};
use crate::services::operations::project_lock;
use crate::services::repo_batch::workspace_lock;

#[derive(Debug)]
pub struct WorkspaceGitPrepareResult {
    pub generation: u64,
    pub result: Result<WorkspaceGitSpec>,
}

#[derive(Debug)]
pub enum WorkspaceGitEventKind {
    Started {
        project: Project,
    },
    Finished {
        project: Project,
        state: RepoProjectState,
        message: String,
    },
    Complete,
}

#[derive(Debug)]
pub struct WorkspaceGitEvent {
    pub generation: u64,
    pub kind: WorkspaceGitEventKind,
}

pub fn spawn_prepare(
    action: WorkspaceGitAction,
    projects: Vec<Project>,
    generation: u64,
    sender: mpsc::UnboundedSender<WorkspaceGitPrepareResult>,
) {
    tokio::spawn(async move {
        let result = prepare(action, projects).await;
        let _ = sender.send(WorkspaceGitPrepareResult { generation, result });
    });
}

async fn prepare(action: WorkspaceGitAction, projects: Vec<Project>) -> Result<WorkspaceGitSpec> {
    if projects.is_empty() {
        bail!("select at least one repository with Space");
    }
    let mut targets = Vec::with_capacity(projects.len());
    for project in projects {
        let changes = git::changes(&project.path)
            .await
            .with_context(|| format!("failed to scan {}", project.relative_path.display()))?;
        if changes.is_empty() {
            bail!(
                "precondition failed: {} has no changes",
                project.relative_path.display()
            );
        }
        if matches!(
            action,
            WorkspaceGitAction::Stage | WorkspaceGitAction::Stash
        ) && changes.iter().any(|entry| entry.conflicted)
        {
            bail!(
                "precondition failed: {} has conflicts and cannot be {} as a whole",
                project.relative_path.display(),
                action.operation_kind().label().to_ascii_lowercase()
            );
        }
        let mut items = Vec::with_capacity(changes.len());
        for change in changes {
            let expected_token = git::change_token(&project.path, &change).await?;
            items.push(BatchOperationItem {
                change,
                expected_token,
            });
        }
        let summary = summary(&items);
        targets.push(WorkspaceGitTarget {
            project,
            items,
            summary,
        });
    }
    Ok(WorkspaceGitSpec { action, targets })
}

pub fn spawn_execute(
    workspace_root: PathBuf,
    spec: WorkspaceGitSpec,
    generation: u64,
    sender: mpsc::UnboundedSender<WorkspaceGitEvent>,
) {
    tokio::spawn(run(workspace_root, spec, generation, sender));
}

async fn run(
    workspace_root: PathBuf,
    mut spec: WorkspaceGitSpec,
    generation: u64,
    sender: mpsc::UnboundedSender<WorkspaceGitEvent>,
) {
    let workspace_lock = workspace_lock(workspace_root.clone());
    let _workspace_guard = workspace_lock.lock().await;
    spec.targets
        .sort_by(|left, right| left.project.path.cmp(&right.project.path));
    let locks = spec
        .targets
        .iter()
        .map(|target| project_lock(target.project.path.clone()))
        .collect::<Vec<_>>();
    let mut guards = Vec::with_capacity(locks.len());
    for lock in &locks {
        guards.push(lock.lock().await);
    }

    if let Err(error) = preflight(&workspace_root, &spec).await {
        for target in spec.targets {
            send(
                &sender,
                generation,
                WorkspaceGitEventKind::Finished {
                    project: target.project,
                    state: RepoProjectState::Failed,
                    message: format!("No repositories were changed: {error}"),
                },
            );
        }
        send(&sender, generation, WorkspaceGitEventKind::Complete);
        return;
    }

    for target in spec.targets {
        send(
            &sender,
            generation,
            WorkspaceGitEventKind::Started {
                project: target.project.clone(),
            },
        );
        let result = execute_target(spec.action, &target).await;
        let (state, message) = match result {
            Ok(()) => (RepoProjectState::Succeeded, "Completed".to_owned()),
            Err(error) => (
                RepoProjectState::Failed,
                format!("{error}; completed repositories were not rolled back"),
            ),
        };
        send(
            &sender,
            generation,
            WorkspaceGitEventKind::Finished {
                project: target.project,
                state,
                message,
            },
        );
    }
    drop(guards);
    send(&sender, generation, WorkspaceGitEventKind::Complete);
}

async fn preflight(workspace_root: &std::path::Path, spec: &WorkspaceGitSpec) -> Result<()> {
    if spec.targets.is_empty() {
        bail!("no repositories were selected");
    }
    let canonical_root =
        std::fs::canonicalize(workspace_root).context("failed to resolve Workspace root")?;
    for target in &spec.targets {
        let canonical_project = std::fs::canonicalize(&target.project.path).with_context(|| {
            format!(
                "failed to resolve {}",
                target.project.relative_path.display()
            )
        })?;
        if !canonical_project.starts_with(&canonical_root) {
            bail!(
                "precondition failed: {} resolves outside the Workspace",
                target.project.relative_path.display()
            );
        }
        if spec.action == WorkspaceGitAction::Stash && !git::has_head(&target.project.path).await? {
            bail!(
                "precondition failed: {} has no initial commit and cannot be stashed",
                target.project.relative_path.display()
            );
        }
        if git::git_path(&target.project.path, "index.lock")
            .await?
            .is_file()
        {
            bail!(
                "{} has an active Git index lock",
                target.project.relative_path.display()
            );
        }
        validate_target(spec.action, target).await?;
    }
    Ok(())
}

async fn validate_target(action: WorkspaceGitAction, target: &WorkspaceGitTarget) -> Result<()> {
    let current = git::changes(&target.project.path).await?;
    if current.len() != target.items.len() {
        bail!(
            "precondition failed: {} changed after confirmation",
            target.project.relative_path.display()
        );
    }
    for item in &target.items {
        let entry = current
            .iter()
            .find(|entry| entry.path == item.change.path)
            .with_context(|| {
                format!(
                    "precondition failed: {} changed after confirmation",
                    target.project.relative_path.display()
                )
            })?;
        if matches!(
            action,
            WorkspaceGitAction::Stage | WorkspaceGitAction::Stash
        ) && entry.conflicted
        {
            bail!(
                "precondition failed: {} has conflicts",
                target.project.relative_path.display()
            );
        }
        let token = git::change_token(&target.project.path, entry).await?;
        if token != item.expected_token {
            bail!(
                "precondition failed: {} changed after confirmation",
                target.project.relative_path.display()
            );
        }
    }
    Ok(())
}

async fn execute_target(action: WorkspaceGitAction, target: &WorkspaceGitTarget) -> Result<()> {
    validate_target(action, target).await?;
    let changes = target
        .items
        .iter()
        .map(|item| item.change.clone())
        .collect::<Vec<_>>();
    match action {
        WorkspaceGitAction::Stage => git::stage_all(&target.project.path).await,
        WorkspaceGitAction::Stash => {
            git::stash_paths(&target.project.path, &changes, "trepo workspace batch").await
        }
        WorkspaceGitAction::Discard => git::discard_paths(&target.project.path, &changes).await,
    }
}

fn summary(items: &[BatchOperationItem]) -> WorktreeSummary {
    let mut summary = WorktreeSummary::default();
    for item in items {
        if item.change.index.is_some() {
            summary.staged += 1;
        }
        if item.change.worktree.is_some() {
            summary.unstaged += 1;
        }
        if item.change.untracked {
            summary.untracked += 1;
        }
        if item.change.conflicted {
            summary.conflicted += 1;
        }
    }
    summary
}

fn send(
    sender: &mpsc::UnboundedSender<WorkspaceGitEvent>,
    generation: u64,
    kind: WorkspaceGitEventKind,
) {
    let _ = sender.send(WorkspaceGitEvent { generation, kind });
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;
    use crate::domain::ProjectId;

    fn run_git(root: &Path, args: &[&str]) {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap()
            .success());
    }

    fn initialize(root: &Path) {
        fs::create_dir_all(root).unwrap();
        run_git(root, &["init", "-q", "-b", "main"]);
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        run_git(root, &["add", "."]);
        run_git(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
    }

    fn project(root: &Path, name: &str) -> Project {
        Project {
            id: ProjectId(root.to_path_buf()),
            name: name.to_owned(),
            path: root.to_path_buf(),
            relative_path: PathBuf::from(name),
        }
    }

    async fn collect_events(
        mut receiver: mpsc::UnboundedReceiver<WorkspaceGitEvent>,
    ) -> Vec<WorkspaceGitEvent> {
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            let complete = matches!(event.kind, WorkspaceGitEventKind::Complete);
            events.push(event);
            if complete {
                break;
            }
        }
        events
    }

    #[tokio::test]
    async fn stashes_all_selected_repositories_with_untracked_files() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        initialize(&first);
        initialize(&second);
        fs::write(first.join("tracked.txt"), "changed\n").unwrap();
        fs::write(first.join("new.txt"), "new\n").unwrap();
        fs::write(second.join("tracked.txt"), "changed\n").unwrap();
        fs::write(second.join("new.txt"), "new\n").unwrap();
        let spec = prepare(
            WorkspaceGitAction::Stash,
            vec![project(&first, "first"), project(&second, "second")],
        )
        .await
        .unwrap();
        let (sender, receiver) = mpsc::unbounded_channel();
        spawn_execute(temp.path().to_path_buf(), spec, 7, sender);
        let events = collect_events(receiver).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    WorkspaceGitEventKind::Finished {
                        state: RepoProjectState::Succeeded,
                        ..
                    }
                ))
                .count(),
            2
        );
        assert!(git::changes(&first).await.unwrap().is_empty());
        assert!(git::changes(&second).await.unwrap().is_empty());
        assert_eq!(
            String::from_utf8(git::git_output(&first, ["stash", "list"]).await.unwrap())
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_eq!(
            String::from_utf8(git::git_output(&second, ["stash", "list"]).await.unwrap())
                .unwrap()
                .lines()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn stages_all_changes_in_selected_repositories() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        initialize(&first);
        initialize(&second);
        fs::write(first.join("tracked.txt"), "changed\n").unwrap();
        fs::write(first.join("new.txt"), "new\n").unwrap();
        fs::remove_file(second.join("tracked.txt")).unwrap();
        fs::write(second.join("new.txt"), "new\n").unwrap();

        let spec = prepare(
            WorkspaceGitAction::Stage,
            vec![project(&first, "first"), project(&second, "second")],
        )
        .await
        .unwrap();
        let (sender, receiver) = mpsc::unbounded_channel();
        spawn_execute(temp.path().to_path_buf(), spec, 8, sender);
        let events = collect_events(receiver).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    WorkspaceGitEventKind::Finished {
                        state: RepoProjectState::Succeeded,
                        ..
                    }
                ))
                .count(),
            2
        );
        for root in [&first, &second] {
            let changes = git::changes(root).await.unwrap();
            assert_eq!(changes.len(), 2);
            assert!(changes.iter().all(|entry| entry.index.is_some()));
            assert!(changes.iter().all(|entry| entry.worktree.is_none()));
            assert!(changes.iter().all(|entry| !entry.untracked));
        }
    }

    #[tokio::test]
    async fn stage_rejects_conflicted_repository_during_prepare() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        run_git(temp.path(), &["switch", "-q", "-c", "other"]);
        fs::write(temp.path().join("tracked.txt"), "other\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git(
            temp.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "other",
            ],
        );
        run_git(temp.path(), &["switch", "-q", "main"]);
        fs::write(temp.path().join("tracked.txt"), "main\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git(
            temp.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "main",
            ],
        );
        assert!(!std::process::Command::new("git")
            .args(["merge", "other"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        let error = prepare(
            WorkspaceGitAction::Stage,
            vec![project(temp.path(), "conflicted")],
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("has conflicts"));
    }

    #[tokio::test]
    async fn discards_all_selected_repositories() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        initialize(&first);
        initialize(&second);
        fs::write(first.join("tracked.txt"), "changed\n").unwrap();
        fs::write(first.join("new.txt"), "new\n").unwrap();
        fs::write(second.join("tracked.txt"), "changed\n").unwrap();
        fs::write(second.join("new.txt"), "new\n").unwrap();
        let spec = prepare(
            WorkspaceGitAction::Discard,
            vec![project(&first, "first"), project(&second, "second")],
        )
        .await
        .unwrap();
        let (sender, receiver) = mpsc::unbounded_channel();
        spawn_execute(temp.path().to_path_buf(), spec, 9, sender);
        let events = collect_events(receiver).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    WorkspaceGitEventKind::Finished {
                        state: RepoProjectState::Succeeded,
                        ..
                    }
                ))
                .count(),
            2
        );
        assert!(git::changes(&first).await.unwrap().is_empty());
        assert!(git::changes(&second).await.unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(first.join("tracked.txt")).unwrap(),
            "base\n"
        );
        assert_eq!(
            fs::read_to_string(second.join("tracked.txt")).unwrap(),
            "base\n"
        );
    }

    #[tokio::test]
    async fn index_lock_preflight_prevents_every_repository_write() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        initialize(&first);
        initialize(&second);
        fs::write(first.join("tracked.txt"), "first selected\n").unwrap();
        fs::write(second.join("tracked.txt"), "second selected\n").unwrap();
        let spec = prepare(
            WorkspaceGitAction::Discard,
            vec![project(&first, "first"), project(&second, "second")],
        )
        .await
        .unwrap();
        fs::write(second.join(".git/index.lock"), "locked\n").unwrap();

        let (sender, receiver) = mpsc::unbounded_channel();
        spawn_execute(temp.path().to_path_buf(), spec, 10, sender);
        let events = collect_events(receiver).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    WorkspaceGitEventKind::Finished {
                        state: RepoProjectState::Failed,
                        ..
                    }
                ))
                .count(),
            2
        );
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            WorkspaceGitEventKind::Finished { message, .. }
                if message.contains("active Git index lock")
        )));
        assert_eq!(
            fs::read_to_string(first.join("tracked.txt")).unwrap(),
            "first selected\n"
        );
        assert_eq!(
            fs::read_to_string(second.join("tracked.txt")).unwrap(),
            "second selected\n"
        );
    }

    #[tokio::test]
    async fn stale_repository_preflight_prevents_every_repository_write() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        initialize(&first);
        initialize(&second);
        fs::write(first.join("tracked.txt"), "first selected\n").unwrap();
        fs::write(second.join("tracked.txt"), "second selected\n").unwrap();
        let spec = prepare(
            WorkspaceGitAction::Discard,
            vec![project(&first, "first"), project(&second, "second")],
        )
        .await
        .unwrap();
        fs::write(second.join("tracked.txt"), "stale after confirmation\n").unwrap();
        let (sender, receiver) = mpsc::unbounded_channel();
        spawn_execute(temp.path().to_path_buf(), spec, 8, sender);
        let events = collect_events(receiver).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    WorkspaceGitEventKind::Finished {
                        state: RepoProjectState::Failed,
                        ..
                    }
                ))
                .count(),
            2
        );
        assert_eq!(
            fs::read_to_string(first.join("tracked.txt")).unwrap(),
            "first selected\n"
        );
        assert_eq!(
            fs::read_to_string(second.join("tracked.txt")).unwrap(),
            "stale after confirmation\n"
        );
    }

    #[tokio::test]
    async fn stage_stale_preflight_prevents_every_repository_write() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        initialize(&first);
        initialize(&second);
        fs::write(first.join("tracked.txt"), "first selected\n").unwrap();
        fs::write(second.join("tracked.txt"), "second selected\n").unwrap();
        let spec = prepare(
            WorkspaceGitAction::Stage,
            vec![project(&first, "first"), project(&second, "second")],
        )
        .await
        .unwrap();
        fs::write(second.join("tracked.txt"), "stale after confirmation\n").unwrap();

        let (sender, receiver) = mpsc::unbounded_channel();
        spawn_execute(temp.path().to_path_buf(), spec, 11, sender);
        let events = collect_events(receiver).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    WorkspaceGitEventKind::Finished {
                        state: RepoProjectState::Failed,
                        ..
                    }
                ))
                .count(),
            2
        );
        for root in [&first, &second] {
            assert!(String::from_utf8(
                git::git_output(root, ["diff", "--cached", "--name-only"])
                    .await
                    .unwrap(),
            )
            .unwrap()
            .is_empty());
        }
    }
}
