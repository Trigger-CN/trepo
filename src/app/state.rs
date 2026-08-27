use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::adapters::git;
use crate::app::repository::{
    choices, form_for, FormField, RepositoryChoice, RepositoryForm, RepositoryTab,
};
use crate::domain::{
    BatchOperationItem, BatchOperationSpec, ChangeEntry, ChangePreview, Commit, CommitOutcome,
    CommitSpec, HunkSource, OperationKind, OperationOutcome, OperationSpec, OperationTarget,
    Project, ProjectId, ProjectSnapshot, RepoBatchAction, RepoBatchSpec, RepoProjectResult,
    RepoProjectState, RepositoryAction, RepositoryActionOutcome, RepositoryActionSpec,
    RepositorySnapshot, RiskLevel, Workspace, WorkspaceGitAction, WorkspaceGitSpec, WorkspaceKind,
    WorkspaceSummary,
};
use crate::services::operations::OperationRunner;
use crate::services::repo_batch::{self, RepoBatchEvent, RepoBatchEventKind, RepoBatchHandle};
use crate::services::scanner::{self, ScanResult};
use crate::services::workspace_git::{
    self, WorkspaceGitEvent, WorkspaceGitEventKind, WorkspaceGitPrepareResult,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphObjectKind {
    Commit,
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphObject {
    pub kind: GraphObjectKind,
    pub name: String,
    pub oid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphActionChoice {
    Changes,
    Commit,
    Amend,
    StashCreate,
    CreateBranch,
    CreateTag,
    CherryPick,
    Revert,
    SwitchBranch,
    Merge,
    Rebase,
    RenameBranch,
    DeleteBranch,
    DeleteTag,
    StashShow,
    StashApply,
    StashPop,
    StashDrop,
    Push,
    ForcePush,
}

impl GraphActionChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Changes => "Open Changes",
            Self::Commit => "Commit staged changes",
            Self::Amend => "Amend current commit",
            Self::StashCreate => "Create stash",
            Self::CreateBranch => "Create branch here",
            Self::CreateTag => "Create tag here",
            Self::CherryPick => "Cherry-pick this commit",
            Self::Revert => "Revert this commit",
            Self::SwitchBranch => "Switch to local branch",
            Self::Merge => "Merge this ref into current branch",
            Self::Rebase => "Rebase current branch onto this ref",
            Self::RenameBranch => "Rename local branch",
            Self::DeleteBranch => "Delete local branch",
            Self::DeleteTag => "Delete tag",
            Self::StashShow => "Show stash patch",
            Self::StashApply => "Apply stash",
            Self::StashPop => "Pop stash",
            Self::StashDrop => "Drop stash",
            Self::Push => "Push branch",
            Self::ForcePush => "Force push with lease",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GraphForm {
    pub choice: GraphActionChoice,
    pub object: GraphObject,
    pub fields: Vec<FormField>,
    pub selected: usize,
}

impl GraphForm {
    fn text(&self, index: usize) -> anyhow::Result<String> {
        match self.fields.get(index) {
            Some(FormField::Text { value, label }) if !value.trim().is_empty() => {
                Ok(value.trim().to_owned())
            }
            Some(FormField::Text { label, .. }) => anyhow::bail!("{label} cannot be empty"),
            _ => anyhow::bail!("invalid graph action form"),
        }
    }
    fn toggle(&self, index: usize) -> anyhow::Result<bool> {
        match self.fields.get(index) {
            Some(FormField::Toggle { value, .. }) => Ok(*value),
            _ => anyhow::bail!("invalid graph action form"),
        }
    }
    pub fn edit(&mut self, input: CommitInput) {
        match input {
            CommitInput::Character(value) => {
                if let Some(FormField::Text { value: text, .. }) =
                    self.fields.get_mut(self.selected)
                {
                    text.push(value);
                }
            }
            CommitInput::Text(_)
            | CommitInput::Newline
            | CommitInput::Delete
            | CommitInput::MoveLeft
            | CommitInput::MoveRight
            | CommitInput::MoveUp
            | CommitInput::MoveDown
            | CommitInput::MoveHome
            | CommitInput::MoveEnd => {}
            CommitInput::Backspace => {
                if let Some(FormField::Text { value, .. }) = self.fields.get_mut(self.selected) {
                    value.pop();
                }
            }
            CommitInput::ToggleAmend | CommitInput::ToggleSignoff | CommitInput::ToggleSigning => {
                if let Some(FormField::Toggle { value, .. }) = self.fields.get_mut(self.selected) {
                    *value = !*value;
                }
            }
        }
    }
    fn commit_spec(&self) -> anyhow::Result<CommitSpec> {
        Ok(CommitSpec {
            message: self.text(0)?,
            amend: matches!(self.choice, GraphActionChoice::Amend),
            signoff: self.toggle(1)?,
            signing: self.toggle(2)?,
        })
    }
    pub fn action(&self) -> anyhow::Result<RepositoryAction> {
        use GraphActionChoice as C;
        use RepositoryAction as A;
        Ok(match self.choice {
            C::StashCreate => A::StashPush {
                message: self.text(0).unwrap_or_default(),
                include_untracked: self.toggle(1)?,
                keep_index: self.toggle(2)?,
                staged_only: self.toggle(3)?,
            },
            C::CreateBranch => A::BranchCreate {
                name: self.text(0)?,
                start: Some(self.text(1)?),
            },
            C::CreateTag => A::TagCreate {
                name: self.text(0)?,
                target: self.text(1)?,
            },
            C::RenameBranch => A::BranchRename {
                old: self.text(0)?,
                new: self.text(1)?,
            },
            C::DeleteBranch => A::BranchDelete {
                name: self.text(0)?,
                force: self.toggle(1)?,
            },
            C::Push | C::ForcePush => A::Push {
                remote: self.text(0)?,
                branch: self.text(1)?,
                set_upstream: self.toggle(2)?,
                force_with_lease: matches!(self.choice, C::ForcePush),
            },
            _ => anyhow::bail!("unsupported graph form action"),
        })
    }
}

pub fn graph_actions(kind: GraphObjectKind) -> &'static [GraphActionChoice] {
    use GraphActionChoice as C;
    match kind {
        GraphObjectKind::Commit | GraphObjectKind::Head => &[
            C::Changes,
            C::Commit,
            C::Amend,
            C::StashCreate,
            C::CreateBranch,
            C::CreateTag,
            C::CherryPick,
            C::Revert,
            C::Merge,
            C::Rebase,
        ],
        GraphObjectKind::LocalBranch => &[
            C::SwitchBranch,
            C::Push,
            C::ForcePush,
            C::Merge,
            C::Rebase,
            C::RenameBranch,
            C::DeleteBranch,
        ],
        GraphObjectKind::RemoteBranch => &[
            C::CreateBranch,
            C::Merge,
            C::Rebase,
            C::CherryPick,
            C::Revert,
        ],
        GraphObjectKind::Tag => &[
            C::CreateBranch,
            C::CherryPick,
            C::Revert,
            C::Merge,
            C::Rebase,
            C::DeleteTag,
        ],
        GraphObjectKind::Stash => &[C::StashShow, C::StashApply, C::StashPop, C::StashDrop],
    }
}

fn graph_form(choice: GraphActionChoice, object: GraphObject) -> Option<GraphForm> {
    use GraphActionChoice as C;
    let fields = match choice {
        C::Commit | C::Amend => vec![
            FormField::Text {
                label: "Commit message",
                value: String::new(),
            },
            FormField::Toggle {
                label: "Sign off",
                value: false,
            },
            FormField::Toggle {
                label: "Sign commit",
                value: false,
            },
        ],
        C::StashCreate => vec![
            FormField::Text {
                label: "Message (optional)",
                value: String::new(),
            },
            FormField::Toggle {
                label: "Include untracked",
                value: false,
            },
            FormField::Toggle {
                label: "Keep index",
                value: false,
            },
            FormField::Toggle {
                label: "Staged only",
                value: false,
            },
        ],
        C::CreateBranch => vec![
            FormField::Text {
                label: "Branch name",
                value: String::new(),
            },
            FormField::Text {
                label: "Start ref",
                value: object.oid.clone(),
            },
        ],
        C::CreateTag => vec![
            FormField::Text {
                label: "Tag name",
                value: String::new(),
            },
            FormField::Text {
                label: "Target",
                value: object.oid.clone(),
            },
        ],
        C::RenameBranch => vec![
            FormField::Text {
                label: "Old name",
                value: object.name.clone(),
            },
            FormField::Text {
                label: "New name",
                value: String::new(),
            },
        ],
        C::DeleteBranch => vec![
            FormField::Text {
                label: "Branch",
                value: object.name.clone(),
            },
            FormField::Toggle {
                label: "Force delete",
                value: false,
            },
        ],
        C::Push | C::ForcePush => vec![
            FormField::Text {
                label: "Remote",
                value: "origin".to_owned(),
            },
            FormField::Text {
                label: "Branch",
                value: object.name.clone(),
            },
            FormField::Toggle {
                label: "Set upstream",
                value: false,
            },
        ],
        _ => return None,
    };
    Some(GraphForm {
        choice,
        object,
        fields,
        selected: 0,
    })
}

fn graph_action(choice: GraphActionChoice, object: &GraphObject) -> Option<RepositoryAction> {
    use GraphActionChoice as C;
    use RepositoryAction as A;
    let reference = match object.kind {
        GraphObjectKind::LocalBranch | GraphObjectKind::RemoteBranch => object.name.clone(),
        _ => object.oid.clone(),
    };
    Some(match choice {
        C::CherryPick => A::CherryPick {
            oid: object.oid.clone(),
        },
        C::Revert => A::Revert {
            oid: object.oid.clone(),
        },
        C::SwitchBranch => A::BranchSwitch {
            name: object.name.clone(),
        },
        C::Merge => A::Merge { reference },
        C::Rebase => A::Rebase { reference },
        C::DeleteTag => A::TagDelete {
            name: object.name.clone(),
        },
        C::StashShow => A::StashShow {
            selector: object.name.clone(),
        },
        C::StashApply => A::StashApply {
            selector: object.name.clone(),
            restore_index: false,
        },
        C::StashPop => A::StashPop {
            selector: object.name.clone(),
            restore_index: false,
        },
        C::StashDrop => A::StashDrop {
            selector: object.name.clone(),
        },
        _ => return None,
    })
}

pub fn graph_object_label(object: &GraphObject) -> String {
    format!(
        "{}: {}",
        match object.kind {
            GraphObjectKind::Commit => "Commit",
            GraphObjectKind::Head => "HEAD",
            GraphObjectKind::LocalBranch => "Local branch",
            GraphObjectKind::RemoteBranch => "Remote branch",
            GraphObjectKind::Tag => "Tag",
            GraphObjectKind::Stash => "Stash",
        },
        object.name
    )
}
fn short_oid(oid: &str) -> String {
    oid.chars().take(8).collect()
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphFilter {
    pub branch: String,
    pub query: String,
    pub author: String,
    pub since: String,
    pub until: String,
}

impl GraphFilter {
    pub fn is_active(&self) -> bool {
        !self.branch.is_empty()
            || !self.query.is_empty()
            || !self.author.is_empty()
            || !self.since.is_empty()
            || !self.until.is_empty()
    }

    pub fn summary(&self) -> String {
        [
            ("branch", self.branch.as_str()),
            ("query", self.query.as_str()),
            ("author", self.author.as_str()),
            ("since", self.since.as_str()),
            ("until", self.until.as_str()),
        ]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| format!("{label}:{value}"))
        .collect::<Vec<_>>()
        .join("  ")
    }

    fn validate(&self) -> anyhow::Result<()> {
        let since = parse_graph_date(&self.since, "Since")?;
        let until = parse_graph_date(&self.until, "Until")?;
        if since.zip(until).is_some_and(|(since, until)| since > until) {
            anyhow::bail!("Since must not be later than Until");
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct GraphFilterForm {
    pub draft: GraphFilter,
    pub selected: usize,
}

impl GraphFilterForm {
    pub fn fields(&self) -> [(&'static str, &str); 5] {
        [
            ("Branch", &self.draft.branch),
            ("Query", &self.draft.query),
            ("Author", &self.draft.author),
            ("Since", &self.draft.since),
            ("Until", &self.draft.until),
        ]
    }

    fn selected_value_mut(&mut self) -> &mut String {
        match self.selected {
            0 => &mut self.draft.branch,
            1 => &mut self.draft.query,
            2 => &mut self.draft.author,
            3 => &mut self.draft.since,
            _ => &mut self.draft.until,
        }
    }

    fn edit(&mut self, input: CommitInput) {
        match input {
            CommitInput::Character(value) => self.selected_value_mut().push(value),
            CommitInput::Backspace => {
                self.selected_value_mut().pop();
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
pub struct GraphState {
    pub project: Project,
    pub commits: Vec<Commit>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub generation: u64,
    pub object_menu: bool,
    pub object_selected: usize,
    pub action_menu: bool,
    pub action_selected: usize,
    pub selected_object: Option<GraphObject>,
    pub form: Option<GraphForm>,
    pub message: Option<(bool, String)>,
    pub selected_oid: Option<String>,
    pub filter: GraphFilter,
    pub filter_form: Option<GraphFilterForm>,
    pub filter_error: Option<String>,
    pub commit_message: String,
    pub commit_amend: bool,
    pub commit_running: bool,
    pub commit_generation: u64,
}

impl GraphState {
    pub fn filtered_indices(&self) -> Vec<usize> {
        let branch = self.filter.branch.to_lowercase();
        let reachable = if branch.is_empty() {
            None
        } else {
            let positions = self
                .commits
                .iter()
                .enumerate()
                .map(|(index, commit)| (commit.oid.as_str(), index))
                .collect::<HashMap<_, _>>();
            let mut stack = self
                .commits
                .iter()
                .enumerate()
                .filter(|(_, commit)| {
                    commit.refs.iter().any(|reference| {
                        matches!(
                            reference.kind,
                            crate::domain::CommitRefKind::LocalBranch
                                | crate::domain::CommitRefKind::RemoteBranch
                        ) && reference.name.to_lowercase().contains(&branch)
                    })
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let mut reachable = HashSet::new();
            while let Some(index) = stack.pop() {
                if !reachable.insert(index) {
                    continue;
                }
                stack.extend(
                    self.commits[index]
                        .parents
                        .iter()
                        .filter_map(|parent| positions.get(parent.as_str()).copied()),
                );
            }
            Some(reachable)
        };
        let query = self.filter.query.to_lowercase();
        let author = self.filter.author.to_lowercase();
        let since = parse_graph_date(&self.filter.since, "Since").ok().flatten();
        let until = parse_graph_date(&self.filter.until, "Until")
            .ok()
            .flatten()
            .map(|value| value.saturating_add(86_399));
        self.commits
            .iter()
            .enumerate()
            .filter(|(index, commit)| {
                reachable
                    .as_ref()
                    .map_or(true, |reachable| reachable.contains(index))
                    && (query.is_empty()
                        || commit.oid.to_lowercase().contains(&query)
                        || commit.subject.to_lowercase().contains(&query)
                        || commit.body.to_lowercase().contains(&query)
                        || commit
                            .refs
                            .iter()
                            .any(|reference| reference.name.to_lowercase().contains(&query)))
                    && (author.is_empty() || commit.author.to_lowercase().contains(&author))
                    && since.map_or(true, |since| commit.timestamp >= since)
                    && until.map_or(true, |until| commit.timestamp <= until)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn restore_filtered_selection(&mut self, selected_oid: Option<&str>) {
        let indices = self.filtered_indices();
        self.selected = selected_oid
            .and_then(|oid| {
                indices
                    .iter()
                    .copied()
                    .find(|index| self.commits[*index].oid == oid)
            })
            .or_else(|| indices.first().copied())
            .unwrap_or(0);
    }
}

fn parse_graph_date(value: &str, label: &str) -> anyhow::Result<Option<i64>> {
    if value.is_empty() {
        return Ok(None);
    }
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        anyhow::bail!("{label} must use YYYY-MM-DD");
    }
    let parse_digits = |range: std::ops::Range<usize>, field: &str| {
        let mut number = 0i64;
        for byte in &bytes[range] {
            if !byte.is_ascii_digit() {
                anyhow::bail!("{label} has an invalid {field}");
            }
            number = number * 10 + i64::from(byte - b'0');
        }
        Ok(number)
    };
    let year = parse_digits(0..4, "year")?;
    let month = parse_digits(5..7, "month")?;
    let day = parse_digits(8..10, "day")?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => anyhow::bail!("{label} has an invalid month"),
    };
    if day == 0 || day > days_in_month {
        anyhow::bail!("{label} has an invalid day");
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Ok(Some(
        (era * 146_097 + day_of_era - 719_468).saturating_mul(86_400),
    ))
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
pub struct BatchPrepareResult {
    pub project_id: ProjectId,
    pub changes_generation: u64,
    pub operation_generation: u64,
    pub result: anyhow::Result<BatchOperationSpec>,
}

#[derive(Debug)]
pub struct CommitResult {
    pub project_id: ProjectId,
    pub changes_generation: u64,
    pub commit_generation: u64,
    pub result: anyhow::Result<CommitOutcome>,
}

#[derive(Debug)]
pub struct GraphCommitResult {
    pub project_id: ProjectId,
    pub generation: u64,
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
#[derive(Debug, Clone)]
pub enum CommitInput {
    Character(char),
    Text(String),
    Newline,
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveHome,
    MoveEnd,
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
pub enum PendingOperation {
    Single {
        kind: OperationKind,
        change: ChangeEntry,
        expected_token: u64,
        target: OperationTarget,
    },
    Batch(BatchOperationSpec),
}

#[derive(Debug)]
pub struct ChangesState {
    pub project: Project,
    pub return_screen: Screen,
    pub entries: Vec<ChangeEntry>,
    pub selected: usize,
    pub selected_files: HashSet<PathBuf>,
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
    pub commit_cursor: usize,
    pub commit_editing: bool,
    pub commit_amend: bool,
    pub commit_signoff: bool,
    pub commit_signing: bool,
    pub commit_running: bool,
    pub commit_generation: u64,
}

fn clamp_char_boundary(text: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

fn previous_char_boundary(text: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(text, cursor);
    text[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_char_boundary(text: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(text, cursor);
    text[cursor..]
        .chars()
        .next()
        .map_or(cursor, |character| cursor + character.len_utf8())
}

fn line_start(text: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(text, cursor);
    text[..cursor].rfind('\n').map_or(0, |index| index + 1)
}

fn line_end(text: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(text, cursor);
    text[cursor..]
        .find('\n')
        .map_or(text.len(), |index| cursor + index)
}

fn byte_at_character_column(text: &str, start: usize, end: usize, column: usize) -> usize {
    text[start..end]
        .char_indices()
        .nth(column)
        .map_or(end, |(offset, _)| start + offset)
}

fn move_cursor_vertical(text: &str, cursor: usize, direction: isize) -> usize {
    let cursor = clamp_char_boundary(text, cursor);
    let current_start = line_start(text, cursor);
    let column = text[current_start..cursor].chars().count();
    if direction < 0 {
        if current_start == 0 {
            return cursor;
        }
        let target_end = current_start - 1;
        let target_start = line_start(text, target_end);
        byte_at_character_column(text, target_start, target_end, column)
    } else {
        let current_end = line_end(text, cursor);
        if current_end == text.len() {
            return cursor;
        }
        let target_start = current_end + 1;
        let target_end = line_end(text, target_start);
        byte_at_character_column(text, target_start, target_end, column)
    }
}

#[derive(Debug, Clone)]
pub struct RepoBatchForm {
    pub action: RepoBatchAction,
    pub value: String,
}

#[derive(Debug)]
pub struct RepoBatchTask {
    pub spec: RepoBatchSpec,
    pub targets: Vec<Project>,
    pub results: Vec<RepoProjectResult>,
    pub workspace_result: Option<(RepoProjectState, String)>,
    pub args: Vec<(Option<ProjectId>, Vec<String>)>,
    pub logs: Vec<String>,
    pub running: bool,
    pub cancelling: bool,
    pub cancelled: bool,
    pub generation: u64,
}

#[derive(Debug, Default)]
pub struct RepoBatchState {
    pub action_menu: bool,
    pub action_selected: usize,
    pub form: Option<RepoBatchForm>,
    pub pending: Option<(RepoBatchSpec, Vec<Project>)>,
    pub task: Option<RepoBatchTask>,
    pub scroll: usize,
    pub message: Option<(bool, String)>,
}

#[derive(Debug)]
pub struct WorkspaceGitTask {
    pub spec: WorkspaceGitSpec,
    pub results: Vec<RepoProjectResult>,
    pub running: bool,
    pub generation: u64,
}

#[derive(Debug, Default)]
pub struct WorkspaceGitState {
    pub preparing: bool,
    pub pending: Option<WorkspaceGitSpec>,
    pub task: Option<WorkspaceGitTask>,
    pub scroll: usize,
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
    pub changed_only: bool,
    pub help: bool,
    pub generation: u64,
    pub scanning: usize,
    pub graph: Option<GraphState>,
    pub changes: Option<ChangesState>,
    pub repository: Option<RepositoryState>,
    pub selected_projects: HashSet<ProjectId>,
    pub repo_batch: RepoBatchState,
    pub workspace_git: WorkspaceGitState,
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
    batch_prepare_tx: mpsc::UnboundedSender<BatchPrepareResult>,
    pub batch_prepare_rx: mpsc::UnboundedReceiver<BatchPrepareResult>,
    pub commit_tx: mpsc::UnboundedSender<CommitResult>,
    pub commit_rx: mpsc::UnboundedReceiver<CommitResult>,
    graph_commit_tx: mpsc::UnboundedSender<GraphCommitResult>,
    pub graph_commit_rx: mpsc::UnboundedReceiver<GraphCommitResult>,
    repository_intent: Option<RepositoryAction>,
    repository_tx: mpsc::UnboundedSender<RepositoryLoadResult>,
    pub repository_rx: mpsc::UnboundedReceiver<RepositoryLoadResult>,
    repository_action_tx: mpsc::UnboundedSender<RepositoryActionResult>,
    pub repository_action_rx: mpsc::UnboundedReceiver<RepositoryActionResult>,
    repo_batch_tx: mpsc::UnboundedSender<RepoBatchEvent>,
    pub repo_batch_rx: mpsc::UnboundedReceiver<RepoBatchEvent>,
    repo_batch_handle: Option<RepoBatchHandle>,
    repo_batch_generation: u64,
    workspace_git_prepare_tx: mpsc::UnboundedSender<WorkspaceGitPrepareResult>,
    pub workspace_git_prepare_rx: mpsc::UnboundedReceiver<WorkspaceGitPrepareResult>,
    workspace_git_tx: mpsc::UnboundedSender<WorkspaceGitEvent>,
    pub workspace_git_rx: mpsc::UnboundedReceiver<WorkspaceGitEvent>,
    workspace_git_generation: u64,
    operation_runner: OperationRunner,
    concurrency: usize,
}
impl App {
    pub fn new(workspace: Workspace, concurrency: usize) -> Self {
        let (scan_tx, scan_rx) = mpsc::unbounded_channel();
        let (graph_tx, graph_rx) = mpsc::unbounded_channel();
        let (changes_tx, changes_rx) = mpsc::unbounded_channel();
        let (operation_tx, operation_rx) = mpsc::unbounded_channel();
        let (batch_prepare_tx, batch_prepare_rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();
        let (commit_tx, commit_rx) = mpsc::unbounded_channel();
        let (graph_commit_tx, graph_commit_rx) = mpsc::unbounded_channel();
        let (repository_tx, repository_rx) = mpsc::unbounded_channel();
        let (repository_action_tx, repository_action_rx) = mpsc::unbounded_channel();
        let (repo_batch_tx, repo_batch_rx) = mpsc::unbounded_channel();
        let (workspace_git_prepare_tx, workspace_git_prepare_rx) = mpsc::unbounded_channel();
        let (workspace_git_tx, workspace_git_rx) = mpsc::unbounded_channel();
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
            changed_only: false,
            help: false,
            generation: 0,
            scanning: 0,
            graph: None,
            changes: None,
            repository: None,
            selected_projects: HashSet::new(),
            repo_batch: RepoBatchState::default(),
            workspace_git: WorkspaceGitState::default(),
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
            batch_prepare_tx,
            batch_prepare_rx,
            commit_tx,
            commit_rx,
            graph_commit_tx,
            graph_commit_rx,
            repository_tx,
            repository_rx,
            repository_action_tx,
            repository_action_rx,
            repo_batch_tx,
            repo_batch_rx,
            repo_batch_handle: None,
            repo_batch_generation: 0,
            workspace_git_prepare_tx,
            workspace_git_prepare_rx,
            workspace_git_tx,
            workspace_git_rx,
            workspace_git_generation: 0,
            operation_runner: OperationRunner,
            concurrency: concurrency.max(1),
            repository_intent: None,
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
        let selected_id = self
            .selected_project()
            .map(|snapshot| snapshot.project.id.clone());
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
        self.restore_workspace_selection(selected_id.as_ref());
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.search.to_lowercase();
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, snapshot)| {
                (!self.changed_only || snapshot.worktree.is_dirty())
                    && (query.is_empty()
                        || snapshot.project.name.to_lowercase().contains(&query)
                        || snapshot
                            .project
                            .relative_path
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&query)
                        || snapshot.head.label().to_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn toggle_changed_only(&mut self) {
        let selected_id = self
            .selected_project()
            .map(|snapshot| snapshot.project.id.clone());
        self.changed_only = !self.changed_only;
        self.restore_workspace_selection(selected_id.as_ref());
    }

    fn restore_workspace_selection(&mut self, selected_id: Option<&ProjectId>) {
        let indices = self.filtered_indices();
        self.selected = selected_id
            .and_then(|id| {
                indices.iter().position(|index| {
                    self.projects
                        .get(*index)
                        .is_some_and(|snapshot| &snapshot.project.id == id)
                })
            })
            .unwrap_or(0)
            .min(indices.len().saturating_sub(1));
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

    pub fn toggle_project_selection(&mut self) {
        let Some(id) = self
            .selected_project()
            .map(|value| value.project.id.clone())
        else {
            return;
        };
        if !self.selected_projects.insert(id.clone()) {
            self.selected_projects.remove(&id);
        }
    }

    pub fn toggle_filtered_selection(&mut self) {
        let ids: Vec<ProjectId> = self
            .filtered_indices()
            .into_iter()
            .filter_map(|index| self.projects.get(index))
            .map(|value| value.project.id.clone())
            .collect();
        let all_selected = ids.iter().all(|id| self.selected_projects.contains(id));
        for id in ids {
            if all_selected {
                self.selected_projects.remove(&id);
            } else {
                self.selected_projects.insert(id);
            }
        }
    }

    pub fn selected_project_count(&self) -> usize {
        self.selected_projects.len()
    }

    pub fn begin_workspace_git(&mut self, action: WorkspaceGitAction) {
        if self.workspace_git.preparing
            || self
                .workspace_git
                .task
                .as_ref()
                .is_some_and(|task| task.running)
        {
            self.workspace_git.message =
                Some((true, "A Workspace Git task is already active".into()));
            return;
        }
        let targets = self
            .workspace
            .projects
            .iter()
            .filter(|project| self.selected_projects.contains(&project.id))
            .cloned()
            .collect::<Vec<_>>();
        if targets.is_empty() {
            self.workspace_git.message =
                Some((true, "Select at least one repository with Space".into()));
            return;
        }
        self.workspace_git_generation = self.workspace_git_generation.wrapping_add(1);
        let generation = self.workspace_git_generation;
        self.workspace_git.preparing = true;
        self.workspace_git.pending = None;
        self.workspace_git.task = None;
        self.workspace_git.message = None;
        workspace_git::spawn_prepare(
            action,
            targets,
            generation,
            self.workspace_git_prepare_tx.clone(),
        );
    }

    pub fn apply_workspace_git_prepare(&mut self, result: WorkspaceGitPrepareResult) {
        if result.generation != self.workspace_git_generation || !self.workspace_git.preparing {
            return;
        }
        self.workspace_git.preparing = false;
        match result.result {
            Ok(spec) => self.workspace_git.pending = Some(spec),
            Err(error) => self.workspace_git.message = Some((true, error.to_string())),
        }
    }

    pub fn confirm_workspace_git(&mut self, confirmed: bool) {
        let pending = self.workspace_git.pending.take();
        if !confirmed {
            return;
        }
        let Some(spec) = pending else {
            return;
        };
        let generation = self.workspace_git_generation;
        let results = spec
            .targets
            .iter()
            .map(|target| RepoProjectResult {
                project: target.project.clone(),
                state: RepoProjectState::Pending,
                message: "Waiting for full-batch preflight".to_owned(),
            })
            .collect();
        self.workspace_git.task = Some(WorkspaceGitTask {
            spec: spec.clone(),
            results,
            running: true,
            generation,
        });
        self.workspace_git.scroll = 0;
        workspace_git::spawn_execute(
            self.workspace.root.clone(),
            spec,
            generation,
            self.workspace_git_tx.clone(),
        );
    }

    pub fn apply_workspace_git(&mut self, event: WorkspaceGitEvent) {
        let Some(task) = self.workspace_git.task.as_mut() else {
            return;
        };
        if event.generation != task.generation {
            return;
        }
        match event.kind {
            WorkspaceGitEventKind::Started { project } => {
                if let Some(result) = task
                    .results
                    .iter_mut()
                    .find(|result| result.project.id == project.id)
                {
                    result.state = RepoProjectState::Running;
                    result.message = "Running".to_owned();
                }
            }
            WorkspaceGitEventKind::Finished {
                project,
                state,
                message,
            } => {
                if let Some(result) = task
                    .results
                    .iter_mut()
                    .find(|result| result.project.id == project.id)
                {
                    result.state = state;
                    result.message = message;
                }
            }
            WorkspaceGitEventKind::Complete => {
                task.running = false;
                self.refresh();
            }
        }
    }

    pub fn close_workspace_git(&mut self) {
        if self
            .workspace_git
            .task
            .as_ref()
            .is_some_and(|task| task.running)
        {
            return;
        }
        self.workspace_git = WorkspaceGitState::default();
    }

    pub fn scroll_workspace_git(&mut self, delta: isize) {
        self.workspace_git.scroll = self.workspace_git.scroll.saturating_add_signed(delta);
    }

    pub fn workspace_git_overlay_active(&self) -> bool {
        self.workspace_git.preparing
            || self.workspace_git.pending.is_some()
            || self.workspace_git.task.is_some()
            || self.workspace_git.message.is_some()
    }

    pub fn open_repo_batch_menu(&mut self) {
        if self.workspace.kind != WorkspaceKind::Repo {
            self.repo_batch.message = Some((true, "Repo actions require a Repo workspace".into()));
            return;
        }
        if self
            .repo_batch
            .task
            .as_ref()
            .is_some_and(|task| task.running)
        {
            self.repo_batch.message = Some((true, "A Repo task is already running".into()));
            return;
        }
        self.repo_batch.action_menu = true;
        self.repo_batch.action_selected = 0;
        self.repo_batch.form = None;
        self.repo_batch.pending = None;
        self.repo_batch.message = None;
    }

    pub fn move_repo_batch_selection(&mut self, delta: isize) {
        self.repo_batch.scroll = 0;
        let len = RepoBatchAction::ALL.len();
        let current = self.repo_batch.action_selected.min(len.saturating_sub(1)) as isize;
        self.repo_batch.action_selected =
            (current + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
    }

    pub fn scroll_repo_batch(&mut self, delta: isize) {
        self.repo_batch.scroll = self.repo_batch.scroll.saturating_add_signed(delta);
    }

    pub fn select_repo_batch_action(&mut self) {
        let action = RepoBatchAction::ALL[self
            .repo_batch
            .action_selected
            .min(RepoBatchAction::ALL.len() - 1)];
        self.repo_batch.action_menu = false;
        if action.input_label().is_some() {
            self.repo_batch.form = Some(RepoBatchForm {
                action,
                value: if action == RepoBatchAction::ManifestExport {
                    "manifest-pinned.xml".to_owned()
                } else {
                    String::new()
                },
            });
        } else {
            self.prepare_repo_batch(action, String::new());
        }
    }

    pub fn edit_repo_batch_form(&mut self, input: CommitInput) {
        let Some(form) = self.repo_batch.form.as_mut() else {
            return;
        };
        match input {
            CommitInput::Character(value) => form.value.push(value),
            CommitInput::Backspace => {
                form.value.pop();
            }
            _ => {}
        }
    }

    pub fn submit_repo_batch_form(&mut self) {
        let Some(form) = self.repo_batch.form.take() else {
            return;
        };
        self.prepare_repo_batch(form.action, form.value);
    }

    fn prepare_repo_batch(&mut self, action: RepoBatchAction, value: String) {
        let value = value.trim();
        let spec = RepoBatchSpec {
            action,
            branch: matches!(
                action,
                RepoBatchAction::Start | RepoBatchAction::Checkout | RepoBatchAction::Abandon
            )
            .then(|| value.to_owned()),
            change: (action == RepoBatchAction::Download).then(|| value.to_owned()),
            output: (action == RepoBatchAction::ManifestExport).then(|| PathBuf::from(value)),
        };
        let targets: Vec<Project> = if action.is_workspace_action() {
            Vec::new()
        } else {
            self.workspace
                .projects
                .iter()
                .filter(|project| self.selected_projects.contains(&project.id))
                .cloned()
                .collect()
        };
        if !action.is_workspace_action() && action != RepoBatchAction::Sync && targets.is_empty() {
            self.repo_batch.message = Some((true, "Select at least one project with Space".into()));
            return;
        }
        let valid = if action.is_workspace_action() {
            crate::adapters::repo::batch_args(&spec, None).map(|_| ())
        } else if action == RepoBatchAction::Sync {
            let paths = targets
                .iter()
                .map(|project| project.relative_path.as_path())
                .collect::<Vec<_>>();
            crate::adapters::repo::sync_args(&paths).map(|_| ())
        } else {
            targets.iter().try_for_each(|project| {
                crate::adapters::repo::batch_args(&spec, Some(&project.relative_path)).map(|_| ())
            })
        };
        match valid {
            Ok(()) => {
                self.repo_batch.scroll = 0;
                self.repo_batch.pending = Some((spec, targets));
                self.repo_batch.message = None;
            }
            Err(error) => self.repo_batch.message = Some((true, error.to_string())),
        }
    }

    pub fn confirm_repo_batch(&mut self, confirmed: bool) {
        let Some((spec, targets)) = self.repo_batch.pending.take() else {
            return;
        };
        if confirmed {
            self.start_repo_batch(spec, targets);
        }
    }

    fn start_repo_batch(&mut self, spec: RepoBatchSpec, targets: Vec<Project>) {
        self.repo_batch_generation = self.repo_batch_generation.wrapping_add(1);
        let generation = self.repo_batch_generation;
        let results = targets
            .iter()
            .cloned()
            .map(|project| RepoProjectResult {
                project,
                state: RepoProjectState::Pending,
                message: "Waiting for workspace lock".to_owned(),
            })
            .collect();
        let workspace_scope = spec.action.is_workspace_action()
            || (spec.action == RepoBatchAction::Sync && targets.is_empty());
        let workspace_result = workspace_scope.then(|| {
            (
                RepoProjectState::Pending,
                "Waiting for workspace lock".to_owned(),
            )
        });
        self.repo_batch.scroll = 0;
        self.repo_batch.task = Some(RepoBatchTask {
            spec: spec.clone(),
            targets: targets.clone(),
            results,
            workspace_result,
            args: Vec::new(),
            logs: Vec::new(),
            running: true,
            cancelling: false,
            cancelled: false,
            generation,
        });
        self.repo_batch.message = None;
        self.repo_batch_handle = Some(repo_batch::spawn(
            self.workspace.root.clone(),
            spec,
            targets,
            generation,
            self.repo_batch_tx.clone(),
        ));
    }

    pub fn apply_repo_batch(&mut self, event: RepoBatchEvent) {
        let Some(task) = self.repo_batch.task.as_mut() else {
            return;
        };
        if event.generation != task.generation {
            return;
        }
        match event.kind {
            RepoBatchEventKind::Started { project, args } => {
                let id = project.as_ref().map(|value| value.id.clone());
                task.args.push((id.clone(), args));
                if let Some(id) = id {
                    if let Some(result) =
                        task.results.iter_mut().find(|value| value.project.id == id)
                    {
                        result.state = RepoProjectState::Running;
                        result.message = "Running".to_owned();
                    }
                } else {
                    task.workspace_result = Some((RepoProjectState::Running, "Running".to_owned()));
                }
            }
            RepoBatchEventKind::StartedBatch { projects, args } => {
                task.args.push((None, args));
                if projects.is_empty() {
                    task.workspace_result = Some((RepoProjectState::Running, "Running".to_owned()));
                }
                for project in projects {
                    if let Some(result) = task
                        .results
                        .iter_mut()
                        .find(|value| value.project.id == project.id)
                    {
                        result.state = RepoProjectState::Running;
                        result.message = "Running in aggregated sync".to_owned();
                    }
                }
            }
            RepoBatchEventKind::Log { project_id, line } => {
                let prefix = project_id
                    .and_then(|id| {
                        task.targets
                            .iter()
                            .find(|value| value.id == id)
                            .map(|value| value.relative_path.display().to_string())
                    })
                    .unwrap_or_else(|| "workspace".to_owned());
                task.logs.push(format!("[{prefix}] {line}"));
                if task.logs.len() > 500 {
                    task.logs.drain(..task.logs.len() - 500);
                }
            }
            RepoBatchEventKind::Finished {
                project,
                state,
                message,
            } => {
                if let Some(project) = project {
                    if let Some(result) = task
                        .results
                        .iter_mut()
                        .find(|value| value.project.id == project.id)
                    {
                        result.state = state;
                        result.message = message;
                    }
                } else {
                    task.workspace_result = Some((state, message));
                }
            }
            RepoBatchEventKind::Complete { cancelled } => {
                task.running = false;
                task.cancelling = false;
                task.cancelled = cancelled;
                self.repo_batch_handle = None;
                self.refresh();
            }
        }
    }

    pub fn cancel_repo_batch(&mut self) {
        let Some(task) = self.repo_batch.task.as_mut() else {
            return;
        };
        if !task.running || task.cancelling {
            return;
        }
        task.cancelling = true;
        task.logs.push(
            "[repo-tui] Cancellation requested; completed changes will not be rolled back".into(),
        );
        if let Some(handle) = &self.repo_batch_handle {
            handle.cancel();
        }
    }

    pub fn retry_failed_repo_batch(&mut self) {
        let Some(task) = self.repo_batch.task.as_ref() else {
            return;
        };
        if task.running {
            return;
        }
        let targets: Vec<Project> = task
            .results
            .iter()
            .filter(|result| result.state == RepoProjectState::Failed)
            .map(|result| result.project.clone())
            .collect();
        let retry_workspace = task.spec.action == RepoBatchAction::Sync
            && task.targets.is_empty()
            && task
                .workspace_result
                .as_ref()
                .is_some_and(|(state, _)| *state == RepoProjectState::Failed);
        if targets.is_empty() && !retry_workspace {
            self.repo_batch.message = Some((true, "There are no failed projects to retry".into()));
            return;
        }
        self.start_repo_batch(task.spec.clone(), targets);
    }

    pub fn close_repo_batch_overlay(&mut self) {
        if self.repo_batch.message.take().is_some() {
            return;
        }
        if self.repo_batch.form.take().is_some()
            || self.repo_batch.pending.take().is_some()
            || self.repo_batch.action_menu
        {
            self.repo_batch.action_menu = false;
            return;
        }
        if self
            .repo_batch
            .task
            .as_ref()
            .is_some_and(|task| !task.running)
        {
            self.repo_batch.task = None;
        }
    }

    pub fn repo_batch_overlay_active(&self) -> bool {
        self.repo_batch.action_menu
            || self.repo_batch.form.is_some()
            || self.repo_batch.pending.is_some()
            || self.repo_batch.task.is_some()
            || self.repo_batch.message.is_some()
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
        let selected_oid = self.graph.as_ref().and_then(|graph| {
            graph
                .commits
                .get(graph.selected)
                .map(|commit| commit.oid.clone())
        });
        let filter = self
            .graph
            .as_ref()
            .map_or_else(GraphFilter::default, |graph| graph.filter.clone());
        self.graph = Some(GraphState {
            project: project.clone(),
            commits: Vec::new(),
            selected: 0,
            loading: true,
            error: None,
            generation,
            object_menu: false,
            object_selected: 0,
            action_menu: false,
            action_selected: 0,
            selected_object: None,
            form: None,
            message: None,
            selected_oid,
            filter,
            filter_form: None,
            filter_error: None,
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
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
            Ok(commits) => {
                graph.commits = commits;
                let selected_oid = graph.selected_oid.take();
                graph.restore_filtered_selection(selected_oid.as_deref());
            }
            Err(error) => graph.error = Some(error.to_string()),
        }
        graph.selected = graph.selected.min(graph.commits.len().saturating_sub(1));
    }

    pub fn move_graph_selection(&mut self, delta: isize) {
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        let indices = graph.filtered_indices();
        if indices.is_empty() {
            graph.selected = 0;
            return;
        }
        let current = indices
            .iter()
            .position(|index| *index == graph.selected)
            .unwrap_or(0) as isize;
        let visible = (current + delta).clamp(0, indices.len() as isize - 1) as usize;
        graph.selected = indices[visible];
    }

    pub fn graph_first(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            graph.selected = graph.filtered_indices().first().copied().unwrap_or(0);
        }
    }

    pub fn graph_last(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            graph.selected = graph.filtered_indices().last().copied().unwrap_or(0);
        }
    }

    pub fn open_graph_filter(&mut self, query_only: bool) {
        if let Some(graph) = self.graph.as_mut() {
            graph.filter_form = Some(GraphFilterForm {
                draft: graph.filter.clone(),
                selected: usize::from(query_only),
            });
            graph.filter_error = None;
        }
    }

    pub fn move_graph_filter_field(&mut self, delta: isize) {
        if let Some(form) = self
            .graph
            .as_mut()
            .and_then(|graph| graph.filter_form.as_mut())
        {
            form.selected = (form.selected as isize + delta).clamp(0, 4) as usize;
        }
    }

    pub fn edit_graph_filter(&mut self, input: CommitInput) {
        if let Some(form) = self
            .graph
            .as_mut()
            .and_then(|graph| graph.filter_form.as_mut())
        {
            form.edit(input);
        }
    }

    pub fn submit_graph_filter(&mut self) {
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        let Some(filter) = graph.filter_form.as_ref().map(|form| form.draft.clone()) else {
            return;
        };
        if let Err(error) = filter.validate() {
            graph.filter_error = Some(error.to_string());
            return;
        }
        let selected_oid = graph
            .commits
            .get(graph.selected)
            .map(|commit| commit.oid.clone());
        graph.filter = filter;
        graph.filter_form = None;
        graph.filter_error = None;
        graph.restore_filtered_selection(selected_oid.as_deref());
    }

    pub fn cancel_graph_filter(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            graph.filter_form = None;
            graph.filter_error = None;
        }
    }

    pub fn clear_graph_filter(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            let selected_oid = graph
                .commits
                .get(graph.selected)
                .map(|commit| commit.oid.clone());
            graph.filter = GraphFilter::default();
            graph.filter_form = None;
            graph.filter_error = None;
            graph.restore_filtered_selection(selected_oid.as_deref());
        }
    }
    pub fn graph_objects(&self) -> Vec<GraphObject> {
        let Some(graph) = self.graph.as_ref() else {
            return Vec::new();
        };
        if !graph.filtered_indices().contains(&graph.selected) {
            return Vec::new();
        }
        let Some(commit) = graph.commits.get(graph.selected) else {
            return Vec::new();
        };
        let mut objects = vec![GraphObject {
            kind: GraphObjectKind::Commit,
            name: format!("commit {}", short_oid(&commit.oid)),
            oid: commit.oid.clone(),
        }];
        for reference in &commit.refs {
            let kind = match reference.kind {
                crate::domain::CommitRefKind::Head => Some(GraphObjectKind::Head),
                crate::domain::CommitRefKind::LocalBranch => Some(GraphObjectKind::LocalBranch),
                crate::domain::CommitRefKind::RemoteBranch => Some(GraphObjectKind::RemoteBranch),
                crate::domain::CommitRefKind::Tag => Some(GraphObjectKind::Tag),
                crate::domain::CommitRefKind::Stash => Some(GraphObjectKind::Stash),
            };
            if let Some(kind) = kind {
                objects.push(GraphObject {
                    kind,
                    name: reference.name.clone(),
                    oid: commit.oid.clone(),
                });
            }
        }
        objects
    }

    pub fn open_graph_object_menu(&mut self) {
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        if graph.loading || !graph.filtered_indices().contains(&graph.selected) {
            return;
        }
        graph.object_menu = true;
        graph.object_selected = 0;
        graph.action_menu = false;
        graph.form = None;
        graph.selected_object = None;
        graph.message = None;
    }

    pub fn move_graph_overlay_selection(&mut self, delta: isize) {
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        if let Some(form) = graph.form.as_mut() {
            if !form.fields.is_empty() {
                form.selected = (form.selected as isize + delta)
                    .clamp(0, form.fields.len() as isize - 1)
                    as usize;
            }
        } else if graph.action_menu {
            if let Some(object) = graph.selected_object.as_ref() {
                let len = graph_actions(object.kind).len();
                if len > 0 {
                    graph.action_selected = (graph.action_selected as isize + delta)
                        .clamp(0, len as isize - 1)
                        as usize;
                }
            }
        } else if graph.object_menu {
            let len = graph
                .commits
                .get(graph.selected)
                .map_or(0, |commit| 1 + commit.refs.len());
            if len > 0 {
                graph.object_selected =
                    (graph.object_selected as isize + delta).clamp(0, len as isize - 1) as usize;
            }
        }
    }

    pub fn select_graph_object(&mut self) {
        let objects = self.graph_objects();
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        let Some(object) = objects.get(graph.object_selected).cloned() else {
            return;
        };
        graph.selected_object = Some(object);
        graph.object_menu = false;
        graph.action_menu = true;
        graph.action_selected = 0;
    }

    pub fn select_graph_action(&mut self) {
        let Some(graph) = self.graph.as_ref() else {
            return;
        };
        let Some(object) = graph.selected_object.clone() else {
            return;
        };
        let Some(choice) = graph_actions(object.kind)
            .get(graph.action_selected)
            .copied()
        else {
            return;
        };
        if matches!(
            choice,
            GraphActionChoice::Changes | GraphActionChoice::Commit | GraphActionChoice::Amend
        ) {
            if choice == GraphActionChoice::Changes {
                if let Some(graph) = self.graph.as_mut() {
                    graph.action_menu = false;
                    graph.selected_object = None;
                }
                self.open_changes();
                return;
            }
            if let Some(form) = graph_form(choice, object.clone()) {
                if let Some(graph) = self.graph.as_mut() {
                    graph.action_menu = false;
                    graph.form = Some(form);
                }
                return;
            }
        }
        if let Some(form) = graph_form(choice, object.clone()) {
            if let Some(graph) = self.graph.as_mut() {
                graph.action_menu = false;
                graph.form = Some(form);
            }
            return;
        }
        if let Some(action) = graph_action(choice, &object) {
            self.open_graph_repository_action(action);
        }
    }

    pub fn edit_graph_form(&mut self, input: CommitInput) {
        if let Some(form) = self.graph.as_mut().and_then(|graph| graph.form.as_mut()) {
            form.edit(input);
        }
    }

    pub fn submit_graph_form(&mut self) {
        let Some(form) = self.graph.as_ref().and_then(|graph| graph.form.as_ref()) else {
            return;
        };
        if matches!(
            form.choice,
            GraphActionChoice::Commit | GraphActionChoice::Amend
        ) {
            match form.commit_spec() {
                Ok(spec) => self.begin_graph_commit(spec),
                Err(error) => {
                    if let Some(graph) = self.graph.as_mut() {
                        graph.message = Some((true, error.to_string()));
                    }
                }
            }
            return;
        }
        match form.action() {
            Ok(action) => self.open_graph_repository_action(action),
            Err(error) => {
                if let Some(graph) = self.graph.as_mut() {
                    graph.message = Some((true, error.to_string()));
                }
            }
        }
    }

    pub fn cancel_graph_overlay(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            if graph.form.take().is_some() {
                return;
            }
            if graph.action_menu {
                graph.action_menu = false;
                graph.object_menu = true;
                return;
            }
            graph.object_menu = false;
            graph.selected_object = None;
        }
    }

    fn open_graph_repository_action(&mut self, action: RepositoryAction) {
        let Some(project) = self.graph.as_ref().map(|graph| graph.project.clone()) else {
            return;
        };
        if let Some(graph) = self.graph.as_mut() {
            graph.form = None;
            graph.action_menu = false;
            graph.object_menu = false;
            graph.message = Some((false, "Loading current repository state...".to_owned()));
        }
        self.repository_intent = Some(action);
        self.load_repository(project, Screen::Graph);
    }

    fn begin_graph_commit(&mut self, spec: CommitSpec) {
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        if graph.commit_running {
            return;
        }
        graph.form = None;
        graph.commit_running = true;
        graph.message = None;
        graph.commit_generation = graph.commit_generation.wrapping_add(1);
        let commit_generation = graph.commit_generation;
        let generation = graph.generation;
        let project = graph.project.clone();
        let project_id = project.id.clone();
        let runner = self.operation_runner.clone();
        let sender = self.graph_commit_tx.clone();
        tokio::spawn(async move {
            let result = runner.execute_commit(project, spec).await;
            let _ = sender.send(GraphCommitResult {
                project_id,
                generation,
                commit_generation,
                result,
            });
        });
    }

    pub fn apply_graph_commit(&mut self, result: GraphCommitResult) {
        let Some(graph) = self.graph.as_ref() else {
            return;
        };
        if graph.project.id != result.project_id
            || graph.generation != result.generation
            || graph.commit_generation != result.commit_generation
        {
            return;
        }
        let project = graph.project.clone();
        match result.result {
            Ok(outcome) => {
                self.load_graph(project);
                if let Some(graph) = self.graph.as_mut() {
                    graph.message = Some((false, outcome.message));
                }
            }
            Err(error) => {
                if let Some(graph) = self.graph.as_mut() {
                    graph.commit_running = false;
                    graph.message = Some((true, error.to_string()));
                }
            }
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
            selected_files: HashSet::new(),
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
            commit_cursor: 0,
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
                changes
                    .selected_files
                    .retain(|path| changes.entries.iter().any(|entry| &entry.path == path));
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

    pub fn toggle_change_selected(&mut self) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.mode != ChangesMode::File || changes.operation_running {
            return;
        }
        let Some(path) = changes
            .entries
            .get(changes.selected)
            .map(|entry| entry.path.clone())
        else {
            return;
        };
        if !changes.selected_files.remove(&path) {
            changes.selected_files.insert(path);
        }
        changes.message = None;
    }

    pub fn toggle_all_changes_selected(&mut self) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.mode != ChangesMode::File || changes.operation_running {
            return;
        }
        if changes.selected_files.len() == changes.entries.len() {
            changes.selected_files.clear();
        } else {
            changes.selected_files = changes
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect();
        }
        changes.message = None;
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
        if changes.mode == ChangesMode::File && !changes.selected_files.is_empty() {
            let kind = if kind == OperationKind::RestoreWorktree {
                OperationKind::Discard
            } else {
                kind
            };
            let selected = changes
                .entries
                .iter()
                .filter(|entry| changes.selected_files.contains(&entry.path))
                .cloned()
                .collect::<Vec<_>>();
            let inapplicable = selected.iter().find(|entry| match kind {
                OperationKind::Stage => !entry.can_stage(),
                OperationKind::Unstage => !entry.can_unstage(),
                OperationKind::Stash => entry.conflicted,
                OperationKind::Discard => false,
                OperationKind::RestoreWorktree => true,
            });
            if let Some(entry) = inapplicable {
                changes.message = Some((
                    true,
                    format!(
                        "{} is not available for {}",
                        kind.label(),
                        entry.path.display()
                    ),
                ));
                return;
            }
            if matches!(kind, OperationKind::Stash | OperationKind::Discard) {
                self.prepare_batch_confirmation(kind, selected);
            } else {
                self.spawn_batch_operation(kind, selected);
            }
            return;
        }
        if matches!(kind, OperationKind::Stash | OperationKind::Discard) {
            changes.message = Some((true, "Select one or more files first".to_owned()));
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
                    OperationKind::Stash | OperationKind::Discard => false,
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
        let pending = PendingOperation::Single {
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
            match pending {
                Some(single @ PendingOperation::Single { .. }) => {
                    self.spawn_operation(single);
                }
                Some(PendingOperation::Batch(spec)) => self.spawn_prepared_batch(spec),
                None => {}
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
        changes.commit_cursor = changes.commit_message.len();
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
        changes.commit_cursor = clamp_char_boundary(&changes.commit_message, changes.commit_cursor);
        match input {
            CommitInput::Character(value) => {
                changes.commit_message.insert(changes.commit_cursor, value);
                changes.commit_cursor += value.len_utf8();
            }
            CommitInput::Text(value) => {
                let value = value.replace("\r\n", "\n").replace('\r', "\n");
                changes
                    .commit_message
                    .insert_str(changes.commit_cursor, &value);
                changes.commit_cursor += value.len();
            }
            CommitInput::Newline => {
                changes.commit_message.insert(changes.commit_cursor, '\n');
                changes.commit_cursor += 1;
            }
            CommitInput::Backspace => {
                let previous =
                    previous_char_boundary(&changes.commit_message, changes.commit_cursor);
                changes
                    .commit_message
                    .replace_range(previous..changes.commit_cursor, "");
                changes.commit_cursor = previous;
            }
            CommitInput::Delete => {
                let next = next_char_boundary(&changes.commit_message, changes.commit_cursor);
                changes
                    .commit_message
                    .replace_range(changes.commit_cursor..next, "");
            }
            CommitInput::MoveLeft => {
                changes.commit_cursor =
                    previous_char_boundary(&changes.commit_message, changes.commit_cursor);
            }
            CommitInput::MoveRight => {
                changes.commit_cursor =
                    next_char_boundary(&changes.commit_message, changes.commit_cursor);
            }
            CommitInput::MoveUp => {
                changes.commit_cursor =
                    move_cursor_vertical(&changes.commit_message, changes.commit_cursor, -1);
            }
            CommitInput::MoveDown => {
                changes.commit_cursor =
                    move_cursor_vertical(&changes.commit_message, changes.commit_cursor, 1);
            }
            CommitInput::MoveHome => {
                changes.commit_cursor = line_start(&changes.commit_message, changes.commit_cursor);
            }
            CommitInput::MoveEnd => {
                changes.commit_cursor = line_end(&changes.commit_message, changes.commit_cursor);
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
    fn prepare_batch_confirmation(&mut self, kind: OperationKind, entries: Vec<ChangeEntry>) {
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
        let sender = self.batch_prepare_tx.clone();
        tokio::spawn(async move {
            let mut items = Vec::with_capacity(entries.len());
            let mut error = None;
            for change in entries {
                match git::change_token(&project.path, &change).await {
                    Ok(expected_token) => items.push(BatchOperationItem {
                        change,
                        expected_token,
                    }),
                    Err(value) => {
                        error = Some(value);
                        break;
                    }
                }
            }
            let result = error.map_or_else(
                || {
                    Ok(BatchOperationSpec {
                        project,
                        items,
                        kind,
                    })
                },
                Err,
            );
            let _ = sender.send(BatchPrepareResult {
                project_id,
                changes_generation,
                operation_generation,
                result,
            });
        });
    }

    pub fn apply_batch_prepare(&mut self, result: BatchPrepareResult) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        if changes.project.id != result.project_id
            || changes.operation_generation != result.operation_generation
            || changes.generation != result.changes_generation
        {
            return;
        }
        changes.operation_running = false;
        match result.result {
            Ok(spec) => changes.confirmation = Some(PendingOperation::Batch(spec)),
            Err(error) => changes.message = Some((true, error.to_string())),
        }
    }

    fn spawn_prepared_batch(&mut self, spec: BatchOperationSpec) {
        let Some(changes) = self.changes.as_mut() else {
            return;
        };
        changes.operation_running = true;
        changes.message = None;
        changes.operation_generation = changes.operation_generation.wrapping_add(1);
        let operation_generation = changes.operation_generation;
        let changes_generation = changes.generation;
        let project_id = spec.project.id.clone();
        let sender = self.operation_tx.clone();
        let runner = self.operation_runner.clone();
        tokio::spawn(async move {
            let result = runner.execute_batch(spec).await;
            let _ = sender.send(OperationResult {
                project_id,
                operation_generation,
                changes_generation,
                result,
            });
        });
    }

    fn spawn_batch_operation(&mut self, kind: OperationKind, entries: Vec<ChangeEntry>) {
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
            let mut items = Vec::with_capacity(entries.len());
            for change in entries {
                let expected_token = match git::change_token(&project.path, &change).await {
                    Ok(token) => token,
                    Err(error) => {
                        let _ = sender.send(OperationResult {
                            project_id,
                            operation_generation,
                            changes_generation,
                            result: Err(error),
                        });
                        return;
                    }
                };
                items.push(BatchOperationItem {
                    change,
                    expected_token,
                });
            }
            let result = runner
                .execute_batch(BatchOperationSpec {
                    project,
                    items,
                    kind,
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

    fn spawn_operation(&mut self, pending: PendingOperation) {
        let PendingOperation::Single {
            kind,
            change,
            expected_token,
            target,
        } = pending
        else {
            return;
        };
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
                    change,
                    kind,
                    target,
                    expected_token,
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
            Err(error) => {
                let message = error.to_string();
                state.error = Some(message.clone());
                if self.repository_intent.take().is_some() {
                    if let Some(graph) = self.graph.as_mut() {
                        graph.message = Some((true, message));
                    }
                }
                return;
            }
        }
        clamp_repository_selection(state);
        if let Some(action) = self.repository_intent.take() {
            self.begin_repository_action(action);
        }
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
            CommitInput::Text(_)
            | CommitInput::Newline
            | CommitInput::Delete
            | CommitInput::MoveLeft
            | CommitInput::MoveRight
            | CommitInput::MoveUp
            | CommitInput::MoveDown
            | CommitInput::MoveHome
            | CommitInput::MoveEnd => {}
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
        let graph_origin = state.return_screen == Screen::Graph;
        match result.result {
            Ok(outcome) => {
                let project = state.project.clone();
                let return_screen = state.return_screen;
                let detail = outcome.detail;
                self.refresh();
                if graph_origin {
                    self.load_graph(project);
                    if let Some(graph) = self.graph.as_mut() {
                        graph.message = Some((false, outcome.message));
                    }
                    self.screen = Screen::Graph;
                } else {
                    self.load_repository(project, return_screen);
                    if let Some(state) = self.repository.as_mut() {
                        state.message = Some((false, outcome.message));
                        state.detail = detail;
                    }
                }
            }
            Err(error) => {
                let message = error.to_string();
                if let Some(state) = self.repository.as_mut() {
                    state.action_running = false;
                    state.message = Some((true, message.clone()));
                }
                if graph_origin {
                    if let Some(graph) = self.graph.as_mut() {
                        graph.message = Some((true, message));
                    }
                    self.screen = Screen::Graph;
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
    use crate::domain::{ChangeHunk, CommitRef, CommitRefKind, WorkspaceKind, WorktreeSummary};

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
    fn changed_only_combines_with_search_and_preserves_project_identity() {
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Repo,
            projects: vec![
                project("clean"),
                project("staged"),
                project("modified"),
                project("untracked"),
                project("conflicted"),
            ],
        };
        let mut app = App::new(workspace, 2);
        app.projects[1].worktree.staged = 1;
        app.projects[2].worktree.unstaged = 1;
        app.projects[3].worktree.untracked = 1;
        app.projects[4].worktree.conflicted = 1;
        app.selected = 2;
        app.toggle_changed_only();
        assert_eq!(app.filtered_indices(), vec![1, 2, 3, 4]);
        assert_eq!(app.selected_project().unwrap().project.name, "modified");
        app.search = "untracked".into();
        app.move_selection(0);
        assert_eq!(app.filtered_indices(), vec![3]);
        assert_eq!(app.selected_project().unwrap().project.name, "untracked");
        let untracked_id = app.projects[3].project.id.clone();
        app.search.clear();
        app.restore_workspace_selection(Some(&untracked_id));
        app.toggle_changed_only();
        assert_eq!(app.selected_project().unwrap().project.name, "untracked");
        app.toggle_changed_only();
        assert_eq!(app.selected_project().unwrap().project.name, "untracked");
        app.search = "clean".into();
        app.restore_workspace_selection(Some(&untracked_id));
        assert!(app.selected_project().is_none());
        assert_eq!(app.selected, 0);
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
            selected_files: HashSet::new(),
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
            commit_cursor: 0,
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
            selected_files: HashSet::new(),
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
            commit_cursor: 0,
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
        assert!(matches!(
            pending,
            PendingOperation::Single {
                target: OperationTarget::Hunk {
                    source: HunkSource::Worktree,
                    fingerprint: 22,
                },
                ..
            }
        ));
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

    #[tokio::test]
    async fn changes_multiselect_uses_stable_paths_and_commit_accepts_multiline_paste() {
        let value = project("alpha");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Git,
            projects: vec![value.clone()],
        };
        let mut app = App::new(workspace, 1);
        let entries = ["src/app.rs", "src/main.rs"]
            .into_iter()
            .map(|path| ChangeEntry {
                path: PathBuf::from(path),
                original_path: None,
                index: None,
                worktree: Some(crate::domain::ChangeCode::Modified),
                untracked: false,
                conflicted: false,
            })
            .collect::<Vec<_>>();
        app.changes = Some(ChangesState {
            project: value.clone(),
            return_screen: Screen::Workspace,
            entries: entries.clone(),
            selected: 0,
            selected_files: HashSet::new(),
            mode: ChangesMode::File,
            selected_hunk: 0,
            selected_hunk_identity: None,
            selected_line: 0,
            selected_line_identity: None,
            loading: false,
            error: None,
            generation: 1,
            preview: None,
            preview_path: None,
            preview_loading: false,
            preview_generation: 0,
            preview_scroll: 0,
            operation_running: false,
            operation_generation: 0,
            confirmation: None,
            message: None,
            commit_message: String::new(),
            commit_cursor: 0,
            commit_editing: true,
            commit_amend: false,
            commit_signoff: false,
            commit_signing: false,
            commit_running: false,
            commit_generation: 0,
        });

        app.toggle_change_selected();
        app.move_change_selection(1);
        app.toggle_change_selected();
        let changes = app.changes.as_ref().unwrap();
        assert_eq!(changes.selected_files.len(), 2);
        assert!(changes.selected_files.contains(&entries[0].path));
        assert!(changes.selected_files.contains(&entries[1].path));
        app.toggle_all_changes_selected();
        assert!(app.changes.as_ref().unwrap().selected_files.is_empty());
        app.toggle_all_changes_selected();
        assert_eq!(app.changes.as_ref().unwrap().selected_files.len(), 2);

        app.edit_commit_message(CommitInput::Text("subject\r\n\r\nbody".into()));
        app.edit_commit_message(CommitInput::Newline);
        app.edit_commit_message(CommitInput::Character('x'));
        let changes = app.changes.as_ref().unwrap();
        assert_eq!(changes.commit_message, "subject\n\nbody\nx");
        assert_eq!(changes.commit_cursor, changes.commit_message.len());

        {
            let changes = app.changes.as_mut().unwrap();
            changes.commit_message = "ab你cd".into();
            changes.commit_cursor = 2;
        }
        app.edit_commit_message(CommitInput::Character('X'));
        app.edit_commit_message(CommitInput::Text("1\r\n2\r3".into()));
        let changes = app.changes.as_ref().unwrap();
        assert_eq!(changes.commit_message, "abX1\n2\n3你cd");
        assert_eq!(changes.commit_cursor, "abX1\n2\n3".len());

        app.edit_commit_message(CommitInput::MoveRight);
        assert_eq!(
            app.changes.as_ref().unwrap().commit_cursor,
            "abX1\n2\n3你".len()
        );
        app.edit_commit_message(CommitInput::Backspace);
        let changes = app.changes.as_ref().unwrap();
        assert_eq!(changes.commit_message, "abX1\n2\n3cd");
        assert_eq!(changes.commit_cursor, "abX1\n2\n3".len());
        app.edit_commit_message(CommitInput::Character('你'));
        app.edit_commit_message(CommitInput::MoveLeft);
        app.edit_commit_message(CommitInput::Delete);
        let changes = app.changes.as_ref().unwrap();
        assert_eq!(changes.commit_message, "abX1\n2\n3cd");
        assert_eq!(changes.commit_cursor, "abX1\n2\n3".len());
        app.edit_commit_message(CommitInput::Backspace);
        assert_eq!(app.changes.as_ref().unwrap().commit_message, "abX1\n2\ncd");

        {
            let changes = app.changes.as_mut().unwrap();
            changes.commit_message = "long\nx\nwide".into();
            changes.commit_cursor = 4;
        }
        app.edit_commit_message(CommitInput::MoveDown);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 6);
        app.edit_commit_message(CommitInput::MoveDown);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 8);
        app.edit_commit_message(CommitInput::MoveEnd);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 11);
        app.edit_commit_message(CommitInput::MoveHome);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 7);
        app.edit_commit_message(CommitInput::MoveUp);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 5);
        app.edit_commit_message(CommitInput::MoveUp);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 0);
        app.edit_commit_message(CommitInput::MoveUp);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 0);
        {
            let changes = app.changes.as_mut().unwrap();
            changes.commit_cursor = usize::MAX;
        }
        app.edit_commit_message(CommitInput::MoveLeft);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 10);
        app.edit_commit_message(CommitInput::MoveDown);
        assert_eq!(app.changes.as_ref().unwrap().commit_cursor, 10);

        {
            let changes = app.changes.as_mut().unwrap();
            changes.commit_cursor = 8;
            changes.commit_running = true;
            changes.commit_editing = false;
            changes.commit_generation = 3;
        }
        app.apply_commit(CommitResult {
            project_id: value.id,
            changes_generation: 1,
            commit_generation: 3,
            result: Err(anyhow::anyhow!("hook failed")),
        });
        let changes = app.changes.as_ref().unwrap();
        assert!(changes.commit_editing);
        assert!(!changes.commit_running);
        assert_eq!(changes.commit_cursor, 8);
        assert_eq!(changes.commit_message, "long\nx\nwide");
        assert!(changes
            .message
            .as_ref()
            .is_some_and(|(error, message)| *error && message == "hook failed"));
    }

    #[test]
    fn graph_objects_and_actions_preserve_ref_identity() {
        let value = project("alpha");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Git,
            projects: vec![value.clone()],
        };
        let mut app = App::new(workspace, 1);
        app.graph = Some(GraphState {
            project: value,
            commits: vec![Commit {
                oid: "aaaaaaaa".into(),
                parents: vec![],
                refs: vec![
                    CommitRef {
                        name: "HEAD".into(),
                        kind: CommitRefKind::Head,
                    },
                    CommitRef {
                        name: "main".into(),
                        kind: CommitRefKind::LocalBranch,
                    },
                    CommitRef {
                        name: "origin/main".into(),
                        kind: CommitRefKind::RemoteBranch,
                    },
                    CommitRef {
                        name: "v1".into(),
                        kind: CommitRefKind::Tag,
                    },
                    CommitRef {
                        name: "stash@{0}".into(),
                        kind: CommitRefKind::Stash,
                    },
                ],
                author: "A".into(),
                timestamp: 0,
                subject: "subject".into(),
                body: "body".into(),
            }],
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            object_menu: false,
            object_selected: 0,
            action_menu: false,
            action_selected: 0,
            selected_object: None,
            form: None,
            message: None,
            selected_oid: None,
            filter: GraphFilter::default(),
            filter_form: None,
            filter_error: None,
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });
        let objects = app.graph_objects();
        assert_eq!(objects.len(), 6);
        assert_eq!(objects[1].kind, GraphObjectKind::Head);
        assert_eq!(objects[2].kind, GraphObjectKind::LocalBranch);
        assert_eq!(objects[3].kind, GraphObjectKind::RemoteBranch);
        assert_eq!(objects[4].kind, GraphObjectKind::Tag);
        assert_eq!(objects[5].kind, GraphObjectKind::Stash);
        assert!(
            graph_actions(GraphObjectKind::LocalBranch).contains(&GraphActionChoice::RenameBranch)
        );
        assert!(
            graph_actions(GraphObjectKind::LocalBranch).contains(&GraphActionChoice::DeleteBranch)
        );
        assert!(graph_actions(GraphObjectKind::LocalBranch).contains(&GraphActionChoice::Push));
        assert!(graph_actions(GraphObjectKind::LocalBranch).contains(&GraphActionChoice::ForcePush));
        assert!(!graph_actions(GraphObjectKind::RemoteBranch)
            .contains(&GraphActionChoice::RenameBranch));
        assert!(!graph_actions(GraphObjectKind::RemoteBranch)
            .contains(&GraphActionChoice::DeleteBranch));
        assert!(!graph_actions(GraphObjectKind::RemoteBranch).contains(&GraphActionChoice::Push));
        assert!(
            !graph_actions(GraphObjectKind::RemoteBranch).contains(&GraphActionChoice::ForcePush)
        );
    }

    #[test]
    fn graph_forms_map_push_and_stash_options_to_structured_actions() {
        let branch = GraphObject {
            kind: GraphObjectKind::LocalBranch,
            name: "topic".into(),
            oid: "aaaaaaaa".into(),
        };
        for (choice, force_with_lease) in [
            (GraphActionChoice::Push, false),
            (GraphActionChoice::ForcePush, true),
        ] {
            let mut form = graph_form(choice, branch.clone()).unwrap();
            if let FormField::Toggle { value, .. } = &mut form.fields[2] {
                *value = true;
            }
            assert!(matches!(
                form.action().unwrap(),
                RepositoryAction::Push {
                    ref remote,
                    ref branch,
                    set_upstream: true,
                    force_with_lease: force,
                } if remote == "origin" && branch == "topic" && force == force_with_lease
            ));
        }

        let mut stash = graph_form(GraphActionChoice::StashCreate, branch).unwrap();
        if let FormField::Text { value, .. } = &mut stash.fields[0] {
            *value = "save work".into();
        }
        for field in stash.fields.iter_mut().skip(1) {
            if let FormField::Toggle { value, .. } = field {
                *value = true;
            }
        }
        assert!(matches!(
            stash.action().unwrap(),
            RepositoryAction::StashPush {
                ref message,
                include_untracked: true,
                keep_index: true,
                staged_only: true,
            } if message == "save work"
        ));
    }

    #[test]
    fn graph_filters_branch_history_text_author_and_date_with_stable_selection() {
        let value = project("alpha");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Git,
            projects: vec![value.clone()],
        };
        let commit = |oid: &str,
                      parents: &[&str],
                      refs: Vec<CommitRef>,
                      author: &str,
                      timestamp: i64,
                      subject: &str| Commit {
            oid: oid.into(),
            parents: parents.iter().map(|parent| (*parent).to_owned()).collect(),
            refs,
            author: author.into(),
            timestamp,
            subject: subject.into(),
            body: format!("body {subject}"),
        };
        let mut app = App::new(workspace, 1);
        app.graph = Some(GraphState {
            project: value,
            commits: vec![
                commit(
                    "feature",
                    &["base"],
                    vec![CommitRef {
                        name: "feature/x".into(),
                        kind: CommitRefKind::LocalBranch,
                    }],
                    "Ada",
                    parse_graph_date("2026-08-20", "date").unwrap().unwrap(),
                    "camera fix",
                ),
                commit(
                    "main",
                    &["base"],
                    vec![CommitRef {
                        name: "main".into(),
                        kind: CommitRefKind::LocalBranch,
                    }],
                    "Bob",
                    parse_graph_date("2026-08-21", "date").unwrap().unwrap(),
                    "main work",
                ),
                commit(
                    "base",
                    &[],
                    Vec::new(),
                    "Ada",
                    parse_graph_date("2026-08-10", "date").unwrap().unwrap(),
                    "base commit",
                ),
            ],
            selected: 0,
            loading: false,
            error: None,
            generation: 1,
            object_menu: false,
            object_selected: 0,
            action_menu: false,
            action_selected: 0,
            selected_object: None,
            form: None,
            message: None,
            selected_oid: None,
            filter: GraphFilter {
                branch: "feature".into(),
                ..GraphFilter::default()
            },
            filter_form: None,
            filter_error: None,
            commit_message: String::new(),
            commit_amend: false,
            commit_running: false,
            commit_generation: 0,
        });
        assert_eq!(app.graph.as_ref().unwrap().filtered_indices(), vec![0, 2]);
        app.graph.as_mut().unwrap().selected = 2;
        app.open_graph_filter(false);
        {
            let form = app.graph.as_mut().unwrap().filter_form.as_mut().unwrap();
            form.draft.query = "base".into();
            form.draft.author = "ada".into();
            form.draft.since = "2026-08-01".into();
            form.draft.until = "2026-08-15".into();
        }
        app.submit_graph_filter();
        let graph = app.graph.as_ref().unwrap();
        assert_eq!(graph.filtered_indices(), vec![2]);
        assert_eq!(graph.commits[graph.selected].oid, "base");
        app.open_graph_filter(false);
        app.graph
            .as_mut()
            .unwrap()
            .filter_form
            .as_mut()
            .unwrap()
            .draft
            .since = "2026-09-01".into();
        app.submit_graph_filter();
        assert_eq!(
            app.graph.as_ref().unwrap().filter_error.as_deref(),
            Some("Since must not be later than Until")
        );
        app.cancel_graph_filter();
        app.clear_graph_filter();
        assert_eq!(
            app.graph.as_ref().unwrap().filtered_indices(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn graph_date_filter_rejects_invalid_calendar_dates() {
        assert!(parse_graph_date("2026-02-29", "Since").is_err());
        assert!(parse_graph_date("2024-02-29", "Since").is_ok());
        assert!(parse_graph_date("2026/08/20", "Since").is_err());
    }

    #[test]
    fn repo_selection_is_stable_and_batch_events_are_generation_scoped() {
        let alpha = project("alpha");
        let beta = project("beta");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Repo,
            projects: vec![alpha.clone(), beta.clone()],
        };
        let mut app = App::new(workspace, 1);
        app.toggle_project_selection();
        app.search = "beta".into();
        assert!(app.selected_projects.contains(&alpha.id));
        assert!(!app.selected_projects.contains(&beta.id));

        app.repo_batch.task = Some(RepoBatchTask {
            spec: RepoBatchSpec {
                action: RepoBatchAction::Sync,
                branch: None,
                change: None,
                output: None,
            },
            targets: vec![alpha.clone(), beta.clone()],
            results: vec![
                RepoProjectResult {
                    project: alpha.clone(),
                    state: RepoProjectState::Pending,
                    message: String::new(),
                },
                RepoProjectResult {
                    project: beta.clone(),
                    state: RepoProjectState::Pending,
                    message: String::new(),
                },
            ],
            workspace_result: None,
            args: Vec::new(),
            logs: Vec::new(),
            running: true,
            cancelling: false,
            cancelled: false,
            generation: 7,
        });
        app.apply_repo_batch(RepoBatchEvent {
            generation: 6,
            kind: RepoBatchEventKind::Finished {
                project: Some(alpha.clone()),
                state: RepoProjectState::Failed,
                message: "stale".into(),
            },
        });
        assert_eq!(
            app.repo_batch.task.as_ref().unwrap().results[0].state,
            RepoProjectState::Pending
        );
        app.apply_repo_batch(RepoBatchEvent {
            generation: 7,
            kind: RepoBatchEventKind::StartedBatch {
                projects: vec![alpha.clone(), beta.clone()],
                args: vec![
                    "sync".into(),
                    "-c".into(),
                    "-j8".into(),
                    "--".into(),
                    "alpha".into(),
                    "beta".into(),
                ],
            },
        });
        let task = app.repo_batch.task.as_ref().unwrap();
        assert!(task
            .results
            .iter()
            .all(|result| result.state == RepoProjectState::Running));
        assert_eq!(task.args.len(), 1);
        app.apply_repo_batch(RepoBatchEvent {
            generation: 7,
            kind: RepoBatchEventKind::Finished {
                project: Some(alpha),
                state: RepoProjectState::Failed,
                message: "current".into(),
            },
        });
        assert_eq!(
            app.repo_batch.task.as_ref().unwrap().results[0].state,
            RepoProjectState::Failed
        );
    }

    #[test]
    fn empty_selection_sync_prepares_workspace_confirmation() {
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Repo,
            projects: vec![project("alpha"), project("beta")],
        };
        let mut app = App::new(workspace, 1);

        app.prepare_repo_batch(RepoBatchAction::Sync, String::new());

        let (spec, targets) = app.repo_batch.pending.as_ref().unwrap();
        assert_eq!(spec.action, RepoBatchAction::Sync);
        assert!(targets.is_empty());
        assert!(app.repo_batch.message.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn workspace_sync_events_and_failed_retry_keep_empty_scope() {
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Repo,
            projects: vec![project("alpha")],
        };
        let mut app = App::new(workspace, 1);
        let spec = RepoBatchSpec {
            action: RepoBatchAction::Sync,
            branch: None,
            change: None,
            output: None,
        };
        app.repo_batch.task = Some(RepoBatchTask {
            spec: spec.clone(),
            targets: Vec::new(),
            results: Vec::new(),
            workspace_result: Some((
                RepoProjectState::Pending,
                "Waiting for workspace lock".into(),
            )),
            args: Vec::new(),
            logs: Vec::new(),
            running: true,
            cancelling: false,
            cancelled: false,
            generation: 3,
        });

        app.apply_repo_batch(RepoBatchEvent {
            generation: 3,
            kind: RepoBatchEventKind::StartedBatch {
                projects: Vec::new(),
                args: vec!["sync".into(), "-c".into(), "-j8".into()],
            },
        });
        assert_eq!(
            app.repo_batch
                .task
                .as_ref()
                .unwrap()
                .workspace_result
                .as_ref()
                .unwrap()
                .0,
            RepoProjectState::Running
        );
        app.apply_repo_batch(RepoBatchEvent {
            generation: 3,
            kind: RepoBatchEventKind::Finished {
                project: None,
                state: RepoProjectState::Failed,
                message: "network failure".into(),
            },
        });
        app.repo_batch.task.as_mut().unwrap().running = false;

        app.retry_failed_repo_batch();
        app.repo_batch_handle.as_ref().unwrap().cancel();

        let task = app.repo_batch.task.as_ref().unwrap();
        assert_eq!(task.spec, spec);
        assert!(task.targets.is_empty());
        assert!(task.results.is_empty());
        assert_eq!(
            task.workspace_result.as_ref().unwrap().0,
            RepoProjectState::Pending
        );
    }

    #[test]
    fn workspace_git_events_are_generation_scoped() {
        let alpha = project("alpha");
        let workspace = Workspace {
            root: PathBuf::from("/tmp"),
            kind: WorkspaceKind::Git,
            projects: vec![alpha.clone()],
        };
        let mut app = App::new(workspace, 1);
        let spec = WorkspaceGitSpec {
            action: WorkspaceGitAction::Discard,
            targets: Vec::new(),
        };
        app.workspace_git_generation = 4;
        app.workspace_git.task = Some(WorkspaceGitTask {
            spec,
            results: vec![RepoProjectResult {
                project: alpha.clone(),
                state: RepoProjectState::Pending,
                message: String::new(),
            }],
            running: true,
            generation: 4,
        });
        app.apply_workspace_git(WorkspaceGitEvent {
            generation: 3,
            kind: WorkspaceGitEventKind::Finished {
                project: alpha.clone(),
                state: RepoProjectState::Failed,
                message: "stale".into(),
            },
        });
        assert_eq!(
            app.workspace_git.task.as_ref().unwrap().results[0].state,
            RepoProjectState::Pending
        );
        app.apply_workspace_git(WorkspaceGitEvent {
            generation: 4,
            kind: WorkspaceGitEventKind::Started {
                project: alpha.clone(),
            },
        });
        assert_eq!(
            app.workspace_git.task.as_ref().unwrap().results[0].state,
            RepoProjectState::Running
        );
        app.apply_workspace_git(WorkspaceGitEvent {
            generation: 4,
            kind: WorkspaceGitEventKind::Finished {
                project: alpha,
                state: RepoProjectState::Succeeded,
                message: "done".into(),
            },
        });
        assert_eq!(
            app.workspace_git.task.as_ref().unwrap().results[0].state,
            RepoProjectState::Succeeded
        );
    }
}
