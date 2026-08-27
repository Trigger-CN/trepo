use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use anyhow::{bail, Context, Result};
use tokio::sync::Mutex;

use crate::adapters::git;
use crate::domain::{
    BatchOperationSpec, ChangeEntry, CommitOutcome, CommitSpec, HunkSource, OperationKind,
    OperationOutcome, OperationSpec, OperationTarget, Project, RepositoryActionOutcome,
    RepositoryActionSpec,
};

use crate::services::repo_batch::workspace_lock_for_project;
#[derive(Debug, Default, Clone)]
pub struct OperationRunner;

impl OperationRunner {
    pub async fn execute(&self, spec: OperationSpec) -> Result<OperationOutcome> {
        let workspace_lock = workspace_lock_for_project(&spec.project.path);
        let _workspace_guard = match &workspace_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        let lock = project_lock(spec.project.path.clone());
        let _guard = lock.lock().await;
        let root = &spec.project.path;

        if git::git_path(root, "index.lock").await?.is_file() {
            bail!("Git index is locked; another writer may be active");
        }

        let current = git::changes(root)
            .await?
            .into_iter()
            .find(|entry| entry.path == spec.change.path)
            .with_context(|| {
                format!(
                    "precondition failed: {} is no longer changed",
                    spec.change.path.display()
                )
            })?;
        ensure_applicable(spec.kind, &current)?;
        let suffix = match spec.target {
            OperationTarget::File => {
                let token = git::change_token(root, &current).await?;
                ensure_token(token, spec.expected_token, &current)?;
                match spec.kind {
                    OperationKind::Stage => git::stage_path(root, &current.path).await?,
                    OperationKind::Unstage => git::unstage_path(root, &current.path).await?,
                    OperationKind::RestoreWorktree => {
                        git::restore_worktree_path(root, &current.path).await?
                    }
                    OperationKind::Stash | OperationKind::Discard => {
                        bail!("{} requires a file batch", spec.kind.label())
                    }
                }
                ""
            }
            OperationTarget::Hunk {
                source,
                fingerprint,
            } => {
                ensure_hunk_applicable(spec.kind, source, &current)?;
                let (token, matches) =
                    git::resolve_hunks(root, &current, source, fingerprint).await?;
                ensure_token(token, spec.expected_token, &current)?;
                let [hunk] = matches.as_slice() else {
                    bail!(
                        "precondition failed: selected hunk in {} is no longer uniquely available; refresh and retry",
                        current.path.display()
                    );
                };
                debug_assert_eq!(hunk.source, source);
                debug_assert_eq!(hunk.fingerprint, fingerprint);
                git::apply_hunk(root, source, spec.kind, &hunk.patch).await?;
                " hunk"
            }
            OperationTarget::Line {
                source,
                hunk_fingerprint,
                fingerprint,
            } => {
                ensure_hunk_applicable(spec.kind, source, &current)?;
                let (token, matches) =
                    git::resolve_lines(root, &current, source, hunk_fingerprint, fingerprint)
                        .await?;
                ensure_token(token, spec.expected_token, &current)?;
                let [line] = matches.as_slice() else {
                    bail!(
                        "precondition failed: selected line in {} is no longer uniquely available; refresh and retry",
                        current.path.display()
                    );
                };
                debug_assert_eq!(line.source, source);
                debug_assert_eq!(line.hunk_fingerprint, hunk_fingerprint);
                debug_assert_eq!(line.fingerprint, fingerprint);
                git::apply_hunk(root, source, spec.kind, &line.patch).await?;
                " line"
            }
        };

        Ok(OperationOutcome {
            kind: spec.kind,
            path: current.path.clone(),
            message: format!(
                "{}{}: {}",
                spec.kind.label(),
                suffix,
                current.path.display()
            ),
        })
    }

