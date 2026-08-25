use anyhow::{bail, Result};

use crate::domain::{RepositoryAction, RepositorySnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryTab {
    Status,
    Stashes,
    Refs,
    Remotes,
}

impl RepositoryTab {
    pub const ALL: [Self; 4] = [Self::Status, Self::Stashes, Self::Refs, Self::Remotes];

    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Stashes => "Stashes",
            Self::Refs => "Branches & Tags",
            Self::Remotes => "Remotes",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryChoice {
    TakeOurs,
    TakeTheirs,
    MarkResolved,
    Continue,
    Skip,
    Abort,
    StashShow,
    StashPush,
    StashApply,
    StashPop,
    StashDrop,
    BranchCreate,
    BranchSwitch,
    BranchRename,
    BranchDelete,
    TagCreate,
    TagDelete,
    Merge,
    Rebase,
    CherryPick,
    Revert,
    RemoteAdd,
    RemoteSetUrl,
    RemoteRemove,
    Fetch,
    Pull,
    Push,
    SetUpstream,
    RemotePrune,
}

impl RepositoryChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::TakeOurs => "Take ours",
            Self::TakeTheirs => "Take theirs",
            Self::MarkResolved => "Mark resolved",
            Self::Continue => "Continue operation",
            Self::Skip => "Skip commit",
            Self::Abort => "Abort operation",
            Self::StashShow => "Show stash patch",
            Self::StashPush => "Create stash",
            Self::StashApply => "Apply stash",
            Self::StashPop => "Pop stash",
            Self::StashDrop => "Drop stash",
            Self::BranchCreate => "Create branch",
            Self::BranchSwitch => "Switch branch",
            Self::BranchRename => "Rename branch",
            Self::BranchDelete => "Delete branch",
            Self::TagCreate => "Create tag",
            Self::TagDelete => "Delete tag",
            Self::Merge => "Merge ref",
            Self::Rebase => "Rebase onto ref",
            Self::CherryPick => "Cherry-pick OID",
            Self::Revert => "Revert OID",
            Self::RemoteAdd => "Add remote",
            Self::RemoteSetUrl => "Set remote URL",
            Self::RemoteRemove => "Remove remote",
            Self::Fetch => "Fetch remote",
            Self::Pull => "Pull branch",
            Self::Push => "Push branch",
            Self::SetUpstream => "Set upstream",
            Self::RemotePrune => "Prune remote",
        }
    }
}

pub fn choices(tab: RepositoryTab) -> &'static [RepositoryChoice] {
    match tab {
        RepositoryTab::Status => &[
            RepositoryChoice::TakeOurs,
            RepositoryChoice::TakeTheirs,
            RepositoryChoice::MarkResolved,
            RepositoryChoice::Continue,
            RepositoryChoice::Skip,
            RepositoryChoice::Abort,
        ],
        RepositoryTab::Stashes => &[
            RepositoryChoice::StashShow,
            RepositoryChoice::StashPush,
            RepositoryChoice::StashApply,
            RepositoryChoice::StashPop,
            RepositoryChoice::StashDrop,
        ],
        RepositoryTab::Refs => &[
            RepositoryChoice::BranchCreate,
            RepositoryChoice::BranchSwitch,
            RepositoryChoice::BranchRename,
            RepositoryChoice::BranchDelete,
            RepositoryChoice::TagCreate,
            RepositoryChoice::TagDelete,
            RepositoryChoice::Merge,
            RepositoryChoice::Rebase,
            RepositoryChoice::CherryPick,
            RepositoryChoice::Revert,
        ],
        RepositoryTab::Remotes => &[
            RepositoryChoice::RemoteAdd,
            RepositoryChoice::RemoteSetUrl,
            RepositoryChoice::RemoteRemove,
            RepositoryChoice::Fetch,
            RepositoryChoice::Pull,
            RepositoryChoice::Push,
            RepositoryChoice::SetUpstream,
            RepositoryChoice::RemotePrune,
        ],
    }
}

