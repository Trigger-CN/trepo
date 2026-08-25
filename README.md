# repo-tui

`repo-tui` is a terminal workspace for Android Repo clients and Git repositories. The current implementation discovers projects, scans Git status concurrently, browses commit history, and provides guarded file- and hunk-level stage, unstage, and discard operations.

See [the design](docs/DESIGN.md) and [implementation roadmap](docs/ROADMAP.md) for the full product direction.

## Requirements

- Rust 1.81 or newer
- Git with porcelain v2 support
- Google/Android `repo` for Repo workspace mode
- Linux or macOS terminal

## Build and run

```bash
cargo build --release
cargo run -- /path/to/repo-client
cargo run -- doctor /path/to/workspace
```

Starting from a subdirectory is supported. If no `.repo` directory is found, repo-tui opens the containing Git repository.

## Keys

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move selection |
| `g` / `G` | First / last item |
| `Enter` | Open selected repository graph |
| `c` | Open Changes for the current repository |
| `Tab` | Toggle file / hunk mode in Changes |
| `s` / `u` | Stage / unstage the selected file or hunk |
| `d` | Preview and confirm discarding the selected file or worktree hunk |
| `PageUp` / `PageDown` | Scroll the selected diff |
| `/` | Search projects |
| `r` | Refresh current page |
| `Esc` | Back, clear search, or exit |
| `q` | Exit from Workspace |
| `?` | Toggle contextual help |

## Current scope

Implemented:

- Repo and single-Git workspace discovery
- Concurrent porcelain v2 status scanning
- staged, unstaged, untracked, conflict, HEAD and ahead/behind summary
- searchable responsive Workspace page
- complete all-refs commit graph covering local branches, remote branches, tags, HEAD, and every stash entry
- multi-color topology lanes and distinct HEAD/local/remote/tag/stash badges
- Changes file list with staged, worktree, and untracked diff previews
- guarded file-level stage, unstage, and restore with per-project locks and stale-state checks
- selectable unified-diff hunks with isolated stage, unstage, and destructive discard
- lock-time hunk revalidation, stable fingerprints, and `git apply --check` before patch writes
- destructive confirmation, failure-state preservation, and automatic Workspace/Changes refresh
- `doctor` diagnostics and parser/integration/UI-focused tests

Planned next: commit/amend, stash, and conflict workflows, followed by branch/remote workflows, Repo batch actions, command palette, and PTY takeover.
