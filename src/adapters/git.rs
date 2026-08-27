use std::collections::{hash_map::DefaultHasher, HashMap};
use std::ffi::{OsStr, OsString};
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::domain::{
    BranchEntry, ChangeCode, ChangeEntry, ChangeHunk, ChangeLine, ChangePreview, Commit, CommitRef,
    CommitRefKind, CommitSpec, GitOperationKind, HeadState, HunkSource, RemoteBranchEntry,
    RemoteEntry, RepositoryAction, RepositoryActionOutcome, RepositorySnapshot, StashEntry,
    TagEntry, UpstreamState, WorktreeSummary,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub head: HeadState,
    pub upstream: Option<UpstreamState>,
    pub worktree: WorktreeSummary,
    pub changes: Vec<ChangeEntry>,
}

pub async fn git_output<I, S>(cwd: &Path, args: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .output()
        .await
        .with_context(|| format!("failed to run git in {}", cwd.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!(
            "git exited with {}{}",
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }
    Ok(output.stdout)
}

async fn git_output_allow<I, S>(cwd: &Path, args: I, allowed_codes: &[i32]) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .output()
        .await
        .with_context(|| format!("failed to run git in {}", cwd.display()))?;
    let accepted = output
        .status
        .code()
        .is_some_and(|code| allowed_codes.contains(&code));
    if !accepted {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!(
            "git exited with {}{}",
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }
    Ok(output.stdout)
}

pub async fn repository_root(path: &Path) -> Result<std::path::PathBuf> {
    let bytes = git_output(path, ["rev-parse", "--show-toplevel"]).await?;
    let root = String::from_utf8(bytes).context("git repository root is not UTF-8")?;
    Ok(std::path::PathBuf::from(root.trim()))
}

pub async fn git_path(root: &Path, name: &str) -> Result<PathBuf> {
    let bytes = git_output(root, ["rev-parse", "--git-path", name]).await?;
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    let path = path_from_bytes(bytes);
    Ok(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

pub(crate) async fn has_head(root: &Path) -> Result<bool> {
    let status = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .await
        .with_context(|| format!("failed to inspect HEAD in {}", root.display()))?;
    Ok(status.success())
}

pub async fn status(path: &Path) -> Result<StatusSnapshot> {
    let bytes = git_output(path, ["status", "--porcelain=v2", "--branch", "-z"]).await?;
    parse_status(&bytes)
}

const MAX_PREVIEW_BYTES: usize = 256 * 1024;

pub async fn changes(path: &Path) -> Result<Vec<ChangeEntry>> {
    let bytes = git_output(path, ["status", "--porcelain=v2", "-z"]).await?;
    parse_changes(&bytes)
}

pub fn parse_changes(bytes: &[u8]) -> Result<Vec<ChangeEntry>> {
    let records: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        if record.is_empty() || record.starts_with(b"# ") {
            index += 1;
            continue;
        }

        let entry = match record.first().copied() {
            Some(b'1') => parse_ordinary_change(record)?,
            Some(b'2') => {
                let original = records
                    .get(index + 1)
                    .copied()
                    .filter(|value| !value.is_empty())
                    .context("rename record is missing its original path")?;
                index += 1;
                let mut entry = parse_renamed_change(record)?;
                entry.original_path = Some(validate_repo_path(original)?);
                entry
            }
            Some(b'u') => parse_unmerged_change(record)?,
            Some(b'?') => ChangeEntry {
                path: validate_repo_path(record.get(2..).context("invalid untracked record")?)?,
                original_path: None,
                index: None,
                worktree: None,
                untracked: true,
                conflicted: false,
            },
            Some(b'!') => {
                index += 1;
                continue;
            }
            Some(value) => bail!(
                "unsupported porcelain v2 record type: {}",
                char::from(value)
            ),
            None => {
                index += 1;
                continue;
            }
        };
        entries.push(entry);
        index += 1;
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn parse_ordinary_change(record: &[u8]) -> Result<ChangeEntry> {
    let fields = split_fields(record, 9, "ordinary")?;
    change_from_fields(&fields, 1, 8, false)
}

fn parse_renamed_change(record: &[u8]) -> Result<ChangeEntry> {
    let fields = split_fields(record, 10, "rename")?;
    change_from_fields(&fields, 1, 9, false)
}

fn parse_unmerged_change(record: &[u8]) -> Result<ChangeEntry> {
    let fields = split_fields(record, 11, "unmerged")?;
    change_from_fields(&fields, 1, 10, true)
}

fn split_fields<'a>(record: &'a [u8], count: usize, label: &str) -> Result<Vec<&'a [u8]>> {
    let fields: Vec<&[u8]> = record.splitn(count, |byte| *byte == b' ').collect();
    if fields.len() != count {
        bail!("invalid porcelain v2 {label} record");
    }
    Ok(fields)
}

fn change_from_fields(
    fields: &[&[u8]],
    xy_index: usize,
    path_index: usize,
    conflicted: bool,
) -> Result<ChangeEntry> {
    let xy = fields
        .get(xy_index)
        .context("change record has no XY field")?;
    if xy.len() != 2 {
        bail!("invalid porcelain v2 XY field");
    }
    Ok(ChangeEntry {
        path: validate_repo_path(fields[path_index])?,
        original_path: None,
        index: ChangeCode::from_byte(xy[0]),
        worktree: ChangeCode::from_byte(xy[1]),
        untracked: false,
        conflicted,
    })
}

fn validate_repo_path(bytes: &[u8]) -> Result<PathBuf> {
    if bytes.is_empty() {
        bail!("empty repository-relative path");
    }
    let path = path_from_bytes(bytes);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("path is outside the repository: {}", path.display());
    }
    Ok(path)
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[derive(Debug)]
struct DiffPart {
    source: HunkSource,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedHunk {
    pub source: HunkSource,
    pub fingerprint: u64,
    pub patch: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedLine {
    pub source: HunkSource,
    pub hunk_fingerprint: u64,
    pub fingerprint: u64,
    pub patch: Vec<u8>,
}

#[derive(Debug)]
struct ParsedHunk {
    source: HunkSource,
    header: String,
    diff_start_line: usize,
    diff_end_line: usize,
    fingerprint: u64,
    patch: Vec<u8>,
    lines: Vec<ParsedLine>,
}

#[derive(Debug)]
struct ParsedLine {
    fingerprint: u64,
    diff_line: usize,
    patch: Vec<u8>,
}

pub async fn preview_change(root: &Path, entry: &ChangeEntry) -> Result<ChangePreview> {
    let (status_bytes, parts) = load_change_parts(root, entry).await?;
    let token = token_for_parts(&status_bytes, &parts);
    let mut text = String::new();
    let mut lines = Vec::new();
    let mut hunks = Vec::new();

    for part in parts {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("== ");
        text.push_str(part.source.label());
        text.push_str(" ==\n");
        let diff_base_line = text.lines().count();
        let parsed = parse_unified_diff(part.source, &part.bytes)?;
        text.push_str(&String::from_utf8_lossy(&part.bytes));
        for hunk in parsed {
            let hunk_fingerprint = hunk.fingerprint;
            hunks.push(ChangeHunk {
                source: hunk.source,
                header: hunk.header,
                display_start: diff_base_line + hunk.diff_start_line,
                display_end: diff_base_line + hunk.diff_end_line,
                fingerprint: hunk_fingerprint,
            });
            lines.extend(hunk.lines.into_iter().map(|line| ChangeLine {
                source: hunk.source,
                hunk_fingerprint,
                fingerprint: line.fingerprint,
                display_line: diff_base_line + line.diff_line,
            }));
        }
    }
    if text.is_empty() {
        text.push_str("No textual diff is available for this change.");
    }
    let (text, truncated, complete_lines) = truncate_preview(text, MAX_PREVIEW_BYTES);
    if truncated {
        hunks.retain(|hunk| hunk.display_end < complete_lines);
    }
    if truncated {
        lines.retain(|line| line.display_line < complete_lines);
    }
    Ok(ChangePreview {
        text,
        token,
        truncated,
        hunks,
        lines,
    })
}

pub async fn change_token(root: &Path, entry: &ChangeEntry) -> Result<u64> {
    let (status_bytes, parts) = load_change_parts(root, entry).await?;
    Ok(token_for_parts(&status_bytes, &parts))
}

pub(crate) async fn resolve_hunks(
    root: &Path,
    entry: &ChangeEntry,
    source: HunkSource,
    fingerprint: u64,
) -> Result<(u64, Vec<ResolvedHunk>)> {
    let (status_bytes, parts) = load_change_parts(root, entry).await?;
    let token = token_for_parts(&status_bytes, &parts);
    let mut matches = Vec::new();
    for part in parts.into_iter().filter(|part| part.source == source) {
        matches.extend(
            parse_unified_diff(part.source, &part.bytes)?
                .into_iter()
                .filter(|hunk| hunk.fingerprint == fingerprint)
                .map(|hunk| ResolvedHunk {
                    source: hunk.source,
                    fingerprint: hunk.fingerprint,
                    patch: hunk.patch,
                }),
        );
    }
    Ok((token, matches))
}

pub(crate) async fn resolve_lines(
    root: &Path,
    entry: &ChangeEntry,
    source: HunkSource,
    hunk_fingerprint: u64,
    fingerprint: u64,
) -> Result<(u64, Vec<ResolvedLine>)> {
    let (status_bytes, parts) = load_change_parts(root, entry).await?;
    let token = token_for_parts(&status_bytes, &parts);
    let mut matches = Vec::new();
    for part in parts.into_iter().filter(|part| part.source == source) {
        for hunk in parse_unified_diff(part.source, &part.bytes)?
            .into_iter()
            .filter(|hunk| hunk.fingerprint == hunk_fingerprint)
        {
            matches.extend(
                hunk.lines
                    .into_iter()
                    .filter(|line| line.fingerprint == fingerprint)
                    .map(|line| ResolvedLine {
                        source,
                        hunk_fingerprint,
                        fingerprint: line.fingerprint,
                        patch: line.patch,
                    }),
            );
        }
    }
    Ok((token, matches))
}

async fn load_change_parts(root: &Path, entry: &ChangeEntry) -> Result<(Vec<u8>, Vec<DiffPart>)> {
    validate_path(entry.path.as_path())?;
    let status_bytes = path_status(root, &entry.path).await?;
    let mut parts = Vec::new();
    if entry.index.is_some() {
        parts.push(DiffPart {
            source: HunkSource::Staged,
            bytes: diff_for_path(root, &entry.path, true).await?,
        });
    }
    if entry.worktree.is_some() || entry.conflicted {
        parts.push(DiffPart {
            source: HunkSource::Worktree,
            bytes: diff_for_path(root, &entry.path, false).await?,
        });
    }
    if entry.untracked {
        parts.push(DiffPart {
            source: HunkSource::Untracked,
            bytes: untracked_diff(root, &entry.path).await?,
        });
    }
    Ok((status_bytes, parts))
}

fn token_for_parts(status_bytes: &[u8], parts: &[DiffPart]) -> u64 {
    let mut hasher = DefaultHasher::new();
    status_bytes.hash(&mut hasher);
    for part in parts {
        part.source.hash(&mut hasher);
        part.bytes.hash(&mut hasher);
    }
    hasher.finish()
}

async fn path_status(root: &Path, path: &Path) -> Result<Vec<u8>> {
    let args = path_args(&["status", "--porcelain=v2", "-z"], path);
    git_output(root, args).await
}

async fn diff_for_path(root: &Path, path: &Path, cached: bool) -> Result<Vec<u8>> {
    let mut prefix = vec!["diff", "--no-ext-diff", "--no-color"];
    if cached {
        prefix.push("--cached");
    }
    git_output(root, path_args(&prefix, path)).await
}

async fn untracked_diff(root: &Path, path: &Path) -> Result<Vec<u8>> {
    let args = path_args(
        &[
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-color",
            "/dev/null",
        ],
        path,
    );
    git_output_allow(root, args, &[0, 1]).await
}

fn parse_unified_diff(source: HunkSource, bytes: &[u8]) -> Result<Vec<ParsedHunk>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let lines = line_ranges(bytes);
    let mut hunks = Vec::new();
    let mut line_index = 0;

    while line_index < lines.len() {
        let (file_start, _) = lines[line_index];
        let first_line = line_bytes(bytes, lines[line_index]);
        if !first_line.starts_with(b"diff --git ") {
            if first_line.iter().all(u8::is_ascii_whitespace) {
                line_index += 1;
                continue;
            }
            if first_line.starts_with(b"diff --cc ") || first_line.starts_with(b"diff --combined ")
            {
                return Ok(Vec::new());
            }
            bail!("unsupported unified diff header");
        }

        let mut file_end_line = line_index + 1;
        while file_end_line < lines.len()
            && !line_bytes(bytes, lines[file_end_line]).starts_with(b"diff --git ")
        {
            file_end_line += 1;
        }

        let first_hunk_line = (line_index + 1..file_end_line).find(|index| {
            let line = line_bytes(bytes, lines[*index]);
            line.starts_with(b"@@ ") || line.starts_with(b"@@@")
        });
        let Some(first_hunk_line) = first_hunk_line else {
            line_index = file_end_line;
            continue;
        };
        if line_bytes(bytes, lines[first_hunk_line]).starts_with(b"@@@") {
            line_index = file_end_line;
            continue;
        }

        let header_end = lines[first_hunk_line].0;
        let mut hunk_line = first_hunk_line;
        while hunk_line < file_end_line {
            let header_line = line_bytes(bytes, lines[hunk_line]);
            validate_hunk_header(header_line)?;
            let next_hunk_line = (hunk_line + 1..file_end_line)
                .find(|index| line_bytes(bytes, lines[*index]).starts_with(b"@@"))
                .unwrap_or(file_end_line);
            if next_hunk_line < file_end_line
                && line_bytes(bytes, lines[next_hunk_line]).starts_with(b"@@@")
            {
                bail!("mixed unified and combined diff hunks are unsupported");
            }
            let hunk_end = if next_hunk_line < lines.len() {
                lines[next_hunk_line].0
            } else {
                bytes.len()
            };
            let mut patch = Vec::with_capacity(
                header_end.saturating_sub(file_start) + hunk_end.saturating_sub(lines[hunk_line].0),
            );
            patch.extend_from_slice(&bytes[file_start..header_end]);
            patch.extend_from_slice(&bytes[lines[hunk_line].0..hunk_end]);

            let mut hasher = DefaultHasher::new();
            source.hash(&mut hasher);
            patch.hash(&mut hasher);
            let fingerprint = hasher.finish();
            let parsed_lines = if source == HunkSource::Untracked {
                Vec::new()
            } else {
                parse_changed_lines(
                    source,
                    fingerprint,
                    bytes,
                    &lines,
                    (file_start, header_end),
                    (hunk_line, next_hunk_line),
                )?
            };
            let header = String::from_utf8_lossy(trim_line_ending(header_line)).into_owned();
            hunks.push(ParsedHunk {
                source,
                header,
                diff_start_line: hunk_line,
                diff_end_line: next_hunk_line - 1,
                fingerprint,
                patch,
                lines: parsed_lines,
            });
            hunk_line = next_hunk_line;
        }
        line_index = file_end_line;
    }
    Ok(hunks)
}

fn parse_changed_lines(
    source: HunkSource,
    hunk_fingerprint: u64,
    bytes: &[u8],
    ranges: &[(usize, usize)],
    file_bounds: (usize, usize),
    hunk_bounds: (usize, usize),
) -> Result<Vec<ParsedLine>> {
    let (file_start, header_end) = file_bounds;
    let (hunk_line, next_hunk_line) = hunk_bounds;
    let (mut old_line, mut new_line) =
        parse_hunk_coordinates(line_bytes(bytes, ranges[hunk_line]))?;
    let mut parsed = Vec::new();
    let mut index = hunk_line + 1;
    while index < next_hunk_line {
        let line = line_bytes(bytes, ranges[index]);
        let marker = (index + 1 < next_hunk_line
            && line_bytes(bytes, ranges[index + 1]).starts_with(b"\\ No newline"))
        .then(|| line_bytes(bytes, ranges[index + 1]));
        let (old_start, old_count, new_start, new_count) = match line.first() {
            Some(b' ') => {
                old_line += 1;
                new_line += 1;
                index += 1;
                continue;
            }
            Some(b'-') => (old_line, 1, new_line.saturating_sub(1), 0),
            Some(b'+') => (old_line.saturating_sub(1), 0, new_line, 1),
            Some(b'\\') => {
                index += 1;
                continue;
            }
            _ => bail!("invalid unified diff body line"),
        };
        let header = format!(
            "@@ -{}{} +{}{} @@\n",
            old_start,
            range_count(old_count),
            new_start,
            range_count(new_count)
        );
        let mut patch = Vec::new();
        patch.extend_from_slice(&bytes[file_start..header_end]);
        patch.extend_from_slice(header.as_bytes());
        patch.extend_from_slice(line);
        if let Some(marker) = marker {
            patch.extend_from_slice(marker);
        }
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        hunk_fingerprint.hash(&mut hasher);
        index.hash(&mut hasher);
        line.hash(&mut hasher);
        parsed.push(ParsedLine {
            fingerprint: hasher.finish(),
            diff_line: index,
            patch,
        });
        match line.first() {
            Some(b'-') => old_line += 1,
            Some(b'+') => new_line += 1,
            _ => unreachable!(),
        }
        index += if marker.is_some() { 2 } else { 1 };
    }
    Ok(parsed)
}

fn range_count(count: usize) -> String {
    if count == 1 {
        String::new()
    } else {
        format!(",{count}")
    }
}

fn parse_hunk_coordinates(line: &[u8]) -> Result<(usize, usize)> {
    let line = trim_line_ending(line);
    let Some(rest) = line.strip_prefix(b"@@ -") else {
        bail!("invalid unified diff hunk header");
    };
    let Some((old_range, rest)) = split_once(rest, b" +") else {
        bail!("invalid unified diff hunk old range");
    };
    let Some((new_range, _)) = split_once(rest, b" @@") else {
        bail!("invalid unified diff hunk new range");
    };
    Ok((range_start(old_range)?, range_start(new_range)?))
}

fn range_start(range: &[u8]) -> Result<usize> {
    let start = range.split(|byte| *byte == b',').next().unwrap_or_default();
    std::str::from_utf8(start)
        .context("invalid unified diff range encoding")?
        .parse()
        .context("invalid unified diff range start")
}

fn line_ranges(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            ranges.push((start, index + 1));
            start = index + 1;
        }
    }
    if start < bytes.len() {
        ranges.push((start, bytes.len()));
    }
    ranges
}

fn line_bytes(bytes: &[u8], range: (usize, usize)) -> &[u8] {
    &bytes[range.0..range.1]
}

fn trim_line_ending(mut line: &[u8]) -> &[u8] {
    if let Some(value) = line.strip_suffix(b"\n") {
        line = value;
    }
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn validate_hunk_header(line: &[u8]) -> Result<()> {
    let line = trim_line_ending(line);
    let Some(rest) = line.strip_prefix(b"@@ -") else {
        bail!("invalid unified diff hunk header");
    };
    let Some((old_range, rest)) = split_once(rest, b" +") else {
        bail!("invalid unified diff hunk old range");
    };
    let Some((new_range, _suffix)) = split_once(rest, b" @@") else {
        bail!("invalid unified diff hunk new range");
    };
    validate_hunk_range(old_range)?;
    validate_hunk_range(new_range)?;
    Ok(())
}

fn split_once<'a>(bytes: &'a [u8], needle: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| (&bytes[..index], &bytes[index + needle.len()..]))
}

fn validate_hunk_range(bytes: &[u8]) -> Result<()> {
    let mut fields = bytes.split(|byte| *byte == b',');
    let start = fields.next().unwrap_or_default();
    let count = fields.next();
    if fields.next().is_some()
        || start.is_empty()
        || !start.iter().all(u8::is_ascii_digit)
        || count.is_some_and(|value| value.is_empty() || !value.iter().all(u8::is_ascii_digit))
    {
        bail!("invalid unified diff hunk range");
    }
    Ok(())
}

pub(crate) async fn apply_hunk(
    root: &Path,
    source: HunkSource,
    kind: crate::domain::OperationKind,
    patch: &[u8],
) -> Result<()> {
    let (cached, reverse) = match (source, kind) {
        (HunkSource::Worktree, crate::domain::OperationKind::Stage)
        | (HunkSource::Untracked, crate::domain::OperationKind::Stage) => (true, false),
        (HunkSource::Staged, crate::domain::OperationKind::Unstage) => (true, true),
        (HunkSource::Worktree, crate::domain::OperationKind::RestoreWorktree) => (false, true),
        _ => bail!("hunk source is not eligible for {}", kind.label()),
    };
    git_apply(root, patch, cached, reverse, true).await?;
    git_apply(root, patch, cached, reverse, false).await
}

async fn git_apply(
    root: &Path,
    patch: &[u8],
    cached: bool,
    reverse: bool,
    check: bool,
) -> Result<()> {
    let mut command = Command::new("git");
    command.arg("apply").arg("--recount").arg("--unidiff-zero");
    if cached {
        command.arg("--cached");
    }
    if reverse {
        command.arg("--reverse");
    }
    if check {
        command.arg("--check");
    }
    let mut child = command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
        .with_context(|| format!("failed to run git apply in {}", root.display()))?;
    child
        .stdin
        .take()
        .context("git apply stdin is unavailable")?
        .write_all(patch)
        .await
        .context("failed to send hunk patch to git apply")?;
    let output = child
        .wait_with_output()
        .await
        .context("failed to wait for git apply")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        bail!(
            "git apply{} failed with {}{}",
            if check { " --check" } else { "" },
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        );
    }
    Ok(())
}

