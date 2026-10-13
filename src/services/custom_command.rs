use std::ffi::OsString;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

use crate::domain::{CommandState, Project, ProjectId};
use crate::services::repo_batch::{project_directory_lock, redact_log_line};

const CANCEL_GRACE: Duration = Duration::from_secs(2);
const MAX_LOG_LINES: usize = 500;

/// Events streamed back to the UI while a custom command runs.
#[derive(Debug)]
pub enum CustomCommandEventKind {
    Started {
        project: Project,
        command: String,
    },
    Log {
        project_id: ProjectId,
        line: String,
    },
    Finished {
        project: Project,
        state: CommandState,
        message: String,
    },
    Complete {
        cancelled: bool,
    },
}

#[derive(Debug)]
pub struct CustomCommandEvent {
    pub generation: u64,
    pub kind: CustomCommandEventKind,
}

#[derive(Debug, Clone)]
pub struct CustomCommandHandle {
    cancelled: Arc<AtomicBool>,
}

impl CustomCommandHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Runs `command` through the user's shell once per project, with the project
/// directory as the working directory.
///
/// The command string is executed verbatim as a single `-c` argument, so pipes,
/// redirections and globs behave the way the user typed them, and no project
/// path is ever appended to argv.
pub fn spawn(
    program: OsString,
    command: String,
    projects: Vec<Project>,
    generation: u64,
    sender: mpsc::UnboundedSender<CustomCommandEvent>,
) -> CustomCommandHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let handle = CustomCommandHandle {
        cancelled: Arc::clone(&cancelled),
    };
    tokio::spawn(run(
        program, command, projects, generation, cancelled, sender,
    ));
    handle
}

async fn run(
    program: OsString,
    command: String,
    projects: Vec<Project>,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    sender: mpsc::UnboundedSender<CustomCommandEvent>,
) {
    for (index, project) in projects.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            for remaining in &projects[index..] {
                send_finished(
                    &sender,
                    generation,
                    remaining.clone(),
                    CommandState::Cancelled,
                    "Not started because cancellation was requested".to_owned(),
                );
            }
            break;
        }
        let lock = project_directory_lock(&project.path);
        let _guard = lock.lock().await;
        if let Err(error) = validate_project(&project.path).await {
            send_finished(
                &sender,
                generation,
                project.clone(),
                CommandState::Failed,
                error.to_string(),
            );
            continue;
        }
        send(
            &sender,
            generation,
            CustomCommandEventKind::Started {
                project: project.clone(),
                command: command.clone(),
            },
        );
        let outcome = run_command(
            &program,
            &command,
            &project.path,
            project.id.clone(),
            generation,
            Arc::clone(&cancelled),
            &sender,
        )
        .await;
        let (state, message) = match outcome {
            Ok(ProcessOutcome::Exited(status)) if status.success() => {
                (CommandState::Succeeded, "Command completed".to_owned())
            }
            Ok(ProcessOutcome::Exited(status)) => (
                CommandState::Failed,
                format!("Command exited with {status}"),
            ),
            Ok(ProcessOutcome::Cancelled) => (
                CommandState::Cancelled,
                "Cancelled; changes already made were not rolled back".to_owned(),
            ),
            Err(error) => (CommandState::Failed, error.to_string()),
        };
        send_finished(&sender, generation, project.clone(), state, message);
    }
    let _ = sender.send(CustomCommandEvent {
        generation,
        kind: CustomCommandEventKind::Complete {
            cancelled: cancelled.load(Ordering::Acquire),
        },
    });
}

async fn validate_project(path: &Path) -> Result<()> {
    if !path.is_dir() {
        anyhow::bail!("project directory no longer exists");
    }
    Ok(())
}

#[derive(Debug)]
enum ProcessOutcome {
    Exited(ExitStatus),
    Cancelled,
}

