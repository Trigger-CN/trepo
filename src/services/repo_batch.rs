use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, timeout};

use crate::adapters::{git, repo};
use crate::domain::{Project, ProjectId, RepoBatchSpec, RepoProjectState};

const CANCEL_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum RepoBatchEventKind {
    Started {
        project: Option<Project>,
        args: Vec<String>,
    },
    Log {
        project_id: Option<ProjectId>,
        line: String,
    },
    Finished {
        project: Option<Project>,
        state: RepoProjectState,
        message: String,
    },
    Complete {
        cancelled: bool,
    },
}

#[derive(Debug)]
pub struct RepoBatchEvent {
    pub generation: u64,
    pub kind: RepoBatchEventKind,
}

#[derive(Debug, Clone)]
pub struct RepoBatchHandle {
    cancelled: Arc<AtomicBool>,
}

impl RepoBatchHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub fn spawn(
    root: PathBuf,
    spec: RepoBatchSpec,
    projects: Vec<Project>,
    generation: u64,
    sender: mpsc::UnboundedSender<RepoBatchEvent>,
) -> RepoBatchHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let handle = RepoBatchHandle {
        cancelled: Arc::clone(&cancelled),
    };
    tokio::spawn(run_batch(
        OsString::from("repo"),
        root,
        spec,
        projects,
        generation,
        cancelled,
        sender,
    ));
    handle
}

async fn run_batch(
    program: OsString,
    root: PathBuf,
    spec: RepoBatchSpec,
    projects: Vec<Project>,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    sender: mpsc::UnboundedSender<RepoBatchEvent>,
) {
    let lock = workspace_lock(root.clone());
    let _guard = lock.lock().await;
    if let Err(error) = validate_manifest_output(&root, &spec) {
        send(
            &sender,
            generation,
            RepoBatchEventKind::Finished {
                project: None,
                state: RepoProjectState::Failed,
                message: error.to_string(),
            },
        );
        send(
            &sender,
            generation,
            RepoBatchEventKind::Complete { cancelled: false },
        );
        return;
    }
    let targets: Vec<Option<Project>> = if spec.action.is_workspace_action() {
        vec![None]
    } else {
        projects.into_iter().map(Some).collect()
    };

    for (index, project) in targets.iter().cloned().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            for skipped in targets[index..].iter().cloned() {
                send(
                    &sender,
                    generation,
                    RepoBatchEventKind::Finished {
                        project: skipped,
                        state: RepoProjectState::Cancelled,
                        message: "Not started because cancellation was requested".to_owned(),
                    },
                );
            }
            break;
        }
        if let Some(target) = project.as_ref() {
            if let Err(error) = validate_project(&root, target, spec.action).await {
                send(
                    &sender,
                    generation,
                    RepoBatchEventKind::Finished {
                        project,
                        state: RepoProjectState::Failed,
                        message: error.to_string(),
                    },
                );
                continue;
            }
        }
        let project_path = project.as_ref().map(|value| value.relative_path.as_path());
        let args = match repo::batch_args(&spec, project_path) {
            Ok(args) => args,
            Err(error) => {
                send(
                    &sender,
                    generation,
                    RepoBatchEventKind::Finished {
                        project,
                        state: RepoProjectState::Failed,
                        message: error.to_string(),
                    },
                );
                continue;
            }
        };
        send(
            &sender,
            generation,
            RepoBatchEventKind::Started {
                project: project.clone(),
                args: args.clone(),
            },
        );
        let project_id = project.as_ref().map(|value| value.id.clone());
        let outcome = run_command(
            &program,
            &root,
            &args,
            project_id,
            generation,
            Arc::clone(&cancelled),
            &sender,
        )
        .await;
        let (state, message) = match outcome {
            Ok(ProcessOutcome::Exited(status)) if status.success() => {
                (RepoProjectState::Succeeded, "Command completed".to_owned())
            }
            Ok(ProcessOutcome::Exited(status)) => (
                RepoProjectState::Failed,
                format!("Command exited with {status}"),
            ),
            Ok(ProcessOutcome::Cancelled) => (
                RepoProjectState::Cancelled,
                "Cancelled; changes already made were not rolled back".to_owned(),
            ),
            Err(error) => (RepoProjectState::Failed, error.to_string()),
        };
        send(
            &sender,
            generation,
            RepoBatchEventKind::Finished {
                project,
                state,
                message,
            },
        );
    }
    send(
        &sender,
        generation,
        RepoBatchEventKind::Complete {
            cancelled: cancelled.load(Ordering::Acquire),
        },
    );
}

#[derive(Debug)]
enum ProcessOutcome {
    Exited(ExitStatus),
    Cancelled,
}

