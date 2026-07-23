use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::DEFAULT_EXCLUDED_PATHS;

#[derive(Debug, Clone)]
pub struct PathPolicy {
    patterns: GlobSet,
    excluded_roots: Vec<PathBuf>,
}

impl PathPolicy {
    pub fn new(additional: &[String]) -> Result<Self, SecurityError> {
        let mut builder = GlobSetBuilder::new();
        for pattern in DEFAULT_EXCLUDED_PATHS
            .iter()
            .copied()
            .chain(additional.iter().map(String::as_str))
        {
            builder.add(
                Glob::new(pattern).map_err(|source| SecurityError::InvalidGlob {
                    pattern: pattern.to_owned(),
                    source,
                })?,
            );
        }
        Ok(Self {
            patterns: builder.build().map_err(SecurityError::BuildGlobSet)?,
            excluded_roots: Vec::new(),
        })
    }

    #[must_use]
    pub fn with_excluded_root(mut self, path: PathBuf) -> Self {
        self.excluded_roots.push(normalize(&path));
        self
    }

    #[must_use]
    pub fn is_excluded(&self, path: &Path) -> bool {
        let normalized = normalize(path);
        self.patterns.is_match(&normalized)
            || self
                .excluded_roots
                .iter()
                .any(|root| normalized.starts_with(root))
            || normalized
                .components()
                .any(|component| component.as_os_str() == ".ssh")
    }
}

fn normalize(path: &Path) -> PathBuf {
    path.components().collect()
}

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("invalid excluded-path glob `{pattern}`")]
    InvalidGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    #[error("could not build excluded-path matcher")]
    BuildGlobSet(#[source] globset::Error),
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::PathPolicy;

    #[test]
    fn excludes_secret_paths() {
        let policy = PathPolicy::new(&[]).expect("default patterns compile");
        assert!(policy.is_excluded(Path::new(".env")));
        assert!(policy.is_excluded(Path::new("nested/secrets/token.txt")));
        assert!(policy.is_excluded(Path::new("node_modules/react/index.js")));
        assert!(policy.is_excluded(Path::new("apps/web/node_modules")));
        assert!(policy.is_excluded(Path::new(".opencode/session.json")));
        assert!(!policy.is_excluded(Path::new("src/lib.rs")));
    }
}
