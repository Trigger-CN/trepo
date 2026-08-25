use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::adapters::{git, repo};
use crate::domain::{Project, ProjectId, Workspace, WorkspaceKind};

pub async fn discover(path: &Path) -> Result<Workspace> {
    let start = absolute_path(path)?;
    let search_from = if start.is_file() {
        start
            .parent()
            .context("input path has no parent")?
            .to_path_buf()
    } else {
        start
    };

    if let Some(root) = find_repo_root(&search_from) {
        let listed = repo::list_projects(&root).await?;
        let projects = listed
            .into_iter()
            .map(|entry| {
                let relative_path = PathBuf::from(entry.path);
                let project_path = root.join(&relative_path);
                Project {
                    id: ProjectId(project_path.clone()),
                    name: entry.name,
                    path: project_path,
                    relative_path,
                }
            })
            .collect();
        return Ok(Workspace {
            root,
            kind: WorkspaceKind::Repo,
            projects,
        });
    }

    match git::repository_root(&search_from).await {
        Ok(root) => {
            let name = root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("repository")
                .to_owned();
            let project = Project {
                id: ProjectId(root.clone()),
                name,
                path: root.clone(),
                relative_path: PathBuf::from("."),
            };
            Ok(Workspace {
                root,
                kind: WorkspaceKind::Git,
                projects: vec![project],
            })
        }
        Err(error) => bail!(
            "{} is not inside a Repo workspace or Git repository: {error}",
            search_from.display()
        ),
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to read current directory")?
            .join(path)
    };
    path.canonicalize()
        .with_context(|| format!("failed to resolve {}", path.display()))
}

pub fn find_repo_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|candidate| candidate.join(".repo").is_dir())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn finds_repo_root_from_nested_directory() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join(".repo")).unwrap();
        let nested = temp.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_repo_root(&nested), Some(temp.path().to_path_buf()));
    }

    #[test]
    fn returns_none_without_repo_metadata() {
        let temp = tempdir().unwrap();
        assert_eq!(find_repo_root(temp.path()), None);
    }

    #[tokio::test]
    async fn discovers_real_git_repository_from_child() {
        let temp = tempdir().unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .status()
            .unwrap();
        assert!(status.success());
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();
        let workspace = discover(&child).await.unwrap();
        assert_eq!(workspace.kind, WorkspaceKind::Git);
        assert_eq!(workspace.projects.len(), 1);
        assert_eq!(workspace.root, temp.path().canonicalize().unwrap());
    }
}
