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
| `Enter` | Open selected repository graph; select active overlay item; insert a newline in the Changes commit editor |
| `Left` / `Right` | Move by one Unicode character in the Changes commit editor |
| `Up` / `Down` | Move between lines in the Changes commit editor while preserving the character column when possible |
| `Home` / `End` | Move to the start / end of the current commit-message line |
| `Backspace` / `Delete` | Delete the character before / at the commit-message cursor |
| `Ctrl-Enter` / `Ctrl-S` | Submit the multiline Changes commit message |
| `Space` | Select/unselect a Workspace project or a file in Changes; toggle an option in forms |
| `A` | Select/unselect all projects in the current Workspace filter or all files in Changes |
| `a` | Open Repo batch actions in Workspace; open fixed actions in Repository |
| `c` | Open Changes; cancel a running Repo task from its task view |
| `f` | Graph: open structured Branch/Query/Author/Since/Until filters; Repo task: retry only failed projects |
| `Tab` | Cycle file / hunk / line mode in Changes; switch tabs or form fields |
| `s` / `u` | Stage / unstage selected Changes files, or the active file, hunk, or line when no file selection exists |
| `d` | Workspace: toggle changed-project filter; Changes: confirm discarding one active file, worktree hunk, or line |
| `m` | Open the bordered multiline commit editor from Changes; typing and multiline paste insert at the cursor |
| `Ctrl-A` / `Ctrl-U` / `Ctrl-G` | Toggle amend / sign-off / signing in Changes commit dialog |
| `o` | Open Repository management from Workspace, Graph, or Changes |
| `PageUp` / `PageDown` | Scroll the selected diff |
| `/` | Workspace: search projects; Graph: open filters focused on commit Query |
| `r` | Refresh current page |
| `x` | Graph: clear all commit filters |
| `Esc` | Close overlay, back, clear search, or exit |
| `q` | Exit from Workspace |
| `?` | Toggle contextual help |

## Current scope

Implemented:

- Repo and single-Git workspace discovery
- Concurrent porcelain v2 status scanning
- staged, unstaged, untracked, conflict, HEAD, ahead/behind summary, and file-level porcelain status captured from the same scan
- searchable responsive Workspace page with an optional changed-project-only filter and a wide-screen Inspector change tree
- complete all-refs commit graph covering local branches, remote branches, tags, HEAD, and every stash entry, with UTC calendar date and relative age
- multi-color topology lanes with visible split/merge connectors and distinct HEAD/local/remote/tag/stash badges
- in-memory Graph filtering by local/remote branch history, commit OID/subject/body/ref text, author, and inclusive UTC date range; conditions combine with AND while selection remains bound to commit OID
- Graph two-level object menu: select a commit node, choose its commit/HEAD/local branch/remote branch/tag/stash object, then choose a fixed contextual action
- Graph contextual commit/amend, stash creation, branch/tag creation, merge, rebase, cherry-pick, revert, and stash actions
- Graph forms and confirmations reuse the protected RepositoryAction/OperationRunner workflow; local and remote branches expose different valid actions
- stable project multi-selection and name/path filtering in Workspace
- fixed Repo batch actions for `sync/start/checkout/abandon/prune/rebase/upload/download` and pinned manifest export; Sync runs once as `repo sync -c -j8` when no project is selected, or `repo sync -c -j8 -- <projects...>` for the frozen selection
- complete scope, target, and argv review before execution; whole-workspace Sync always requires explicit confirmation and destructive actions are visually distinguished
- workspace-exclusive execution coordinated with project Git locks and lock-time path/index checks
- workspace or per-project pending/running/success/failure/cancelled display with bounded, credential-redacted stdout/stderr logs; aggregated Sync uses only its command exit status for the participating scope
- process-group cancellation without rollback claims, automatic real-state rescan, and retry-failed-scope
- background upload uses the explicitly reviewed `--current-branch --yes` mode; interactive authentication and advanced upload parameters wait for M5 PTY takeover
- shared directory-tree rendering for changed files in Workspace Inspector and Changes, while operations remain bound to exact file paths
- stable Changes file multi-selection with single-lock, all-token-preflight batch stage/unstage; destructive discard remains one explicit scope at a time
- guarded file-, hunk-, and changed-line stage, unstage, and discard with lock-time patch reconstruction
- `git apply --check`, stale token/fingerprint rejection, destructive confirmation, and failure-state preservation
- multiline commit/amend input and paste, sign-off, signing, hook output, and message recovery
- Repository page with Status, Stashes, Branches & Tags, and Remotes tabs
- project locks, worktree-aware index-lock checks, snapshot tokens, generation checks, and automatic scoped refresh
- `doctor` diagnostics and parser/real Git/TestBackend-focused tests

Planned next: command palette and PTY takeover (M5), including interactive authentication and external mergetool/editor handoff.
