use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

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
}
