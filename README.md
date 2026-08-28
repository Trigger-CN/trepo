# repo-tui

`repo-tui` is a terminal workspace for Android Repo clients and Git repositories. It discovers projects, scans Git status concurrently, renders the complete all-refs commit graph, and provides guarded Git repository and Repo batch workflows.

See [the user guide and operation flowcharts](docs/USER_GUIDE.md), [the design](docs/DESIGN.md), [implementation roadmap](docs/ROADMAP.md), and [Agent guide](AGENTS.md).

## Requirements

- Rust 1.81 or newer
- Git with porcelain v2 support
- Google/Android `repo` for Repo workspace mode
- Linux or macOS terminal

## Build and run

```bash
cargo build --release
cargo run -- /path/to/repo-client          # English (default)
cargo run -- -zh /path/to/repo-client      # Chinese
cargo run -- --en /path/to/repo-client     # explicit English
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
| `Space` | Select/unselect a Workspace repository or a file in Changes; toggle an option in forms |
| `A` | Select/unselect all repositories in the current Workspace filter or all files in Changes |
| `Z` / `D` | Workspace: confirm Stash / complete Discard for the frozen repository selection |
| `a` | Open Repo batch actions in Workspace; open fixed actions in Repository |
| `c` | Open Changes; cancel a running Repo task from its task view |
| `f` | Graph: open structured Branch/Query/Author/Since/Until filters; Repo task: retry only failed projects |
| `Tab` | Cycle file / hunk / line mode in Changes; switch tabs or form fields |
| `z` | Changes: confirm stashing the selected files, including untracked files |
| `s` / `u` | Stage / unstage selected Changes files, or the active file, hunk, or line when no file selection exists |
| `d` | Workspace: cycle all projects → changed projects → changed projects with file trees; Changes: confirm complete Discard for selected files or discard the active file/hunk/line |
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
- searchable responsive Workspace page where `d` cycles all projects, changed projects, and changed projects expanded with their file trees; repository selection remains bound to stable project identity
- complete all-refs commit graph covering local branches, remote branches, tags, HEAD, and every stash entry, ordered with pure topological order and showing UTC calendar dates
- compact pipe-based topology lanes with left-shifting continuations, solid split/merge connectors, explicit `~N` hidden-lane markers, and `◉` missing-parent boundaries
- responsive Graph columns preserve topology, wrapped subject text, and important refs first; rows use their real visual height, commit body keeps original line breaks, and dense remote/tag badges fold into `R:+N`/`T:+N` while Inspector/object menus retain every ref
- in-memory Graph filtering by local/remote branch history, commit OID/subject/body/ref text, author, and inclusive UTC date range; conditions combine with AND while selection remains bound to commit OID
- Graph two-level object menu: select a commit node, choose its commit/HEAD/local branch/remote branch/tag/stash object, then choose a fixed contextual action
- Graph contextual commit/amend, advanced stash creation, branch/tag creation, merge, rebase, cherry-pick, revert, stash actions, and local-branch Push/Force Push
- Graph forms and confirmations reuse the protected RepositoryAction/OperationRunner workflow; local branches expose separate Push and force-with-lease actions while remote branches expose only valid local operations
- stable project multi-selection and name/path filtering in Workspace
- selected-repository Workspace Git Stash/Discard with frozen per-repository change counts, full-batch path/token/index-lock preflight, and per-repository pending/running/success/failure results; cross-repository execution makes no rollback claim
- fixed Repo batch actions for `sync/start/checkout/abandon/prune/rebase/upload/download` and pinned manifest export; Sync runs once as `repo sync -c -j8` when no project is selected, or `repo sync -c -j8 -- <projects...>` for the frozen selection
- complete scope, target, and argv review before execution; whole-workspace Sync always requires explicit confirmation and destructive actions are visually distinguished
- workspace-exclusive execution coordinated with project Git locks and lock-time path/index checks
- workspace or per-project pending/running/success/failure/cancelled display with bounded, credential-redacted stdout/stderr logs; aggregated Sync uses only its command exit status for the participating scope
- process-group cancellation without rollback claims, automatic real-state rescan, and retry-failed-scope
- background upload uses the explicitly reviewed `--current-branch --yes` mode; interactive authentication and advanced upload parameters wait for M5 PTY takeover
- shared directory-tree rendering for changed files in Workspace main-list expansion, Workspace Inspector, and Changes, while operations remain bound to exact repository/file identities
- stable Changes file multi-selection with all-token-preflight batch Stage/Unstage, confirmed selected-path Stash, and confirmed complete Discard of tracked index/worktree plus untracked paths
- guarded file-, hunk-, and changed-line stage, unstage, and discard with lock-time patch reconstruction
- `git apply --check`, stale token/fingerprint rejection, destructive confirmation, and failure-state preservation
- multiline commit/amend input and paste, sign-off, signing, hook output, and message recovery
- Repository page with Status, Stashes, Branches & Tags, and Remotes tabs
- structured Stash workflows for include-untracked, keep-index, staged-only, apply/pop index restore, branch creation, drop, and clear; staged-only rejects incompatible modes before execution
- separate Push and Force Push entries in Repository and Graph; force updates always use `--force-with-lease`, with exact refspec/OID-range preview and explicit remote-history warning
- project locks, worktree-aware index-lock checks, snapshot tokens, generation checks, and automatic scoped refresh
- `doctor` diagnostics and parser/real Git/TestBackend-focused tests
- English UI by default, with instance-scoped Chinese/English selection through exact `-zh`/`-en` compatibility flags or standard `--zh`/`--en` flags
- terminal-column-aware sanitization, wrapping, and truncation for Git output and paths; control characters are rendered visibly and long diff source lines never auto-wrap across panel borders

Planned next: command palette and PTY takeover (M5), including interactive authentication and external mergetool/editor handoff.
