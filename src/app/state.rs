use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::adapters::git;
use crate::app::repository::{choices, form_for, RepositoryChoice, RepositoryForm, RepositoryTab};
use crate::domain::{
    ChangeEntry, ChangePreview, Commit, CommitOutcome, CommitSpec, HunkSource, OperationKind,
    OperationOutcome, OperationSpec, OperationTarget, Project, ProjectId, ProjectSnapshot,
    RepositoryAction, RepositoryActionOutcome, RepositoryActionSpec, RepositorySnapshot, RiskLevel,
    Workspace, WorkspaceSummary,
};
use crate::services::operations::OperationRunner;
use crate::services::scanner::{self, ScanResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Workspace,
    Graph,
    Changes,
    Repository,
}

#[derive(Debug)]
pub struct GraphResult {
    pub project_id: ProjectId,
    pub generation: u64,
    pub result: anyhow::Result<Vec<Commit>>,
}

#[derive(Debug)]
pub struct GraphState {
    pub project: Project,
    pub commits: Vec<Commit>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub generation: u64,
}

#[derive(Debug)]
pub struct ChangesResult {
    pub project_id: ProjectId,
    pub generation: u64,
    pub result: anyhow::Result<Vec<ChangeEntry>>,
}

#[derive(Debug)]
pub struct PreviewResult {
    pub project_id: ProjectId,
    pub changes_generation: u64,
    pub preview_generation: u64,
    pub path: PathBuf,
    pub result: anyhow::Result<ChangePreview>,
}

#[derive(Debug)]
pub struct OperationResult {
    pub project_id: ProjectId,
    pub changes_generation: u64,
    pub operation_generation: u64,
    pub result: anyhow::Result<OperationOutcome>,
}

#[derive(Debug)]
pub struct CommitResult {
    pub project_id: ProjectId,
    pub changes_generation: u64,
    pub commit_generation: u64,
    pub result: anyhow::Result<CommitOutcome>,
}

#[derive(Debug)]
pub struct RepositoryLoadResult {
    pub project_id: ProjectId,
    pub generation: u64,
    pub result: anyhow::Result<RepositorySnapshot>,
}

#[derive(Debug)]
pub struct RepositoryActionResult {
    pub project_id: ProjectId,
    pub generation: u64,
    pub action_generation: u64,
    pub result: anyhow::Result<RepositoryActionOutcome>,
}

#[derive(Debug)]
pub struct RepositoryState {
    pub project: Project,
    pub return_screen: Screen,
    pub snapshot: Option<RepositorySnapshot>,
    pub tab: RepositoryTab,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub generation: u64,
    pub action_menu: bool,
    pub action_selected: usize,
    pub form: Option<RepositoryForm>,
    pub pending: Option<RepositoryAction>,
    pub action_running: bool,
    pub action_generation: u64,
    pub message: Option<(bool, String)>,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Copy)]
pub enum CommitInput {
    Character(char),
    Backspace,
    ToggleAmend,
    ToggleSignoff,
    ToggleSigning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangesMode {
    File,
    Hunk,
    Line,
}

#[derive(Debug, Clone)]
pub struct PendingOperation {
    pub kind: OperationKind,
    pub change: ChangeEntry,
    pub expected_token: u64,
    pub target: OperationTarget,
}

#[derive(Debug)]
pub struct ChangesState {
    pub project: Project,
    pub return_screen: Screen,
    pub entries: Vec<ChangeEntry>,
    pub selected: usize,
    pub mode: ChangesMode,
    pub selected_hunk: usize,
    pub selected_hunk_identity: Option<(HunkSource, u64)>,
    pub selected_line: usize,
    pub selected_line_identity: Option<(HunkSource, u64, u64)>,
    pub loading: bool,
    pub error: Option<String>,
    pub generation: u64,
    pub preview: Option<ChangePreview>,
    pub preview_path: Option<PathBuf>,
    pub preview_loading: bool,
    pub preview_generation: u64,
    pub preview_scroll: usize,
    pub operation_running: bool,
    pub operation_generation: u64,
    pub confirmation: Option<PendingOperation>,
    pub message: Option<(bool, String)>,
    pub commit_message: String,
    pub commit_editing: bool,
    pub commit_amend: bool,
    pub commit_signoff: bool,
    pub commit_signing: bool,
    pub commit_running: bool,
    pub commit_generation: u64,
}

#[derive(Debug)]
pub struct App {
    pub workspace: Workspace,
    pub projects: Vec<ProjectSnapshot>,
    pub screen: Screen,
    pub selected: usize,
    pub search: String,
    pub search_mode: bool,
    pub help: bool,
    pub generation: u64,
    pub scanning: usize,
    pub graph: Option<GraphState>,
    pub changes: Option<ChangesState>,
    pub repository: Option<RepositoryState>,
    pub should_quit: bool,
    scan_tx: mpsc::UnboundedSender<ScanResult>,
    pub scan_rx: mpsc::UnboundedReceiver<ScanResult>,
    graph_tx: mpsc::UnboundedSender<GraphResult>,
    pub graph_rx: mpsc::UnboundedReceiver<GraphResult>,
    changes_tx: mpsc::UnboundedSender<ChangesResult>,
    pub changes_rx: mpsc::UnboundedReceiver<ChangesResult>,
    preview_tx: mpsc::UnboundedSender<PreviewResult>,
    pub preview_rx: mpsc::UnboundedReceiver<PreviewResult>,
    pub operation_tx: mpsc::UnboundedSender<OperationResult>,
    pub operation_rx: mpsc::UnboundedReceiver<OperationResult>,
    pub commit_tx: mpsc::UnboundedSender<CommitResult>,
    pub commit_rx: mpsc::UnboundedReceiver<CommitResult>,
    repository_tx: mpsc::UnboundedSender<RepositoryLoadResult>,
    pub repository_rx: mpsc::UnboundedReceiver<RepositoryLoadResult>,
    repository_action_tx: mpsc::UnboundedSender<RepositoryActionResult>,
    pub repository_action_rx: mpsc::UnboundedReceiver<RepositoryActionResult>,
    operation_runner: OperationRunner,
    concurrency: usize,
}
impl App {
    pub fn new(workspace: Workspace, concurrency: usize) -> Self {
        let (scan_tx, scan_rx) = mpsc::unbounded_channel();
        let (graph_tx, graph_rx) = mpsc::unbounded_channel();
        let (changes_tx, changes_rx) = mpsc::unbounded_channel();
        let (operation_tx, operation_rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();
        let (commit_tx, commit_rx) = mpsc::unbounded_channel();
        let (repository_tx, repository_rx) = mpsc::unbounded_channel();
        let (repository_action_tx, repository_action_rx) = mpsc::unbounded_channel();
        let projects = workspace
            .projects
            .iter()
            .cloned()
            .map(|project| ProjectSnapshot::pending(project, 0))
            .collect();
        Self {
            workspace,
            projects,
            screen: Screen::Workspace,
            selected: 0,
            search: String::new(),
            search_mode: false,
            help: false,
            generation: 0,
            scanning: 0,
            graph: None,
            changes: None,
            repository: None,
            should_quit: false,
            scan_tx,
            scan_rx,
            graph_tx,
            graph_rx,
            changes_tx,
            changes_rx,
            preview_tx,
            preview_rx,
            operation_tx,
            operation_rx,
            commit_tx,
            commit_rx,
            repository_tx,
            repository_rx,
            repository_action_tx,
            repository_action_rx,
            operation_runner: OperationRunner,
            concurrency: concurrency.max(1),
        }
    }

    pub fn refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.scanning = self.workspace.projects.len();
        self.projects = self
            .workspace
            .projects
            .iter()
            .cloned()
            .map(|project| ProjectSnapshot::pending(project, self.generation))
            .collect();
        scanner::spawn_scan(
            self.workspace.projects.clone(),
            self.generation,
            self.concurrency,
            self.scan_tx.clone(),
        );
        self.clamp_selection();
    }