async fn run_command(
    program: &OsStr,
    root: &Path,
    args: &[String],
    project_id: Option<ProjectId>,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    sender: &mpsc::UnboundedSender<RepoBatchEvent>,
) -> Result<ProcessOutcome> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to run repo {}", args[0]))?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .context("repo stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("repo stderr was not captured")?;
    let stdout_task = tokio::spawn(stream_lines(
        stdout,
        project_id.clone(),
        generation,
        sender.clone(),
    ));
    let stderr_task = tokio::spawn(stream_lines(stderr, project_id, generation, sender.clone()));

    let outcome = tokio::select! {
        status = child.wait() => ProcessOutcome::Exited(status.context("failed to wait for repo")?),
        () = wait_for_cancel(Arc::clone(&cancelled)) => {
            if let Some(pid) = pid {
                interrupt_group(pid);
            }
            match timeout(CANCEL_GRACE, child.wait()).await {
                Ok(status) => {
                    let _ = status.context("failed to wait for interrupted repo")?;
                }
                Err(_) => {
                    terminate_group(pid);
                    let _ = child.wait().await;
                }
            }
            ProcessOutcome::Cancelled
        }
    };
    let _ = stdout_task.await;
    let _ = stderr_task.await;
    Ok(outcome)
}

async fn stream_lines<R>(
    stream: R,
    project_id: Option<ProjectId>,
    generation: u64,
    sender: mpsc::UnboundedSender<RepoBatchEvent>,
) where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        send(
            &sender,
            generation,
            RepoBatchEventKind::Log {
                project_id: project_id.clone(),
                line: redact_log_line(&line),
            },
        );
    }
}

fn redact_log_line(line: &str) -> String {
    let mut result = line.to_owned();
    let lower = result.to_ascii_lowercase();
    let mut ranges = Vec::new();
    for name in ["token", "password", "credential", "authorization"] {
        for separator in ['=', ':'] {
            let prefix = format!("{name}{separator}");
            let mut offset = 0;
            while let Some(relative) = lower[offset..].find(&prefix) {
                let start = offset + relative + prefix.len();
                let end = result[start..]
                    .find(char::is_whitespace)
                    .map_or(result.len(), |length| start + length);
                ranges.push((start, end));
                offset = end;
            }
        }
    }
    ranges.sort_unstable_by_key(|range| range.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    for (start, end) in merged.into_iter().rev() {
        result.replace_range(start..end, "***");
    }
    redact_url_userinfo(&result)
}

fn redact_url_userinfo(line: &str) -> String {
    let mut result = line.to_owned();
    let mut search_from = 0;
    while let Some(relative_scheme) = result[search_from..].find("://") {
        let authority_start = search_from + relative_scheme + 3;
        let authority_end = result[authority_start..]
            .find(['/', '?', '#', ' '])
            .map_or(result.len(), |index| authority_start + index);
        let Some(relative_at) = result[authority_start..authority_end].rfind('@') else {
            search_from = authority_end;
            continue;
        };
        let at = authority_start + relative_at;
        result.replace_range(authority_start..at, "***");
        search_from = authority_start + 4;
    }
    result
}

async fn wait_for_cancel(cancelled: Arc<AtomicBool>) {
    while !cancelled.load(Ordering::Acquire) {
        sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
fn interrupt_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGINT);
    }
}

#[cfg(unix)]
fn terminate_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn interrupt_group(_pid: u32) {}

#[cfg(not(unix))]
fn terminate_group(_pid: Option<u32>) {}

fn send(sender: &mpsc::UnboundedSender<RepoBatchEvent>, generation: u64, kind: RepoBatchEventKind) {
    let _ = sender.send(RepoBatchEvent { generation, kind });
}

fn validate_manifest_output(root: &Path, spec: &RepoBatchSpec) -> Result<()> {
    let Some(output) = spec.output.as_deref() else {
        return Ok(());
    };
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let absolute_parent = root.join(parent);
    let canonical_root = root
        .canonicalize()
        .context("failed to resolve Repo workspace root")?;
    let canonical_parent = absolute_parent.canonicalize().with_context(|| {
        format!(
            "manifest output directory does not exist: {}",
            parent.display()
        )
    })?;
    let target = root.join(output);
    if target
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        anyhow::bail!("manifest output must not be a symbolic link");
    }
    if !canonical_parent.starts_with(&canonical_root) {
        anyhow::bail!("manifest output resolves outside the Repo workspace");
    }
    Ok(())
}

async fn validate_project(
    root: &Path,
    project: &Project,
    action: crate::domain::RepoBatchAction,
) -> Result<()> {
    let expected = root.join(&project.relative_path);
    if expected != project.path {
        anyhow::bail!("project path changed after selection");
    }
    if !project.path.is_dir() {
        if action == crate::domain::RepoBatchAction::Sync {
            return Ok(());
        }
        anyhow::bail!("project directory no longer exists");
    }
    let canonical_root = root
        .canonicalize()
        .context("failed to resolve Repo workspace root")?;
    let canonical_project = project
        .path
        .canonicalize()
        .context("failed to resolve selected project")?;
    if !canonical_project.starts_with(&canonical_root) {
        anyhow::bail!("selected project resolves outside the Repo workspace");
    }
    if git::git_path(&project.path, "index.lock").await?.is_file() {
        anyhow::bail!("Git index is locked; another writer may be active");
    }
    Ok(())
}

