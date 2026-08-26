# repo-tui

`repo-tui` is a terminal workspace for Android Repo clients and Git repositories. It discovers projects, scans Git status concurrently, renders the complete all-refs commit graph, and provides guarded M2/M3 workflows for changes, commits, stashes, conflicts, branches, tags, integration, and remotes.

See [the design](docs/DESIGN.md), [implementation roadmap](docs/ROADMAP.md), and [Agent guide](AGENTS.md) for the product direction and development rules.

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
| `j` / `k`, arrows | Move selection; in Graph object/action menus move context selection |
| `g` / `G` | First / last item |
| `Enter` | Open selected repository graph; in Graph open node objects, then object actions |
| `c` | Open Changes for the current repository |
| `Tab` | Cycle file / hunk / line mode in Changes; switch tabs in Repository; move Graph form field |
| `s` / `u` | Stage / unstage the selected file, hunk, or line |
| `d` | Preview and confirm discarding the selected file, worktree hunk, or line |
| `m` | Open commit dialog from Changes |
| `Ctrl-A` / `Ctrl-U` / `Ctrl-G` | Toggle amend / sign-off / signing in Changes commit dialog |
| `o` | Open Repository management from Workspace, Graph, or Changes |
| `a` | Open fixed Repository actions for the current tab |
| `Space` | Toggle a form option, including Graph stash/commit options |
| `Enter` / `Esc` | Select, submit, confirm flow, cancel, or return |
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
- multi-color topology lanes with visible split/merge connectors and distinct HEAD/local/remote/tag/stash badges
- Graph two-level object menu: select a commit node, choose its commit/HEAD/local branch/remote branch/tag/stash object, then choose a fixed contextual action
- Graph contextual commit/amend, stash creation, branch/tag creation, merge, rebase, cherry-pick, revert, and stash actions
- Graph forms and confirmations reuse the protected RepositoryAction/OperationRunner workflow; local and remote branches expose different valid actions
- Changes file list with staged, worktree, and untracked diff previews
- guarded file-, hunk-, and changed-line stage, unstage, and discard with lock-time patch reconstruction
- `git apply --check`, stale token/fingerprint rejection, destructive confirmation, and failure-state preservation
- commit/amend, sign-off, signing, hook output, and message recovery
- Repository page with Status, Stashes, Branches & Tags, and Remotes tabs
- project locks, worktree-aware index-lock checks, snapshot tokens, generation checks, and automatic scoped refresh
- `doctor` diagnostics and parser/real Git/TestBackend-focused tests

Planned next: Repo batch actions (M4), command palette and PTY takeover (M5), including external mergetool/editor handoff.
