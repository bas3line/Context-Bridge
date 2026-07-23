use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactReference {
    pub content_hash: String,
    pub path: Option<PathBuf>,
    pub media_type: Option<String>,
    pub byte_len: u64,
}