    pub fn apply_scan(&mut self, result: ScanResult) {
        let snapshot = result.snapshot;
        if snapshot.generation != self.generation {
            return;
        }
        if let Some(slot) = self
            .projects
            .iter_mut()
            .find(|value| value.project.id == snapshot.project.id)
        {
            *slot = snapshot;
            self.scanning = self.scanning.saturating_sub(1);
        }
        self.clamp_selection();
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.search.to_lowercase();
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, snapshot)| {
                query.is_empty()
                    || snapshot.project.name.to_lowercase().contains(&query)
                    || snapshot
                        .project
                        .relative_path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
                    || snapshot.head.label().to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn selected_project(&self) -> Option<&ProjectSnapshot> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected)
            .and_then(|index| self.projects.get(*index))
    }

    pub fn move_selection(&mut self, delta: isize) {
        let len = self.filtered_indices().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected.min(len - 1) as isize;
        self.selected = (current + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.filtered_indices().len().saturating_sub(1);
    }

    pub fn open_graph(&mut self) {
        let Some(project) = self.selected_project().map(|value| value.project.clone()) else {
            return;
        };
        self.screen = Screen::Graph;
        self.load_graph(project);
    }

    pub fn reload_graph(&mut self) {
        if let Some(project) = self.graph.as_ref().map(|graph| graph.project.clone()) {
            self.load_graph(project);
        }
    }

    fn load_graph(&mut self, project: Project) {
        let generation = self
            .graph
            .as_ref()
            .map_or(1, |graph| graph.generation.wrapping_add(1));
        self.graph = Some(GraphState {
            project: project.clone(),
            commits: Vec::new(),
            selected: 0,
            loading: true,
            error: None,
            generation,
        });
        let sender = self.graph_tx.clone();
        tokio::spawn(async move {
            let result = git::log_all(&project.path).await;
            let _ = sender.send(GraphResult {
                project_id: project.id,
                generation,
                result,
            });
        });
    }

    pub fn apply_graph(&mut self, result: GraphResult) {
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        if graph.project.id != result.project_id || graph.generation != result.generation {
            return;
        }
        graph.loading = false;
        match result.result {
            Ok(commits) => graph.commits = commits,
            Err(error) => graph.error = Some(error.to_string()),
        }
        graph.selected = graph.selected.min(graph.commits.len().saturating_sub(1));
    }

    pub fn move_graph_selection(&mut self, delta: isize) {
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        if graph.commits.is_empty() {
            graph.selected = 0;
            return;
        }
        graph.selected =
            (graph.selected as isize + delta).clamp(0, graph.commits.len() as isize - 1) as usize;
    }

    pub fn graph_first(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            graph.selected = 0;
        }
    }