pub fn action_preview(action: &RepositoryAction, snapshot: &RepositorySnapshot) -> Vec<String> {
    match action {
        RepositoryAction::Push {
            remote,
            branch,
            set_upstream,
            force_with_lease,
        } => {
            let local = snapshot.branches.iter().find(|entry| entry.name == *branch);
            let remote_name = format!("{remote}/{branch}");
            let remote_branch = snapshot
                .remote_branches
                .iter()
                .find(|entry| entry.name == remote_name);
            let range = match (remote_branch, local) {
                (Some(remote), Some(local)) => format!("{}..{}", remote.oid, local.oid),
                (None, Some(local)) => format!("new branch -> {}", local.oid),
                _ => "unresolved until execution".to_owned(),
            };
            vec![
                format!("Remote: {remote}"),
                format!("Refspec: {branch}:{branch}"),
                format!("Commit range: {range}"),
                format!("Set upstream: {}", on_off(*set_upstream)),
                format!("Force with lease: {}", on_off(*force_with_lease)),
            ]
        }
        RepositoryAction::RemoteAdd { name, url }
        | RepositoryAction::RemoteSetUrl { name, url } => vec![
            format!("Remote: {name}"),
            format!("URL: {}", redact_url(url)),
        ],
        RepositoryAction::Pull {
            remote,
            branch,
            rebase,
        } => vec![
            format!("Remote: {remote}"),
            format!("Branch: {branch}"),
            format!("Rebase: {}", on_off(*rebase)),
        ],
        RepositoryAction::Fetch { remote, prune } => vec![
            format!("Remote: {remote}"),
            format!("Prune: {}", on_off(*prune)),
        ],
        RepositoryAction::RemotePrune { remote }
        | RepositoryAction::RemoteRemove { name: remote } => vec![format!("Remote: {remote}")],
        RepositoryAction::SetUpstream { branch, upstream } => {
            vec![format!("Branch: {branch}"), format!("Upstream: {upstream}")]
        }
        _ => Vec::new(),
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn redact_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let authority_start = scheme_end + 3;
    let authority_end = url[authority_start..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |index| authority_start + index);
    let Some(at) = url[authority_start..authority_end].rfind('@') else {
        return url.to_owned();
    };
    let at = authority_start + at;
    format!("{}***{}", &url[..authority_start], &url[at..])
}

#[derive(Debug, Clone)]
pub enum FormField {
    Text { label: &'static str, value: String },
    Toggle { label: &'static str, value: bool },
}

impl FormField {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Text { label, .. } | Self::Toggle { label, .. } => label,
        }
    }

    pub fn display_value(&self) -> String {
        match self {
            Self::Text { value, .. } => value.clone(),
            Self::Toggle { value, .. } => if *value { "on" } else { "off" }.to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepositoryForm {
    pub choice: RepositoryChoice,
    pub fields: Vec<FormField>,
    pub selected: usize,
}

impl RepositoryForm {
    pub fn edit_char(&mut self, value: char) {
        if let Some(FormField::Text { value: text, .. }) = self.fields.get_mut(self.selected) {
            text.push(value);
        }
    }

    pub fn backspace(&mut self) {
        if let Some(FormField::Text { value, .. }) = self.fields.get_mut(self.selected) {
            value.pop();
        }
    }

    pub fn toggle(&mut self) {
        if let Some(FormField::Toggle { value, .. }) = self.fields.get_mut(self.selected) {
            *value = !*value;
        }
    }

    fn text(&self, index: usize) -> Result<String> {
        match self.fields.get(index) {
            Some(FormField::Text { value, .. }) if !value.trim().is_empty() => {
                Ok(value.trim().to_owned())
            }
            Some(FormField::Text { label, .. }) => bail!("{label} cannot be empty"),
            _ => bail!("invalid action form"),
        }
    }

    fn optional_text(&self, index: usize) -> Result<Option<String>> {
        match self.fields.get(index) {
            Some(FormField::Text { value, .. }) => {
                Ok((!value.trim().is_empty()).then(|| value.trim().to_owned()))
            }
            _ => bail!("invalid action form"),
        }
    }

    fn toggle_value(&self, index: usize) -> Result<bool> {
        match self.fields.get(index) {
            Some(FormField::Toggle { value, .. }) => Ok(*value),
            _ => bail!("invalid action form"),
        }
    }

    pub fn action(&self) -> Result<RepositoryAction> {
        use RepositoryAction as A;
        use RepositoryChoice as C;
        Ok(match self.choice {
            C::TakeOurs => A::ConflictTakeOurs {
                path: self.text(0)?.into(),
            },
            C::TakeTheirs => A::ConflictTakeTheirs {
                path: self.text(0)?.into(),
            },
            C::MarkResolved => A::ConflictMarkResolved {
                path: self.text(0)?.into(),
            },
            C::Continue => bail!("operation action requires current state"),
            C::Skip => bail!("operation action requires current state"),
            C::Abort => bail!("operation action requires current state"),
            C::StashShow => A::StashShow {
                selector: self.text(0)?,
            },
            C::StashPush => A::StashPush {
                message: self.optional_text(0)?.unwrap_or_default(),
                include_untracked: self.toggle_value(1)?,
            },
            C::StashApply => A::StashApply {
                selector: self.text(0)?,
            },
            C::StashPop => A::StashPop {
                selector: self.text(0)?,
            },
            C::StashDrop => A::StashDrop {
                selector: self.text(0)?,
            },
            C::BranchCreate => A::BranchCreate {
                name: self.text(0)?,
                start: self.optional_text(1)?,
            },
            C::BranchSwitch => A::BranchSwitch {
                name: self.text(0)?,
            },
            C::BranchRename => A::BranchRename {
                old: self.text(0)?,
                new: self.text(1)?,
            },
            C::BranchDelete => A::BranchDelete {
                name: self.text(0)?,
                force: self.toggle_value(1)?,
            },
            C::TagCreate => A::TagCreate {
                name: self.text(0)?,
                target: self.text(1)?,
            },
            C::TagDelete => A::TagDelete {
                name: self.text(0)?,
            },
            C::Merge => A::Merge {
                reference: self.text(0)?,
            },
            C::Rebase => A::Rebase {
                reference: self.text(0)?,
            },
            C::CherryPick => A::CherryPick { oid: self.text(0)? },
            C::Revert => A::Revert { oid: self.text(0)? },
            C::RemoteAdd => A::RemoteAdd {
                name: self.text(0)?,
                url: self.text(1)?,
            },
            C::RemoteSetUrl => A::RemoteSetUrl {
                name: self.text(0)?,
                url: self.text(1)?,
            },
            C::RemoteRemove => A::RemoteRemove {
                name: self.text(0)?,
            },
            C::Fetch => A::Fetch {
                remote: self.text(0)?,
                prune: self.toggle_value(1)?,
            },
            C::Pull => A::Pull {
                remote: self.text(0)?,
                branch: self.text(1)?,
                rebase: self.toggle_value(2)?,
            },
            C::Push => A::Push {
                remote: self.text(0)?,
                branch: self.text(1)?,
                set_upstream: self.toggle_value(2)?,
                force_with_lease: self.toggle_value(3)?,
            },
            C::SetUpstream => A::SetUpstream {
                branch: self.text(0)?,
                upstream: self.text(1)?,
            },
            C::RemotePrune => A::RemotePrune {
                remote: self.text(0)?,
            },
        })
    }
}

fn text_field(label: &'static str, value: impl Into<String>) -> FormField {
    FormField::Text {
        label,
        value: value.into(),
    }
}

fn toggle_field(label: &'static str, value: bool) -> FormField {
    FormField::Toggle { label, value }
}

pub fn form_for(
    choice: RepositoryChoice,
    snapshot: &RepositorySnapshot,
    selected: usize,
) -> Result<Option<RepositoryForm>> {
    use RepositoryChoice as C;
    if matches!(choice, C::Continue | C::Skip | C::Abort) {
        return Ok(None);
    }
    let conflict = snapshot
        .conflicts
        .get(selected)
        .map(|path| path.to_string_lossy().into_owned());
    let stash = snapshot
        .stashes
        .get(selected)
        .map(|entry| entry.selector.clone());
    let branch = snapshot
        .branches
        .get(selected)
        .map(|entry| entry.name.clone());
    let tag_index = selected.saturating_sub(snapshot.branches.len());
    let tag = snapshot.tags.get(tag_index).map(|entry| entry.name.clone());
    let remote = snapshot
        .remotes
        .get(selected)
        .map(|entry| entry.name.clone());
    let current_branch = snapshot
        .branches
        .iter()
        .find(|entry| entry.current)
        .map(|entry| entry.name.clone());
    let fields = match choice {
        C::TakeOurs | C::TakeTheirs | C::MarkResolved => {
            vec![text_field("Path", conflict.unwrap_or_default())]
        }
        C::StashShow | C::StashApply | C::StashPop | C::StashDrop => vec![text_field(
            "Stash",
            stash.unwrap_or_else(|| "stash@{0}".to_owned()),
        )],
        C::StashPush => vec![
            text_field("Message", ""),
            toggle_field("Include untracked", false),
        ],
        C::BranchCreate => vec![
            text_field("Name", ""),
            text_field("Start ref (optional)", ""),
        ],
        C::BranchSwitch => vec![text_field("Branch", branch.unwrap_or_default())],
        C::BranchRename => vec![
            text_field("Old name", branch.unwrap_or_default()),
            text_field("New name", ""),
        ],
        C::BranchDelete => vec![
            text_field("Branch", branch.unwrap_or_default()),
            toggle_field("Force", false),
        ],
        C::TagCreate => vec![text_field("Tag", ""), text_field("Target", "HEAD")],
        C::TagDelete => vec![text_field("Tag", tag.unwrap_or_default())],
        C::Merge | C::Rebase => vec![text_field("Reference", branch.unwrap_or_default())],
        C::CherryPick | C::Revert => vec![text_field("Commit OID", "")],
        C::RemoteAdd => vec![text_field("Remote", ""), text_field("URL", "")],
        C::RemoteSetUrl => vec![
            text_field("Remote", remote.unwrap_or_default()),
            text_field("URL", ""),
        ],
        C::RemoteRemove | C::RemotePrune => vec![text_field("Remote", remote.unwrap_or_default())],
        C::Fetch => vec![
            text_field("Remote", remote.unwrap_or_else(|| "origin".to_owned())),
            toggle_field("Prune", true),
        ],
        C::Pull => vec![
            text_field("Remote", remote.unwrap_or_else(|| "origin".to_owned())),
            text_field("Branch", current_branch.clone().unwrap_or_default()),
            toggle_field("Rebase", false),
        ],
        C::Push => vec![
            text_field("Remote", remote.unwrap_or_else(|| "origin".to_owned())),
            text_field("Branch", current_branch.clone().unwrap_or_default()),
            toggle_field("Set upstream", false),
            toggle_field("Force with lease", false),
        ],
        C::SetUpstream => vec![
            text_field("Branch", current_branch.unwrap_or_default()),
            text_field("Upstream", "origin/"),
        ],
        C::Continue | C::Skip | C::Abort => unreachable!(),
    };
    Ok(Some(RepositoryForm {
        choice,
        fields,
        selected: 0,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{BranchEntry, RemoteBranchEntry, RemoteEntry, RepositorySnapshot};

    fn snapshot() -> RepositorySnapshot {
        RepositorySnapshot {
            operation: None,
            conflicts: Vec::new(),
            stashes: Vec::new(),
            branches: vec![BranchEntry {
                name: "main".into(),
                oid: "bbbbbbbb".into(),
                upstream: Some("origin/main".into()),
                ahead: 1,
                behind: 0,
                current: true,
            }],
            tags: Vec::new(),
            remotes: vec![RemoteEntry {
                name: "origin".into(),
                fetch_url: "https://example.com/repo.git".into(),
                push_url: "https://example.com/repo.git".into(),
            }],
            remote_branches: vec![RemoteBranchEntry {
                name: "origin/main".into(),
                oid: "aaaaaaaa".into(),
            }],
            worktree_token: 0,
            token: 1,
        }
    }

    #[test]
    fn push_preview_contains_exact_ref_range_and_force_mode() {
        let lines = action_preview(
            &RepositoryAction::Push {
                remote: "origin".into(),
                branch: "main".into(),
                set_upstream: true,
                force_with_lease: true,
            },
            &snapshot(),
        );
        assert!(lines.iter().any(|line| line == "Refspec: main:main"));
        assert!(lines
            .iter()
            .any(|line| line == "Commit range: aaaaaaaa..bbbbbbbb"));
        assert!(lines.iter().any(|line| line == "Force with lease: on"));
    }

    #[test]
    fn remote_url_preview_redacts_userinfo() {
        let lines = action_preview(
            &RepositoryAction::RemoteAdd {
                name: "origin".into(),
                url: "https://user:secret@example.com/repo.git".into(),
            },
            &snapshot(),
        );
        assert_eq!(lines[1], "URL: https://***@example.com/repo.git");
        assert!(!lines[1].contains("secret"));
    }
}
