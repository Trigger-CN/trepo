use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Component;

use crate::domain::ChangeEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChangeTreeRow {
    Directory {
        name: String,
        prefix: String,
    },
    File {
        entry_index: usize,
        name: String,
        prefix: String,
    },
}

impl ChangeTreeRow {
    pub(crate) fn display(&self) -> String {
        match self {
            Self::Directory { name, prefix } => format!("{prefix}{name}/"),
            Self::File { name, prefix, .. } => format!("{prefix}{name}"),
        }
    }
}

#[derive(Debug, Default)]
struct TreeNode {
    children: BTreeMap<OsString, TreeNode>,
    entry_index: Option<usize>,
}

pub(crate) fn change_tree_rows(entries: &[ChangeEntry]) -> Vec<ChangeTreeRow> {
    let mut root = TreeNode::default();
    for (entry_index, entry) in entries.iter().enumerate() {
        let mut node = &mut root;
        for component in entry.path.components() {
            let name = match component {
                Component::Normal(name) => name.to_os_string(),
                other => OsString::from(other.as_os_str()),
            };
            node = node.children.entry(name).or_default();
        }
        node.entry_index = Some(entry_index);
    }

    let mut rows = Vec::new();
    flatten_tree(&root, entries, &mut Vec::new(), &mut rows);
    rows
}

fn flatten_tree(
    node: &TreeNode,
    entries: &[ChangeEntry],
    ancestors_last: &mut Vec<bool>,
    rows: &mut Vec<ChangeTreeRow>,
) {
    let child_count = node.children.len();
    for (position, (name, child)) in node.children.iter().enumerate() {
        let is_last = position + 1 == child_count;
        let prefix = tree_prefix(ancestors_last, is_last);
        let display_name = name.to_string_lossy().into_owned();
        if let Some(entry_index) = child.entry_index {
            let entry = &entries[entry_index];
            let name = entry
                .original_path
                .as_ref()
                .map_or(display_name, |original| {
                    format!("{} -> {}", original.display(), entry.path.display())
                });
            rows.push(ChangeTreeRow::File {
                entry_index,
                name,
                prefix,
            });
        } else {
            rows.push(ChangeTreeRow::Directory {
                name: display_name,
                prefix,
            });
        }
        ancestors_last.push(is_last);
        flatten_tree(child, entries, ancestors_last, rows);
        ancestors_last.pop();
    }
}

fn tree_prefix(ancestors_last: &[bool], is_last: bool) -> String {
    let mut prefix = ancestors_last
        .iter()
        .map(|ancestor_last| if *ancestor_last { "  " } else { "│ " })
        .collect::<String>();
    prefix.push_str(if is_last { "└─" } else { "├─" });
    prefix
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(path: &str) -> ChangeEntry {
        ChangeEntry {
            path: PathBuf::from(path),
            original_path: None,
            index: None,
            worktree: None,
            untracked: true,
            conflicted: false,
        }
    }

    #[test]
    fn groups_changed_files_by_directory_without_changing_entry_identity() {
        let entries = vec![
            entry("README.md"),
            entry("src/app/state.rs"),
            entry("src/main.rs"),
            entry("tests/ui.rs"),
        ];
        let rows = change_tree_rows(&entries);
        assert_eq!(
            rows.iter().map(ChangeTreeRow::display).collect::<Vec<_>>(),
            vec![
                "├─README.md",
                "├─src/",
                "│ ├─app/",
                "│ │ └─state.rs",
                "│ └─main.rs",
                "└─tests/",
                "  └─ui.rs",
            ]
        );
        assert!(matches!(
            rows[3],
            ChangeTreeRow::File { entry_index: 1, .. }
        ));
        assert!(matches!(
            rows[4],
            ChangeTreeRow::File { entry_index: 2, .. }
        ));
    }
}
