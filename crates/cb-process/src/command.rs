use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub current_dir: PathBuf,
}

impl ProcessSpec {
    #[must_use]
    pub fn new(executable: PathBuf, current_dir: PathBuf) -> Self {
        Self {
            executable,
            args: Vec::new(),
            environment: BTreeMap::new(),
            current_dir,
        }
    }
}

pub fn resolve_executable(value: &Path) -> Option<PathBuf> {
    if value.components().count() > 1 {
        return value.is_file().then(|| value.to_path_buf());
    }
    let search_path = std::env::var_os("PATH")?;
    std::env::split_paths(&search_path)
        .map(|directory| directory.join(value))
        .find(|candidate| candidate.is_file() && is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