pub async fn stage_path(root: &Path, path: &Path) -> Result<()> {
    validate_path(path)?;
    git_output(root, path_args(&["add"], path)).await?;
    Ok(())
}

pub async fn unstage_path(root: &Path, path: &Path) -> Result<()> {
    validate_path(path)?;
    if has_head(root).await? {
        git_output(root, path_args(&["restore", "--staged"], path)).await?;
    } else {
        git_output(root, path_args(&["rm", "--cached", "--quiet"], path)).await?;
    }
    Ok(())
}

pub async fn restore_worktree_path(root: &Path, path: &Path) -> Result<()> {
    validate_path(path)?;
    git_output(root, path_args(&["restore", "--worktree"], path)).await?;
    Ok(())
}

pub async fn stash_paths(root: &Path, entries: &[ChangeEntry], message: &str) -> Result<()> {
    if entries.is_empty() {
        bail!("no files were selected");
    }
    if !has_head(root).await? {
        bail!("stash requires an initial commit");
    }
    let paths = entries
        .iter()
        .map(|entry| entry.path.as_path())
        .collect::<Vec<_>>();
    let mut args = vec![
        OsString::from("stash"),
        OsString::from("push"),
        OsString::from("--include-untracked"),
        OsString::from("--message"),
        OsString::from(message),
    ];
    append_paths(&mut args, &paths)?;
    git_output(root, args).await?;

    let original_paths = entries
        .iter()
        .filter_map(|entry| entry.original_path.as_deref())
        .collect::<Vec<_>>();
    if !original_paths.is_empty() {
        let mut restore = vec![
            OsString::from("restore"),
            OsString::from("--source=HEAD"),
            OsString::from("--staged"),
            OsString::from("--worktree"),
        ];
        append_paths(&mut restore, &original_paths)?;
        git_output(root, restore)
            .await
            .context("stash was created, but cleaning the original rename paths failed")?;
    }
    Ok(())
}

