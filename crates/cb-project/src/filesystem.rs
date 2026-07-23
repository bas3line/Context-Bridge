use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::ProjectError;
use cb_security::PathPolicy;

const HASH_LIMIT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    pub size: u64,
    pub modified_unix_nanos: u128,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemSnapshot {
    pub files: BTreeMap<PathBuf, FileMetadata>,
}

impl FilesystemSnapshot {
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        for (path, metadata) in &self.files {
            hasher.update(path.to_string_lossy().as_bytes());
            hasher.update(&metadata.size.to_le_bytes());
            hasher.update(&metadata.modified_unix_nanos.to_le_bytes());
            if let Some(content_hash) = &metadata.content_hash {
                hasher.update(content_hash.as_bytes());
            }
        }
        hasher.finalize().to_hex().to_string()
    }
}

pub fn capture_filesystem_snapshot(root: &Path) -> Result<FilesystemSnapshot, ProjectError> {
    let policy = PathPolicy::new(&[]).map_err(ProjectError::Security)?;
    capture_filesystem_snapshot_with_policy(root, &policy)
}

pub fn capture_filesystem_snapshot_with_policy(
    root: &Path,
    policy: &PathPolicy,
) -> Result<FilesystemSnapshot, ProjectError> {
    let mut files = BTreeMap::new();
    let filter_root = root.to_path_buf();
    let filter_policy = policy.clone();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(move |entry| {
            let name = entry.file_name().to_string_lossy();
            let relative = entry
                .path()
                .strip_prefix(&filter_root)
                .unwrap_or_else(|_| entry.path());
            name != ".git"
                && name != "target"
                && name != ".context-bridge"
                && !filter_policy.is_excluded(relative)
        })
        .build();

    for entry in walker {
        let entry = entry.map_err(ProjectError::Walk)?;
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let absolute = entry.path();
        let relative = absolute
            .strip_prefix(root)
            .map_err(|source| ProjectError::StripPrefix {
                path: absolute.to_path_buf(),
                root: root.to_path_buf(),
                source,
            })?
            .to_path_buf();
        let metadata = entry.metadata().map_err(|source| ProjectError::Metadata {
            path: absolute.to_path_buf(),
            source: source
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("metadata error")),
        })?;
        let modified_unix_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        let content_hash = if metadata.len() <= HASH_LIMIT_BYTES {
            Some(hash_file(absolute)?)
        } else {
            None
        };
        files.insert(
            relative,
            FileMetadata {
                size: metadata.len(),
                modified_unix_nanos,
                content_hash,
            },
        );
    }
    Ok(FilesystemSnapshot { files })
}

fn hash_file(path: &Path) -> Result<String, ProjectError> {
    let mut file = File::open(path).map_err(|source| ProjectError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| ProjectError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}
