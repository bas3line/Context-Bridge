use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use cb_core::GitContext;
use cb_security::PathPolicy;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, process::Command};

use crate::ProjectError;

const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_GIT_STDERR_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitSnapshot {
    pub available: bool,
    pub branch: Option<String>,
    pub detached: bool,
    pub head: Option<String>,
    pub status: String,
    pub staged_diff: String,
    pub unstaged_diff: String,
    pub untracked_files: Vec<PathBuf>,
    pub submodules: String,
    pub truncated: bool,
}

impl From<&GitSnapshot> for GitContext {
    fn from(snapshot: &GitSnapshot) -> Self {
        Self {
            branch: snapshot.branch.clone(),
            head: snapshot.head.clone(),
            status: snapshot.status.clone(),
            staged_diff: snapshot.staged_diff.clone(),
            unstaged_diff: snapshot.unstaged_diff.clone(),
            untracked_files: snapshot.untracked_files.clone(),
        }
    }
}

pub async fn capture_git_snapshot(root: &Path) -> Result<GitSnapshot, ProjectError> {
    if !git_succeeds(root, &["rev-parse", "--is-inside-work-tree"]).await {
        return Ok(GitSnapshot::default());
    }

    let branch = git_optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?;
    let head = git_optional(root, &["rev-parse", "--verify", "HEAD"]).await?;
    let status = git_required(root, &["status", "--porcelain=v1", "--branch"]).await?;
    let (staged_diff, staged_truncated) =
        git_required_limited(root, &["diff", "--cached", "--no-ext-diff", "--binary"]).await?;
    let (unstaged_diff, unstaged_truncated) =
        git_required_limited(root, &["diff", "--no-ext-diff", "--binary"]).await?;
    let untracked_raw =
        git_required(root, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    let untracked_files = untracked_raw
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect();
    let submodules = git_required(root, &["submodule", "status", "--recursive"])
        .await
        .unwrap_or_default();
    Ok(GitSnapshot {
        available: true,
        branch,
        detached: head.is_some() && git_detached(root).await,
        head,
        status,
        staged_diff,
        unstaged_diff,
        untracked_files,
        submodules,
        truncated: staged_truncated || unstaged_truncated,
    })
}

pub async fn capture_git_snapshot_with_policy(
    root: &Path,
    policy: &PathPolicy,
) -> Result<GitSnapshot, ProjectError> {
    let mut snapshot = capture_git_snapshot(root).await?;
    snapshot.status = filter_status(&snapshot.status, policy);
    snapshot.staged_diff = filter_diff(&snapshot.staged_diff, policy);
    snapshot.unstaged_diff = filter_diff(&snapshot.unstaged_diff, policy);
    snapshot
        .untracked_files
        .retain(|path| !policy.is_excluded(path));
    Ok(snapshot)
}

async fn git_succeeds(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .is_ok_and(|output| output.status.success())
}

async fn git_detached(root: &Path) -> bool {
    !git_succeeds(root, &["symbolic-ref", "--quiet", "HEAD"]).await
}

async fn git_optional(root: &Path, args: &[&str]) -> Result<Option<String>, ProjectError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .map_err(|source| ProjectError::Git {
            command: format!("git {}", args.join(" ")),
            source,
        })?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ))
    } else {
        Ok(None)
    }
}

