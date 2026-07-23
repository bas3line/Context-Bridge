use cb_core::ProjectId;

use crate::ResolvedProject;

#[must_use]
pub fn project_id(project: &ResolvedProject) -> ProjectId {
    ProjectId::from_canonical_path(&project.root)
}
