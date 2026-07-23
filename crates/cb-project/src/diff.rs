use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::ProjectSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
    Moved { from: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
}

#[must_use]
pub fn diff_snapshots(before: &ProjectSnapshot, after: &ProjectSnapshot) -> Vec<ProjectFileChange> {
    let before_paths: BTreeSet<_> = before.filesystem.files.keys().cloned().collect();
    let after_paths: BTreeSet<_> = after.filesystem.files.keys().cloned().collect();
    let mut deleted: Vec<_> = before_paths.difference(&after_paths).cloned().collect();
    let mut created: Vec<_> = after_paths.difference(&before_paths).cloned().collect();
    let mut changes = Vec::new();

    let mut moved = Vec::new();
    for deleted_path in &deleted {
        let Some(deleted_hash) = before.filesystem.files[deleted_path].content_hash.as_ref() else {
            continue;
        };
        if let Some((index, created_path)) = created.iter().enumerate().find(|(_, created_path)| {
            after.filesystem.files[*created_path].content_hash.as_ref() == Some(deleted_hash)
        }) {
            moved.push((deleted_path.clone(), index, created_path.clone()));
        }
    }
    for (from, index, path) in moved.into_iter().rev() {
        created.remove(index);
        deleted.retain(|candidate| candidate != &from);
        changes.push(ProjectFileChange {
            path,
            kind: FileChangeKind::Moved { from },
        });
    }

    changes.extend(created.into_iter().map(|path| ProjectFileChange {
        path,
        kind: FileChangeKind::Created,
    }));
    changes.extend(deleted.into_iter().map(|path| ProjectFileChange {
        path,
        kind: FileChangeKind::Deleted,
    }));
    changes.extend(
        before_paths
            .intersection(&after_paths)
            .filter(|path| before.filesystem.files[*path] != after.filesystem.files[*path])
            .cloned()
            .map(|path| ProjectFileChange {
                path,
                kind: FileChangeKind::Modified,
            }),
    );
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    changes
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::Path};

    use chrono::Utc;

    use crate::{FileMetadata, FilesystemSnapshot, GitSnapshot, ProjectSnapshot};

    use super::{FileChangeKind, diff_snapshots};

    #[test]
    fn detects_created_modified_deleted_and_moved_files() {
        let before = snapshot([
            ("deleted.txt", metadata("deleted", 1)),
            ("modified.txt", metadata("before", 1)),
            ("old-name.txt", metadata("moved", 1)),
        ]);
        let after = snapshot([
            ("created.txt", metadata("created", 1)),
            ("modified.txt", metadata("after", 2)),
            ("new-name.txt", metadata("moved", 1)),
        ]);
        let changes = diff_snapshots(&before, &after);
        assert!(changes.iter().any(|change| {
            change.path == Path::new("created.txt") && change.kind == FileChangeKind::Created
        }));
        assert!(changes.iter().any(|change| {
            change.path == Path::new("modified.txt") && change.kind == FileChangeKind::Modified
        }));
        assert!(changes.iter().any(|change| {
            change.path == Path::new("deleted.txt") && change.kind == FileChangeKind::Deleted
        }));
        assert!(changes.iter().any(|change| {
            change.path == Path::new("new-name.txt")
                && change.kind
                    == FileChangeKind::Moved {
                        from: "old-name.txt".into(),
                    }
        }));
    }

    fn snapshot<const N: usize>(files: [(&str, FileMetadata); N]) -> ProjectSnapshot {
        ProjectSnapshot {
            captured_at: Utc::now(),
            git: GitSnapshot::default(),
            filesystem: FilesystemSnapshot {
                files: files
                    .into_iter()
                    .map(|(path, metadata)| (path.into(), metadata))
                    .collect::<BTreeMap<_, _>>(),
            },
        }
    }

    fn metadata(hash: &str, modified: u128) -> FileMetadata {
        FileMetadata {
            size: 1,
            modified_unix_nanos: modified,
            content_hash: Some(hash.to_owned()),
        }
    }
}
