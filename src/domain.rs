use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceKind {
    Repo,
    Git,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub kind: WorkspaceKind,
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectId(pub PathBuf);

#[derive(Debug, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadState {
    Branch(String),
    Detached(String),
    Unborn(String),
    Unknown,
}

impl HeadState {
    pub fn label(&self) -> &str {
        match self {
            Self::Branch(name) | Self::Detached(name) | Self::Unborn(name) => name,
            Self::Unknown => "-",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeSummary {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl WorktreeSummary {
    pub fn is_dirty(&self) -> bool {
        self.staged + self.unstaged + self.untracked + self.conflicted > 0
    }

    pub fn status_label(&self) -> String {
        if self.conflicted > 0 {
            format!("!{}", self.conflicted)
        } else if self.is_dirty() {
            format!("M{} S{} ?{}", self.unstaged, self.staged, self.untracked)
        } else {
            "clean".to_owned()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpstreamState {
    pub name: String,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Debug, Clone)]
pub enum ScanState {
    Pending,
    Ready,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    pub project: Project,
    pub head: HeadState,
    pub upstream: Option<UpstreamState>,
    pub worktree: WorktreeSummary,
    pub scan: ScanState,
    pub generation: u64,
}

impl ProjectSnapshot {
    pub fn pending(project: Project, generation: u64) -> Self {
        Self {
            project,
            head: HeadState::Unknown,
            upstream: None,
            worktree: WorktreeSummary::default(),
            scan: ScanState::Pending,
            generation,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceSummary {
    pub total: usize,
    pub dirty: usize,
    pub conflicted: usize,
    pub ahead: usize,
    pub behind: usize,
    pub errors: usize,
}

impl WorkspaceSummary {
    pub fn from_projects(projects: &[ProjectSnapshot]) -> Self {
        let mut summary = Self {
            total: projects.len(),
            ..Self::default()
        };
        for project in projects {
            if project.worktree.is_dirty() {
                summary.dirty += 1;
            }
            if project.worktree.conflicted > 0 {
                summary.conflicted += 1;
            }
            if let Some(upstream) = &project.upstream {
                if upstream.ahead > 0 {
                    summary.ahead += 1;
                }
                if upstream.behind > 0 {
                    summary.behind += 1;
                }
            }
            if matches!(project.scan, ScanState::Error(_)) {
                summary.errors += 1;
            }
        }
        summary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeCode {
    Added,
    Copied,
    Deleted,
    Modified,
    Renamed,
    TypeChanged,
    Updated,
    Unknown(char),
}

impl ChangeCode {
    pub fn from_byte(value: u8) -> Option<Self> {
        match value {
            b'.' | b' ' => None,
            b'A' => Some(Self::Added),
            b'C' => Some(Self::Copied),
            b'D' => Some(Self::Deleted),
            b'M' => Some(Self::Modified),
            b'R' => Some(Self::Renamed),
            b'T' => Some(Self::TypeChanged),
            b'U' => Some(Self::Updated),
            value => Some(Self::Unknown(char::from(value))),
        }
    }

    pub fn symbol(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Copied => 'C',
            Self::Deleted => 'D',
            Self::Modified => 'M',
            Self::Renamed => 'R',
            Self::TypeChanged => 'T',
            Self::Updated => 'U',
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEntry {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub index: Option<ChangeCode>,
    pub worktree: Option<ChangeCode>,
    pub untracked: bool,
    pub conflicted: bool,
}

impl ChangeEntry {
    pub fn status_label(&self) -> String {
        if self.untracked {
            "??".to_owned()
        } else if self.conflicted {
            "UU".to_owned()
        } else {
            format!(
                "{}{}",
                self.index.map_or('.', ChangeCode::symbol),
                self.worktree.map_or('.', ChangeCode::symbol)
            )
        }
    }

    pub fn can_stage(&self) -> bool {
        self.untracked || self.worktree.is_some() || self.conflicted
    }

    pub fn can_unstage(&self) -> bool {
        !self.untracked && self.index.is_some()
    }

    pub fn can_restore(&self) -> bool {
        !self.untracked && !self.conflicted && self.worktree.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HunkSource {
    Staged,
    Worktree,
    Untracked,
}

impl HunkSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Staged => "Staged",
            Self::Worktree => "Worktree",
            Self::Untracked => "Untracked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeHunk {
    pub source: HunkSource,
    pub header: String,
    pub display_start: usize,
    pub display_end: usize,
    pub fingerprint: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePreview {
    pub text: String,
    pub token: u64,
    pub truncated: bool,
    pub hunks: Vec<ChangeHunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Stage,
    Unstage,
    RestoreWorktree,
}

impl OperationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stage => "Stage",
            Self::Unstage => "Unstage",
            Self::RestoreWorktree => "Discard worktree changes",
        }
    }

    pub fn risk(self) -> RiskLevel {
        match self {
            Self::Stage | Self::Unstage => RiskLevel::ReversibleWrite,
            Self::RestoreWorktree => RiskLevel::Destructive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    ReversibleWrite,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationTarget {
    File,
    Hunk {
        source: HunkSource,
        fingerprint: u64,
    },
}

#[derive(Debug, Clone)]
pub struct OperationSpec {
    pub project: Project,
    pub change: ChangeEntry,
    pub kind: OperationKind,
    pub target: OperationTarget,
    pub expected_token: u64,
}

#[derive(Debug, Clone)]
pub struct OperationOutcome {
    pub kind: OperationKind,
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct CommitSpec {
    pub message: String,
    pub amend: bool,
    pub signoff: bool,
    pub signing: bool,
}

#[derive(Debug, Clone)]
pub struct CommitOutcome {
    pub oid: String,
    pub message: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitRefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CommitRef {
    pub name: String,
    pub kind: CommitRefKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub oid: String,
    pub parents: Vec<String>,
    pub refs: Vec<CommitRef>,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    pub body: String,
}