    pub fn graph_last(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            graph.selected = graph.commits.len().saturating_sub(1);
        }
    }

    pub fn open_changes(&mut self) {
        let return_screen = self.screen;
        let project = match self.screen {
            Screen::Workspace => self.selected_project().map(|value| value.project.clone()),
            Screen::Graph => self.graph.as_ref().map(|graph| graph.project.clone()),
            Screen::Changes => self.changes.as_ref().map(|changes| changes.project.clone()),
            Screen::Repository => self.repository.as_ref().map(|state| state.project.clone()),
        };
        if let Some(project) = project {
            self.screen = Screen::Changes;
            self.load_changes(project, return_screen);
        }
    }

    pub fn reload_changes(&mut self) {
        if let Some(changes) = self.changes.as_ref() {
            let project = changes.project.clone();
            let return_screen = changes.return_screen;
            self.load_changes(project, return_screen);
        }
    }

    fn load_changes(&mut self, project: Project, return_screen: Screen) {
        let generation = self
            .changes
            .as_ref()
            .filter(|changes| changes.project.id == project.id)
            .map_or(1, |changes| changes.generation.wrapping_add(1));
        self.changes = Some(ChangesState {
            project: project.clone(),
            return_screen,
            entries: Vec::new(),
            selected: 0,
            mode: ChangesMode::File,
            selected_hunk: 0,
            selected_hunk_identity: None,
            selected_line: 0,
            selected_line_identity: None,
            loading: true,
            error: None,
            preview_loading: false,
            preview_generation: 0,
            preview_scroll: 0,
            generation,
            preview: None,
            preview_path: None,
            operation_running: false,
            operation_generation: 0,
            confirmation: None,
            message: None,
            commit_message: String::new(),
            commit_editing: false,
            commit_amend: false,
            commit_signoff: false,
            commit_signing: false,
            commit_running: false,
            commit_generation: 0,
        });
        let sender = self.changes_tx.clone();
        tokio::spawn(async move {
            let result = git::changes(&project.path).await;
            let _ = sender.send(ChangesResult {
                project_id: project.id,
                generation,
                result,
            });
        });
    }

    pub fn apply_changes(&mut self, result: ChangesResult) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.project.id != result.project_id || changes.generation != result.generation {
            return;
        }
        changes.loading = false;
        match result.result {
            Ok(entries) => {
                changes.entries = entries;
                changes.selected = changes
                    .selected
                    .min(changes.entries.len().saturating_sub(1));
            }
            Err(error) => changes.error = Some(error.to_string()),
        }
        self.request_selected_preview();
    }

    pub fn selected_change(&self) -> Option<&ChangeEntry> {
        self.changes
            .as_ref()
            .and_then(|changes| changes.entries.get(changes.selected))
    }

    pub fn move_change_selection(&mut self, delta: isize) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.operation_running {
            return;
        }
        if changes.mode != ChangesMode::File {
            let item_count = changes
                .preview
                .as_ref()
                .map_or(0, |preview| match changes.mode {
                    ChangesMode::Hunk => preview.hunks.len(),
                    ChangesMode::Line => preview.lines.len(),
                    ChangesMode::File => 0,
                });
            if item_count == 0 {
                return;
            }
            match changes.mode {
                ChangesMode::Hunk => {
                    changes.selected_hunk = (changes.selected_hunk as isize + delta)
                        .clamp(0, item_count as isize - 1)
                        as usize
                }
                ChangesMode::Line => {
                    changes.selected_line = (changes.selected_line as isize + delta)
                        .clamp(0, item_count as isize - 1)
                        as usize
                }
                ChangesMode::File => unreachable!(),
            }
            sync_preview_selection(changes);
            changes.confirmation = None;
            changes.message = None;
            return;
        }
        if changes.entries.is_empty() {
            return;
        }
        changes.selected = (changes.selected as isize + delta)
            .clamp(0, changes.entries.len() as isize - 1) as usize;
        reset_hunk_selection(changes);
        changes.confirmation = None;
        changes.preview_scroll = 0;
        changes.message = None;
        self.request_selected_preview();
    }

    pub fn changes_first(&mut self) {
        if let Some(changes) = self.changes.as_mut() {
            if changes.mode != ChangesMode::File {
                match changes.mode {
                    ChangesMode::Hunk => changes.selected_hunk = 0,
                    ChangesMode::Line => changes.selected_line = 0,
                    ChangesMode::File => unreachable!(),
                }
                sync_preview_selection(changes);
                changes.confirmation = None;
                changes.message = None;
                return;
            }
            changes.selected = 0;
            reset_hunk_selection(changes);
            changes.confirmation = None;
            changes.preview_scroll = 0;
            changes.message = None;
        }
        self.request_selected_preview();
    }

    pub fn changes_last(&mut self) {
        if let Some(changes) = self.changes.as_mut() {
            if changes.mode != ChangesMode::File {
                match changes.mode {
                    ChangesMode::Hunk => {
                        changes.selected_hunk = changes
                            .preview
                            .as_ref()
                            .map_or(0, |preview| preview.hunks.len().saturating_sub(1));
                    }
                    ChangesMode::Line => {
                        changes.selected_line = changes
                            .preview
                            .as_ref()
                            .map_or(0, |preview| preview.lines.len().saturating_sub(1));
                    }
                    ChangesMode::File => unreachable!(),
                }
                sync_preview_selection(changes);
                changes.confirmation = None;
                changes.message = None;
                return;
            }
            changes.selected = changes.entries.len().saturating_sub(1);
            reset_hunk_selection(changes);
            changes.confirmation = None;
            changes.preview_scroll = 0;
            changes.message = None;
        }
        self.request_selected_preview();
    }

    pub fn toggle_changes_mode(&mut self) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.operation_running {
            return;
        }
        match changes.mode {
            ChangesMode::File => {
                let Some(preview) = changes.preview.as_ref() else {
                    changes.message = Some((true, "Wait for the selected diff to load".to_owned()));
                    return;
                };
                if preview.hunks.is_empty() {
                    changes.message = Some((
                        true,
                        "No selectable textual hunks are available for this file".to_owned(),
                    ));
                    return;
                }
                changes.mode = ChangesMode::Hunk;
                changes.selected_hunk = changes
                    .selected_hunk
                    .min(preview.hunks.len().saturating_sub(1));
                sync_preview_selection(changes);
            }
            ChangesMode::Hunk => {
                let Some(preview) = changes.preview.as_ref() else {
                    return;
                };
                if preview.lines.is_empty() {
                    changes.mode = ChangesMode::File;
                    changes.message = Some((
                        true,
                        "No selectable changed lines are available for this file".to_owned(),
                    ));
                } else {
                    changes.mode = ChangesMode::Line;
                    changes.selected_line = changes
                        .selected_line
                        .min(preview.lines.len().saturating_sub(1));
                    sync_preview_selection(changes);
                }
            }
            ChangesMode::Line => {
                changes.mode = ChangesMode::File;
                changes.confirmation = None;
                changes.message = None;
            }
        }
    }

    fn request_selected_preview(&mut self) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        let Some(entry) = changes.entries.get(changes.selected).cloned() else {
            changes.preview = None;
            changes.preview_path = None;
            changes.preview_loading = false;
            return;
        };
        changes.preview_generation = changes.preview_generation.wrapping_add(1);
        let preview_generation = changes.preview_generation;
        let changes_generation = changes.generation;
        let project = changes.project.clone();
        if changes.preview_path.as_ref() == Some(&entry.path) {
            changes.selected_hunk_identity = changes.preview.as_ref().and_then(|preview| {
                preview
                    .hunks
                    .get(changes.selected_hunk)
                    .map(|hunk| (hunk.source, hunk.fingerprint))
            });
        }
        changes.preview = None;
        changes.preview_path = Some(entry.path.clone());
        changes.preview_scroll = 0;
        changes.preview_loading = true;
        let sender = self.preview_tx.clone();
        tokio::spawn(async move {
            let result = git::preview_change(&project.path, &entry).await;
            let _ = sender.send(PreviewResult {
                project_id: project.id,
                changes_generation,
                preview_generation,
                path: entry.path,
                result,
            });
        });
    }

    pub fn apply_preview(&mut self, result: PreviewResult) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.project.id != result.project_id
            || changes.generation != result.changes_generation
            || changes.preview_generation != result.preview_generation
            || changes.preview_path.as_ref() != Some(&result.path)
        {
            return;
        }
        changes.preview_loading = false;
        match result.result {
            Ok(preview) => {
                changes.selected_hunk = changes
                    .selected_hunk_identity
                    .and_then(|identity| {
                        preview
                            .hunks
                            .iter()
                            .position(|hunk| (hunk.source, hunk.fingerprint) == identity)
                    })
                    .unwrap_or_else(|| {
                        changes
                            .selected_hunk
                            .min(preview.hunks.len().saturating_sub(1))
                    });
                changes.selected_line = changes
                    .selected_line_identity
                    .and_then(|identity| {
                        preview.lines.iter().position(|line| {
                            (line.source, line.hunk_fingerprint, line.fingerprint) == identity
                        })
                    })
                    .unwrap_or_else(|| {
                        changes
                            .selected_line
                            .min(preview.lines.len().saturating_sub(1))
                    });
                if (changes.mode == ChangesMode::Hunk && preview.hunks.is_empty())
                    || (changes.mode == ChangesMode::Line && preview.lines.is_empty())
                {
                    changes.mode = ChangesMode::File;
                    changes.message = Some((
                        true,
                        "The selected diff scope is no longer available".to_owned(),
                    ));
                }
                changes.preview = Some(preview);
                sync_preview_selection(changes);
            }
            Err(error) => changes.message = Some((true, error.to_string())),
        }
    }

    pub fn scroll_preview(&mut self, delta: isize) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        let line_count = changes
            .preview
            .as_ref()
            .map_or(0, |preview| preview.text.lines().count());
        let current = changes.preview_scroll as isize;
        changes.preview_scroll =
            (current + delta).clamp(0, line_count.saturating_sub(1) as isize) as usize;
    }

    pub fn begin_operation(&mut self, kind: OperationKind) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.operation_running {
            return;
        }
        let Some(change) = changes.entries.get(changes.selected).cloned() else {
            return;
        };
        let Some(preview) = changes.preview.as_ref() else {
            changes.message = Some((true, "Wait for the selected diff to load".to_owned()));
            return;
        };
        if changes.preview_path.as_ref() != Some(&change.path) {
            changes.message = Some((true, "Selected diff is stale; refresh and retry".to_owned()));
            return;
        }
        let (target, applicable, scope) = match changes.mode {
            ChangesMode::File => {
                let applicable = match kind {
                    OperationKind::Stage => change.can_stage(),
                    OperationKind::Unstage => change.can_unstage(),
                    OperationKind::RestoreWorktree => change.can_restore(),
                };
                (OperationTarget::File, applicable, "file")
            }
            ChangesMode::Hunk => {
                let Some(hunk) = preview.hunks.get(changes.selected_hunk) else {
                    changes.message = Some((true, "Selected hunk is unavailable".to_owned()));
                    return;
                };
                (
                    OperationTarget::Hunk {
                        source: hunk.source,
                        fingerprint: hunk.fingerprint,
                    },
                    hunk_operation_applicable(kind, hunk.source, &change),
                    "hunk",
                )
            }
            ChangesMode::Line => {
                let Some(line) = preview.lines.get(changes.selected_line) else {
                    changes.message = Some((true, "Selected line is unavailable".to_owned()));
                    return;
                };
                (
                    OperationTarget::Line {
                        source: line.source,
                        hunk_fingerprint: line.hunk_fingerprint,
                        fingerprint: line.fingerprint,
                    },
                    hunk_operation_applicable(kind, line.source, &change),
                    "line",
                )
            }
        };
        if !applicable {
            changes.message = Some((
                true,
                format!("{} is not available for this {scope}", kind.label()),
            ));
            return;
        }
        let pending = PendingOperation {
            kind,
            target,
            change,
            expected_token: preview.token,
        };
        if kind.risk() == RiskLevel::Destructive {
            changes.confirmation = Some(pending);
        } else {
            self.spawn_operation(pending);
        }
    }

    pub fn confirm_operation(&mut self, accepted: bool) {
        let pending = self
            .changes
            .as_mut()
            .and_then(|changes| changes.confirmation.take());
        if accepted {
            if let Some(pending) = pending {
                self.spawn_operation(pending);
            }
        }
    }

    pub fn start_commit_editing(&mut self) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.commit_running || changes.operation_running {
            return;
        }
        changes.commit_editing = true;
        changes.message = None;
    }

    pub fn cancel_commit_editing(&mut self) {
        if let Some(changes) = self.changes.as_mut() {
            changes.commit_editing = false;
            changes.message = None;
        }
    }

    pub fn edit_commit_message(&mut self, input: CommitInput) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if !changes.commit_editing || changes.commit_running {
            return;
        }
        match input {
            CommitInput::Character(value) => changes.commit_message.push(value),
            CommitInput::Backspace => {
                changes.commit_message.pop();
            }
            CommitInput::ToggleAmend => changes.commit_amend = !changes.commit_amend,
            CommitInput::ToggleSignoff => changes.commit_signoff = !changes.commit_signoff,
            CommitInput::ToggleSigning => changes.commit_signing = !changes.commit_signing,
        }
    }

    pub fn submit_commit(&mut self) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if !changes.commit_editing || changes.commit_running {
            return;
        }
        if changes.commit_message.trim().is_empty() {
            changes.message = Some((true, "Commit message cannot be empty".to_owned()));
            return;
        }
        changes.commit_editing = false;
        changes.commit_running = true;
        changes.message = None;
        changes.commit_generation = changes.commit_generation.wrapping_add(1);
        let commit_generation = changes.commit_generation;
        let changes_generation = changes.generation;
        let project = changes.project.clone();
        let project_id = project.id.clone();
        let spec = CommitSpec {
            message: changes.commit_message.clone(),
            amend: changes.commit_amend,
            signoff: changes.commit_signoff,
            signing: changes.commit_signing,
        };
        let runner = self.operation_runner.clone();
        let sender = self.commit_tx.clone();
        tokio::spawn(async move {
            let result = runner.execute_commit(project, spec).await;
            let _ = sender.send(CommitResult {
                project_id,
                changes_generation,
                commit_generation,
                result,
            });
        });
    }

    pub fn apply_commit(&mut self, result: CommitResult) {
        let Some(changes) = self.changes.as_ref() else {
            return;
        };
        if changes.project.id != result.project_id
            || changes.generation != result.changes_generation
            || changes.commit_generation != result.commit_generation
        {
            return;
        }
        match result.result {
            Ok(outcome) => {
                let project = changes.project.clone();
                let return_screen = changes.return_screen;
                self.refresh();
                self.load_changes(project, return_screen);
                if let Some(changes) = self.changes.as_mut() {
                    changes.message = Some((false, outcome.message));
                }
            }
            Err(error) => {
                if let Some(changes) = self.changes.as_mut() {
                    changes.commit_running = false;
                    changes.commit_editing = true;
                    changes.message = Some((true, error.to_string()));
                }
            }
        }
    }
    fn spawn_operation(&mut self, pending: PendingOperation) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        changes.operation_running = true;
        changes.message = None;
        changes.operation_generation = changes.operation_generation.wrapping_add(1);
        let operation_generation = changes.operation_generation;
        let changes_generation = changes.generation;
        let project = changes.project.clone();
        let project_id = project.id.clone();
        let sender = self.operation_tx.clone();
        let runner = self.operation_runner.clone();
        tokio::spawn(async move {
            let result = runner
                .execute(OperationSpec {
                    project,
                    change: pending.change,
                    kind: pending.kind,
                    target: pending.target,
                    expected_token: pending.expected_token,
                })
                .await;
            let _ = sender.send(OperationResult {
                project_id,
                operation_generation,
                changes_generation,
                result,
            });
        });
    }

    pub fn apply_operation(&mut self, result: OperationResult) {
        let Some(changes) = self.changes.as_ref() else {
            return;
        };
        if changes.project.id != result.project_id
            || changes.operation_generation != result.operation_generation
            || changes.generation != result.changes_generation
        {
            return;
        }
        match result.result {
            Err(error) => {
                if let Some(changes) = self.changes.as_mut() {
                    changes.operation_running = false;
                    changes.message = Some((true, error.to_string()));
                }
            }
            Ok(outcome) => {
                let project = changes.project.clone();
                let return_screen = changes.return_screen;
                self.refresh();
                self.load_changes(project, return_screen);
                if let Some(changes) = self.changes.as_mut() {
                    changes.message = Some((false, outcome.message));
                }
            }
        }
    }

    pub fn open_repository(&mut self) {
        let return_screen = self.screen;
        let project = match self.screen {
            Screen::Workspace => self.selected_project().map(|value| value.project.clone()),
            Screen::Graph => self.graph.as_ref().map(|state| state.project.clone()),
            Screen::Changes => self.changes.as_ref().map(|state| state.project.clone()),
            Screen::Repository => self.repository.as_ref().map(|state| state.project.clone()),
        };
        if let Some(project) = project {
            self.screen = Screen::Repository;
            self.load_repository(project, return_screen);
        }
    }

    pub fn reload_repository(&mut self) {
        if let Some(state) = self.repository.as_ref() {
            self.load_repository(state.project.clone(), state.return_screen);
        }
    }

    fn load_repository(&mut self, project: Project, return_screen: Screen) {
        let generation = self
            .repository
            .as_ref()
            .filter(|state| state.project.id == project.id)
            .map_or(1, |state| state.generation.wrapping_add(1));
        let tab = self
            .repository
            .as_ref()
            .map_or(RepositoryTab::Status, |state| state.tab);
        self.repository = Some(RepositoryState {
            project: project.clone(),
            return_screen,
            snapshot: None,
            tab,
            selected: 0,
            loading: true,
            error: None,
            generation,
            action_menu: false,
            action_selected: 0,
            form: None,
            pending: None,
            action_running: false,
            action_generation: 0,
            message: None,
            detail: None,
        });
        let sender = self.repository_tx.clone();
        tokio::spawn(async move {
            let result = git::repository_snapshot(&project.path).await;
            let _ = sender.send(RepositoryLoadResult {
                project_id: project.id,
                generation,
                result,
            });
        });
    }

    pub fn apply_repository_load(&mut self, result: RepositoryLoadResult) {
        let Some(state) = self.repository.as_mut() else {
            return;
        };
        if state.project.id != result.project_id || state.generation != result.generation {
            return;
        }
        state.loading = false;
        match result.result {
            Ok(snapshot) => state.snapshot = Some(snapshot),
            Err(error) => state.error = Some(error.to_string()),
        }
        clamp_repository_selection(state);
    }

    pub fn next_repository_tab(&mut self, delta: isize) {
        let Some(state) = self.repository.as_mut() else {
            return;
        };
        if state.action_running || state.form.is_some() || state.action_menu {
            return;
        }
        let current = RepositoryTab::ALL
            .iter()
            .position(|tab| *tab == state.tab)
            .unwrap_or(0);
        let next =
            (current as isize + delta).rem_euclid(RepositoryTab::ALL.len() as isize) as usize;
        state.tab = RepositoryTab::ALL[next];
        state.selected = 0;
        state.detail = None;
    }

    pub fn move_repository_selection(&mut self, delta: isize) {
        let Some(state) = self.repository.as_mut() else {
            return;
        };
        if let Some(form) = state.form.as_mut() {
            let len = form.fields.len();
            if len > 0 {
                form.selected =
                    (form.selected as isize + delta).clamp(0, len as isize - 1) as usize;
            }
        } else if state.action_menu {
            let len = choices(state.tab).len();
            if len > 0 {
                state.action_selected =
                    (state.action_selected as isize + delta).clamp(0, len as isize - 1) as usize;
            }
        } else {
            let len = repository_item_count(state);
            if len > 0 {
                state.selected =
                    (state.selected as isize + delta).clamp(0, len as isize - 1) as usize;
            }
        }
    }

    pub fn toggle_repository_action_menu(&mut self) {
        let Some(state) = self.repository.as_mut() else {
            return;
        };
        if state.action_running || state.loading || state.snapshot.is_none() {
            return;
        }
        state.action_menu = !state.action_menu;
        state.action_selected = 0;
        state.form = None;
        state.message = None;
    }

    pub fn select_repository_action(&mut self) {
        let Some(state) = self.repository.as_mut() else {
            return;
        };
        let Some(snapshot) = state.snapshot.as_ref() else {
            return;
        };
        let Some(choice) = choices(state.tab).get(state.action_selected).copied() else {
            return;
        };
        if matches!(
            choice,
            RepositoryChoice::Continue | RepositoryChoice::Skip | RepositoryChoice::Abort
        ) {
            let Some(operation) = snapshot.operation else {
                state.message = Some((true, "No Git operation is active".to_owned()));
                return;
            };
            let action = match choice {
                RepositoryChoice::Continue => RepositoryAction::Continue { operation },
                RepositoryChoice::Skip => RepositoryAction::Skip { operation },
                RepositoryChoice::Abort => RepositoryAction::Abort { operation },
                _ => unreachable!(),
            };
            state.action_menu = false;
            self.begin_repository_action(action);
            return;
        }
        match form_for(choice, snapshot, state.selected) {
            Ok(Some(form)) => {
                state.form = Some(form);
                state.action_menu = false;
            }
            Ok(None) => {}
            Err(error) => state.message = Some((true, error.to_string())),
        }
    }

    pub fn edit_repository_form(&mut self, input: CommitInput) {
        let Some(form) = self
            .repository
            .as_mut()
            .and_then(|state| state.form.as_mut())
        else {
            return;
        };
        match input {
            CommitInput::Character(value) => form.edit_char(value),
            CommitInput::Backspace => form.backspace(),
            CommitInput::ToggleAmend | CommitInput::ToggleSignoff | CommitInput::ToggleSigning => {
                form.toggle()
            }
        }
    }

    pub fn cancel_repository_overlay(&mut self) {
        if let Some(state) = self.repository.as_mut() {
            if state.pending.take().is_some() {
                return;
            }
            if state.form.take().is_some() {
                return;
            }
            state.action_menu = false;
        }
    }

    pub fn submit_repository_form(&mut self) {
        let action = self
            .repository
            .as_ref()
            .and_then(|state| state.form.as_ref())
            .map(RepositoryForm::action);
        match action {
            Some(Ok(action)) => {
                if let Some(state) = self.repository.as_mut() {
                    state.form = None;
                }
                self.begin_repository_action(action);
            }
            Some(Err(error)) => {
                if let Some(state) = self.repository.as_mut() {
                    state.message = Some((true, error.to_string()));
                }
            }
            None => {}
        }
    }

    fn begin_repository_action(&mut self, action: RepositoryAction) {
        let risk = action.risk();
        if matches!(risk, RiskLevel::Destructive | RiskLevel::RemoteWrite) {
            if let Some(state) = self.repository.as_mut() {
                state.pending = Some(action);
            }
        } else {
            self.spawn_repository_action(action);
        }
    }

    pub fn confirm_repository_action(&mut self, accepted: bool) {
        let action = self
            .repository
            .as_mut()
            .and_then(|state| state.pending.take());
        if accepted {
            if let Some(action) = action {
                self.spawn_repository_action(action);
            }
        }
    }

    fn spawn_repository_action(&mut self, action: RepositoryAction) {
        let Some(state) = self.repository.as_mut() else {
            return;
        };
        state.action_running = true;
        state.message = None;
        state.detail = None;
        state.action_generation = state.action_generation.wrapping_add(1);
        let action_generation = state.action_generation;
        let generation = state.generation;
        let project = state.project.clone();
        let project_id = project.id.clone();
        let sender = self.repository_action_tx.clone();
        let runner = self.operation_runner.clone();
        let expected_token = state.snapshot.as_ref().map_or(0, |snapshot| snapshot.token);
        tokio::spawn(async move {
            let result = runner
                .execute_repository_action(RepositoryActionSpec {
                    project,
                    action,
                    expected_token,
                })
                .await;
            let _ = sender.send(RepositoryActionResult {
                project_id,
                generation,
                action_generation,
                result,
            });
        });
    }

    pub fn apply_repository_action(&mut self, result: RepositoryActionResult) {
        let Some(state) = self.repository.as_ref() else {
            return;
        };
        if state.project.id != result.project_id
            || state.generation != result.generation
            || state.action_generation != result.action_generation
        {
            return;
        }
        match result.result {
            Ok(outcome) => {
                let project = state.project.clone();
                let return_screen = state.return_screen;
                let detail = outcome.detail;
                self.refresh();
                self.load_repository(project, return_screen);
                if let Some(state) = self.repository.as_mut() {
                    state.message = Some((false, outcome.message));
                    state.detail = detail;
                }
            }
            Err(error) => {
                if let Some(state) = self.repository.as_mut() {
                    state.action_running = false;
                    state.message = Some((true, error.to_string()));
                }
            }
        }
    }
    pub fn back(&mut self) {
        match self.screen {
            Screen::Repository => {
                if let Some(state) = self.repository.as_mut() {
                    if state.pending.take().is_some()
                        || state.form.take().is_some()
                        || state.action_menu
                    {
                        state.action_menu = false;
                        return;
                    }
                    if state.action_running {
                        state.message = Some((true, "Wait for the operation to finish".to_owned()));
                        return;
                    }
                    self.screen = state.return_screen;
                } else {
                    self.screen = Screen::Workspace;
                }
            }
            Screen::Graph => self.screen = Screen::Workspace,
            Screen::Changes => {
                if let Some(changes) = self.changes.as_mut() {
                    if changes.confirmation.take().is_some() {
                        return;
                    }
                    if changes.operation_running {
                        changes.message =
                            Some((true, "Wait for the operation to finish".to_owned()));
                        return;
                    }
                    self.screen = changes.return_screen;
                } else {
                    self.screen = Screen::Workspace;
                }
            }
            Screen::Workspace if self.search_mode => self.search_mode = false,
            Screen::Workspace if !self.search.is_empty() => {
                self.search.clear();
                self.selected = 0;
            }
            Screen::Workspace => self.should_quit = true,
        }
    }

    pub fn summary(&self) -> WorkspaceSummary {
        WorkspaceSummary::from_projects(&self.projects)
    }

    pub fn workspace_label(&self) -> String {
        self.workspace
            .root
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.workspace.root.display().to_string())
    }

    pub fn project_path(&self) -> PathBuf {
        self.selected_project()
            .map(|value| value.project.path.clone())
            .unwrap_or_else(|| self.workspace.root.clone())
    }

    fn clamp_selection(&mut self) {
        self.selected = self
            .selected
            .min(self.filtered_indices().len().saturating_sub(1));
    }
}

