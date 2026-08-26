use std::sync::Arc;

use tokio::sync::{mpsc, Semaphore};

use crate::adapters::git;
use crate::domain::{Project, ProjectSnapshot, ScanState};

#[derive(Debug)]
pub struct ScanResult {
    pub snapshot: ProjectSnapshot,
}

pub fn spawn_scan(
    projects: Vec<Project>,
    generation: u64,
    concurrency: usize,
    sender: mpsc::UnboundedSender<ScanResult>,
) {
    let concurrency = concurrency.max(1);
    tokio::spawn(async move {
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut tasks = tokio::task::JoinSet::new();

        for project in projects {
            let semaphore = Arc::clone(&semaphore);
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await.expect("semaphore closed");
                scan_project(project, generation).await
            });
        }

        while let Some(result) = tasks.join_next().await {
            if let Ok(snapshot) = result {
                let _ = sender.send(ScanResult { snapshot });
            }
        }
    });
}

async fn scan_project(project: Project, generation: u64) -> ProjectSnapshot {
    let mut snapshot = ProjectSnapshot::pending(project, generation);
    if !snapshot.project.path.is_dir() {
        snapshot.scan = ScanState::Error("project path is missing".to_owned());
        return snapshot;
    }

    match git::status(&snapshot.project.path).await {
        Ok(status) => {
            snapshot.head = status.head;
            snapshot.upstream = status.upstream;
            snapshot.worktree = status.worktree;
            snapshot.changes = status.changes;
            snapshot.scan = ScanState::Ready;
        }
        Err(error) => snapshot.scan = ScanState::Error(error.to_string()),
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::domain::{ProjectId, ScanState};

    #[tokio::test]
    async fn scans_real_repository_and_reports_missing_project() {
        let temp = tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        fs::write(temp.path().join("new.txt"), "new").unwrap();

        let existing = Project {
            id: ProjectId(temp.path().to_path_buf()),
            name: "existing".into(),
            path: temp.path().to_path_buf(),
            relative_path: PathBuf::from("."),
        };
        let missing_path = temp.path().join("missing");
        let missing = Project {
            id: ProjectId(missing_path.clone()),
            name: "missing".into(),
            path: missing_path.clone(),
            relative_path: PathBuf::from("missing"),
        };
        let (sender, mut receiver) = mpsc::unbounded_channel();
        spawn_scan(vec![existing, missing], 7, 2, sender);

        let first = receiver.recv().await.unwrap().snapshot;
        let second = receiver.recv().await.unwrap().snapshot;
        let snapshots = [first, second];
        assert!(snapshots.iter().any(|value| {
            value.worktree.untracked == 1
                && value.changes.len() == 1
                && value.changes[0].path == std::path::Path::new("new.txt")
        }));
        assert!(snapshots.iter().any(|value| {
            value.project.path == missing_path && matches!(value.scan, ScanState::Error(_))
        }));
    }
}