pub async fn discard_paths(root: &Path, entries: &[ChangeEntry]) -> Result<()> {
    if entries.is_empty() {
        bail!("no files were selected");
    }
    let head = has_head(root).await?;
    let mut restore_paths = Vec::new();
    let mut remove_from_index = Vec::new();
    let mut clean_paths = Vec::new();
    for entry in entries {
        validate_path(&entry.path)?;
        if let Some(original) = entry.original_path.as_deref() {
            validate_path(original)?;
            if head {
                restore_paths.push(original);
            }
            if entry.index.is_some() {
                remove_from_index.push(entry.path.as_path());
            }
            clean_paths.push(entry.path.as_path());
        } else if head && path_exists_in_head(root, &entry.path).await? {
            restore_paths.push(entry.path.as_path());
        } else {
            if entry.index.is_some() {
                remove_from_index.push(entry.path.as_path());
            }
            clean_paths.push(entry.path.as_path());
        }
    }

    if !restore_paths.is_empty() {
        let mut args = vec![
            OsString::from("restore"),
            OsString::from("--source=HEAD"),
            OsString::from("--staged"),
            OsString::from("--worktree"),
        ];
        append_paths(&mut args, &restore_paths)?;
        git_output(root, args).await?;
    }
    if !remove_from_index.is_empty() {
        let mut args = if head {
            vec![OsString::from("restore"), OsString::from("--staged")]
        } else {
            vec![
                OsString::from("rm"),
                OsString::from("--cached"),
                OsString::from("--quiet"),
            ]
        };
        append_paths(&mut args, &remove_from_index)?;
        git_output(root, args).await?;
    }
    if !clean_paths.is_empty() {
        let mut args = vec![
            OsString::from("clean"),
            OsString::from("-f"),
            OsString::from("-d"),
        ];
        append_paths(&mut args, &clean_paths)?;
        git_output(root, args).await?;
    }
    Ok(())
}

async fn path_exists_in_head(root: &Path, path: &Path) -> Result<bool> {
    validate_path(path)?;
    let mut spec = OsString::from("HEAD:");
    spec.push(path.as_os_str());
    let status = Command::new("git")
        .args([OsString::from("cat-file"), OsString::from("-e"), spec])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .await
        .with_context(|| format!("failed to inspect HEAD path in {}", root.display()))?;
    Ok(status.success())
}

pub async fn commit(root: &Path, spec: &CommitSpec) -> Result<String> {
    if spec.message.trim().is_empty() {
        bail!("commit message cannot be empty");
    }
    let mut args = vec![OsString::from("commit")];
    if spec.amend {
        args.push(OsString::from("--amend"));
    }
    if spec.signoff {
        args.push(OsString::from("--signoff"));
    }
    if spec.signing {
        args.push(OsString::from("--gpg-sign"));
    }
    args.push(OsString::from("--message"));
    args.push(OsString::from(&spec.message));

    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .with_context(|| format!("failed to run git commit in {}", root.display()))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let details = [stdout, stderr]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "git commit exited with {}{}",
            output.status,
            if details.is_empty() {
                String::new()
            } else {
                format!(": {details}")
            }
        );
    }
    let oid = git_output(root, ["rev-parse", "HEAD"]).await?;
    let oid = String::from_utf8(oid).context("commit OID is not UTF-8")?;
    Ok(oid.trim().to_owned())
}

const BRANCH_FORMAT: &str =
    "%(refname:short)%00%(objectname)%00%(upstream:short)%00%(upstream:track)%00%(HEAD)%00";
const TAG_FORMAT: &str = "%(refname:short)%00%(objectname)%00";
const STASH_DETAIL_FORMAT: &str = "%gd%x00%H%x00%gs%x00";

pub async fn repository_snapshot(root: &Path) -> Result<RepositorySnapshot> {
    let branch_bytes = git_output(
        root,
        [
            "for-each-ref",
            "refs/heads",
            format!("--format={BRANCH_FORMAT}").as_str(),
        ],
    )
    .await?;
    let tag_bytes = git_output(
        root,
        [
            "for-each-ref",
            "refs/tags",
            format!("--format={TAG_FORMAT}").as_str(),
        ],
    )
    .await?;
    let stash_bytes = git_output(
        root,
        [
            "stash",
            "list",
            format!("--format={STASH_DETAIL_FORMAT}").as_str(),
        ],
    )
    .await?;
    let remote_bytes = git_output(
        root,
        [
            "for-each-ref",
            "refs/remotes",
            "--format=%(refname:short)%00%(objectname)%00",
        ],
    )
    .await?;
    let remote_branch_entries = parse_remote_branch_entries(&remote_bytes)?;
    let conflicts = git_output(root, ["diff", "--name-only", "--diff-filter=U", "-z"]).await?;
    let worktree_bytes = git_output(root, ["status", "--porcelain=v2", "-z"]).await?;
    let mut worktree_hasher = DefaultHasher::new();
    worktree_bytes.hash(&mut worktree_hasher);
    let operation = detect_operation(root).await?;
    let mut snapshot = RepositorySnapshot {
        operation,
        conflicts: parse_path_list(&conflicts)?,
        stashes: parse_stash_entries(&stash_bytes)?,
        branches: parse_branch_entries(&branch_bytes)?,
        tags: parse_tag_entries(&tag_bytes)?,
        remotes: load_remotes(root, &remote_bytes).await?,
        remote_branches: remote_branch_entries,
        worktree_token: worktree_hasher.finish(),
        token: 0,
    };
    snapshot.token = repository_snapshot_token(&snapshot);
    Ok(snapshot)
}

async fn detect_operation(root: &Path) -> Result<Option<GitOperationKind>> {
    let checks = [
        ("MERGE_HEAD", GitOperationKind::Merge),
        ("rebase-merge", GitOperationKind::Rebase),
        ("rebase-apply", GitOperationKind::Rebase),
        ("CHERRY_PICK_HEAD", GitOperationKind::CherryPick),
        ("REVERT_HEAD", GitOperationKind::Revert),
    ];
    for (name, kind) in checks {
        if git_path(root, name).await?.exists() {
            return Ok(Some(kind));
        }
    }
    Ok(None)
}

fn parse_path_list(bytes: &[u8]) -> Result<Vec<PathBuf>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(validate_repo_path)
        .collect()
}

fn parse_stash_entries(bytes: &[u8]) -> Result<Vec<StashEntry>> {
    let fields = split_nul_records(bytes);
    fields
        .chunks_exact(3)
        .map(|record| {
            Ok(StashEntry {
                selector: text(record[0]).trim().to_owned(),
                oid: text(record[1]).trim().to_owned(),
                subject: text(record[2]).trim().to_owned(),
            })
        })
        .collect()
}

fn parse_branch_entries(bytes: &[u8]) -> Result<Vec<BranchEntry>> {
    let fields = split_nul_records(bytes);
    fields
        .chunks_exact(5)
        .map(|record| {
            let track = text(record[3]);
            let (ahead, behind) = parse_track_counts(&track)?;
            let upstream = text(record[2]).trim().to_owned();
            Ok(BranchEntry {
                name: text(record[0]).trim().to_owned(),
                oid: text(record[1]).trim().to_owned(),
                upstream: (!upstream.is_empty()).then_some(upstream),
                ahead,
                behind,
                current: text(record[4]).trim() == "*",
            })
        })
        .collect()
}

fn parse_track_counts(track: &str) -> Result<(usize, usize)> {
    let mut ahead = 0;
    let mut behind = 0;
    for value in track.trim_matches(['[', ']']).split(',') {
        let value = value.trim();
        if let Some(count) = value.strip_prefix("ahead ") {
            ahead = count.parse().context("invalid branch ahead count")?;
        } else if let Some(count) = value.strip_prefix("behind ") {
            behind = count.parse().context("invalid branch behind count")?;
        }
    }
    Ok((ahead, behind))
}

fn parse_tag_entries(bytes: &[u8]) -> Result<Vec<TagEntry>> {
    let fields = split_nul_records(bytes);
    Ok(fields
        .chunks_exact(2)
        .map(|record| TagEntry {
            name: text(record[0]).trim().to_owned(),
            target: text(record[1]).trim().to_owned(),
        })
        .collect())
}

fn split_nul_records(bytes: &[u8]) -> Vec<&[u8]> {
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .map(trim_record_separator)
        .collect::<Vec<_>>();
    while fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    fields
}

fn parse_remote_branch_entries(bytes: &[u8]) -> Result<Vec<RemoteBranchEntry>> {
    let fields = split_nul_records(bytes);
    Ok(fields
        .chunks_exact(2)
        .filter_map(|record| {
            let name = text(record[0]).trim().to_owned();
            let oid = text(record[1]).trim().to_owned();
            (!name.ends_with("/HEAD")).then_some(RemoteBranchEntry { name, oid })
        })
        .collect())
}