    pub async fn execute_batch(&self, spec: BatchOperationSpec) -> Result<OperationOutcome> {
        if spec.items.is_empty() {
            bail!("no files were selected");
        }
        if spec.kind == OperationKind::RestoreWorktree {
            bail!("worktree-scope discard is not a file batch operation");
        }
        let workspace_lock = workspace_lock_for_project(&spec.project.path);
        let _workspace_guard = match &workspace_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        let lock = project_lock(spec.project.path.clone());
        let _guard = lock.lock().await;
        let root = &spec.project.path;

        if git::git_path(root, "index.lock").await?.is_file() {
            bail!("Git index is locked; another writer may be active");
        }
        let current = git::changes(root).await?;
        let mut validated = Vec::with_capacity(spec.items.len());
        for item in &spec.items {
            let entry = current
                .iter()
                .find(|entry| entry.path == item.change.path)
                .with_context(|| {
                    format!(
                        "precondition failed: {} is no longer changed",
                        item.change.path.display()
                    )
                })?;
            ensure_applicable(spec.kind, entry)?;
            let token = git::change_token(root, entry).await?;
            ensure_token(token, item.expected_token, entry)?;
            validated.push(entry.clone());
        }

        match spec.kind {
            OperationKind::Stash => {
                git::stash_paths(root, &validated, "repo-tui selected files").await?
            }
            OperationKind::Discard => git::discard_paths(root, &validated).await?,
            OperationKind::Stage | OperationKind::Unstage => {
                for entry in &validated {
                    match spec.kind {
                        OperationKind::Stage => git::stage_path(root, &entry.path).await?,
                        OperationKind::Unstage => git::unstage_path(root, &entry.path).await?,
                        _ => unreachable!(),
                    }
                }
            }
            OperationKind::RestoreWorktree => unreachable!(),
        }
        let count = validated.len();
        Ok(OperationOutcome {
            kind: spec.kind,
            path: validated[0].path.clone(),
            message: format!("{} {count} files", spec.kind.label()),
        })
    }

    pub async fn execute_commit(
        &self,
        project: Project,
        spec: CommitSpec,
    ) -> Result<CommitOutcome> {
        let workspace_lock = workspace_lock_for_project(&project.path);
        let _workspace_guard = match &workspace_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        let lock = project_lock(project.path.clone());
        let _guard = lock.lock().await;
        let root = &project.path;
        if git::git_path(root, "index.lock").await?.is_file() {
            bail!("Git index is locked; another writer may be active");
        }
        if git::changes(root)
            .await?
            .iter()
            .all(|entry| entry.index.is_none())
            && !spec.amend
        {
            bail!("nothing is staged; stage at least one change before committing");
        }
        let oid = git::commit(root, &spec).await?;
        Ok(CommitOutcome {
            oid: oid.clone(),
            message: format!("Committed {}", &oid[..oid.len().min(12)]),
        })
    }

    pub async fn execute_repository_action(
        &self,
        spec: RepositoryActionSpec,
    ) -> Result<RepositoryActionOutcome> {
        let workspace_lock = workspace_lock_for_project(&spec.project.path);
        let _workspace_guard = match &workspace_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        let lock = project_lock(spec.project.path.clone());
        let _guard = lock.lock().await;
        let root = &spec.project.path;
        if git::git_path(root, "index.lock").await?.is_file() {
            bail!("Git index is locked; another writer may be active");
        }
        git::validate_repository_action(root, &spec.action, Some(spec.expected_token)).await?;
        git::execute_repository_action(root, &spec.action, false).await
    }
}

fn ensure_token(token: u64, expected: u64, change: &ChangeEntry) -> Result<()> {
    if token != expected {
        bail!(
            "precondition failed: {} changed after preview; refresh and retry",
            change.path.display()
        );
    }
    Ok(())
}

fn ensure_hunk_applicable(
    kind: OperationKind,
    source: HunkSource,
    change: &ChangeEntry,
) -> Result<()> {
    let applicable = match (source, kind) {
        (HunkSource::Staged, OperationKind::Unstage) => change.index.is_some(),
        (HunkSource::Worktree, OperationKind::Stage) => {
            change.worktree.is_some() || change.conflicted
        }
        (HunkSource::Worktree, OperationKind::RestoreWorktree) => change.can_restore(),
        (HunkSource::Untracked, OperationKind::Stage) => change.untracked,
        _ => false,
    };
    if !applicable {
        bail!(
            "precondition failed: {} {} hunk is not eligible for {}",
            change.path.display(),
            source.label(),
            kind.label()
        );
    }
    Ok(())
}

