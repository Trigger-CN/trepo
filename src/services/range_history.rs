use std::sync::Arc;

use tokio::sync::{mpsc, Semaphore};

use crate::adapters::git::{self, LogRangeFilter};
use crate::domain::{Commit, Project, ProjectId, RangeHistorySpec};

/// One repository's response to a workspace-wide range query.
#[derive(Debug)]
pub struct RangeHistoryResult {
    pub generation: u64,
    pub project_id: ProjectId,
    pub project_name: String,
    pub result: anyhow::Result<Vec<Commit>>,
}

/// Loads the range-limited history for every project concurrently, streaming one
/// result per repository so partial output becomes visible immediately.
///
/// Each repository is independent: a failure (missing path, not a repository,
/// bad revision) is reported for that project only and never aborts the others.
pub fn spawn_range_history(
    projects: Vec<Project>,
    spec: RangeHistorySpec,
    generation: u64,
    concurrency: usize,
    sender: mpsc::UnboundedSender<RangeHistoryResult>,
) {
    let concurrency = concurrency.max(1);
    tokio::spawn(async move {
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut tasks = tokio::task::JoinSet::new();

        for project in projects {
            let semaphore = Arc::clone(&semaphore);
            let spec = spec.clone();
            tasks.spawn(async move {
                let _permit = semaphore.acquire_owned().await.expect("semaphore closed");
                let filter = LogRangeFilter {
                    since: spec.since.clone(),
                    until: spec.until.clone(),
                    author: spec.author.clone(),
                    query: spec.query.clone(),
                };
                let result = git::log_range(&project.path, &filter).await;
                RangeHistoryResult {
                    generation,
                    project_id: project.id,
                    project_name: project.name,
                    result,
                }
            });
        }

        while let Some(result) = tasks.join_next().await {
            if let Ok(result) = result {
                let _ = sender.send(result);
            }
        }
    });
}
