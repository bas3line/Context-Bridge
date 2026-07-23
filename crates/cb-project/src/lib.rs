//! Project identity, filesystem snapshots, and non-destructive Git inspection.

mod diff;
mod filesystem;
mod git;
mod identity;
mod root;
mod snapshot;

pub use diff::*;
pub use filesystem::*;
pub use git::*;
pub use identity::*;
pub use root::*;
pub use snapshot::*;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("could not canonicalize project path `{path}`")]
    Canonicalize {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not execute `{command}`")]
    Git {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{command}` failed: {stderr}")]
    GitCommand { command: String, stderr: String },
    #[error("could not walk project files")]
    Walk(#[source] ignore::Error),
    #[error("could not inspect metadata for `{path}`")]
    Metadata {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read project file `{path}`")]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("path `{path}` is outside project root `{root}`")]
    StripPrefix {
        path: std::path::PathBuf,
        root: std::path::PathBuf,
        #[source]
        source: std::path::StripPrefixError,
    },
    #[error("could not compile project path exclusions")]
    Security(#[source] cb_security::SecurityError),
}