async fn git_required(root: &Path, args: &[&str]) -> Result<String, ProjectError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .map_err(|source| ProjectError::Git {
            command: format!("git {}", args.join(" ")),
            source,
        })?;
    if !output.status.success() {
        return Err(ProjectError::GitCommand {
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn git_required_limited(root: &Path, args: &[&str]) -> Result<(String, bool), ProjectError> {
    let command = format!("git {}", args.join(" "));
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ProjectError::Git {
            command: command.clone(),
            source,
        })?;
    let stdout = child.stdout.take().ok_or_else(|| ProjectError::Git {
        command: command.clone(),
        source: std::io::Error::other("git stdout was not captured"),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| ProjectError::Git {
        command: command.clone(),
        source: std::io::Error::other("git stderr was not captured"),
    })?;
    let (status, stdout, stderr) = tokio::join!(
        child.wait(),
        read_limited(stdout, MAX_DIFF_BYTES),
        read_limited(stderr, MAX_GIT_STDERR_BYTES),
    );
    let status = status.map_err(|source| ProjectError::Git {
        command: command.clone(),
        source,
    })?;
    let (stdout, truncated) = stdout.map_err(|source| ProjectError::Git {
        command: command.clone(),
        source,
    })?;
    let (stderr, stderr_truncated) = stderr.map_err(|source| ProjectError::Git {
        command: command.clone(),
        source,
    })?;
    if !status.success() {
        let mut stderr = String::from_utf8_lossy(&stderr).trim().to_owned();
        if stderr_truncated {
            stderr.push_str("\n[git stderr truncated by Context Bridge]");
        }
        return Err(ProjectError::GitCommand { command, stderr });
    }
    let mut diff = String::from_utf8_lossy(&stdout).into_owned();
    if truncated {
        diff.push_str("\n[diff truncated by Context Bridge]\n");
    }
    Ok((diff, truncated))
}

async fn read_limited<R>(mut reader: R, limit: usize) -> std::io::Result<(Vec<u8>, bool)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut captured = Vec::with_capacity(limit.min(16 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(captured.len());
        let accepted = read.min(remaining);
        captured.extend_from_slice(&buffer[..accepted]);
        truncated |= accepted < read;
    }
    Ok((captured, truncated))
}

fn filter_status(status: &str, policy: &PathPolicy) -> String {
    status
        .lines()
        .filter(|line| {
            if line.starts_with("##") {
                return true;
            }
            status_paths(line).is_some_and(|paths| {
                paths
                    .iter()
                    .all(|path| !policy.is_excluded(Path::new(path)))
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn filter_diff(diff: &str, policy: &PathPolicy) -> String {
    let mut output = Vec::new();
    // A malformed diff header is treated as excluded. It is safer to omit an
    // unparseable patch than to accidentally persist a secret path or its
    // contents.
    let mut include = false;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            include = diff_header_paths(line).is_some_and(|(old_path, new_path)| {
                !policy.is_excluded(Path::new(&old_path))
                    && !policy.is_excluded(Path::new(&new_path))
            });
        }
        if include {
            output.push(line);
        }
    }
    output.join("\n")
}

/// Extract the old and new paths from a `diff --git` header. Git quotes paths
/// containing special characters, so this understands both quoted and plain
/// path tokens. The caller fails closed if parsing cannot establish both sides.
fn diff_header_paths(line: &str) -> Option<(String, String)> {
    let remainder = line.strip_prefix("diff --git ")?;
    let (old_path, remainder) = parse_git_path_token(remainder)?;
    let remainder = remainder.strip_prefix(' ')?;
    let (new_path, remainder) = parse_git_path_token(remainder)?;
    if !remainder.is_empty() {
        return None;
    }

    Some((
        old_path.strip_prefix("a/")?.to_owned(),
        new_path.strip_prefix("b/")?.to_owned(),
    ))
}

/// Extract all paths referenced by one porcelain-v1 status entry. Renames and
/// copies name both their old and new sides, and either side must be safe.
fn status_paths(line: &str) -> Option<Vec<String>> {
    let paths = line.get(3..)?;
    match split_status_rename(paths) {
        Some((old_path, new_path)) => Some(vec![
            parse_single_git_path(old_path)?,
            parse_single_git_path(new_path)?,
        ]),
        None => Some(vec![parse_single_git_path(paths)?]),
    }
}

fn split_status_rename(paths: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in paths.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if !quoted && paths[index..].starts_with(" -> ") => {
                return Some((&paths[..index], &paths[index + " -> ".len()..]));
            }
            _ => {}
        }
    }
    None
}

fn parse_single_git_path(value: &str) -> Option<String> {
    let (path, remainder) = parse_git_path_token(value.trim())?;
    remainder.is_empty().then_some(path)
}

fn parse_git_path_token(value: &str) -> Option<(String, &str)> {
    if let Some(value) = value.strip_prefix('"') {
        return parse_quoted_git_path(value);
    }

    let end = value.find(char::is_whitespace).unwrap_or(value.len());
    (end > 0).then(|| (value[..end].to_owned(), &value[end..]))
}

fn parse_quoted_git_path(value: &str) -> Option<(String, &str)> {
    let mut output = String::new();
    let mut characters = value.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => return Some((output, &value[index + character.len_utf8()..])),
            '\\' => {
                let (_, escaped) = characters.next()?;
                match escaped {
                    '\\' | '"' => output.push(escaped),
                    'n' => output.push('\n'),
                    'r' => output.push('\r'),
                    't' => output.push('\t'),
                    // Git may use octal escapes for non-UTF-8 paths. Those
                    // cannot be represented losslessly here, so fail closed.
                    '0'..='7' => return None,
                    _ => return None,
                }
            }
            _ => output.push(character),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use cb_security::PathPolicy;
    use tokio::io::AsyncWriteExt;

    use super::{filter_diff, filter_status, read_limited};

    fn policy() -> PathPolicy {
        PathPolicy::new(&[]).expect("default policy is valid")
    }

    #[test]
    fn excludes_diff_when_either_header_path_is_secret() {
        let diff = concat!(
            "diff --git a/.env b/safe.txt\n",
            "similarity index 100%\n",
            "rename from .env\n",
            "rename to safe.txt\n",
            "diff --git a/safe.txt b/.env.local\n",
            "similarity index 100%\n",
            "rename from safe.txt\n",
            "rename to .env.local\n",
            "diff --git a/src/lib.rs b/src/main.rs\n",
            "index 1111111..2222222 100644\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/main.rs\n",
            "+safe change\n",
        );

        let filtered = filter_diff(diff, &policy());

        assert!(!filtered.contains("rename from .env"));
        assert!(!filtered.contains("rename to .env.local"));
        assert!(filtered.contains("diff --git a/src/lib.rs b/src/main.rs"));
        assert!(filtered.contains("+safe change"));
    }

    #[test]
    fn excludes_quoted_secret_path_in_diff_header() {
        let diff = concat!(
            "diff --git \"a/.env.production\" \"b/safe file.txt\"\n",
            "similarity index 100%\n",
            "rename from .env.production\n",
            "rename to safe file.txt\n",
        );

        assert!(filter_diff(diff, &policy()).is_empty());
    }

    #[test]
    fn excludes_status_when_either_rename_side_is_secret() {
        let status = concat!(
            "## main\n",
            "R  .env -> safe.txt\n",
            "R  safe.txt -> .env.local\n",
            " M src/lib.rs\n",
            "?? \"safe file.txt\"\n",
        );

        assert_eq!(
            filter_status(status, &policy()),
            "## main\n M src/lib.rs\n?? \"safe file.txt\""
        );
    }

    #[tokio::test]
    async fn bounded_reader_drains_overflow_without_retaining_it() {
        let (mut writer, reader) = tokio::io::duplex(8);
        let producer = tokio::spawn(async move {
            writer
                .write_all(b"0123456789abcdefghijklmnopqrstuvwxyz")
                .await
                .expect("write test stream");
        });

        let (captured, truncated) = read_limited(reader, 10).await.expect("read stream");
        producer.await.expect("join producer");
        assert_eq!(captured, b"0123456789");
        assert!(truncated);
    }
}
