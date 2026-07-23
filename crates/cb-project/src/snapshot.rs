use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    FilesystemSnapshot, GitSnapshot, ProjectError, capture_filesystem_snapshot,
    capture_filesystem_snapshot_with_policy, capture_git_snapshot_with_policy,
};
use cb_security::PathPolicy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub captured_at: DateTime<Utc>,
    pub git: GitSnapshot,
    pub filesystem: FilesystemSnapshot,
}

pub async fn capture_project_snapshot(root: &Path) -> Result<ProjectSnapshot, ProjectError> {
    let policy = PathPolicy::new(&[]).map_err(ProjectError::Security)?;
    let filesystem = capture_filesystem_snapshot(root)?;
    let git = capture_git_snapshot_with_policy(root, &policy).await?;
    Ok(ProjectSnapshot {
        captured_at: Utc::now(),
        git,
        filesystem,
    })
}

pub async fn capture_project_snapshot_with_policy(
    root: &Path,
    policy: &PathPolicy,
) -> Result<ProjectSnapshot, ProjectError> {
    let filesystem = capture_filesystem_snapshot_with_policy(root, policy)?;
    let git = capture_git_snapshot_with_policy(root, policy).await?;
    Ok(ProjectSnapshot {
        captured_at: Utc::now(),
        git,
        filesystem,
    })
}
