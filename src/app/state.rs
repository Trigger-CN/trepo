use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::adapters::git;
use crate::domain::{
    ChangeEntry, ChangeHunk, ChangePreview, Commit, HunkSource, OperationKind, OperationOutcome,
    OperationSpec, OperationTarget, Project, ProjectId, ProjectSnapshot, RiskLevel, Workspace,
    WorkspaceSummary,
};
use crate::services::operations::OperationRunner;
use crate::services::scanner::{self, ScanResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Workspace,
    Graph,
    Changes,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangesMode {
    File,
    Hunk,
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
    pub should_quit: bool,
    scan_tx: mpsc::UnboundedSender<ScanResult>,
    pub scan_rx: mpsc::UnboundedReceiver<ScanResult>,
    graph_tx: mpsc::UnboundedSender<GraphResult>,
    pub graph_rx: mpsc::UnboundedReceiver<GraphResult>,
    changes_tx: mpsc::UnboundedSender<ChangesResult>,
    pub changes_rx: mpsc::UnboundedReceiver<ChangesResult>,
    preview_tx: mpsc::UnboundedSender<PreviewResult>,
    pub preview_rx: mpsc::UnboundedReceiver<PreviewResult>,
    operation_tx: mpsc::UnboundedSender<OperationResult>,
    pub operation_rx: mpsc::UnboundedReceiver<OperationResult>,
    operation_runner: OperationRunner,
    concurrency: usize,
}

impl App {
    pub fn new(workspace: Workspace, concurrency: usize) -> Self {
        let (scan_tx, scan_rx) = mpsc::unbounded_channel();
        let (graph_tx, graph_rx) = mpsc::unbounded_channel();
        let (changes_tx, changes_rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();
        let (operation_tx, operation_rx) = mpsc::unbounded_channel();
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
            loading: true,
            error: None,
            generation,
            preview: None,
            preview_path: None,
            preview_loading: false,
            preview_generation: 0,
            preview_scroll: 0,
            operation_running: false,
            operation_generation: 0,
            confirmation: None,
            message: None,
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
        if changes.mode == ChangesMode::Hunk {
            let hunk_count = changes
                .preview
                .as_ref()
                .map_or(0, |preview| preview.hunks.len());
            if hunk_count == 0 {
                return;
            }
            changes.selected_hunk =
                (changes.selected_hunk as isize + delta).clamp(0, hunk_count as isize - 1) as usize;
            sync_hunk_selection(changes);
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
            if changes.mode == ChangesMode::Hunk {
                changes.selected_hunk = 0;
                sync_hunk_selection(changes);
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
            if changes.mode == ChangesMode::Hunk {
                changes.selected_hunk = changes
                    .preview
                    .as_ref()
                    .map_or(0, |preview| preview.hunks.len().saturating_sub(1));
                sync_hunk_selection(changes);
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
                sync_hunk_selection(changes);
            }
            ChangesMode::Hunk => {
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
                let selected = changes
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
                changes.selected_hunk = selected;
                if changes.mode == ChangesMode::Hunk && preview.hunks.is_empty() {
                    changes.mode = ChangesMode::File;
                    changes.message = Some((
                        true,
                        "No selectable textual hunks are available for this file".to_owned(),
                    ));
                }
                changes.preview = Some(preview);
                sync_hunk_selection(changes);
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
                    hunk_operation_applicable(kind, hunk, &change),
                    "hunk",
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

    pub fn back(&mut self) {
        match self.screen {
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
fn reset_hunk_selection(changes: &mut ChangesState) {
    changes.selected_hunk = 0;
    changes.selected_hunk_identity = None;
}

fn sync_hunk_selection(changes: &mut ChangesState) {
    let Some(hunk) = changes
        .preview
        .as_ref()
        .and_then(|preview| preview.hunks.get(changes.selected_hunk))
    else {
        changes.selected_hunk_identity = None;
        return;
    };
    changes.selected_hunk_identity = Some((hunk.source, hunk.fingerprint));
    if changes.mode == ChangesMode::Hunk {
        changes.preview_scroll = hunk.display_start.saturating_sub(1);
    }
}

fn hunk_operation_applicable(kind: OperationKind, hunk: &ChangeHunk, change: &ChangeEntry) -> bool {
    match (hunk.source, kind) {
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
    use crate::domain::{WorkspaceKind, WorktreeSummary};

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
            loading: true,
            error: None,
            generation: 2,
            preview: None,
            preview_path: None,
            preview_loading: false,
            preview_generation: 3,
            preview_scroll: 0,
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
            }),
            preview_path: Some(entry.path.clone()),
            preview_loading: false,
            preview_generation: 1,
            preview_scroll: 0,
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