fn repository_item_count(state: &RepositoryState) -> usize {
    let Some(snapshot) = state.snapshot.as_ref() else {
        return 0;
    };
    match state.tab {
        RepositoryTab::Status => snapshot.conflicts.len().max(1),
        RepositoryTab::Stashes => snapshot.stashes.len(),
        RepositoryTab::Refs => snapshot.branches.len() + snapshot.tags.len(),
        RepositoryTab::Remotes => snapshot.remotes.len(),
    }
}

fn clamp_repository_selection(state: &mut RepositoryState) {
    state.selected = state
        .selected
        .min(repository_item_count(state).saturating_sub(1));
}
fn reset_hunk_selection(changes: &mut ChangesState) {
    changes.selected_hunk = 0;
    changes.selected_hunk_identity = None;
    changes.selected_line = 0;
    changes.selected_line_identity = None;
}

fn sync_preview_selection(changes: &mut ChangesState) {
    match changes.mode {
        ChangesMode::File => {}
        ChangesMode::Hunk => {
            let Some(hunk) = changes
                .preview
                .as_ref()
                .and_then(|preview| preview.hunks.get(changes.selected_hunk))
            else {
                changes.selected_hunk_identity = None;
                return;
            };
            changes.selected_hunk_identity = Some((hunk.source, hunk.fingerprint));
            changes.preview_scroll = hunk.display_start.saturating_sub(1);
        }
        ChangesMode::Line => {
            let Some(line) = changes
                .preview
                .as_ref()
                .and_then(|preview| preview.lines.get(changes.selected_line))
            else {
                changes.selected_line_identity = None;
                return;
            };
            changes.selected_line_identity =
                Some((line.source, line.hunk_fingerprint, line.fingerprint));
            changes.preview_scroll = line.display_line.saturating_sub(1);
        }
    }
}