fn repository_snapshot_token(snapshot: &RepositorySnapshot) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{:?}", snapshot.operation).hash(&mut hasher);
    snapshot.worktree_token.hash(&mut hasher);
    snapshot.conflicts.hash(&mut hasher);
    for stash in &snapshot.stashes {
        stash.selector.hash(&mut hasher);
        stash.oid.hash(&mut hasher);
    }
    for branch in &snapshot.branches {
        branch.name.hash(&mut hasher);
        branch.oid.hash(&mut hasher);
        branch.upstream.hash(&mut hasher);
        branch.current.hash(&mut hasher);
    }
    for tag in &snapshot.tags {
        tag.name.hash(&mut hasher);
        tag.target.hash(&mut hasher);
    }
    for remote in &snapshot.remotes {
        remote.name.hash(&mut hasher);
        remote.fetch_url.hash(&mut hasher);
        remote.push_url.hash(&mut hasher);
    }
    for branch in &snapshot.remote_branches {
        branch.name.hash(&mut hasher);
        branch.oid.hash(&mut hasher);
    }
    hasher.finish()
}

async fn load_remotes(root: &Path, _bytes: &[u8]) -> Result<Vec<RemoteEntry>> {
    let mut names = Vec::new();
    let configured = git_output(root, ["remote"]).await?;
    names.extend(
        String::from_utf8_lossy(&configured)
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned),
    );
    names.sort();
    names.dedup();
    let mut remotes = Vec::with_capacity(names.len());
    for name in names {
        let fetch_url =
            String::from_utf8_lossy(&git_output(root, ["remote", "get-url", name.as_str()]).await?)
                .trim()
                .to_owned();
        let push_url = String::from_utf8_lossy(
            &git_output(root, ["remote", "get-url", "--push", name.as_str()]).await?,
        )
        .trim()
        .to_owned();
        remotes.push(RemoteEntry {
            name,
            fetch_url,
            push_url,
        });
    }
    Ok(remotes)
}

pub async fn execute_repository_action(
    root: &Path,
    action: &RepositoryAction,
    check_only: bool,
) -> Result<RepositoryActionOutcome> {
    if check_only {
        validate_repository_action(root, action, None).await?;
        return Ok(RepositoryActionOutcome {
            message: format!("{} is ready", action.label()),
            detail: None,
        });
    }
    let (args, detail_output) = action_args(action)?;
    let output = git_output(root, args).await?;
    Ok(RepositoryActionOutcome {
        message: format!("{} completed", action.label()),
        detail: detail_output.then(|| String::from_utf8_lossy(&output).into_owned()),
    })
}