pub(crate) fn workspace_lock_for_project(path: &Path) -> Option<Arc<Mutex<()>>> {
    path.ancestors()
        .find(|candidate| candidate.join(".repo").is_dir())
        .map(|root| workspace_lock(root.to_path_buf()))
}

fn workspace_lock(path: PathBuf) -> Arc<Mutex<()>> {
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
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    use super::*;
    use crate::domain::{ProjectId, RepoBatchAction};

    fn git_project(root: &Path, name: &str) -> Project {
        let path = root.join(name);
        fs::create_dir_all(&path).unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&path)
            .status()
            .unwrap()
            .success());
        Project {
            id: ProjectId(path.clone()),
            name: name.to_owned(),
            path,
            relative_path: PathBuf::from(name),
        }
    }

    fn fake_repo(root: &Path, body: &str) -> PathBuf {
        let path = root.join("fake-repo");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn sync_spec() -> RepoBatchSpec {
        RepoBatchSpec {
            action: RepoBatchAction::Sync,
            branch: None,
            change: None,
            output: None,
        }
    }

    #[tokio::test]
    async fn streams_logs_and_reports_partial_failure_per_project() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join(".repo")).unwrap();
        let alpha = git_project(temp.path(), "alpha");
        let beta = git_project(temp.path(), "beta");
        let program = fake_repo(
            temp.path(),
            "echo stdout:$*\necho stderr:$* >&2\ncase \"$*\" in *beta*) exit 7;; esac",
        );
        let (tx, mut rx) = mpsc::unbounded_channel();
        run_batch(
            program.into_os_string(),
            temp.path().to_path_buf(),
            sync_spec(),
            vec![alpha.clone(), beta.clone()],
            9,
            Arc::new(AtomicBool::new(false)),
            tx,
        )
        .await;

        let mut logs = Vec::new();
        let mut states = HashMap::new();
        while let Ok(event) = rx.try_recv() {
            assert_eq!(event.generation, 9);
            match event.kind {
                RepoBatchEventKind::Log { line, .. } => logs.push(line),
                RepoBatchEventKind::Finished {
                    project: Some(project),
                    state,
                    ..
                } => {
                    states.insert(project.id, state);
                }
                _ => {}
            }
        }
        assert!(logs
            .iter()
            .any(|line| line.starts_with("stdout:sync -- alpha")));
        assert!(logs
            .iter()
            .any(|line| line.starts_with("stderr:sync -- beta")));
        assert_eq!(states[&alpha.id], RepoProjectState::Succeeded);
        assert_eq!(states[&beta.id], RepoProjectState::Failed);
    }

    #[tokio::test]
    async fn cancellation_interrupts_process_group_and_marks_remaining_targets() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join(".repo")).unwrap();
        let alpha = git_project(temp.path(), "alpha");
        let beta = git_project(temp.path(), "beta");
        let program = fake_repo(temp.path(), "trap 'exit 130' INT\necho started\nsleep 30");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(run_batch(
            program.into_os_string(),
            temp.path().to_path_buf(),
            sync_spec(),
            vec![alpha.clone(), beta.clone()],
            10,
            Arc::clone(&cancelled),
            tx,
        ));

        timeout(Duration::from_secs(2), async {
            loop {
                if let Some(event) = rx.recv().await {
                    if matches!(event.kind, RepoBatchEventKind::Log { .. }) {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        cancelled.store(true, Ordering::Release);
        timeout(Duration::from_secs(4), task)
            .await
            .unwrap()
            .unwrap();

        let mut states = HashMap::new();
        while let Ok(event) = rx.try_recv() {
            if let RepoBatchEventKind::Finished {
                project: Some(project),
                state,
                ..
            } = event.kind
            {
                states.insert(project.id, state);
            }
        }
        assert_eq!(states[&alpha.id], RepoProjectState::Cancelled);
        assert_eq!(states[&beta.id], RepoProjectState::Cancelled);
    }

    #[test]
    fn redacts_credentials_without_dropping_log_context() {
        let line =
            redact_log_line("fetch https://user:secret@example.com/repo token=abc Password:xyz ok");
        assert_eq!(
            line,
            "fetch https://***@example.com/repo token=*** Password:*** ok"
        );
        assert!(!line.contains("secret"));
        assert!(!line.contains("abc"));
        assert!(!line.contains("xyz"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_manifest_output_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join(".repo")).unwrap();
        let outside = tempdir().unwrap();
        let output = temp.path().join("pinned.xml");
        symlink(outside.path().join("stolen.xml"), &output).unwrap();
        let spec = RepoBatchSpec {
            action: RepoBatchAction::ManifestExport,
            branch: None,
            change: None,
            output: Some(PathBuf::from("pinned.xml")),
        };
        assert!(validate_manifest_output(temp.path(), &spec).is_err());
    }
}