async fn run_command(
    program: &OsString,
    command: &str,
    directory: &Path,
    project_id: ProjectId,
    generation: u64,
    cancelled: Arc<AtomicBool>,
    sender: &mpsc::UnboundedSender<CustomCommandEvent>,
) -> Result<ProcessOutcome> {
    let mut process = Command::new(program);
    process
        .args(["-c", command])
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    let mut child = process
        .spawn()
        .with_context(|| format!("failed to run {}", program.to_string_lossy()))?;
    let pid = child.id();
    let stdout = child.stdout.take().context("stdout was not captured")?;
    let stderr = child.stderr.take().context("stderr was not captured")?;
    let stdout_task = tokio::spawn(stream_lines(
        stdout,
        project_id.clone(),
        generation,
        sender.clone(),
    ));
    let stderr_task = tokio::spawn(stream_lines(stderr, project_id, generation, sender.clone()));

    let outcome = tokio::select! {
        status = child.wait() => ProcessOutcome::Exited(status.context("failed to wait for command")?),
        () = wait_for_cancel(Arc::clone(&cancelled)) => {
            if let Some(pid) = pid {
                interrupt_group(pid);
            }
            match timeout(CANCEL_GRACE, child.wait()).await {
                Ok(status) => {
                    let _ = status.context("failed to wait for interrupted command")?;
                }
                Err(_) => terminate_process(&mut child, pid).await,
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
    project_id: ProjectId,
    generation: u64,
    sender: mpsc::UnboundedSender<CustomCommandEvent>,
) where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let _ = sender.send(CustomCommandEvent {
            generation,
            kind: CustomCommandEventKind::Log {
                project_id: project_id.clone(),
                line: redact_log_line(&line),
            },
        });
    }
}

fn send_finished(
    sender: &mpsc::UnboundedSender<CustomCommandEvent>,
    generation: u64,
    project: Project,
    state: CommandState,
    message: String,
) {
    send(
        sender,
        generation,
        CustomCommandEventKind::Finished {
            project,
            state,
            message,
        },
    );
}

fn send(
    sender: &mpsc::UnboundedSender<CustomCommandEvent>,
    generation: u64,
    kind: CustomCommandEventKind,
) {
    let _ = sender.send(CustomCommandEvent { generation, kind });
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
async fn terminate_process(child: &mut tokio::process::Child, pid: Option<u32>) {
    if let Some(pid) = pid {
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.wait().await;
}

#[cfg(not(unix))]
async fn terminate_process(child: &mut tokio::process::Child, _pid: Option<u32>) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(not(unix))]
fn interrupt_group(_pid: u32) {}

/// Chooses the shell used to interpret a custom command line.
pub fn shell_program() -> OsString {
    std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/sh"))
}

/// Rejects a command line that would do nothing useful.
pub fn validate_command(command: &str) -> Result<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        anyhow::bail!("enter a command to run");
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        anyhow::bail!("the command must be a single line");
    }
    Ok(trimmed.to_owned())
}

/// Trims the streamed log to the same bound the Repo batch task uses so a
/// chatty command cannot grow the buffer without limit.
pub fn push_log_bounded(logs: &mut Vec<String>, line: String) {
    logs.push(line);
    if logs.len() > MAX_LOG_LINES {
        logs.drain(..logs.len() - MAX_LOG_LINES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_blank_command_lines() {
        assert!(validate_command("   ").is_err());
        assert!(validate_command("\t\n").is_err());
        assert!(validate_command("echo hi\necho bye").is_err());
    }

    #[test]
    fn keeps_a_single_line_command_untouched() {
        assert_eq!(
            validate_command("  git log --oneline | head -3  ").unwrap(),
            "git log --oneline | head -3"
        );
    }

    #[test]
    fn shell_falls_back_when_unset() {
        let program = shell_program();
        assert!(!program.is_empty());
    }

    #[test]
    fn log_buffer_stays_bounded() {
        let mut logs = Vec::new();
        for index in 0..(MAX_LOG_LINES + 25) {
            push_log_bounded(&mut logs, index.to_string());
        }
        assert_eq!(logs.len(), MAX_LOG_LINES);
        assert_eq!(logs.first().unwrap(), "25");
    }
}