pub(crate) async fn validate_repository_action(
    root: &Path,
    action: &RepositoryAction,
    expected_token: Option<u64>,
) -> Result<()> {
    if let Some(expected_token) = expected_token {
        let current = repository_snapshot(root).await?;
        if current.token != expected_token {
            bail!("precondition failed: repository state changed after confirmation; refresh and retry");
        }
    }
    validate_repository_action_values(action)?;
    match action {
        RepositoryAction::StashShow { selector }
        | RepositoryAction::StashApply { selector, .. }
        | RepositoryAction::StashPop { selector, .. }
        | RepositoryAction::StashDrop { selector }
        | RepositoryAction::StashBranch { selector, .. } => {
            git_output(root, ["rev-parse", "--verify", selector.as_str()]).await?;
        }
        RepositoryAction::ConflictTakeOurs { path }
        | RepositoryAction::ConflictTakeTheirs { path }
        | RepositoryAction::ConflictMarkResolved { path } => {
            validate_path(path)?;
            if !repository_snapshot(root).await?.conflicts.contains(path) {
                bail!(
                    "precondition failed: {} is no longer conflicted",
                    path.display()
                );
            }
        }
        RepositoryAction::Continue { operation }
        | RepositoryAction::Skip { operation }
        | RepositoryAction::Abort { operation } => {
            if detect_operation(root).await? != Some(*operation) {
                bail!(
                    "precondition failed: {} is no longer active",
                    operation.label()
                );
            }
        }
        RepositoryAction::BranchDelete { name, .. } => {
            git_output(
                root,
                [
                    "show-ref",
                    "--verify",
                    format!("refs/heads/{name}").as_str(),
                ],
            )
            .await?;
        }
        RepositoryAction::TagDelete { name } => {
            git_output(
                root,
                ["show-ref", "--verify", format!("refs/tags/{name}").as_str()],
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_repository_action_values(action: &RepositoryAction) -> Result<()> {
    let mut values = Vec::new();
    match action {
        RepositoryAction::StashShow { selector }
        | RepositoryAction::StashApply { selector, .. }
        | RepositoryAction::StashPop { selector, .. }
        | RepositoryAction::StashDrop { selector } => values.push(("stash", selector.as_str())),
        RepositoryAction::StashBranch { name, selector } => {
            values.push(("branch", name));
            values.push(("stash", selector));
        }
        RepositoryAction::BranchCreate { name, start } => {
            values.push(("branch", name));
            if let Some(start) = start {
                values.push(("start ref", start));
            }
        }
        RepositoryAction::BranchSwitch { name } | RepositoryAction::BranchDelete { name, .. } => {
            values.push(("branch", name))
        }
        RepositoryAction::BranchRename { old, new } => {
            values.push(("branch", old));
            values.push(("new branch", new));
        }
        RepositoryAction::TagCreate { name, target } => {
            values.push(("tag", name));
            values.push(("target", target));
        }
        RepositoryAction::TagDelete { name } => values.push(("tag", name)),
        RepositoryAction::Merge { reference } | RepositoryAction::Rebase { reference } => {
            values.push(("reference", reference))
        }
        RepositoryAction::CherryPick { oid } | RepositoryAction::Revert { oid } => {
            values.push(("commit", oid));
        }
        RepositoryAction::RemoteAdd { name, url }
        | RepositoryAction::RemoteSetUrl { name, url } => {
            values.push(("remote", name));
            values.push(("URL", url));
        }
        RepositoryAction::RemoteRemove { name } => values.push(("remote", name)),
        RepositoryAction::Fetch { remote, .. } | RepositoryAction::RemotePrune { remote } => {
            values.push(("remote", remote))
        }
        RepositoryAction::Pull { remote, branch, .. }
        | RepositoryAction::Push { remote, branch, .. } => {
            values.push(("remote", remote));
            values.push(("branch", branch));
        }
        RepositoryAction::SetUpstream { branch, upstream } => {
            values.push(("branch", branch));
            values.push(("upstream", upstream));
        }
        RepositoryAction::StashPush {
            include_untracked,
            keep_index,
            staged_only,
            ..
        } => {
            if *staged_only && *include_untracked {
                bail!("staged-only stash cannot include untracked files");
            }
            if *staged_only && *keep_index {
                bail!("staged-only stash cannot keep the index");
            }
        }
        RepositoryAction::StashClear
        | RepositoryAction::ConflictTakeOurs { .. }
        | RepositoryAction::ConflictTakeTheirs { .. }
        | RepositoryAction::ConflictMarkResolved { .. }
        | RepositoryAction::Continue { .. }
        | RepositoryAction::Skip { .. }
        | RepositoryAction::Abort { .. } => {}
    }
    for (label, value) in values {
        if value.is_empty() || value.starts_with('-') || value.contains('\0') {
            bail!("invalid {label}: {value:?}");
        }
    }
    Ok(())
}

fn action_args(action: &RepositoryAction) -> Result<(Vec<OsString>, bool)> {
    let mut args = Vec::new();
    let mut detail = false;
    match action {
        RepositoryAction::StashShow { selector } => {
            args.extend(["stash", "show", "--patch", "--no-color"].map(OsString::from));
            args.push(selector.into());
            detail = true;
        }
        RepositoryAction::StashPush {
            message,
            include_untracked,
            keep_index,
            staged_only,
        } => {
            args.extend(["stash", "push"].map(OsString::from));
            if *include_untracked {
                args.push("--include-untracked".into());
            }
            if *keep_index {
                args.push("--keep-index".into());
            }
            if *staged_only {
                args.push("--staged".into());
            }
            if !message.is_empty() {
                args.extend(["--message".into(), message.into()]);
            }
        }
        RepositoryAction::StashApply {
            selector,
            restore_index,
        } => {
            args.extend(["stash", "apply"].map(OsString::from));
            if *restore_index {
                args.push("--index".into());
            }
            args.push(selector.into());
        }
        RepositoryAction::StashPop {
            selector,
            restore_index,
        } => {
            args.extend(["stash", "pop"].map(OsString::from));
            if *restore_index {
                args.push("--index".into());
            }
            args.push(selector.into());
        }
        RepositoryAction::StashDrop { selector } => {
            args.extend(["stash", "drop"].map(OsString::from));
            args.push(selector.into());
        }
        RepositoryAction::StashBranch { name, selector } => {
            args.extend([
                "stash".into(),
                "branch".into(),
                name.into(),
                selector.into(),
            ]);
        }
        RepositoryAction::StashClear => {
            args.extend(["stash", "clear"].map(OsString::from));
        }
        RepositoryAction::ConflictTakeOurs { path } => {
            args.extend(["checkout", "--ours", "--"].map(OsString::from));
            args.push(path.into());
        }
        RepositoryAction::ConflictTakeTheirs { path } => {
            args.extend(["checkout", "--theirs", "--"].map(OsString::from));
            args.push(path.into());
        }
        RepositoryAction::ConflictMarkResolved { path } => {
            args.extend(["add", "--"].map(OsString::from));
            args.push(path.into());
        }
        RepositoryAction::Continue { operation } => {
            operation_args(&mut args, *operation, "--continue")?
        }
        RepositoryAction::Skip { operation } => operation_args(&mut args, *operation, "--skip")?,
        RepositoryAction::Abort { operation } => operation_args(&mut args, *operation, "--abort")?,
        RepositoryAction::BranchCreate { name, start } => {
            args.extend(["branch".into(), "--".into(), name.into()]);
            if let Some(start) = start {
                args.push(start.into());
            }
        }
        RepositoryAction::BranchSwitch { name } => {
            args.extend(["switch".into(), "--".into(), name.into()]);
        }
        RepositoryAction::BranchRename { old, new } => {
            args.extend([
                "branch".into(),
                "--move".into(),
                "--".into(),
                old.into(),
                new.into(),
            ]);
        }
        RepositoryAction::BranchDelete { name, force } => {
            args.extend([
                "branch".into(),
                if *force { "-D".into() } else { "-d".into() },
                "--".into(),
                name.into(),
            ]);
        }
        RepositoryAction::TagCreate { name, target } => {
            args.extend(["tag".into(), "--".into(), name.into(), target.into()]);
        }
        RepositoryAction::TagDelete { name } => {
            args.extend(["tag".into(), "--delete".into(), "--".into(), name.into()]);
        }
        RepositoryAction::Merge { reference } => {
            args.extend([
                "merge".into(),
                "--no-edit".into(),
                "--".into(),
                reference.into(),
            ]);
        }
        RepositoryAction::Rebase { reference } => {
            args.extend(["rebase".into(), "--".into(), reference.into()]);
        }
        RepositoryAction::CherryPick { oid } => {
            args.extend(["cherry-pick".into(), "--".into(), oid.into()]);
        }
        RepositoryAction::Revert { oid } => {
            args.extend(["revert".into(), "--no-edit".into(), "--".into(), oid.into()]);
        }
        RepositoryAction::RemoteAdd { name, url } => {
            args.extend(["remote".into(), "add".into(), name.into(), url.into()]);
        }
        RepositoryAction::RemoteSetUrl { name, url } => {
            args.extend(["remote".into(), "set-url".into(), name.into(), url.into()]);
        }
        RepositoryAction::RemoteRemove { name } => {
            args.extend(["remote".into(), "remove".into(), name.into()]);
        }
        RepositoryAction::Fetch { remote, prune } => {
            args.extend(["fetch".into(), remote.into()]);
            if *prune {
                args.push("--prune".into());
            }
        }
        RepositoryAction::Pull {
            remote,
            branch,
            rebase,
        } => {
            args.extend(["pull".into()]);
            if *rebase {
                args.push("--rebase".into());
            }
            args.extend([remote.into(), branch.into()]);
        }
        RepositoryAction::Push {
            remote,
            branch,
            set_upstream,
            force_with_lease,
        } => {
            args.extend(["push".into()]);
            if *set_upstream {
                args.push("--set-upstream".into());
            }
            if *force_with_lease {
                args.push("--force-with-lease".into());
            }
            args.extend([remote.into(), format!("{branch}:{branch}").into()]);
        }
        RepositoryAction::SetUpstream { branch, upstream } => {
            args.extend([
                "branch".into(),
                "--set-upstream-to".into(),
                upstream.into(),
                branch.into(),
            ]);
        }
        RepositoryAction::RemotePrune { remote } => {
            args.extend(["remote".into(), "prune".into(), remote.into()]);
        }
    }
    Ok((args, detail))
}

fn operation_args(args: &mut Vec<OsString>, operation: GitOperationKind, flag: &str) -> Result<()> {
    let command = match operation {
        GitOperationKind::Merge => {
            if flag == "--skip" {
                bail!("merge does not support skip");
            }
            "merge"
        }
        GitOperationKind::Rebase => "rebase",
        GitOperationKind::CherryPick => "cherry-pick",
        GitOperationKind::Revert => "revert",
    };
    args.extend([command.into(), flag.into()]);
    Ok(())
}
fn path_args(prefix: &[&str], path: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = prefix.iter().map(OsString::from).collect();
    args.push(OsString::from("--"));
    args.push(path.as_os_str().to_owned());
    args
}

fn append_paths(args: &mut Vec<OsString>, paths: &[&Path]) -> Result<()> {
    args.push(OsString::from("--"));
    for path in paths {
        validate_path(path)?;
        args.push(path.as_os_str().to_owned());
    }
    Ok(())
}

fn validate_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("invalid repository-relative path: {}", path.display());
    }
    Ok(())
}

fn truncate_preview(mut text: String, max_bytes: usize) -> (String, bool, usize) {
    if text.len() <= max_bytes {
        let complete_lines = text.bytes().filter(|byte| *byte == b'\n').count();
        return (text, false, complete_lines);
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let complete_lines = text[..boundary]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    text.truncate(boundary);
    text.push_str("\n\n[preview truncated]");
    (text, true, complete_lines)
}

pub fn parse_status(bytes: &[u8]) -> Result<StatusSnapshot> {
    let mut branch_oid: Option<String> = None;
    let mut branch_name: Option<String> = None;
    let mut upstream_name: Option<String> = None;
    let mut ahead = 0;
    let mut behind = 0;
    let mut worktree = WorktreeSummary::default();

    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        if let Some(value) = record.strip_prefix(b"# branch.oid ") {
            branch_oid = Some(String::from_utf8_lossy(value).into_owned());
        } else if let Some(value) = record.strip_prefix(b"# branch.head ") {
            branch_name = Some(String::from_utf8_lossy(value).into_owned());
        } else if let Some(value) = record.strip_prefix(b"# branch.upstream ") {
            upstream_name = Some(String::from_utf8_lossy(value).into_owned());
        } else if let Some(value) = record.strip_prefix(b"# branch.ab ") {
            for field in value.split(|byte| *byte == b' ') {
                if let Some(value) = field.strip_prefix(b"+") {
                    ahead = parse_count(value, "ahead")?;
                } else if let Some(value) = field.strip_prefix(b"-") {
                    behind = parse_count(value, "behind")?;
                }
            }
        } else {
            match record.first().copied() {
                Some(b'1' | b'2') => count_xy(record, &mut worktree),
                Some(b'u') => worktree.conflicted += 1,
                Some(b'?') => worktree.untracked += 1,
                _ => {}
            }
        }
    }

    let oid = branch_oid.unwrap_or_default();
    let name = branch_name.unwrap_or_default();
    let head = if oid == "(initial)" || oid == "(unknown)" {
        HeadState::Unborn(if name.is_empty() {
            "unborn".to_owned()
        } else {
            name
        })
    } else if name == "(detached)" {
        HeadState::Detached(short_oid(&oid))
    } else if !name.is_empty() {
        HeadState::Branch(name)
    } else if !oid.is_empty() {
        HeadState::Detached(short_oid(&oid))
    } else {
        HeadState::Unknown
    };

    let upstream = upstream_name.map(|name| UpstreamState {
        name,
        ahead,
        behind,
    });

    let changes = parse_changes(bytes)?;

    Ok(StatusSnapshot {
        head,
        upstream,
        worktree,
        changes,
    })
}

fn parse_count(bytes: &[u8], label: &str) -> Result<usize> {
    let value = std::str::from_utf8(bytes).with_context(|| format!("invalid {label} count"))?;
    value
        .parse()
        .with_context(|| format!("invalid {label} count: {value}"))
}

fn count_xy(record: &[u8], summary: &mut WorktreeSummary) {
    if record.len() < 4 {
        return;
    }
    let x = record[2];
    let y = record[3];
    if x != b'.' && x != b' ' {
        summary.staged += 1;
    }
    if y != b'.' && y != b' ' {
        summary.unstaged += 1;
    }
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(8).collect()
}

const LOG_FORMAT: &str = "format:%H%x00%P%x00%an%x00%at%x00%s%x00%B%x00";
const REF_FORMAT: &str = "%(objectname)%00%(*objectname)%00%(refname)%00";
const STASH_FORMAT: &str = "%H%x00%gd%x00";

pub async fn log_all(path: &Path) -> Result<Vec<Commit>> {
    let ref_bytes = git_output(
        path,
        ["for-each-ref", format!("--format={REF_FORMAT}").as_str()],
    )
    .await?;
    let stash_bytes = git_output(
        path,
        ["stash", "list", format!("--format={STASH_FORMAT}").as_str()],
    )
    .await?;
    let head_bytes =
        git_output_allow(path, ["rev-parse", "--verify", "--quiet", "HEAD"], &[0, 1]).await?;
    let mut refs = parse_ref_records(&ref_bytes)?;
    let stash_refs = parse_stash_records(&stash_bytes)?;
    for (oid, reference) in &stash_refs {
        refs.entry(oid.clone()).or_default().push(reference.clone());
    }
    if let Some(oid) = parse_optional_oid(&head_bytes)? {
        refs.entry(oid).or_default().push(CommitRef {
            name: "HEAD".to_owned(),
            kind: CommitRefKind::Head,
        });
    }

    let pretty = format!("--pretty={LOG_FORMAT}");
    let mut args = vec![
        OsString::from("log"),
        OsString::from("--topo-order"),
        OsString::from("--date-order"),
        OsString::from("--all"),
        OsString::from(pretty),
    ];
    args.extend(stash_refs.into_iter().map(|(oid, _)| OsString::from(oid)));
    let bytes = git_output(path, args).await?;
    let mut commits = parse_log(&bytes)?;
    for commit in &mut commits {
        commit.refs = refs.remove(&commit.oid).unwrap_or_default();
        commit.refs.sort_by(|left, right| {
            ref_kind_order(left.kind)
                .cmp(&ref_kind_order(right.kind))
                .then_with(|| left.name.cmp(&right.name))
        });
        commit.refs.dedup();
    }
    Ok(commits)
}

pub fn parse_log(bytes: &[u8]) -> Result<Vec<Commit>> {
    if bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(Vec::new());
    }

    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut commits = Vec::new();
    let mut offset = 0;
    while offset + 6 <= fields.len() {
        let oid = trim_record_separator(fields[offset]);
        if oid.is_empty() {
            offset += 1;
            continue;
        }
        let timestamp = text(fields[offset + 3])
            .trim()
            .parse::<i64>()
            .context("invalid commit timestamp")?;
        commits.push(Commit {
            oid: text(oid).into_owned(),
            parents: text(fields[offset + 1])
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            refs: Vec::new(),
            author: text(fields[offset + 2]).into_owned(),
            timestamp,
            subject: text(fields[offset + 4]).trim_end().to_owned(),
            body: text(fields[offset + 5]).trim_end().to_owned(),
        });
        offset += 6;
    }

    if fields[offset..]
        .iter()
        .any(|field| !field.iter().all(|byte| byte.is_ascii_whitespace()))
    {
        bail!("incomplete git log record");
    }
    Ok(commits)
}

fn trim_record_separator(mut bytes: &[u8]) -> &[u8] {
    while matches!(bytes.first(), Some(b'\n' | b'\r')) {
        bytes = &bytes[1..];
    }
    bytes
}

fn text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

fn parse_ref_records(bytes: &[u8]) -> Result<HashMap<String, Vec<CommitRef>>> {
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut refs = HashMap::<String, Vec<CommitRef>>::new();
    let mut offset = 0;
    while offset + 3 <= fields.len() {
        let object = trim_record_separator(fields[offset]);
        if object.is_empty() {
            offset += 1;
            continue;
        }
        let peeled = fields[offset + 1];
        let name = text(fields[offset + 2]).trim().to_owned();
        let (name, kind) = if let Some(name) = name.strip_prefix("refs/heads/") {
            (name.to_owned(), CommitRefKind::LocalBranch)
        } else if let Some(name) = name.strip_prefix("refs/remotes/") {
            (name.to_owned(), CommitRefKind::RemoteBranch)
        } else if let Some(name) = name.strip_prefix("refs/tags/") {
            (name.to_owned(), CommitRefKind::Tag)
        } else {
            offset += 3;
            continue;
        };
        let oid = if peeled.is_empty() { object } else { peeled };
        refs.entry(text(oid).into_owned())
            .or_default()
            .push(CommitRef { name, kind });
        offset += 3;
    }
    ensure_empty_tail(&fields[offset..], "ref")?;
    Ok(refs)
}

fn parse_stash_records(bytes: &[u8]) -> Result<Vec<(String, CommitRef)>> {
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut refs = Vec::new();
    let mut offset = 0;
    while offset + 2 <= fields.len() {
        let oid = trim_record_separator(fields[offset]);
        if oid.is_empty() {
            offset += 1;
            continue;
        }
        refs.push((
            text(oid).into_owned(),
            CommitRef {
                name: text(fields[offset + 1]).trim().to_owned(),
                kind: CommitRefKind::Stash,
            },
        ));
        offset += 2;
    }
    ensure_empty_tail(&fields[offset..], "stash")?;
    Ok(refs)
}

fn parse_optional_oid(bytes: &[u8]) -> Result<Option<String>> {
    let value = text(bytes).trim().to_owned();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() < 4 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid HEAD object id");
    }
    Ok(Some(value))
}