fn ensure_applicable(kind: OperationKind, change: &crate::domain::ChangeEntry) -> Result<()> {
    let applicable = match kind {
        OperationKind::Stage => change.can_stage(),
        OperationKind::Unstage => change.can_unstage(),
        OperationKind::RestoreWorktree => change.can_restore(),
        OperationKind::Stash => !change.conflicted,
        OperationKind::Discard => true,
    };
    if !applicable {
        bail!(
            "precondition failed: {} is not eligible for {}",
            change.path.display(),
            kind.label()
        );
    }
    Ok(())
}

pub(crate) fn project_lock(path: PathBuf) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(path)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;
    use crate::domain::{OperationKind, OperationSpec, Project, ProjectId};

    fn run_git(root: &Path, args: &[&str]) {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap()
            .success());
    }

    fn project(root: &Path) -> Project {
        Project {
            id: ProjectId(root.to_path_buf()),
            name: "test".into(),
            path: root.to_path_buf(),
            relative_path: PathBuf::from("."),
        }
    }

    fn initialize(root: &Path) {
        run_git(root, &["init", "-q", "-b", "main"]);
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        run_git(root, &["add", "tracked.txt"]);
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

    fn initialize_multihunk(root: &Path) {
        run_git(root, &["init", "-q", "-b", "main"]);
        let content = (1..=30)
            .map(|line| format!("line {line:02}\n"))
            .collect::<String>();
        fs::write(root.join("tracked.txt"), content).unwrap();
        run_git(root, &["add", "tracked.txt"]);
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

    fn git_text(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    #[tokio::test]
    async fn stages_unstages_and_restores_with_fresh_tokens() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("tracked.txt"), "changed\n").unwrap();
        fs::write(temp.path().join("new.txt"), "new\n").unwrap();
        let runner = OperationRunner;
        let repository = project(temp.path());

        let new_entry = git::changes(temp.path())
            .await
            .unwrap()
            .into_iter()
            .find(|entry| entry.path == Path::new("new.txt"))
            .unwrap();
        let token = git::change_token(temp.path(), &new_entry).await.unwrap();
        runner
            .execute(OperationSpec {
                project: repository.clone(),
                change: new_entry,
                kind: OperationKind::Stage,
                target: OperationTarget::File,
                expected_token: token,
            })
            .await
            .unwrap();

        let staged = git::changes(temp.path())
            .await
            .unwrap()
            .into_iter()
            .find(|entry| entry.path == Path::new("new.txt"))
            .unwrap();
        assert!(staged.can_unstage());
        let token = git::change_token(temp.path(), &staged).await.unwrap();
        runner
            .execute(OperationSpec {
                project: repository.clone(),
                change: staged,
                kind: OperationKind::Unstage,
                target: OperationTarget::File,
                expected_token: token,
            })
            .await
            .unwrap();

        let modified = git::changes(temp.path())
            .await
            .unwrap()
            .into_iter()
            .find(|entry| entry.path == Path::new("tracked.txt"))
            .unwrap();
        let token = git::change_token(temp.path(), &modified).await.unwrap();
        runner
            .execute(OperationSpec {
                project: repository,
                change: modified,
                kind: OperationKind::RestoreWorktree,
                target: OperationTarget::File,
                expected_token: token,
            })
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("tracked.txt")).unwrap(),
            "base\n"
        );
    }

    #[tokio::test]
    async fn batches_stage_and_unstage_for_multiple_files() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("first.txt"), "first\n").unwrap();
        fs::write(temp.path().join("second.txt"), "second\n").unwrap();
        let repository = project(temp.path());
        let entries = git::changes(temp.path()).await.unwrap();
        let mut items = Vec::new();
        for change in entries.into_iter().filter(|entry| entry.untracked) {
            let expected_token = git::change_token(temp.path(), &change).await.unwrap();
            items.push(crate::domain::BatchOperationItem {
                change,
                expected_token,
            });
        }
        let outcome = OperationRunner
            .execute_batch(BatchOperationSpec {
                project: repository.clone(),
                items,
                kind: OperationKind::Stage,
            })
            .await
            .unwrap();
        assert_eq!(outcome.message, "Stage 2 files");
        let staged = git::changes(temp.path())
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| entry.can_unstage())
            .collect::<Vec<_>>();
        assert_eq!(staged.len(), 2);

        let mut items = Vec::new();
        for change in staged {
            let expected_token = git::change_token(temp.path(), &change).await.unwrap();
            items.push(crate::domain::BatchOperationItem {
                change,
                expected_token,
            });
        }
        OperationRunner
            .execute_batch(BatchOperationSpec {
                project: repository,
                items,
                kind: OperationKind::Unstage,
            })
            .await
            .unwrap();
        let current = git::changes(temp.path()).await.unwrap();
        assert_eq!(current.iter().filter(|entry| entry.untracked).count(), 2);
    }

    #[tokio::test]
    async fn stashes_selected_mixed_changes_and_preserves_unselected_paths() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("tracked.txt"), "staged\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        fs::write(temp.path().join("tracked.txt"), "worktree\n").unwrap();
        fs::write(temp.path().join("selected.txt"), "selected\n").unwrap();
        fs::write(temp.path().join("other.txt"), "other\n").unwrap();
        let entries = git::changes(temp.path()).await.unwrap();
        let mut items = Vec::new();
        for change in entries
            .into_iter()
            .filter(|entry| entry.path != Path::new("other.txt"))
        {
            let expected_token = git::change_token(temp.path(), &change).await.unwrap();
            items.push(crate::domain::BatchOperationItem {
                change,
                expected_token,
            });
        }
        let outcome = OperationRunner
            .execute_batch(BatchOperationSpec {
                project: project(temp.path()),
                items,
                kind: OperationKind::Stash,
            })
            .await
            .unwrap();
        assert_eq!(outcome.message, "Stash 2 files");
        let current = git::changes(temp.path()).await.unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].path, Path::new("other.txt"));
        assert_eq!(
            git_text(
                temp.path(),
                &[
                    "stash",
                    "show",
                    "--name-only",
                    "--include-untracked",
                    "stash@{0}",
                ],
            ),
            "selected.txt\ntracked.txt\n"
        );
    }

    #[tokio::test]
    async fn stashes_staged_rename_and_restores_it_from_stash() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("before.txt"), "renamed content\n").unwrap();
        run_git(temp.path(), &["add", "before.txt"]);
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
                "add rename source",
            ],
        );
        run_git(temp.path(), &["mv", "before.txt", "after.txt"]);
        let change = git::changes(temp.path())
            .await
            .unwrap()
            .into_iter()
            .find(|entry| entry.path == Path::new("after.txt"))
            .unwrap();
        assert_eq!(
            change.original_path.as_deref(),
            Some(Path::new("before.txt"))
        );
        let expected_token = git::change_token(temp.path(), &change).await.unwrap();
        OperationRunner
            .execute_batch(BatchOperationSpec {
                project: project(temp.path()),
                items: vec![crate::domain::BatchOperationItem {
                    change,
                    expected_token,
                }],
                kind: OperationKind::Stash,
            })
            .await
            .unwrap();
        assert!(git::changes(temp.path()).await.unwrap().is_empty());

        run_git(temp.path(), &["stash", "apply", "--index", "stash@{0}"]);
        let restored = git::changes(temp.path()).await.unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].path, Path::new("after.txt"));
        assert_eq!(
            restored[0].original_path.as_deref(),
            Some(Path::new("before.txt"))
        );
    }

    #[tokio::test]
    async fn discards_selected_tracked_added_untracked_and_rename_changes() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("rename.txt"), "rename\n").unwrap();
        fs::write(temp.path().join("keep.txt"), "keep\n").unwrap();
        run_git(temp.path(), &["add", "."]);
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
                "second",
            ],
        );
        fs::write(temp.path().join("tracked.txt"), "changed\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        fs::write(temp.path().join("added.txt"), "added\n").unwrap();
        run_git(temp.path(), &["add", "added.txt"]);
        fs::write(temp.path().join("loose.txt"), "loose\n").unwrap();
        fs::write(temp.path().join("keep.txt"), "preserved\n").unwrap();
        run_git(temp.path(), &["mv", "rename.txt", "renamed.txt"]);
        let entries = git::changes(temp.path()).await.unwrap();
        let mut items = Vec::new();
        for change in entries
            .into_iter()
            .filter(|entry| entry.path != Path::new("keep.txt"))
        {
            let expected_token = git::change_token(temp.path(), &change).await.unwrap();
            items.push(crate::domain::BatchOperationItem {
                change,
                expected_token,
            });
        }
        OperationRunner
            .execute_batch(BatchOperationSpec {
                project: project(temp.path()),
                items,
                kind: OperationKind::Discard,
            })
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("tracked.txt")).unwrap(),
            "base\n"
        );
        assert!(temp.path().join("rename.txt").is_file());
        assert!(!temp.path().join("renamed.txt").exists());
        assert!(!temp.path().join("added.txt").exists());
        assert!(!temp.path().join("loose.txt").exists());
        let current = git::changes(temp.path()).await.unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].path, Path::new("keep.txt"));
    }

    #[tokio::test]
    async fn discard_batch_preflight_is_zero_write_when_one_token_is_stale() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("first.txt"), "first\n").unwrap();
        fs::write(temp.path().join("second.txt"), "second\n").unwrap();
        let entries = git::changes(temp.path()).await.unwrap();
        let mut items = Vec::new();
        for change in entries {
            let expected_token = git::change_token(temp.path(), &change).await.unwrap();
            items.push(crate::domain::BatchOperationItem {
                change,
                expected_token,
            });
        }
        fs::write(temp.path().join("second.txt"), "stale\n").unwrap();
        let error = OperationRunner
            .execute_batch(BatchOperationSpec {
                project: project(temp.path()),
                items,
                kind: OperationKind::Discard,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed after preview"));
        assert!(temp.path().join("first.txt").is_file());
        assert_eq!(
            fs::read_to_string(temp.path().join("second.txt")).unwrap(),
            "stale\n"
        );
    }

    #[tokio::test]
    async fn discards_staged_and_untracked_files_on_unborn_branch() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        fs::write(temp.path().join("staged.txt"), "staged\n").unwrap();
        fs::write(temp.path().join("loose.txt"), "loose\n").unwrap();
        run_git(temp.path(), &["add", "staged.txt"]);
        let entries = git::changes(temp.path()).await.unwrap();
        let mut items = Vec::new();
        for change in entries {
            let expected_token = git::change_token(temp.path(), &change).await.unwrap();
            items.push(crate::domain::BatchOperationItem {
                change,
                expected_token,
            });
        }
        OperationRunner
            .execute_batch(BatchOperationSpec {
                project: project(temp.path()),
                items,
                kind: OperationKind::Discard,
            })
            .await
            .unwrap();
        assert!(git::changes(temp.path()).await.unwrap().is_empty());
        assert!(!temp.path().join("staged.txt").exists());
        assert!(!temp.path().join("loose.txt").exists());
    }

    #[tokio::test]
    async fn batch_preflight_rejects_all_writes_when_one_token_is_stale() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("first.txt"), "first\n").unwrap();
        fs::write(temp.path().join("second.txt"), "second\n").unwrap();
        let repository = project(temp.path());
        let entries = git::changes(temp.path()).await.unwrap();
        let mut items = Vec::new();
        for change in entries.into_iter().filter(|entry| entry.untracked) {
            let expected_token = git::change_token(temp.path(), &change).await.unwrap();
            items.push(crate::domain::BatchOperationItem {
                change,
                expected_token,
            });
        }
        fs::write(temp.path().join("second.txt"), "changed after selection\n").unwrap();
        let error = OperationRunner
            .execute_batch(BatchOperationSpec {
                project: repository,
                items,
                kind: OperationKind::Stage,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed after preview"));
        assert!(git_text(temp.path(), &["diff", "--cached"]).is_empty());
    }

    #[tokio::test]
    async fn unstages_new_file_on_unborn_branch_without_deleting_it() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        fs::write(temp.path().join("first.txt"), "first\n").unwrap();
        run_git(temp.path(), &["add", "first.txt"]);
        let repository = project(temp.path());
        let staged = git::changes(temp.path()).await.unwrap().remove(0);
        assert!(staged.can_unstage());
        let token = git::change_token(temp.path(), &staged).await.unwrap();
        OperationRunner
            .execute(OperationSpec {
                project: repository,
                change: staged,
                kind: OperationKind::Unstage,
                target: OperationTarget::File,
                expected_token: token,
            })
            .await
            .unwrap();

        assert!(temp.path().join("first.txt").is_file());
        let current = git::changes(temp.path()).await.unwrap().remove(0);
        assert!(current.untracked);
    }

    #[tokio::test]
    async fn rejects_stale_token_and_index_lock() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("tracked.txt"), "first\n").unwrap();
        let repository = project(temp.path());
        let entry = git::changes(temp.path()).await.unwrap().remove(0);
        let token = git::change_token(temp.path(), &entry).await.unwrap();
        fs::write(temp.path().join("tracked.txt"), "second\n").unwrap();

        let error = OperationRunner
            .execute(OperationSpec {
                project: repository.clone(),
                change: entry.clone(),
                kind: OperationKind::Stage,
                target: OperationTarget::File,
                expected_token: token,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed after preview"));

        fs::write(temp.path().join(".git/index.lock"), "").unwrap();
        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let token = git::change_token(temp.path(), &current).await.unwrap();
        let error = OperationRunner
            .execute(OperationSpec {
                project: repository,
                change: current,
                kind: OperationKind::Stage,
                target: OperationTarget::File,
                expected_token: token,
            })
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("index is locked"),
            "unexpected error: {error:#}"
        );
    }

    #[tokio::test]
    async fn hunk_operations_leave_other_hunks_unchanged() {
        let temp = tempdir().unwrap();
        initialize_multihunk(temp.path());
        let mut lines = fs::read_to_string(temp.path().join("tracked.txt"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        lines[1] = "changed 02".into();
        lines[24] = "changed 25".into();
        fs::write(
            temp.path().join("tracked.txt"),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();
        let runner = OperationRunner;
        let repository = project(temp.path());

        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let worktree_hunks = preview
            .hunks
            .iter()
            .filter(|hunk| hunk.source == HunkSource::Worktree)
            .collect::<Vec<_>>();
        assert_eq!(worktree_hunks.len(), 2);
        runner
            .execute(OperationSpec {
                project: repository.clone(),
                change: current,
                kind: OperationKind::Stage,
                target: OperationTarget::Hunk {
                    source: HunkSource::Worktree,
                    fingerprint: worktree_hunks[0].fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap();
        let cached = git_text(temp.path(), &["diff", "--cached"]);
        let worktree = git_text(temp.path(), &["diff"]);
        assert!(cached.contains("changed 02"));
        assert!(!cached.contains("changed 25"));
        assert!(!worktree.contains("changed 02"));
        assert!(worktree.contains("changed 25"));

        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let staged = preview
            .hunks
            .iter()
            .find(|hunk| hunk.source == HunkSource::Staged)
            .unwrap();
        runner
            .execute(OperationSpec {
                project: repository.clone(),
                change: current,
                kind: OperationKind::Unstage,
                target: OperationTarget::Hunk {
                    source: HunkSource::Staged,
                    fingerprint: staged.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap();
        assert!(git_text(temp.path(), &["diff", "--cached"]).is_empty());
        let worktree = git_text(temp.path(), &["diff"]);
        assert!(worktree.contains("changed 02"));
        assert!(worktree.contains("changed 25"));

        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let first = preview
            .hunks
            .iter()
            .find(|hunk| hunk.source == HunkSource::Worktree)
            .unwrap();
        runner
            .execute(OperationSpec {
                project: repository,
                change: current,
                kind: OperationKind::RestoreWorktree,
                target: OperationTarget::Hunk {
                    source: HunkSource::Worktree,
                    fingerprint: first.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap();
        let worktree = git_text(temp.path(), &["diff"]);
        assert!(!worktree.contains("changed 02"));
        assert!(worktree.contains("changed 25"));
        let content = fs::read_to_string(temp.path().join("tracked.txt")).unwrap();
        assert!(content.contains("line 02"));
        assert!(content.contains("changed 25"));
    }

    #[tokio::test]
    async fn stages_untracked_hunk_and_rejects_stale_hunk() {
        let temp = tempdir().unwrap();
        initialize(temp.path());
        fs::write(temp.path().join("new.txt"), "first\n").unwrap();
        let repository = project(temp.path());
        let entry = git::changes(temp.path())
            .await
            .unwrap()
            .into_iter()
            .find(|entry| entry.path == Path::new("new.txt"))
            .unwrap();
        let preview = git::preview_change(temp.path(), &entry).await.unwrap();
        let hunk = preview.hunks.first().unwrap();
        fs::write(temp.path().join("new.txt"), "first\nsecond\n").unwrap();
        let error = OperationRunner
            .execute(OperationSpec {
                project: repository.clone(),
                change: entry,
                kind: OperationKind::Stage,
                target: OperationTarget::Hunk {
                    source: HunkSource::Untracked,
                    fingerprint: hunk.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed after preview"));

        let current = git::changes(temp.path())
            .await
            .unwrap()
            .into_iter()
            .find(|entry| entry.path == Path::new("new.txt"))
            .unwrap();
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let hunk = preview.hunks.first().unwrap();
        OperationRunner
            .execute(OperationSpec {
                project: repository,
                change: current,
                kind: OperationKind::Stage,
                target: OperationTarget::Hunk {
                    source: HunkSource::Untracked,
                    fingerprint: hunk.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap();
        let cached = git_text(temp.path(), &["diff", "--cached"]);
        assert!(cached.contains("first"));
        assert!(cached.contains("second"));
        assert!(git_text(temp.path(), &["diff"]).is_empty());
    }

    #[tokio::test]
    async fn line_operations_isolate_selected_lines_and_reject_stale_state() {
        let temp = tempdir().unwrap();
        initialize_multihunk(temp.path());
        let original = fs::read_to_string(temp.path().join("tracked.txt")).unwrap();
        let mut lines = original.lines().map(str::to_owned).collect::<Vec<_>>();
        lines.insert(2, "insert A".into());
        lines.insert(4, "insert B".into());
        fs::write(
            temp.path().join("tracked.txt"),
            format!("{}\n", lines.join("\n")),
        )
        .unwrap();
        let repository = project(temp.path());
        let runner = OperationRunner;

        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let inserted = preview
            .lines
            .iter()
            .filter(|line| line.source == HunkSource::Worktree)
            .collect::<Vec<_>>();
        assert_eq!(inserted.len(), 2);
        let first = inserted[0].clone();
        runner
            .execute(OperationSpec {
                project: repository.clone(),
                change: current,
                kind: OperationKind::Stage,
                target: OperationTarget::Line {
                    source: first.source,
                    hunk_fingerprint: first.hunk_fingerprint,
                    fingerprint: first.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap();
        let cached = git_text(temp.path(), &["show", ":tracked.txt"]);
        let worktree = fs::read_to_string(temp.path().join("tracked.txt")).unwrap();
        assert!(cached.contains("insert A"));
        assert!(!cached.contains("insert B"));
        assert!(worktree.contains("insert A"));
        assert!(worktree.contains("insert B"));

        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let staged = preview
            .lines
            .iter()
            .find(|line| line.source == HunkSource::Staged)
            .unwrap();
        runner
            .execute(OperationSpec {
                project: repository.clone(),
                change: current,
                kind: OperationKind::Unstage,
                target: OperationTarget::Line {
                    source: staged.source,
                    hunk_fingerprint: staged.hunk_fingerprint,
                    fingerprint: staged.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap();
        assert!(git_text(temp.path(), &["diff", "--cached"]).is_empty());

        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let first = preview
            .lines
            .iter()
            .find(|line| {
                line.source == HunkSource::Worktree
                    && preview.text.lines().nth(line.display_line) == Some("+insert A")
            })
            .unwrap();
        runner
            .execute(OperationSpec {
                project: repository.clone(),
                change: current,
                kind: OperationKind::RestoreWorktree,
                target: OperationTarget::Line {
                    source: first.source,
                    hunk_fingerprint: first.hunk_fingerprint,
                    fingerprint: first.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap();
        let content = fs::read_to_string(temp.path().join("tracked.txt")).unwrap();
        assert!(!content.contains("insert A"));
        assert!(content.contains("insert B"));

        let current = git::changes(temp.path()).await.unwrap().remove(0);
        let preview = git::preview_change(temp.path(), &current).await.unwrap();
        let line = preview.lines.first().unwrap();
        fs::write(
            temp.path().join("tracked.txt"),
            format!("{original}different\n"),
        )
        .unwrap();
        let error = runner
            .execute(OperationSpec {
                project: repository,
                change: current,
                kind: OperationKind::RestoreWorktree,
                target: OperationTarget::Line {
                    source: line.source,
                    hunk_fingerprint: line.hunk_fingerprint,
                    fingerprint: line.fingerprint,
                },
                expected_token: preview.token,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed after preview"));
    }
}