fn hunk_operation_applicable(
    kind: OperationKind,
    source: HunkSource,
    change: &ChangeEntry,
) -> bool {
    match (source, kind) {
        (HunkSource::Staged, OperationKind::Unstage) => change.index.is_some(),
        (HunkSource::Worktree, OperationKind::Stage) => {
            change.worktree.is_some() || change.conflicted
        }
        (HunkSource::Worktree, OperationKind::RestoreWorktree) => change.can_restore(),
        (HunkSource::Untracked, OperationKind::Stage) => change.untracked,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::{ChangeHunk, WorkspaceKind, WorktreeSummary};

    fn project(name: &str) -> Project {
        let path = PathBuf::from(format!("/tmp/{name}"));
        Project {
            id: ProjectId(path.clone()),
            name: name.into(),
            path,
            relative_path: PathBuf::from(name),
        }
    }

    #[test]
    fn filters_and_clamps_selection() {
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Repo,
            projects: vec![project("alpha"), project("beta")],
        };
        let mut app = App::new(workspace, 2);
        app.projects[0].worktree = WorktreeSummary {
            untracked: 1,
            ..WorktreeSummary::default()
        };
        app.select_last();
        assert_eq!(app.selected, 1);
        app.search = "alpha".into();
        app.move_selection(0);
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_project().unwrap().project.name, "alpha");
    }

    #[test]
    fn ignores_stale_scan_results() {
        let value = project("alpha");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Git,
            projects: vec![value.clone()],
        };
        let mut app = App::new(workspace, 1);
        app.generation = 2;
        let mut stale = ProjectSnapshot::pending(value, 1);
        stale.worktree.untracked = 4;
        app.apply_scan(ScanResult { snapshot: stale });
        assert_eq!(app.projects[0].worktree.untracked, 0);
    }

    #[test]
    fn ignores_stale_change_and_preview_results() {
        let value = project("alpha");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Git,
            projects: vec![value.clone()],
        };
        let mut app = App::new(workspace, 1);
        app.screen = Screen::Changes;
        app.changes = Some(ChangesState {
            project: value.clone(),
            return_screen: Screen::Workspace,
            entries: Vec::new(),
            selected: 0,
            mode: ChangesMode::File,
            selected_hunk: 0,
            selected_hunk_identity: None,
            selected_line: 0,
            selected_line_identity: None,
            loading: true,
            error: None,
            generation: 2,
            preview: None,
            preview_path: None,
            preview_loading: false,
            preview_generation: 3,
            preview_scroll: 0,
            commit_message: String::new(),
            commit_editing: false,
            commit_amend: false,
            commit_signoff: false,
            commit_signing: false,
            commit_running: false,
            commit_generation: 0,
            operation_running: false,
            operation_generation: 0,
            confirmation: None,
            message: None,
        });
        app.apply_changes(ChangesResult {
            project_id: value.id.clone(),
            generation: 1,
            result: Ok(Vec::new()),
        });
        assert!(app.changes.as_ref().unwrap().loading);
        app.apply_preview(PreviewResult {
            project_id: value.id,
            changes_generation: 2,
            preview_generation: 2,
            path: PathBuf::from("old.txt"),
            result: Ok(ChangePreview {
                text: "old".into(),
                token: 1,
                truncated: false,
                hunks: Vec::new(),
                lines: Vec::new(),
            }),
        });
        assert!(app.changes.as_ref().unwrap().preview.is_none());
    }

    #[test]
    fn hunk_mode_navigates_builds_target_and_preserves_failed_selection() {
        let value = project("alpha");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Git,
            projects: vec![value.clone()],
        };
        let mut app = App::new(workspace, 1);
        let entry = ChangeEntry {
            path: PathBuf::from("tracked.txt"),
            original_path: None,
            index: None,
            worktree: Some(crate::domain::ChangeCode::Modified),
            untracked: false,
            conflicted: false,
        };
        app.screen = Screen::Changes;
        app.changes = Some(ChangesState {
            project: value.clone(),
            return_screen: Screen::Workspace,
            entries: vec![entry.clone()],
            selected: 0,
            mode: ChangesMode::File,
            selected_hunk: 0,
            selected_hunk_identity: None,
            selected_line: 0,
            selected_line_identity: None,
            loading: false,
            error: None,
            generation: 4,
            preview: Some(ChangePreview {
                text: "@@ -1 +1 @@\n-a\n+A\n@@ -9 +9 @@\n-b\n+B\n".into(),
                token: 42,
                truncated: false,
                hunks: vec![
                    ChangeHunk {
                        source: HunkSource::Worktree,
                        header: "@@ -1 +1 @@".into(),
                        display_start: 0,
                        display_end: 2,
                        fingerprint: 11,
                    },
                    ChangeHunk {
                        source: HunkSource::Worktree,
                        header: "@@ -9 +9 @@".into(),
                        display_start: 3,
                        display_end: 5,
                        fingerprint: 22,
                    },
                ],
                lines: Vec::new(),
            }),
            preview_path: Some(entry.path.clone()),
            preview_loading: false,
            preview_generation: 1,
            preview_scroll: 0,
            commit_message: String::new(),
            commit_editing: false,
            commit_amend: false,
            commit_signoff: false,
            commit_signing: false,
            commit_running: false,
            commit_generation: 0,
            operation_running: false,
            operation_generation: 0,
            confirmation: None,
            message: None,
        });

        app.toggle_changes_mode();
        assert_eq!(app.changes.as_ref().unwrap().mode, ChangesMode::Hunk);
        app.move_change_selection(1);
        let changes = app.changes.as_ref().unwrap();
        assert_eq!(changes.selected_hunk, 1);
        assert_eq!(changes.preview_scroll, 2);
        app.begin_operation(OperationKind::RestoreWorktree);
        let pending = app.changes.as_ref().unwrap().confirmation.as_ref().unwrap();
        assert_eq!(
            pending.target,
            OperationTarget::Hunk {
                source: HunkSource::Worktree,
                fingerprint: 22,
            }
        );
        app.confirm_operation(false);
        app.changes.as_mut().unwrap().operation_running = true;
        app.changes.as_mut().unwrap().operation_generation = 9;
        app.apply_operation(OperationResult {
            project_id: value.id,
            changes_generation: 4,
            operation_generation: 9,
            result: Err(anyhow::anyhow!("patch failed")),
        });
        let changes = app.changes.as_ref().unwrap();
        assert!(!changes.operation_running);
        assert_eq!(changes.selected_hunk, 1);
        assert_eq!(changes.mode, ChangesMode::Hunk);
        assert!(changes
            .message
            .as_ref()
            .is_some_and(|(error, message)| *error && message == "patch failed"));
    }
}
