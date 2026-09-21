//! Reads the system clipboard by invoking a platform helper with a fixed
//! argv. Every candidate is a separate program invocation with explicit
//! arguments; no shell string is ever built from user data.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

/// One clipboard helper command line: program plus literal arguments.
struct ClipboardCommand {
    program: &'static str,
    args: &'static [&'static str],
}

#[cfg(target_os = "linux")]
const CANDIDATES: &[ClipboardCommand] = &[
    ClipboardCommand {
        program: "wl-paste",
        args: &["--no-newline"],
    },
    ClipboardCommand {
        program: "xclip",
        args: &["-selection", "clipboard", "-o"],
    },
    ClipboardCommand {
        program: "xsel",
        args: &["--clipboard", "--output"],
    },
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[ClipboardCommand] = &[ClipboardCommand {
    program: "pbpaste",
    args: &[],
}];

#[cfg(target_os = "windows")]
const CANDIDATES: &[ClipboardCommand] = &[ClipboardCommand {
    program: "powershell",
    args: &["-NoProfile", "-Command", "Get-Clipboard -Raw"],
}];

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
const CANDIDATES: &[ClipboardCommand] = &[];

const READ_TIMEOUT: Duration = Duration::from_millis(1500);

/// Reads the current clipboard text through the first helper that succeeds.
///
/// A helper that is missing, times out, exits non-zero, or produces non-UTF-8
/// output is skipped so a partially working desktop (for example Wayland
/// without X11 tools) still finds a working reader. When no candidate works,
/// the error tells the user to fall back to the terminal's own paste.
pub async fn read_text() -> Result<String> {
    if CANDIDATES.is_empty() {
        bail!("no clipboard helper is known for this platform");
    }
    let mut failures = Vec::new();
    for candidate in CANDIDATES {
        match read_with(candidate).await {
            Ok(text) => return Ok(text),
            Err(error) => failures.push(format!("{}: {error}", candidate.program)),
        }
    }
    bail!(
        "clipboard unavailable ({}); use the terminal's own paste (Ctrl-Shift-V)",
        failures.join("; ")
    )
}

async fn read_with(candidate: &ClipboardCommand) -> Result<String> {
    let program = candidate.program;
    let output = tokio::time::timeout(
        READ_TIMEOUT,
        Command::new(program)
            .args(candidate.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .with_context(|| format!("{program} timed out"))?
    .with_context(|| format!("failed to run {program}"))?;
    if !output.status.success() {
        bail!("exited with {}", output.status);
    }
    String::from_utf8(output.stdout).context("clipboard content is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_argv_contains_no_shell_metacharacters() {
        // The helpers are invoked directly with literal arguments, so none of
        // the canned argv entries may smuggle shell syntax.
        for candidate in CANDIDATES {
            assert!(!candidate.program.is_empty());
            for arg in candidate.args {
                assert!(
                    !arg.contains(|character: char| {
                        matches!(character, ';' | '|' | '&' | '$' | '`' | '\n')
                    }),
                    "unexpected shell syntax in {arg:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn read_text_never_panics_and_reports_a_fallback_hint_when_unavailable() {
        // On a headless machine no helper exists, and on a desktop one may
        // legitimately succeed. Either way the call must complete and never
        // panic; the unavailable error must point at the terminal fallback.
        match read_text().await {
            Ok(text) => {
                assert!(
                    !text.contains('\0'),
                    "clipboard text must not contain NUL bytes"
                );
            }
            Err(error) => {
                let message = error.to_string();
                assert!(message.contains("clipboard unavailable"));
                assert!(
                    message.contains("Ctrl-Shift-V") || message.contains("no clipboard helper")
                );
            }
        }
    }
}
