use std::sync::Arc;

use tokio::sync::{mpsc, Semaphore};

use crate::adapters::git::{self, FileLog};
use crate::domain::{FileSearchSpec, Project, ProjectId};

/// Per-repository cap on listed files. A very large repository can hold tens of
/// thousands of paths; listing them all would duplicate work the search is
/// about to throw away, so the cap bounds what is parsed and matched.
pub const PER_REPOSITORY_FILE_LIMIT: usize = 4000;

/// One repository's response to a workspace-wide file search.
#[derive(Debug)]
pub struct FileSearchResult {
    pub generation: u64,
    pub project_id: ProjectId,
    pub project_name: String,
    pub result: anyhow::Result<Vec<String>>,
}

/// One file's history, requested by the file-search page for a chosen result.
#[derive(Debug)]
pub struct FileHistoryResult {
    pub generation: u64,
    pub project_id: ProjectId,
    /// Repository-relative path this history belongs to, so a stale response
    /// for a previously selected file cannot land in the current view.
    pub path: String,
    pub result: anyhow::Result<FileLog>,
}

/// Searches every project concurrently, streaming one result per repository so
/// partial output becomes visible immediately.
///
/// Matching happens here rather than in Git: the repositories are queried in
/// parallel, and every listed path is tested with [`FileSearchSpec::matches`]
/// before it reaches the UI. A repository failure only marks that repository.
pub fn spawn_file_search(
    projects: Vec<Project>,
    spec: FileSearchSpec,
    generation: u64,
    concurrency: usize,
    sender: mpsc::UnboundedSender<FileSearchResult>,
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
                let result = list_matching_files(&project, &spec).await;
                FileSearchResult {
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

/// Lists a repository's files and keeps the matching ones, bounded by
/// [`PER_REPOSITORY_FILE_LIMIT`] so a pathological repository cannot stall the
/// search after Git has already been asked for its whole path list.
async fn list_matching_files(
    project: &Project,
    spec: &FileSearchSpec,
) -> anyhow::Result<Vec<String>> {
    let files = git::list_files(&project.path).await?;
    Ok(files
        .into_iter()
        .filter(|file| spec.matches(file, &project.path))
        .take(PER_REPOSITORY_FILE_LIMIT)
        .collect())
}

/// Loads one file's commit history asynchronously.
pub fn spawn_file_history(
    project: Project,
    path: String,
    generation: u64,
    sender: mpsc::UnboundedSender<FileHistoryResult>,
) {
    tokio::spawn(async move {
        let result = git::file_log(&project.path, &path, PER_REPOSITORY_FILE_LIMIT).await;
        let _ = sender.send(FileHistoryResult {
            generation,
            project_id: project.id,
            path,
            result,
        });
    });
}
