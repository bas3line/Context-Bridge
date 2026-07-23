use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::ProjectError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProject {
    pub root: PathBuf,
    pub is_git: bool,
}

pub async fn resolve_project_root(path: &Path) -> Result<ResolvedProject, ProjectError> {
    let candidate = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|source| ProjectError::Canonicalize {
            path: candidate.to_path_buf(),
            source,
        })?;

    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&canonical)
        .output()
        .await;
    match output {
        Ok(output) if output.status.success() => {
            let raw_root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
            let root = raw_root
                .canonicalize()
                .map_err(|source| ProjectError::Canonicalize {
                    path: raw_root,
                    source,
                })?;
            Ok(ResolvedProject { root, is_git: true })
        }
        _ => Ok(ResolvedProject {
            root: canonical,
            is_git: false,
        }),
    }
}
