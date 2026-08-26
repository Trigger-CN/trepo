use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use crate::domain::{RepoBatchAction, RepoBatchSpec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoProject {
    pub path: String,
    pub name: String,
}

pub async fn list_projects(root: &Path) -> Result<Vec<RepoProject>> {
    let output = Command::new("repo")
        .arg("list")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to run repo in {}", root.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!("repo list exited with {}: {stderr}", output.status);
    }
    parse_list(&output.stdout)
}

pub fn sync_args(project_paths: &[&Path]) -> Result<Vec<String>> {
    let mut args = vec!["sync".to_owned(), "-c".to_owned(), "-j8".to_owned()];
    if !project_paths.is_empty() {
        args.push("--".to_owned());
    }
    for project_path in project_paths {
        validate_project_path(project_path)?;
        args.push(project_path.to_string_lossy().into_owned());
    }
    Ok(args)
}

pub fn batch_args(spec: &RepoBatchSpec, project_path: Option<&Path>) -> Result<Vec<String>> {
    let mut args = vec![spec.action.command().to_owned()];
    if spec.action.is_workspace_action() {
        let output = spec
            .output
            .as_deref()
            .context("manifest output path is required")?;
        if output.as_os_str().is_empty()
            || output.is_absolute()
            || output.components().any(|part| part.as_os_str() == "..")
        {
            bail!("manifest output must be a non-empty path inside the workspace");
        }
        args.extend([
            "-r".to_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ]);
        return Ok(args);
    }

    let project_path = project_path.context("project path is required")?;
    validate_project_path(project_path)?;
    if spec.action == RepoBatchAction::Upload {
        args.extend(["--current-branch".to_owned(), "--yes".to_owned()]);
    }
    args.push("--".to_owned());
    match spec.action {
        RepoBatchAction::Start | RepoBatchAction::Checkout | RepoBatchAction::Abandon => {
            let branch = spec
                .branch
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .context("branch is required")?;
            args.push(branch.to_owned());
            args.push(project_path.to_string_lossy().into_owned());
        }
        RepoBatchAction::Download => {
            let change = spec
                .change
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .context("change is required")?;
            args.push(project_path.to_string_lossy().into_owned());
            args.push(change.to_owned());
        }
        RepoBatchAction::Sync => {
            bail!("repo sync requires the aggregated project argument builder");
        }
        RepoBatchAction::Prune | RepoBatchAction::Rebase | RepoBatchAction::Upload => {
            args.push(project_path.to_string_lossy().into_owned());
        }
        RepoBatchAction::ManifestExport => unreachable!("handled above"),
    }
    Ok(args)
}

fn validate_project_path(project_path: &Path) -> Result<()> {
    if project_path.as_os_str().is_empty()
        || project_path.is_absolute()
        || project_path
            .components()
            .any(|part| part.as_os_str() == "..")
    {
        bail!("project path must stay relative to the workspace");
    }
    Ok(())
}
pub fn parse_list(bytes: &[u8]) -> Result<Vec<RepoProject>> {
    let text = std::str::from_utf8(bytes).context("repo list output is not UTF-8")?;
    let mut projects = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((path, name)) = line.split_once(" : ") else {
            bail!("invalid repo list line {}: {line}", line_number + 1);
        };
        if path.is_empty() || name.is_empty() {
            bail!("invalid repo list line {}: {line}", line_number + 1);
        }
        projects.push(RepoProject {
            path: path.to_owned(),
            name: name.to_owned(),
        });
    }
    Ok(projects)
}

pub async fn version() -> Result<String> {
    let output = Command::new("repo")
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to run repo version")?;
    let combined = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    Ok(version_line(&String::from_utf8_lossy(combined)))
}

fn version_line(output: &str) -> String {
    output
        .lines()
        .find(|line| {
            let line = line.trim().to_ascii_lowercase();
            line.starts_with("repo version") || line.starts_with("repo launcher version")
        })
        .or_else(|| output.lines().find(|line| !line.trim().starts_with('<')))
        .unwrap_or("repo version unknown")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repo_list() {
        let projects = parse_list(
            b"frameworks/base : platform/frameworks/base\nexternal/lib : platform/external/lib\n",
        )
        .unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].path, "frameworks/base");
        assert_eq!(projects[1].name, "platform/external/lib");
    }

    #[test]
    fn rejects_unstructured_output() {
        assert!(parse_list(b"not a project line\n").is_err());
    }

    #[test]
    fn selects_launcher_version_from_non_client_output() {
        let output = "<repo not installed>\nrepo launcher version 2.54\n(from /bin/repo)\n";
        assert_eq!(version_line(output), "repo launcher version 2.54");
    }

    fn spec(action: RepoBatchAction) -> RepoBatchSpec {
        RepoBatchSpec {
            action,
            branch: None,
            change: None,
            output: None,
        }
    }

    #[test]
    fn builds_one_parallel_sync_command_for_all_projects() {
        assert_eq!(
            sync_args(&[Path::new("alpha"), Path::new("platform/beta")]).unwrap(),
            ["sync", "-c", "-j8", "--", "alpha", "platform/beta"]
        );
        assert_eq!(sync_args(&[]).unwrap(), ["sync", "-c", "-j8"]);
        assert!(sync_args(&[Path::new("../outside")]).is_err());
    }

    #[test]
    fn builds_project_scoped_batch_arguments() {
        let mut value = spec(RepoBatchAction::Start);
        value.branch = Some("topic/x".into());
        assert_eq!(
            batch_args(&value, Some(Path::new("platform/demo"))).unwrap(),
            ["start", "--", "topic/x", "platform/demo"]
        );

        let mut value = spec(RepoBatchAction::Download);
        value.change = Some("12345/2".into());
        assert_eq!(
            batch_args(&value, Some(Path::new("platform/demo"))).unwrap(),
            ["download", "--", "platform/demo", "12345/2"]
        );
    }

    #[test]
    fn builds_workspace_manifest_arguments_and_rejects_escape() {
        let mut value = spec(RepoBatchAction::ManifestExport);
        value.output = Some("manifests/pinned.xml".into());
        assert_eq!(
            batch_args(&value, None).unwrap(),
            ["manifest", "-r", "-o", "manifests/pinned.xml"]
        );
        value.output = Some("../outside.xml".into());
        assert!(batch_args(&value, None).is_err());
    }
}