fn ensure_empty_tail(fields: &[&[u8]], label: &str) -> Result<()> {
    if fields
        .iter()
        .any(|field| !field.iter().all(u8::is_ascii_whitespace))
    {
        bail!("incomplete git {label} record");
    }
    Ok(())
}

fn ref_kind_order(kind: CommitRefKind) -> u8 {
    match kind {
        CommitRefKind::Head => 0,
        CommitRefKind::LocalBranch => 1,
        CommitRefKind::RemoteBranch => 2,
        CommitRefKind::Tag => 3,
        CommitRefKind::Stash => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use tempfile::tempdir;

    fn run_git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn commit_file(root: &Path, path: &str, content: &str, message: &str) {
        fs::write(root.join(path), content).unwrap();
        run_git(root, &["add", path]);
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
                message,
            ],
        );
    }

    #[tokio::test]
    async fn commits_signs_off_amends_and_preserves_hook_failure_output() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        run_git(temp.path(), &["config", "user.email", "test@example.com"]);
        commit_file(temp.path(), "tracked.txt", "base\n", "base");

        fs::write(temp.path().join("tracked.txt"), "base\nsecond\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        let oid = commit(
            temp.path(),
            &CommitSpec {
                message: "second\n\nbody".into(),
                amend: false,
                signoff: true,
                signing: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(oid, run_git(temp.path(), &["rev-parse", "HEAD"]));
        let message = run_git(temp.path(), &["show", "-s", "--format=%B", "HEAD"]);
        assert!(message.contains("second"));
        assert!(message.contains("Signed-off-by: Test <test@example.com>"));

        fs::write(temp.path().join("tracked.txt"), "base\nthird\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        commit(
            temp.path(),
            &CommitSpec {
                message: "amended".into(),
                amend: true,
                signoff: false,
                signing: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(run_git(temp.path(), &["rev-list", "--count", "HEAD"]), "2");
        assert!(run_git(temp.path(), &["show", "-s", "--format=%s", "HEAD"]) == "amended");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let hook = temp.path().join(".git/hooks/pre-commit");
            fs::write(&hook, "#!/bin/sh\necho hook-output >&2\nexit 1\n").unwrap();
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(temp.path().join("tracked.txt"), "base\nhook\n").unwrap();
            run_git(temp.path(), &["add", "tracked.txt"]);
            let error = commit(
                temp.path(),
                &CommitSpec {
                    message: "hook failure".into(),
                    amend: false,
                    signoff: false,
                    signing: false,
                },
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("hook-output"));
        }
    }
    #[test]
    fn parses_porcelain_v2_status() {
        let input = b"# branch.oid 0123456789abcdef\x00# branch.head feature/demo\x00# branch.upstream origin/feature/demo\x00# branch.ab +2 -3\x00\
1 M. N... 100644 100644 100644 a b staged.txt\x00\
1 .M N... 100644 100644 100644 a b changed.txt\x00? new file.txt\x00u UU N... 100644 100644 100644 100644 a b c conflict.txt\x00";
        let status = parse_status(input).unwrap();
        assert_eq!(status.head, HeadState::Branch("feature/demo".into()));
        assert_eq!(status.worktree.staged, 1);
        assert_eq!(status.worktree.unstaged, 1);
        assert_eq!(status.worktree.untracked, 1);
        assert_eq!(status.worktree.conflicted, 1);
        assert_eq!(status.changes.len(), 4);
        assert!(status
            .changes
            .iter()
            .any(|entry| entry.status_label() == "M." && entry.path == Path::new("staged.txt")));
        assert!(status
            .changes
            .iter()
            .any(|entry| entry.status_label() == ".M" && entry.path == Path::new("changed.txt")));
        assert!(status.changes.iter().any(|entry| entry.untracked));
        assert!(status.changes.iter().any(|entry| entry.conflicted));
        assert_eq!(status.upstream.unwrap().ahead, 2);
    }

    #[test]
    fn parses_rename_and_non_ascii_paths_without_losing_boundaries() {
        let input = b"# branch.oid 0123456789abcdef\x00# branch.head main\x00\
2 R. N... 100644 100644 100644 a b R100 renamed file.txt\x00old file.txt\x00\
? \xe6\x96\xb0\xe6\x96\x87\xe4\xbb\xb6.txt\x00";
        let status = parse_status(input).unwrap();
        assert_eq!(status.worktree.staged, 1);
        assert_eq!(status.worktree.unstaged, 0);
        assert_eq!(status.worktree.untracked, 1);
        assert_eq!(status.worktree.conflicted, 0);
    }

    #[test]
    fn parses_file_changes_and_consumes_rename_source() {
        let input = b"1 M. N... 100644 100644 100644 a b staged.txt\x00\
1 .M N... 100644 100644 100644 a b changed file.txt\x00\
2 R. N... 100644 100644 100644 a b R100 renamed.txt\x00old.txt\x00\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\x00\
? new.txt\x00";
        let entries = parse_changes(input).unwrap();
        assert_eq!(entries.len(), 5);
        let renamed = entries
            .iter()
            .find(|entry| entry.path == Path::new("renamed.txt"))
            .unwrap();
        assert_eq!(renamed.original_path.as_deref(), Some(Path::new("old.txt")));
        assert_eq!(renamed.status_label(), "R.");
        assert!(entries
            .iter()
            .any(|entry| entry.untracked && entry.path == Path::new("new.txt")));
        assert!(entries.iter().any(|entry| entry.conflicted));
    }

    #[test]
    fn rejects_repository_path_escape_and_truncates_on_utf8_boundary() {
        let input = b"? ../outside.txt\x00";
        assert!(parse_changes(input).is_err());
        let (text, truncated, _) = truncate_preview("abcd\u{754c}".to_owned(), 5);
        assert!(truncated);
        assert!(text.starts_with("abcd"));
        assert!(text.is_char_boundary(text.len()));
    }

    #[test]
    fn parses_and_rebuilds_individual_unified_diff_hunks() {
        let input = concat!(
            "diff --git a/demo.txt b/demo.txt\n",
            "index 1111111..2222222 100644\n",
            "--- a/demo.txt\n",
            "+++ b/demo.txt\n",
            "@@ -1,3 +1,3 @@\n",
            " one\n",
            "-two\n",
            "+TWO\n",
            " three\n",
            "@@ -10,3 +10,3 @@\n",
            " ten\n",
            "-eleven\n",
            "+ELEVEN\n",
            " twelve\n"
        )
        .as_bytes();
        let hunks = parse_unified_diff(HunkSource::Worktree, input).unwrap();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header, "@@ -1,3 +1,3 @@");
        assert_eq!(hunks[1].header, "@@ -10,3 +10,3 @@");
        assert!(hunks[0].patch.starts_with(b"diff --git a/demo.txt"));
        assert!(hunks[0].patch.ends_with(b" three\n"));
        assert!(!hunks[0].patch.windows(6).any(|value| value == b"eleven"));
        assert!(hunks[1].patch.ends_with(b" twelve\n"));
        assert!(!hunks[1].patch.windows(4).any(|value| value == b"-two"));
        assert_ne!(hunks[0].fingerprint, hunks[1].fingerprint);
    }

    #[test]
    fn rejects_malformed_hunks_and_skips_binary_diffs() {
        let malformed = b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ bad header\n";
        assert!(parse_unified_diff(HunkSource::Worktree, malformed).is_err());
        let binary = b"diff --git a/image.png b/image.png\nindex 111..222 100644\nBinary files a/image.png and b/image.png differ\n";
        assert!(parse_unified_diff(HunkSource::Worktree, binary)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn previews_real_staged_unstaged_and_untracked_changes_with_tokens() {
        let temp = tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        fs::write(temp.path().join("tracked.txt"), "base\n").unwrap();
        assert!(std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "base",
            ])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());

        fs::write(temp.path().join("tracked.txt"), "base\nstaged\n").unwrap();
        assert!(std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        fs::write(temp.path().join("tracked.txt"), "base\nstaged\nworktree\n").unwrap();
        fs::write(temp.path().join("new.txt"), "untracked\n").unwrap();

        let entries = changes(temp.path()).await.unwrap();
        assert_eq!(entries.len(), 2);
        let tracked = entries
            .iter()
            .find(|entry| entry.path == Path::new("tracked.txt"))
            .unwrap();
        let preview = preview_change(temp.path(), tracked).await.unwrap();
        assert!(preview.text.contains("== Staged =="));
        assert!(preview.text.contains("== Worktree =="));
        let old_token = preview.token;
        assert_eq!(preview.hunks.len(), 2);
        assert_eq!(preview.hunks[0].source, HunkSource::Staged);
        assert_eq!(preview.hunks[1].source, HunkSource::Worktree);

        fs::write(
            temp.path().join("tracked.txt"),
            "base\nstaged\nworktree changed\n",
        )
        .unwrap();
        assert_ne!(change_token(temp.path(), tracked).await.unwrap(), old_token);

        let untracked = entries
            .iter()
            .find(|entry| entry.path == Path::new("new.txt"))
            .unwrap();
        let preview = preview_change(temp.path(), untracked).await.unwrap();
        assert!(preview.text.contains("== Untracked =="));
        assert!(preview.text.contains("untracked"));
        assert_eq!(preview.hunks.len(), 1);
        assert_eq!(preview.hunks[0].source, HunkSource::Untracked);
    }

    #[test]
    fn parses_detached_and_unborn_heads() {
        let detached =
            parse_status(b"# branch.oid abcdef0123456789\0# branch.head (detached)\0").unwrap();
        assert_eq!(detached.head, HeadState::Detached("abcdef01".into()));

        let unborn = parse_status(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert_eq!(unborn.head, HeadState::Unborn("main".into()));
    }

    #[test]
    fn parses_log_records_and_merge_parents() {
        let input = b"aaaaaaaa\x00bbbbbbbb cccccccc\x00Ada\x001700000000\x00Merge work\x00Merge work\n\nDetails\x00\nbbbbbbbb\x00\x00Bob\x001699999999\x00Initial\x00Initial\x00\n";
        let commits = parse_log(input).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, vec!["bbbbbbbb", "cccccccc"]);
        assert!(commits[0].refs.is_empty());
        assert!(commits[0].body.contains("Details"));
        assert_eq!(commits[1].subject, "Initial");
    }

    #[tokio::test]
    async fn loads_all_branches_tags_remotes_and_stash_entries() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        commit_file(temp.path(), "tracked.txt", "base\n", "base");
        run_git(
            temp.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "tag",
                "-a",
                "v1",
                "-m",
                "version one",
            ],
        );
        run_git(temp.path(), &["switch", "-q", "-c", "feature"]);
        commit_file(temp.path(), "feature.txt", "feature\n", "feature-only");
        let feature_oid = run_git(temp.path(), &["rev-parse", "HEAD"]);
        run_git(
            temp.path(),
            &["update-ref", "refs/remotes/origin/release", &feature_oid],
        );
        run_git(temp.path(), &["switch", "-q", "main"]);
        commit_file(temp.path(), "main.txt", "main\n", "main-only");

        fs::write(temp.path().join("tracked.txt"), "base\nstash one\n").unwrap();
        run_git(temp.path(), &["stash", "push", "-q", "-m", "one"]);
        fs::write(temp.path().join("tracked.txt"), "base\nstash two\n").unwrap();
        run_git(temp.path(), &["stash", "push", "-q", "-m", "two"]);

        let commits = log_all(temp.path()).await.unwrap();
        assert!(commits
            .iter()
            .any(|commit| commit.subject == "feature-only"));
        let feature = commits
            .iter()
            .find(|commit| commit.oid == feature_oid)
            .unwrap();
        assert!(feature.refs.contains(&CommitRef {
            name: "feature".into(),
            kind: CommitRefKind::LocalBranch,
        }));
        assert!(feature.refs.contains(&CommitRef {
            name: "origin/release".into(),
            kind: CommitRefKind::RemoteBranch,
        }));
        assert!(commits.iter().any(|commit| {
            commit
                .refs
                .iter()
                .any(|reference| reference.kind == CommitRefKind::Head && reference.name == "HEAD")
        }));
        assert!(commits.iter().any(|commit| {
            commit
                .refs
                .iter()
                .any(|reference| reference.kind == CommitRefKind::Tag && reference.name == "v1")
        }));
        let stash_names = commits
            .iter()
            .flat_map(|commit| &commit.refs)
            .filter(|reference| reference.kind == CommitRefKind::Stash)
            .map(|reference| reference.name.as_str())
            .collect::<Vec<_>>();
        assert!(stash_names.contains(&"stash@{0}"));
        assert!(stash_names.contains(&"stash@{1}"));
    }

    #[test]
    fn parses_structured_ref_and_stash_records() {
        let refs = parse_ref_records(
            b"aaaaaaaa\x00\x00refs/heads/main\x00\nbbbbbbbb\x00cccccccc\x00refs/tags/v1\x00\n",
        )
        .unwrap();
        assert_eq!(refs["aaaaaaaa"][0].kind, CommitRefKind::LocalBranch);
        assert_eq!(refs["cccccccc"][0].kind, CommitRefKind::Tag);
        let stash =
            parse_stash_records(b"dddddddd\x00stash@{0}\x00\neeeeeeee\x00stash@{1}\x00\n").unwrap();
        assert_eq!(stash.len(), 2);
        assert_eq!(stash[1].1.name, "stash@{1}");
    }

    #[tokio::test]
    async fn repository_actions_cover_stash_conflict_refs_and_remote_workflows() {
        let temp = tempdir().unwrap();
        let remote = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        run_git(temp.path(), &["config", "user.email", "test@example.com"]);
        commit_file(temp.path(), "tracked.txt", "base\n", "base");
        run_git(remote.path(), &["init", "--bare", "-q"]);

        fs::write(temp.path().join("tracked.txt"), "stash\n").unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashPush {
                message: "saved".into(),
                include_untracked: false,
                keep_index: false,
                staged_only: false,
            },
            false,
        )
        .await
        .unwrap();
        let snapshot = repository_snapshot(temp.path()).await.unwrap();
        assert_eq!(snapshot.stashes.len(), 1);
        let shown = execute_repository_action(
            temp.path(),
            &RepositoryAction::StashShow {
                selector: "stash@{0}".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert!(shown.detail.unwrap().contains("tracked.txt"));
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashApply {
                selector: "stash@{0}".into(),
                restore_index: false,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("tracked.txt")).unwrap(),
            "stash\n"
        );
        run_git(temp.path(), &["restore", "tracked.txt"]);
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashDrop {
                selector: "stash@{0}".into(),
            },
            false,
        )
        .await
        .unwrap();
        fs::write(temp.path().join("tracked.txt"), "stash pop\n").unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashPush {
                message: "pop".into(),
                include_untracked: false,
                keep_index: false,
                staged_only: false,
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashPop {
                selector: "stash@{0}".into(),
                restore_index: false,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("tracked.txt")).unwrap(),
            "stash pop\n"
        );
        assert!(repository_snapshot(temp.path())
            .await
            .unwrap()
            .stashes
            .is_empty());
        run_git(temp.path(), &["restore", "tracked.txt"]);

        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchCreate {
                name: "feature".into(),
                start: None,
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::TagCreate {
                name: "v1".into(),
                target: "HEAD".into(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::RemoteAdd {
                name: "origin".into(),
                url: remote.path().to_string_lossy().into_owned(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::Push {
                remote: "origin".into(),
                branch: "main".into(),
                set_upstream: true,
                force_with_lease: false,
            },
            false,
        )
        .await
        .unwrap();
        let snapshot = repository_snapshot(temp.path()).await.unwrap();
        assert!(snapshot
            .branches
            .iter()
            .any(|branch| branch.name == "feature"));
        let main = snapshot
            .branches
            .iter()
            .find(|branch| branch.name == "main")
            .unwrap();
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert!(snapshot.tags.iter().any(|tag| tag.name == "v1"));
        assert!(snapshot
            .remotes
            .iter()
            .any(|remote| remote.name == "origin"));

        run_git(temp.path(), &["switch", "-q", "feature"]);
        fs::write(temp.path().join("tracked.txt"), "feature\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git(temp.path(), &["commit", "-q", "-m", "feature"]);
        run_git(temp.path(), &["switch", "-q", "main"]);
        fs::write(temp.path().join("tracked.txt"), "main\n").unwrap();
        run_git(temp.path(), &["add", "tracked.txt"]);
        run_git(temp.path(), &["commit", "-q", "-m", "main"]);
        let merge = execute_repository_action(
            temp.path(),
            &RepositoryAction::Merge {
                reference: "feature".into(),
            },
            false,
        )
        .await;
        assert!(merge.is_err());
        let snapshot = repository_snapshot(temp.path()).await.unwrap();
        assert_eq!(snapshot.operation, Some(GitOperationKind::Merge));
        assert_eq!(snapshot.conflicts, vec![PathBuf::from("tracked.txt")]);
        execute_repository_action(
            temp.path(),
            &RepositoryAction::ConflictTakeTheirs {
                path: "tracked.txt".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("tracked.txt")).unwrap(),
            "feature\n"
        );
        execute_repository_action(
            temp.path(),
            &RepositoryAction::ConflictTakeOurs {
                path: "tracked.txt".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("tracked.txt")).unwrap(),
            "main\n"
        );
        execute_repository_action(
            temp.path(),
            &RepositoryAction::ConflictMarkResolved {
                path: "tracked.txt".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert!(repository_snapshot(temp.path())
            .await
            .unwrap()
            .conflicts
            .is_empty());
        execute_repository_action(
            temp.path(),
            &RepositoryAction::Continue {
                operation: GitOperationKind::Merge,
            },
            false,
        )
        .await
        .unwrap();
        assert!(repository_snapshot(temp.path())
            .await
            .unwrap()
            .operation
            .is_none());
    }

    #[tokio::test]
    async fn repository_actions_cover_branch_tag_merge_and_rebase() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        run_git(temp.path(), &["config", "user.email", "test@example.com"]);
        commit_file(temp.path(), "base.txt", "base\n", "base");

        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchCreate {
                name: "topic".into(),
                start: None,
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchSwitch {
                name: "topic".into(),
            },
            false,
        )
        .await
        .unwrap();
        commit_file(temp.path(), "topic.txt", "topic\n", "topic");
        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchRename {
                old: "topic".into(),
                new: "renamed".into(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::TagCreate {
                name: "temp-tag".into(),
                target: "HEAD".into(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchSwitch {
                name: "main".into(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::Merge {
                reference: "renamed".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert!(temp.path().join("topic.txt").is_file());
        execute_repository_action(
            temp.path(),
            &RepositoryAction::TagDelete {
                name: "temp-tag".into(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchDelete {
                name: "renamed".into(),
                force: false,
            },
            false,
        )
        .await
        .unwrap();
        let snapshot = repository_snapshot(temp.path()).await.unwrap();
        assert!(!snapshot
            .branches
            .iter()
            .any(|branch| branch.name == "renamed"));
        assert!(!snapshot.tags.iter().any(|tag| tag.name == "temp-tag"));

        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchCreate {
                name: "rebased".into(),
                start: None,
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchSwitch {
                name: "rebased".into(),
            },
            false,
        )
        .await
        .unwrap();
        commit_file(temp.path(), "rebased.txt", "rebased\n", "rebased");
        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchSwitch {
                name: "main".into(),
            },
            false,
        )
        .await
        .unwrap();
        commit_file(temp.path(), "main.txt", "main\n", "main");
        execute_repository_action(
            temp.path(),
            &RepositoryAction::BranchSwitch {
                name: "rebased".into(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::Rebase {
                reference: "main".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert!(run_git(
            temp.path(),
            &["merge-base", "--is-ancestor", "main", "HEAD"]
        )
        .is_empty());
    }

    #[tokio::test]
    async fn repository_actions_cover_cherry_pick_revert_and_stale_snapshot() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        run_git(temp.path(), &["config", "user.email", "test@example.com"]);
        commit_file(temp.path(), "base.txt", "base\n", "base");
        run_git(temp.path(), &["switch", "-q", "-c", "source"]);
        commit_file(temp.path(), "picked.txt", "picked\n", "picked");
        let picked = run_git(temp.path(), &["rev-parse", "HEAD"]);
        run_git(temp.path(), &["switch", "-q", "main"]);

        execute_repository_action(
            temp.path(),
            &RepositoryAction::CherryPick {
                oid: picked.clone(),
            },
            false,
        )
        .await
        .unwrap();
        assert!(temp.path().join("picked.txt").is_file());
        execute_repository_action(
            temp.path(),
            &RepositoryAction::Revert { oid: "HEAD".into() },
            false,
        )
        .await
        .unwrap();
        assert!(!temp.path().join("picked.txt").exists());

        let snapshot = repository_snapshot(temp.path()).await.unwrap();
        run_git(temp.path(), &["branch", "changed-after-confirmation"]);
        let error = validate_repository_action(
            temp.path(),
            &RepositoryAction::TagCreate {
                name: "stale".into(),
                target: "HEAD".into(),
            },
            Some(snapshot.token),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("changed after confirmation"));
        assert!(operation_args(&mut Vec::new(), GitOperationKind::Merge, "--skip").is_err());
    }

    #[tokio::test]
    async fn repository_actions_cover_advanced_stash_modes() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        run_git(temp.path(), &["config", "user.email", "test@example.com"]);
        commit_file(temp.path(), "staged.txt", "base staged\n", "base staged");
        commit_file(
            temp.path(),
            "unstaged.txt",
            "base unstaged\n",
            "base unstaged",
        );

        fs::write(temp.path().join("staged.txt"), "saved staged\n").unwrap();
        run_git(temp.path(), &["add", "staged.txt"]);
        fs::write(temp.path().join("unstaged.txt"), "left unstaged\n").unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashPush {
                message: "staged only".into(),
                include_untracked: false,
                keep_index: false,
                staged_only: true,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("staged.txt")).unwrap(),
            "base staged\n"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("unstaged.txt")).unwrap(),
            "left unstaged\n"
        );
        assert!(run_git(temp.path(), &["diff", "--cached", "--name-only"]).is_empty());

        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashApply {
                selector: "stash@{0}".into(),
                restore_index: true,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            run_git(temp.path(), &["diff", "--cached", "--name-only"]),
            "staged.txt"
        );
        run_git(temp.path(), &["reset", "--hard", "-q", "HEAD"]);
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashPop {
                selector: "stash@{0}".into(),
                restore_index: true,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            run_git(temp.path(), &["diff", "--cached", "--name-only"]),
            "staged.txt"
        );
        assert!(repository_snapshot(temp.path())
            .await
            .unwrap()
            .stashes
            .is_empty());
        run_git(temp.path(), &["reset", "--hard", "-q", "HEAD"]);

        fs::write(temp.path().join("staged.txt"), "kept index\n").unwrap();
        run_git(temp.path(), &["add", "staged.txt"]);
        fs::write(temp.path().join("staged.txt"), "stashed worktree\n").unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashPush {
                message: "keep index".into(),
                include_untracked: false,
                keep_index: true,
                staged_only: false,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            run_git(temp.path(), &["diff", "--cached", "--name-only"]),
            "staged.txt"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("staged.txt")).unwrap(),
            "kept index\n"
        );
        run_git(temp.path(), &["reset", "--hard", "-q", "HEAD"]);
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashDrop {
                selector: "stash@{0}".into(),
            },
            false,
        )
        .await
        .unwrap();

        fs::write(temp.path().join("unstaged.txt"), "branch stash\n").unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashPush {
                message: "branch source".into(),
                include_untracked: false,
                keep_index: false,
                staged_only: false,
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            temp.path(),
            &RepositoryAction::StashBranch {
                name: "from-stash".into(),
                selector: "stash@{0}".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            run_git(temp.path(), &["branch", "--show-current"]),
            "from-stash"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("unstaged.txt")).unwrap(),
            "branch stash\n"
        );
        assert!(repository_snapshot(temp.path())
            .await
            .unwrap()
            .stashes
            .is_empty());
        run_git(temp.path(), &["reset", "--hard", "-q", "HEAD"]);

        for (message, content) in [("one", "stash one\n"), ("two", "stash two\n")] {
            fs::write(temp.path().join("unstaged.txt"), content).unwrap();
            execute_repository_action(
                temp.path(),
                &RepositoryAction::StashPush {
                    message: message.into(),
                    include_untracked: false,
                    keep_index: false,
                    staged_only: false,
                },
                false,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            repository_snapshot(temp.path())
                .await
                .unwrap()
                .stashes
                .len(),
            2
        );
        execute_repository_action(temp.path(), &RepositoryAction::StashClear, false)
            .await
            .unwrap();
        assert!(repository_snapshot(temp.path())
            .await
            .unwrap()
            .stashes
            .is_empty());

        assert!(
            validate_repository_action_values(&RepositoryAction::StashPush {
                message: String::new(),
                include_untracked: true,
                keep_index: false,
                staged_only: true,
            })
            .is_err()
        );
        assert!(
            validate_repository_action_values(&RepositoryAction::StashPush {
                message: String::new(),
                include_untracked: false,
                keep_index: true,
                staged_only: true,
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn repository_actions_cover_remote_round_trip_and_force_with_lease() {
        let root = tempdir().unwrap();
        let remote = root.path().join("remote.git");
        let seed = root.path().join("seed");
        let client = root.path().join("client");
        run_git(
            root.path(),
            &["init", "--bare", "-q", remote.to_str().unwrap()],
        );
        run_git(
            root.path(),
            &["init", "-q", "-b", "main", seed.to_str().unwrap()],
        );
        run_git(&seed, &["config", "user.name", "Test"]);
        run_git(&seed, &["config", "user.email", "test@example.com"]);
        commit_file(&seed, "shared.txt", "base\n", "base");
        run_git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run_git(&seed, &["push", "-q", "-u", "origin", "main"]);
        run_git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        run_git(
            root.path(),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                client.to_str().unwrap(),
            ],
        );
        run_git(&client, &["config", "user.name", "Test"]);
        run_git(&client, &["config", "user.email", "test@example.com"]);

        commit_file(&seed, "upstream.txt", "upstream\n", "upstream");
        run_git(&seed, &["push", "-q", "origin", "main"]);
        execute_repository_action(
            &client,
            &RepositoryAction::Fetch {
                remote: "origin".into(),
                prune: true,
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            &client,
            &RepositoryAction::Pull {
                remote: "origin".into(),
                branch: "main".into(),
                rebase: true,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            run_git(&client, &["rev-parse", "HEAD"]),
            run_git(&seed, &["rev-parse", "HEAD"])
        );

        run_git(&client, &["branch", "--unset-upstream"]);
        execute_repository_action(
            &client,
            &RepositoryAction::SetUpstream {
                branch: "main".into(),
                upstream: "origin/main".into(),
            },
            false,
        )
        .await
        .unwrap();
        commit_file(&client, "client.txt", "client\n", "client");
        let push = RepositoryAction::Push {
            remote: "origin".into(),
            branch: "main".into(),
            set_upstream: true,
            force_with_lease: true,
        };
        let (args, _) = action_args(&push).unwrap();
        assert!(args.iter().any(|arg| arg == "--force-with-lease"));
        assert!(!args.iter().any(|arg| arg == "--force"));
        assert!(args.iter().any(|arg| arg == "main:main"));
        execute_repository_action(&client, &push, false)
            .await
            .unwrap();
        assert_eq!(
            run_git(&remote, &["rev-parse", "refs/heads/main"]),
            run_git(&client, &["rev-parse", "main"])
        );

        let peer = root.path().join("peer");
        run_git(
            root.path(),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                peer.to_str().unwrap(),
            ],
        );
        run_git(&peer, &["config", "user.name", "Peer"]);
        run_git(&peer, &["config", "user.email", "peer@example.com"]);
        commit_file(&peer, "peer.txt", "peer\n", "peer advance");
        run_git(&peer, &["push", "-q", "origin", "main"]);

        run_git(&client, &["reset", "--hard", "-q", "HEAD~1"]);
        commit_file(&client, "replacement.txt", "replacement\n", "replacement");
        let normal_push = RepositoryAction::Push {
            remote: "origin".into(),
            branch: "main".into(),
            set_upstream: false,
            force_with_lease: false,
        };
        assert!(execute_repository_action(&client, &normal_push, false)
            .await
            .is_err());
        assert!(execute_repository_action(&client, &push, false)
            .await
            .is_err());

        execute_repository_action(
            &client,
            &RepositoryAction::Fetch {
                remote: "origin".into(),
                prune: false,
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(&client, &push, false)
            .await
            .unwrap();
        assert_eq!(
            run_git(&remote, &["rev-parse", "refs/heads/main"]),
            run_git(&client, &["rev-parse", "main"])
        );

        let head = run_git(&client, &["rev-parse", "HEAD"]);
        run_git(&remote, &["update-ref", "refs/heads/stale", &head]);
        execute_repository_action(
            &client,
            &RepositoryAction::Fetch {
                remote: "origin".into(),
                prune: false,
            },
            false,
        )
        .await
        .unwrap();
        run_git(&remote, &["update-ref", "-d", "refs/heads/stale"]);
        execute_repository_action(
            &client,
            &RepositoryAction::RemotePrune {
                remote: "origin".into(),
            },
            false,
        )
        .await
        .unwrap();
        let stale = std::process::Command::new("git")
            .args(["show-ref", "--verify", "refs/remotes/origin/stale"])
            .current_dir(&client)
            .status()
            .unwrap();
        assert!(!stale.success());

        execute_repository_action(
            &client,
            &RepositoryAction::RemoteSetUrl {
                name: "origin".into(),
                url: remote.to_string_lossy().into_owned(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            &client,
            &RepositoryAction::RemoteAdd {
                name: "backup".into(),
                url: remote.to_string_lossy().into_owned(),
            },
            false,
        )
        .await
        .unwrap();
        execute_repository_action(
            &client,
            &RepositoryAction::RemoteRemove {
                name: "backup".into(),
            },
            false,
        )
        .await
        .unwrap();
        assert!(!repository_snapshot(&client)
            .await
            .unwrap()
            .remotes
            .iter()
            .any(|remote| remote.name == "backup"));
        assert!(validate_repository_action_values(&RepositoryAction::Fetch {
            remote: "--all".into(),
            prune: false,
        })
        .is_err());
    }

    #[tokio::test]
    async fn repository_action_skip_finishes_conflicting_cherry_pick() {
        let temp = tempdir().unwrap();
        run_git(temp.path(), &["init", "-q", "-b", "main"]);
        run_git(temp.path(), &["config", "user.name", "Test"]);
        run_git(temp.path(), &["config", "user.email", "test@example.com"]);
        commit_file(temp.path(), "tracked.txt", "base\n", "base");
        run_git(temp.path(), &["switch", "-q", "-c", "source"]);
        fs::write(temp.path().join("tracked.txt"), "source\n").unwrap();
        run_git(temp.path(), &["commit", "-qam", "source"]);
        let oid = run_git(temp.path(), &["rev-parse", "HEAD"]);
        run_git(temp.path(), &["switch", "-q", "main"]);
        fs::write(temp.path().join("tracked.txt"), "main\n").unwrap();
        run_git(temp.path(), &["commit", "-qam", "main"]);
        assert!(execute_repository_action(
            temp.path(),
            &RepositoryAction::CherryPick { oid },
            false,
        )
        .await
        .is_err());
        assert_eq!(
            repository_snapshot(temp.path()).await.unwrap().operation,
            Some(GitOperationKind::CherryPick)
        );
        execute_repository_action(
            temp.path(),
            &RepositoryAction::Skip {
                operation: GitOperationKind::CherryPick,
            },
            false,
        )
        .await
        .unwrap();
        assert!(repository_snapshot(temp.path())
            .await
            .unwrap()
            .operation
            .is_none());
    }
    #[test]
    fn rejects_incomplete_log_record() {
        assert!(parse_log(b"oid\0parent\0").is_err());
    }
}
