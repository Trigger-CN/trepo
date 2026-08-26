# repo-tui

`repo-tui` is a terminal workspace for Android Repo clients and Git repositories. It discovers projects, scans Git status concurrently, renders the complete all-refs commit graph, and provides guarded Git repository and Repo batch workflows.

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
| `j` / `k`, arrows | Move selection; move the active menu selection |
| `g` / `G` | First / last item |
| `Enter` | Open selected repository graph; select or submit the active overlay |
| `Space` | Select/unselect a Workspace project; toggle an option in forms |
| `A` | Select/unselect all projects in the current Workspace filter |
| `a` | Open Repo batch actions in Workspace; open fixed actions in Repository |
| `c` | Open Changes; cancel a running Repo task from its task view |
| `f` | Retry only failed projects after a Repo task |
| `Tab` | Cycle file / hunk / line mode in Changes; switch tabs or form fields |
| `s` / `u` | Stage / unstage the selected file, hunk, or line |
| `d` | Preview and confirm discarding the selected file, worktree hunk, or line |
| `m` | Open commit dialog from Changes |
| `Ctrl-A` / `Ctrl-U` / `Ctrl-G` | Toggle amend / sign-off / signing in Changes commit dialog |
| `o` | Open Repository management from Workspace, Graph, or Changes |
| `PageUp` / `PageDown` | Scroll the selected diff |
| `/` | Search/filter projects; multi-selection remains bound to stable project identity |
| `r` | Refresh current page |
| `Esc` | Close overlay, back, clear search, or exit |
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
- stable project multi-selection and name/path filtering in Workspace
- fixed Repo batch actions for `sync/start/checkout/abandon/prune/rebase/upload/download` and pinned manifest export
- complete target and argv review before execution; destructive actions are visually distinguished
- workspace-exclusive execution coordinated with project Git locks and lock-time path/index checks
- per-project pending/running/success/failure/cancelled results with bounded, credential-redacted stdout/stderr logs
- process-group cancellation without rollback claims, automatic real-state rescan, and retry-failed-only
- background upload uses the explicitly reviewed `--current-branch --yes` mode; interactive authentication and advanced upload parameters wait for M5 PTY takeover
- Changes file list with staged, worktree, and untracked diff previews
- guarded file-, hunk-, and changed-line stage, unstage, and discard with lock-time patch reconstruction
- `git apply --check`, stale token/fingerprint rejection, destructive confirmation, and failure-state preservation
- commit/amend, sign-off, signing, hook output, and message recovery
- Repository page with Status, Stashes, Branches & Tags, and Remotes tabs
- project locks, worktree-aware index-lock checks, snapshot tokens, generation checks, and automatic scoped refresh
- `doctor` diagnostics and parser/real Git/TestBackend-focused tests

Planned next: command palette and PTY takeover (M5), including interactive authentication and external mergetool/editor handoff.
